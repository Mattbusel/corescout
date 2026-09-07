//! Everything CoreScout knows, and every way of changing it.
//!
//! # One object, one lock
//!
//! The service holds a single [`Engine`] behind one mutex. The observation
//! thread, the HTTP server, and the MCP bridge all go through it. That is a
//! deliberately unfashionable design and it is the right one here: the whole
//! product is a small amount of state updated a few times a second, and a
//! single lock makes "what did CoreScout know when it decided that" a question
//! with one answer rather than a race.
//!
//! # The engine does not own the hardware
//!
//! The reflector lives in the observation thread and hands finished
//! reflections in. So the engine has no sensors, opens no devices, and can be
//! constructed and driven entirely from recorded frames, which is exactly what
//! the tests do and what the demo does.
//!
//! # What persists and what does not
//!
//! Learned state is written to the document store as it changes. The
//! observation-rate history goes to the ring. Nothing else survives a restart,
//! and the parts that do not are the parts that should not: an in-flight
//! session, a rate-limit window, a lookback buffer.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use corescout_agent_experience::{Basis, Experience, Pattern};
use corescout_agent_observation::{Action, Agent, AgentKind, Ingest, Raw, Session};
use corescout_capability_runtime::capability::Provenance;
use corescout_capability_runtime::{Capability, Context, Operation, Outcome, Runtime};
use corescout_core::error::{Error, Result};
use corescout_mirror::MirrorSnapshot;
use corescout_permissions::{Autonomy, Grant, Permissions, Risk};
use corescout_represent::{features, LatentCatalogue, Normalizer};
use corescout_storage::docs::now_ms;
use corescout_storage::events::{Event, EventKind, Severity};
use corescout_storage::ring::{Sample, FEATURES};
use corescout_storage::{Kind, Ring, Store};
use corescout_substrate::Topology;

use crate::view;

/// The build, reported to clients and stamped on stored state.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What CoreScout says when it is deliberately not offering its own suggestion.
///
/// Worded so an agent reads it as an instruction rather than as an absence,
/// and so a person reading a transcript can see the experiment happening.
const WITHHELD: &str = "CoreScout is checking whether its own suggestion actually helps, so it \
                        is not offering it this time. Do the operation as you normally would, \
                        and tell CoreScout how it went.";

/// What it says about something it has noticed and not yet tested.
const UNTESTED: &str = "CoreScout noticed this and has not tested it yet. Try it and report the \
                        outcome, which is what turns it into something it can be sure about.";

/// Rows used to fit the normaliser before recognition starts.
///
/// Fitting on everything and then recognising over the same data lets the
/// future leak into the scaling. Fitting on a prefix costs a short warm-up and
/// keeps the recognition honest.
const FIT_ROWS: usize = 120;

/// Novelty distance at which a reflection becomes a new state.
const NOVELTY: f64 = 0.75;
/// The most states the catalogue may hold.
const MAX_STATES: usize = 32;

/// How long an agent may be silent before it counts as disconnected.
const AGENT_TIMEOUT_MS: u64 = 5 * 60 * 1000;

/// Samples kept in memory for the live view.
const TRAIL: usize = 120;

/// Everything CoreScout knows.
pub struct Engine {
    store: Store,
    ring: Ring,
    permissions: Permissions,
    experience: Experience,
    ingest: Ingest,
    capabilities: BTreeMap<String, Capability>,
    agents: BTreeMap<String, Agent>,
    sessions: BTreeMap<String, Session>,
    latent: LatentCatalogue,
    normalizer: Option<Normalizer>,
    fitting: Vec<Vec<f64>>,
    previous: Option<MirrorSnapshot>,
    latest: Option<MirrorSnapshot>,
    topology: Option<Topology>,
    self_description: Vec<String>,
    started: Instant,
    observations: u64,
    observe_total_ns: u128,
    observe_worst_ns: u64,
    interval_ms: u64,
    observing: bool,
    trail: Vec<Sample>,
    /// Trials handed out and not yet settled, by session and operation.
    ///
    /// Not persisted: a trial whose outcome nobody reported is a trial that
    /// never happened, and carrying one across a restart would let a stale
    /// assignment collect an outcome from a run it had nothing to do with.
    pending: BTreeMap<(String, String), Pending>,
    seed: u64,
    /// Set when learned state has changed and has not yet been written.
    dirty: bool,
    /// What that resolves to right now. Recomputed when anything changes
    /// rather than on every call, because it is read on every observation.
    /// What the Microsoft Store last said about this copy.
    ///
    /// A cache of an answer, not a decision: the Store enforces the licence,
    /// and this is only so the interface can say something true about it.
    licence: Option<corescout_store_commerce::Licence>,
}

/// A trial handed out and waiting for its outcome.
#[derive(Clone, Debug)]
struct Pending {
    procedure: String,
    applied: bool,
    /// Whether the coin decided rather than belief.
    ///
    /// Never sent to the agent. An agent that knew it was in the randomised
    /// arm might behave differently, and then the arm would not be measuring
    /// what it claims to be measuring.
    randomised: bool,
    at_ms: u64,
}

/// What CoreScout suggests before an operation.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Advice {
    /// The operation this is about, normalised.
    pub operation: String,
    /// Whether CoreScout has anything to suggest at all.
    pub has_advice: bool,
    /// What to do first, if anything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<String>,
    /// Why, in one sentence a person or a model can read.
    pub because: String,
    /// Whether this rests on randomised evidence or only on observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
    /// What is known about how this operation tends to go here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<String>,
    /// Faults belonging to the machine, which no choice of arguments avoids.
    ///
    /// Separate from `history` because they are not about this operation. They
    /// are reported for every operation precisely because the operation is not
    /// what is wrong.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub machine: Vec<corescout_agent_experience::Fault>,
}

impl Engine {
    /// Open the engine over a store and a ring, restoring what was learned.
    pub fn open(store: Store, ring: Ring) -> Result<Engine> {
        let permissions = store
            .load::<corescout_permissions::ruling::Stored>(Kind::Setting, "permissions")?
            .map(Permissions::from_stored)
            .unwrap_or_else(Permissions::new);
        let experience = store
            .load::<Experience>(Kind::Setting, "experience")?
            .unwrap_or_else(Experience::new);
        let latent = store
            .load::<LatentCatalogue>(Kind::Setting, "latent")?
            .unwrap_or_else(|| LatentCatalogue::new(NOVELTY, MAX_STATES));
        let normalizer = store.load::<Normalizer>(Kind::Setting, "normalizer")?;
        let capabilities: BTreeMap<String, Capability> = store
            .list_as::<Capability>(Kind::Capability)?
            .into_iter()
            .map(|capability| (capability.id.clone(), capability))
            .collect();
        let agents: BTreeMap<String, Agent> = store
            .list_as::<Agent>(Kind::Agent)?
            .into_iter()
            .map(|mut agent| {
                // Nothing is connected at startup, whatever was true when the
                // service last stopped. Restoring `connected` from disk would
                // have the Home screen claim an agent is attached to a process
                // that no longer exists.
                agent.connected = false;
                (agent.id.clone(), agent)
            })
            .collect();

        // The clock is read once here and never trusted to go backwards. A
        // fresh installation starts its trial at this moment.

        let seed = store
            .meta("seed")?
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| {
                let seed = now_ms().wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
                let _ = store.set_meta("seed", &seed.to_string());
                seed
            });

        Ok(Engine {
            store,
            ring,
            permissions,
            experience,
            ingest: Ingest::new(),
            capabilities,
            agents,
            sessions: BTreeMap::new(),
            latent,
            normalizer,
            fitting: Vec::new(),
            previous: None,
            latest: None,
            topology: None,
            self_description: Vec::new(),
            started: Instant::now(),
            observations: 0,
            observe_total_ns: 0,
            observe_worst_ns: 0,
            interval_ms: 250,
            observing: false,
            trail: Vec::new(),
            pending: BTreeMap::new(),
            licence: corescout_store_commerce::licence().ok(),
            seed,
            dirty: false,
        })
    }

    /// Open at the standard location for this user.
    pub fn open_default() -> Result<Engine> {
        corescout_storage::paths::ensure_data_dir()
            .map_err(|source| Error::io(corescout_storage::paths::data_dir(), source))?;
        let store = Store::open_default()?;
        let ring = Ring::open(
            &corescout_storage::paths::ring(),
            corescout_storage::ring::DEFAULT_CAPACITY,
        )?;
        Engine::open(store, ring)
    }

    /// The event log and document store.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The permission state.
    pub fn permissions(&self) -> &Permissions {
        &self.permissions
    }

    /// What the Store said about this copy, if it has answered.
    ///
    /// `None` outside a Store installation, which is every development build.
    /// Nothing in CoreScout behaves differently either way: the Store enforces
    /// the licence by not letting an unlicensed copy run, and this exists so
    /// Settings can say "you own CoreScout" rather than nothing.
    pub fn licence(&self) -> Option<&corescout_store_commerce::Licence> {
        self.licence.as_ref()
    }

    /// Ask the Store again.
    ///
    /// # Why failure is silent
    ///
    /// Outside a Store installation this cannot work and is not meant to. It
    /// can also fail for ordinary reasons inside one: no network, nobody
    /// signed in. None of those are worth telling a user about, and none of
    /// them change what CoreScout does, because CoreScout does not gate
    /// anything on the answer.
    ///
    /// The last good answer is kept rather than replaced with nothing, so a
    /// dropped network does not make Settings forget that somebody owns this.
    ///
    /// Returns whether the answer changed.
    pub fn recheck_licence(&mut self) -> bool {
        match corescout_store_commerce::licence() {
            Ok(fresh) => {
                let changed = self.licence.as_ref() != Some(&fresh);
                if changed {
                    let _ = self.store.append(Event::new(
                        EventKind::Permission,
                        Severity::Notice,
                        fresh.headline(),
                    ));
                }
                self.licence = Some(fresh);
                changed
            }
            Err(corescout_store_commerce::Unavailable::Unreachable(why)) => {
                // Worth recording: if this happens to a paying customer it is
                // the explanation for a support conversation that would
                // otherwise be a mystery. Not worth interrupting anyone for.
                let _ = self.store.append(Event::new(
                    EventKind::Fault,
                    Severity::Info,
                    format!("could not ask the Microsoft Store about this copy: {why}"),
                ));
                false
            }
            Err(_) => false,
        }
    }

    /// The permission state, to change it.
    pub fn permissions_mut(&mut self) -> &mut Permissions {
        &mut self.permissions
    }

    /// Operations that recur and go wrong here.
    pub fn failures(&self) -> Vec<corescout_agent_experience::FailureMode> {
        self.experience
            .failure_modes()
            .into_iter()
            .cloned()
            .collect()
    }

    /// Operations that have gone wrong here, whether or not often enough to
    /// be called a pattern.
    ///
    /// The threshold exists so CoreScout does not present one bad afternoon as
    /// a tendency, and that is right for what it *claims*. It is wrong as a
    /// reason to stay silent: an agent asking whether something has failed here
    /// is better served by "once, and here is what it was" than by "no", which
    /// is what it used to get. Callers must say which they are showing.
    pub fn failures_below_threshold(&self) -> Vec<corescout_agent_experience::FailureMode> {
        let recurring: std::collections::BTreeSet<String> = self
            .experience
            .failure_modes()
            .into_iter()
            .map(|mode| mode.key.clone())
            .collect();
        let mut out: Vec<_> = self
            .experience
            .operations()
            .filter(|mode| mode.failures > 0 && !recurring.contains(&mode.key))
            .cloned()
            .collect();
        out.sort_by(|a, b| b.failures.cmp(&a.failures).then(a.key.cmp(&b.key)));
        out
    }

    /// Faults this machine has that no operation can work around.
    ///
    /// Re-checked against the filesystem on the way out rather than trusted
    /// from storage, so a drive that came back stops being warned about the
    /// moment it does. That check is the evidence these rest on, in place of
    /// the repetition everything else here needs.
    pub fn environment_faults(&mut self) -> Vec<corescout_agent_experience::Fault> {
        let resolved = self
            .experience
            .environment
            .resolve(|path| std::path::Path::new(path).exists());
        for fault in &resolved {
            self.dirty = true;
            self.note(Event::new(
                EventKind::AgentActivity,
                Severity::Info,
                format!("{} is back; that failure should stop happening", fault.path),
            ));
        }
        self.experience.environment.all().cloned().collect()
    }

    /// Read a failed action for a machine fault, and remember any that is real.
    ///
    /// Called on the observe path. The filesystem check happens here, once, at
    /// the moment of the report: it is the difference between a claim about the
    /// machine and a guess copied out of an error message.
    fn learn_environment(&mut self, action: &corescout_agent_observation::Action) {
        if !action.visibly_failed() {
            return;
        }
        let Some(detail) = action.reported.detail() else {
            return;
        };
        let now = now_ms();
        for blamed in corescout_agent_experience::environment::candidates(detail) {
            if std::path::Path::new(&blamed.path).exists() {
                continue;
            }
            let fault = self
                .experience
                .environment
                .record(&blamed, &action.fingerprint, now)
                .clone();
            if fault.seen == 1 {
                self.note(Event::new(
                    EventKind::Discovery,
                    Severity::Warning,
                    format!(
                        "your computer is missing {}, and it is being used",
                        fault.path
                    ),
                ));
            }
        }
    }

    /// What CoreScout is currently testing and cannot yet answer.
    ///
    /// Shown as its own list rather than folded into the learned cards. A
    /// product that only displays what it has concluded looks more certain
    /// than it is; the open questions are the honest half of the same picture.
    pub fn open_questions(&self) -> Vec<view::Question> {
        self.experience
            .procedures()
            // Only the ones still open. A settled procedure listed under
            // "still uncertain", with "nothing: this is settled" beside it,
            // is a screen contradicting itself.
            .filter(|procedure| !procedure.is_established())
            // And not one that already produced a capability. Evidence ages
            // out, so a procedure that was established when its capability was
            // created can fall back below the bar later. That is the belief
            // machinery working, but the question "does this help" was already
            // answered: what is happening now is re-verification, which
            // belongs on the capability's own card rather than in a list of
            // things CoreScout does not know.
            .filter(|procedure| {
                !self
                    .capabilities
                    .contains_key(&format!("cap-{}", procedure.id))
            })
            .map(|procedure| view::Question {
                id: procedure.id.clone(),
                name: procedure.name.clone(),
                about: procedure.target.clone(),
                settled: procedure.is_established(),
                missing: procedure
                    .missing()
                    .map(|missing| missing.describe())
                    .unwrap_or_else(|| "nothing: this is settled".into()),
                randomised_trials: procedure.estimate.treatment.randomised
                    + procedure.estimate.control.randomised,
                observed_trials: procedure.estimate.treatment.trials
                    + procedure.estimate.control.trials,
            })
            .collect()
    }

    /// Tell the engine what machine it is on.
    pub fn set_topology(&mut self, topology: Topology) {
        self.topology = Some(topology);
    }

    /// Tell the engine what the machine says about itself.
    pub fn set_self_description(&mut self, lines: Vec<String>) {
        self.self_description = lines;
    }

    /// Ask the machine to describe itself from what has been measured.
    ///
    /// Every proposition it produces is guarded by something actually
    /// observed, so a fresh install describes itself in one line and a machine
    /// that has been running for a day describes itself in several. The
    /// interface shows these verbatim; nothing rewrites them into something
    /// more confident.
    pub fn refresh_self_description(&mut self) {
        let description = corescout_identity::describe(
            self.latest.as_ref(),
            Some(&self.latent),
            &[],
            &corescout_identity::description::Evidence {
                frames: self.observations as usize,
                scored_predictions: 0,
                model_skill: None,
                actions_taken: self
                    .capabilities
                    .values()
                    .map(|capability| capability.uses)
                    .sum(),
                responsive_entities: 0,
                largest_variable_group: None,
            },
        );
        self.self_description = description
            .by_evidence()
            .into_iter()
            .map(|proposition| proposition.prose.clone())
            .collect();
    }

    /// Set how often the mirror is sampled.
    pub fn set_interval(&mut self, interval: Duration) {
        self.interval_ms = interval.as_millis().max(1) as u64;
    }

    /// Note that observation has started or stopped.
    pub fn set_observing(&mut self, observing: bool) {
        self.observing = observing;
    }

    /// Record a reflection.
    ///
    /// The one path from the mirror into everything else. It turns cumulative
    /// counters into rates, fits the scaling on a prefix, recognises the state,
    /// and writes a fixed-width sample to the ring.
    pub fn record_reflection(&mut self, snapshot: &MirrorSnapshot) {
        self.observations += 1;
        let cost = snapshot.observation_cost_ns();
        self.observe_total_ns += cost as u128;
        self.observe_worst_ns = self.observe_worst_ns.max(cost);

        let row = self
            .previous
            .as_ref()
            .and_then(|previous| features::row_from(previous, snapshot));
        self.previous = Some(snapshot.clone());

        let mut latent_state = None;
        if let Some(row) = &row {
            let width = row.len();
            match &self.normalizer {
                None => {
                    self.fitting.push(row.clone());
                    if self.fitting.len() >= FIT_ROWS {
                        let fitted = Normalizer::fit(&self.fitting, width);
                        self.fitting.clear();
                        self.normalizer = Some(fitted);
                        self.note(Event::new(
                            EventKind::Observation,
                            Severity::Debug,
                            "CoreScout has enough of a baseline to start recognising states",
                        ));
                        self.dirty = true;
                    }
                }
                Some(normalizer) => {
                    let (point, _) = normalizer.apply_filled(row, width);
                    let (id, novel) = self.latent.observe(&point, snapshot.monotonic_ns);
                    latent_state = Some(id.0);
                    if novel {
                        self.dirty = true;
                        self.note(
                            Event::new(
                                EventKind::Discovery,
                                Severity::Debug,
                                format!(
                                    "your computer noticed a way of running it had not seen before \
                                     ({} in total)",
                                    self.latent.len()
                                ),
                            )
                            .about(format!("state:{}", id.0)),
                        );
                    }
                }
            }
        }

        let sample = Sample {
            monotonic_ns: snapshot.monotonic_ns,
            wall_ms: now_ms(),
            latent_state,
            observe_ns: cost.min(u32::MAX as u64) as u32,
            entities: snapshot.entities.len() as u32,
            observed_cells: observed_cells(snapshot) as u32,
            features: aggregate(snapshot),
        };
        let _ = self.ring.push(&sample);
        self.trail.push(sample);
        if self.trail.len() > TRAIL {
            self.trail.remove(0);
        }
        self.latest = Some(snapshot.clone());
    }

    /// Write anything that has changed since the last time.
    ///
    /// Called on a timer rather than on every reflection: a transaction per
    /// observation would be the one part of this product with a measurable
    /// cost.
    pub fn persist(&mut self) -> Result<()> {
        self.store
            .put(Kind::Setting, "permissions", &self.permissions.to_stored())?;
        if !self.dirty {
            return Ok(());
        }
        self.store
            .put(Kind::Setting, "experience", &self.experience)?;
        self.store.put(Kind::Setting, "latent", &self.latent)?;
        if let Some(normalizer) = &self.normalizer {
            self.store.put(Kind::Setting, "normalizer", normalizer)?;
        }
        for capability in self.capabilities.values() {
            self.store
                .put(Kind::Capability, &capability.id, capability)?;
        }
        for agent in self.agents.values() {
            self.store.put(Kind::Agent, &agent.id, agent)?;
        }
        self.dirty = false;
        Ok(())
    }

    /// Append to the log, ignoring a storage failure.
    ///
    /// A log write that fails must not take down the observation loop. The
    /// failure is visible in Diagnostics as a stalled event count.
    fn note(&self, event: Event) {
        let _ = self.store.append(event);
    }

    // ---------------------------------------------------------------- agents

    /// An agent has identified itself.
    pub fn agent_hello(&mut self, name: &str, version: Option<String>) -> String {
        let kind = AgentKind::recognise(name);
        let id = kind.slug();
        let now = now_ms();
        let entry = self
            .agents
            .entry(id.clone())
            .or_insert_with(|| Agent::new(name, version.clone(), now));
        entry.touch(now);
        if version.is_some() {
            entry.version = version;
        }
        entry.sessions += 1;
        self.dirty = true;
        self.note(
            Event::new(
                EventKind::Connection,
                Severity::Info,
                format!("{} connected", kind.title()),
            )
            .about(id.clone()),
        );
        id
    }

    /// An agent reported something it did.
    pub fn agent_observe(&mut self, raw: &Raw) -> Action {
        let now = now_ms();
        let raw = &Raw {
            workspace: raw.workspace.as_deref().map(workspace_id),
            ..raw.clone()
        };
        let mut action = self.ingest.observe(raw, now);
        action.machine_state = self.current_state();

        // Resolved before the entry is taken: `connected_id` reads `self`, and
        // an `or_insert_with` closure holds a mutable borrow of it.
        let attributed = self.connected_id();
        let session = self
            .sessions
            .entry(action.session.clone())
            .or_insert_with(|| Session::new(action.session.clone(), attributed, now));
        session.actions += 1;
        session.last_ms = now;
        if action.visibly_failed() {
            session.failures += 1;
        }
        if action.retry_of.is_some() {
            session.retries += 1;
        }
        if action.workspace.is_some() {
            session.workspace = action.workspace.clone();
        }
        let agent_id = session.agent.clone();
        if let Some(agent) = self.agents.get_mut(&agent_id) {
            agent.actions += 1;
            agent.touch(now);
        }

        self.experience.observe(&action);
        self.learn_environment(&action);
        self.dirty = true;

        // Settle any trial this action was the outcome of. Without this the
        // procedures accumulate no randomised evidence at all, and no
        // capability could ever be created from real use.
        if let Some(trial) = self
            .pending
            .remove(&(action.session.clone(), action.fingerprint.clone()))
        {
            let failed = action.visibly_failed();
            if let Some(procedure) = self.experience.procedure_mut(&trial.procedure) {
                procedure.record(trial.applied, failed, trial.randomised, now);
            }
        }

        if action.is_silent_failure() {
            self.note(
                Event::new(
                    EventKind::AgentActivity,
                    Severity::Warning,
                    format!("{} said it worked and it had not", action.name),
                )
                .about(action.id.clone())
                .with(&action),
            );
        }

        // A fresh association is worth turning into something testable. Doing
        // it here rather than on a timer means the experiment starts the
        // moment the evidence exists.
        for id in self.experience.propose(self.seed) {
            self.note(
                Event::new(
                    EventKind::Discovery,
                    Severity::Info,
                    "your computer noticed something and started testing whether it is real",
                )
                .about(id),
            );
        }
        self.promote();
        let _ = self.store.put(Kind::Action, &action.id, &action);
        action
    }

    /// What to do before an operation, and the experiment that goes with it.
    ///
    /// This is where randomisation actually happens. An agent asks what to do;
    /// on most occasions CoreScout answers from what it believes, and on a
    /// small fraction it answers from a coin. Only the second kind of trial
    /// can tell a cause from a coincidence, and this is the only place in the
    /// product that produces one.
    ///
    /// The cost is real and worth stating plainly: on a randomised occasion
    /// CoreScout may deliberately withhold a procedure that would have saved
    /// somebody a failed build. That is what the evidence costs.
    pub fn advise(&mut self, session: &str, operation: &str, workspace: Option<&str>) -> Advice {
        let fingerprint = corescout_agent_observation::fingerprint::normalise(operation);
        // Asked for before anything about the operation, because a machine
        // fault outranks everything else here: there is no point suggesting a
        // better way to run something that cannot run at all.
        let machine = self.environment_faults();
        // Identified here too, so a hook that reported a folder and an agent
        // that asks about the same folder are talking about one workspace.
        let identified = workspace.map(workspace_id);
        let workspace = identified.as_deref();
        let history = self
            .experience
            .operation(&fingerprint, workspace)
            .filter(|mode| mode.attempts >= 3)
            .map(|mode| mode.headline());

        let chosen = self
            .experience
            .procedures()
            .find(|procedure| {
                procedure.target == fingerprint && procedure.workspace.as_deref() == workspace
            })
            .map(|procedure| procedure.id.clone());

        let Some(id) = chosen else {
            return Advice {
                operation: fingerprint,
                // A machine fault is advice, and the most actionable kind
                // there is: it names the cause and it is checkable.
                has_advice: !machine.is_empty(),
                steps: machine
                    .iter()
                    .filter_map(|fault| fault.workaround.clone())
                    .collect(),
                because: match (machine.first(), &history) {
                    (Some(fault), _) => fault.headline(),
                    (None, Some(line)) => {
                        format!("{line}. CoreScout has no better way to offer yet.")
                    }
                    (None, None) => "CoreScout knows nothing about this operation yet.".into(),
                },
                // Not a randomised finding and not a correlation. It is a fact
                // about the machine, checked against the machine just now, and
                // saying so is more honest than borrowing either label.
                basis: (!machine.is_empty()).then(|| "checked just now".to_string()),
                history,
                machine,
            };
        };

        // Believe the procedure helps unless the evidence says otherwise. An
        // untested one is worth applying while it is being tested, which is
        // what makes the trial cheap for the person whose machine it is.
        let believed = self
            .experience
            .procedure_mut(&id)
            .and_then(|procedure| procedure.basis())
            .map(|basis| match basis {
                Basis::Causal { delta, .. } => delta < 0.0,
                Basis::Association {
                    rate_with,
                    rate_without,
                    ..
                } => rate_with < rate_without,
            })
            .unwrap_or(true);

        let Some(procedure) = self.experience.procedure_mut(&id) else {
            return Advice {
                operation: fingerprint,
                has_advice: !machine.is_empty(),
                steps: Vec::new(),
                because: match machine.first() {
                    Some(fault) => fault.headline(),
                    None => "CoreScout knows nothing about this operation yet.".into(),
                },
                basis: (!machine.is_empty()).then(|| "checked just now".to_string()),
                history,
                machine,
            };
        };
        let (apply, randomised) = procedure.decide(believed);
        let steps: Vec<String> = procedure
            .steps
            .iter()
            .map(|step| step.action.clone())
            .collect();
        let basis = procedure.basis();

        self.pending.insert(
            (session.to_string(), fingerprint.clone()),
            Pending {
                procedure: id,
                applied: apply,
                randomised,
                at_ms: now_ms(),
            },
        );
        // Bounded: an agent that asks and never reports would otherwise leave
        // one of these behind on every call.
        if self.pending.len() > 512 {
            let floor = now_ms().saturating_sub(60 * 60 * 1000);
            self.pending.retain(|_, trial| trial.at_ms >= floor);
        }

        let label = basis.as_ref().map(|basis| basis.label().to_string());
        let because = match (&basis, apply) {
            (Some(basis), true) => basis.explain(),
            (Some(_), false) => WITHHELD.into(),
            (None, true) => UNTESTED.into(),
            (None, false) => "Do the operation as you normally would.".into(),
        };
        // A machine fault leads, whatever the procedure had to say. Advising a
        // better way to run a command that cannot run is worse than useless:
        // the agent follows the advice, fails anyway, and concludes the advice
        // was wrong.
        let because = match machine.first() {
            Some(fault) => format!("{} {because}", fault.headline()),
            None => because,
        };

        Advice {
            operation: fingerprint,
            has_advice: (apply && !steps.is_empty()) || !machine.is_empty(),
            steps: if apply { steps } else { Vec::new() },
            because,
            basis: label,
            history,
            machine,
        }
    }

    /// An agent's session is over.
    pub fn agent_goodbye(&mut self, session: &str) {
        if let Some(entry) = self.sessions.get_mut(session) {
            entry.ended_ms = Some(now_ms());
        }
        self.experience.close_session(session);
    }

    /// Which agent is currently connected, for attributing a loose session.
    fn connected_id(&self) -> String {
        let now = now_ms();
        self.agents
            .values()
            .filter(|agent| !agent.is_stale(now, AGENT_TIMEOUT_MS))
            .max_by_key(|agent| agent.last_seen_ms)
            .map(|agent| agent.id.clone())
            .unwrap_or_else(|| "mcp".to_string())
    }

    /// Turn procedures with randomised evidence into capabilities.
    ///
    /// A capability created here is not yet usable: it arrives unapproved, and
    /// the user decides. Creating it automatically and enabling it
    /// automatically are different things, and only the first happens without
    /// being asked.
    fn promote(&mut self) {
        for candidate in self.experience.candidates() {
            let id = format!("cap-{}", candidate.procedure.id);
            if self.capabilities.contains_key(&id) {
                continue;
            }
            let steps: Vec<Operation> = candidate
                .procedure
                .steps
                .iter()
                .filter_map(|step| operation_from(&step.action))
                .collect();
            let Some(verification) = steps.last().cloned() else {
                continue;
            };
            let capability = Capability {
                id: id.clone(),
                name: candidate.name.clone(),
                purpose: format!(
                    "{} is more reliable here when the first step runs before it.",
                    candidate.procedure.target
                ),
                workspace: candidate.procedure.workspace.clone(),
                preconditions: Vec::new(),
                steps,
                verification,
                rollback: Vec::new(),
                risk: Risk::Low,
                reversible: true,
                provenance: Provenance::Learned {
                    procedure: candidate.procedure.id.clone(),
                    because: candidate.because.clone(),
                },
                uses: 0,
                successes: 0,
                created_ms: now_ms(),
                last_used_ms: 0,
            };
            // A generated definition is checked like any other. One that does
            // not pass is dropped rather than stored, and the reason is
            // logged, because a capability that cannot run is worse than none.
            match capability.validate() {
                Ok(()) => {
                    self.note(
                        Event::new(
                            EventKind::Discovery,
                            Severity::Notice,
                            format!(
                                "CoreScout found a more reliable way to {}",
                                candidate.procedure.target
                            ),
                        )
                        .about(id.clone())
                        .with(&candidate.because),
                    );
                    self.capabilities.insert(id, capability);
                    self.dirty = true;
                }
                Err(invalid) => self.note(
                    Event::new(
                        EventKind::Fault,
                        Severity::Debug,
                        format!("a learned procedure could not become a capability: {invalid}"),
                    )
                    .about(id),
                ),
            }
        }
    }

    // ------------------------------------------------------------ decisions

    /// Change the autonomy mode.
    pub fn set_autonomy(&mut self, autonomy: Autonomy) -> Result<()> {
        let before = self.permissions.autonomy();
        self.permissions.set_autonomy(autonomy);
        self.note(
            Event::decision(
                format!(
                    "autonomy changed from {} to {}",
                    before.title(),
                    autonomy.title()
                ),
                "settings",
                autonomy.summary(),
            )
            .outcome("in force from now on"),
        );
        Ok(())
    }

    /// Stop everything.
    pub fn pause(&mut self) {
        self.permissions.pause();
        self.note(
            Event::decision("CoreScout paused", "settings", "nothing further is changed")
                .outcome("every change is now refused, including anything already approved"),
        );
    }

    /// Start again.
    pub fn resume(&mut self) {
        self.permissions.resume();
        self.permissions.limiter_mut().forgive();
        self.note(
            Event::decision(
                "CoreScout resumed",
                "settings",
                "changes may be made again, within your limits",
            )
            .outcome("the pause is lifted and the failure count is cleared"),
        );
    }

    /// Approve, disable, or set a capability to be used automatically.
    pub fn decide_capability(&mut self, id: &str, grant: Grant) -> Result<()> {
        if !self.capabilities.contains_key(id) {
            return Err(Error::invalid(format!(
                "there is no capability called {id}"
            )));
        }
        self.permissions.set_grant(id, grant.clone());
        let (summary, expected) = if grant.disabled {
            (
                format!("{id} switched off"),
                "it will not run again until you switch it back on",
            )
        } else if grant.auto_use {
            (
                format!("{id} approved for your AI to use on its own"),
                "a connected AI may run it without asking you each time",
            )
        } else if grant.approved {
            (
                format!("{id} approved"),
                "it may run, and you are asked before each use",
            )
        } else {
            (
                format!("{id} approval withdrawn"),
                "it will not run until you approve it again",
            )
        };
        self.note(Event::decision(summary, id.to_string(), expected).outcome("recorded"));
        Ok(())
    }

    /// Rename a capability.
    pub fn rename_capability(&mut self, id: &str, name: &str) -> Result<()> {
        let capability = self
            .capabilities
            .get_mut(id)
            .ok_or_else(|| Error::invalid(format!("there is no capability called {id}")))?;
        capability.name = name.trim().to_string();
        self.dirty = true;
        Ok(())
    }

    /// Delete a capability.
    pub fn delete_capability(&mut self, id: &str) -> Result<()> {
        self.capabilities
            .remove(id)
            .ok_or_else(|| Error::invalid(format!("there is no capability called {id}")))?;
        self.permissions.forget_grant(id);
        self.store.delete(Kind::Capability, id)?;
        self.note(
            Event::decision(
                format!("{id} deleted"),
                id.to_string(),
                "the capability and the evidence behind it are removed",
            )
            .outcome("gone"),
        );
        Ok(())
    }

    /// Run a capability.
    ///
    /// Every run produces an audited event carrying the action, the target,
    /// the reason, what was expected and what happened.
    pub fn run_capability(&mut self, id: &str, dry_run: bool) -> Result<Outcome> {
        let capability = self
            .capabilities
            .get(id)
            .cloned()
            .ok_or_else(|| Error::invalid(format!("there is no capability called {id}")))?;
        let context = Context {
            root: None,
            reason: capability.purpose.clone(),
        };
        let runtime = if dry_run {
            Runtime::dry()
        } else {
            Runtime::live()
        };
        let now_ns = self.started.elapsed().as_nanos() as u64;
        let outcome = runtime.execute(&capability, &context, &self.permissions, now_ns);

        if outcome.executed() && !dry_run {
            self.permissions
                .limiter_mut()
                .record(now_ns, outcome.succeeded());
            if let Some(entry) = self.capabilities.get_mut(id) {
                entry.uses += 1;
                entry.last_used_ms = now_ms();
                if outcome.succeeded() {
                    entry.successes += 1;
                }
            }
            self.dirty = true;
        }

        self.note(
            Event::action(
                format!("ran {}", capability.name),
                id.to_string(),
                capability.purpose.clone(),
                "the operation completes and verification confirms it".to_string(),
            )
            .outcome(outcome.summary())
            .with(&outcome),
        );
        Ok(outcome)
    }

    // ---------------------------------------------------------------- views

    /// Whether the mirror recognises a state right now.
    pub fn current_state(&self) -> Option<u32> {
        self.latent.current().map(|id| id.0)
    }

    /// The status line.
    pub fn status(&self) -> Status {
        Status {
            running: true,
            version: VERSION.into(),
            uptime_ms: self.started.elapsed().as_millis() as u64,
            learning_for_ms: now_ms().saturating_sub(self.store.created_ms().unwrap_or(0)),
            autonomy: self.permissions.autonomy().as_str().into(),
            paused: self.permissions.is_paused(),
            observing: self.observing,
            connected: self
                .agents
                .values()
                .filter(|agent| agent.connected)
                .map(|agent| agent.kind.title())
                .collect(),
            observations: self.observations,
            states: self.latent.len(),
            actions: self.experience.observed,
        }
    }

    /// The Home screen.
    pub fn home(&self) -> Home {
        let connected: Vec<view::AgentSummary> = self.agent_summaries();
        let today = self.today();
        let cards = self.learned();
        let headline = match connected.iter().find(|agent| agent.connected) {
            Some(agent) => format!("Your computer is learning how to work with {}.", agent.name),
            None if self.experience.observed > 0 => {
                "Your computer is watching how it runs. Connect an AI to learn more.".into()
            }
            None => "Your computer is watching how it runs.".into(),
        };
        Home {
            headline,
            connected,
            today,
            machine: self.mood(),
            latest: cards.first().cloned(),
            empty: cards.is_empty().then(|| view::Empty {
                title: "CoreScout hasn't learned anything yet.".into(),
                body: "That's normal. Use your AI as usual. CoreScout will start noticing what \
                       repeats, and will only tell you about the things that survive its \
                       evidence requirements."
                    .into(),
            }),
        }
    }

    /// Everything learned, as cards.
    pub fn learned(&self) -> Vec<view::LearnedCard> {
        let mut cards: Vec<view::LearnedCard> = Vec::new();

        // A procedure that has become a capability is one thing, not three.
        // Without this the Learned screen shows the capability, the causal
        // finding behind it and the association it grew out of, all saying
        // roughly the same sentence, and a reader counts three discoveries
        // where there was one.
        let mut superseded: std::collections::HashSet<String> = std::collections::HashSet::new();
        for capability in self.capabilities.values() {
            if let Some(procedure) = capability.id.strip_prefix("cap-") {
                superseded.insert(format!("pat-{procedure}"));
                if let Some(suffix) = procedure.strip_prefix("proc-") {
                    superseded.insert(format!("pat-assoc-{suffix}"));
                }
            }
        }

        for capability in self.capabilities.values() {
            let grant = self.permissions.grant(&capability.id);
            let causal = matches!(capability.provenance, Provenance::Learned { .. });
            cards.push(view::LearnedCard {
                id: capability.id.clone(),
                title: capability.name.clone(),
                detail: capability.purpose.clone(),
                basis: if causal { "Verified" } else { "Defined" }.into(),
                causal,
                confidence: capability.reliability().unwrap_or(0.0),
                reliability: capability.reliability(),
                uses: capability.uses,
                last_ms: capability.last_used_ms.max(capability.created_ms),
                usable_by_ai: grant.is_usable() && grant.auto_use,
                is_capability: true,
                approved: grant.approved && !grant.disabled,
            });
        }

        for pattern in self.experience.patterns() {
            if superseded.contains(&pattern.id) {
                continue;
            }
            cards.push(card_from(&pattern));
        }

        cards.sort_by(|a, b| {
            b.causal
                .cmp(&a.causal)
                .then(
                    b.confidence
                        .partial_cmp(&a.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(b.last_ms.cmp(&a.last_ms))
        });
        cards
    }

    /// The evidence behind one card.
    pub fn explain(&self, id: &str) -> Option<view::Explanation> {
        if let Some(capability) = self.capabilities.get(id) {
            let (kind, evidence) = match &capability.provenance {
                Provenance::Learned { because, .. } => ("randomised", because.clone()),
                Provenance::UserDefined => ("you wrote it", "You defined this yourself.".into()),
                Provenance::BuiltIn => ("built in", "This came with CoreScout.".into()),
            };
            return Some(view::Explanation {
                id: id.into(),
                title: capability.name.clone(),
                simple: capability.purpose.clone(),
                evidence,
                kind: kind.into(),
                confidence: capability.reliability().unwrap_or(0.0),
                alternative: Some(format!("doing {} on its own", capability.name)),
                result: capability.reliability().map(|rate| {
                    format!(
                        "Used {} times; verification confirmed {} of them.",
                        capability.uses,
                        (rate * capability.uses as f64).round() as u64
                    )
                }),
                technical: serde_json::to_value(capability).ok(),
            });
        }
        let pattern = self
            .experience
            .patterns()
            .into_iter()
            .find(|pattern| pattern.id == id)?;
        Some(view::Explanation {
            id: id.into(),
            title: pattern.headline.clone(),
            simple: pattern.detail.clone(),
            evidence: pattern.basis.explain(),
            kind: if pattern.basis.is_causal() {
                "randomised".into()
            } else {
                "observed".into()
            },
            confidence: pattern.confidence(),
            alternative: (!pattern.basis.is_causal())
                .then(|| "CoreScout is testing whether this holds up.".to_string()),
            result: None,
            technical: serde_json::to_value(&pattern).ok(),
        })
    }

    /// The Activity screen.
    pub fn activity(&self, limit: usize, technical: bool) -> Result<Vec<view::Moment>> {
        let events = self.store.recent_events(limit.clamp(1, 500))?;
        Ok(events
            .into_iter()
            .filter(|event| technical || event.severity > Severity::Debug)
            .map(|event| view::Moment {
                sequence: event.sequence,
                at_ms: event.at_ms,
                summary: event.summary,
                kind: event.kind.as_str().into(),
                severity: format!("{:?}", event.severity).to_lowercase(),
                subject: event.subject,
                reason: event.reason,
                actual: event.actual,
                detail: technical.then_some(event.detail).flatten(),
            })
            .collect())
    }

    /// The AI screen.
    pub fn agent_summaries(&self) -> Vec<view::AgentSummary> {
        let now = now_ms();
        let usable = self
            .permissions
            .grants()
            .values()
            .filter(|grant| grant.is_usable())
            .count();
        let mut out: Vec<view::AgentSummary> = self
            .agents
            .values()
            .map(|agent| view::AgentSummary {
                id: agent.id.clone(),
                name: agent.kind.title(),
                connected: agent.connected && !agent.is_stale(now, AGENT_TIMEOUT_MS),
                summary: agent.summary(now),
                actions: agent.actions,
                sessions: agent.sessions,
                capabilities: usable,
                last_seen_ms: agent.last_seen_ms,
            })
            .collect();
        out.sort_by(|a, b| {
            b.connected
                .cmp(&a.connected)
                .then(b.last_seen_ms.cmp(&a.last_seen_ms))
        });
        out
    }

    /// The Computer screen.
    pub fn machine(&self) -> view::Machine {
        let snapshot = self.latest.as_ref();
        view::Machine {
            cpu: self
                .topology
                .as_ref()
                .map(|t| t.model_name.clone())
                .unwrap_or_else(|| "unknown".into()),
            cores: self
                .topology
                .as_ref()
                .map(|t| t.physical_count())
                .unwrap_or(0),
            threads: self
                .topology
                .as_ref()
                .map(|t| t.logical_count())
                .unwrap_or(0),
            hybrid: self.topology.as_ref().is_some_and(|t| t.hybrid),
            entities: snapshot.map(|s| s.entities.len()).unwrap_or(0),
            channels: snapshot.map(|s| s.channels.len()).unwrap_or(0),
            observed_cells: snapshot.map(observed_cells).unwrap_or(0),
            observe_ns: snapshot.map(|s| s.observation_cost_ns()).unwrap_or(0),
            self_description: self.self_description.clone(),
        }
    }

    /// The states discovered, most visited first.
    pub fn states(&self) -> Vec<view::StateCard> {
        self.latent
            .ranked()
            .into_iter()
            .map(|state| view::StateCard {
                id: state.id.0,
                plain: describe_state(state.entries, state.mean_dwell_ns()),
                entries: state.entries,
                mean_dwell_ms: state.mean_dwell_ns() / 1e6,
                then: self
                    .latent
                    .most_likely_successor(state.id)
                    .map(|(id, share)| (id.0, share)),
            })
            .collect()
    }

    /// The live mirror view.
    pub fn live(&self) -> view::Live {
        let mood = self.mood();
        view::Live {
            plain: mood.plain,
            state: mood.state,
            seen: mood.seen,
            nodes: self.nodes(),
            trail: self.trail.iter().map(|sample| sample.features[0]).collect(),
            recent_states: self.trail.iter().filter_map(|s| s.latent_state).collect(),
            agent_activity: self
                .sessions
                .values()
                .filter(|session| session.is_open())
                .map(|session| format!("{} active", session.agent))
                .collect(),
        }
    }

    /// The "what my computer knows" page.
    pub fn knowledge(&self) -> view::Knowledge {
        let patterns = self.experience.patterns();
        let mut about_agents: Vec<String> = Vec::new();
        for agent in self.agents.values() {
            about_agents.push(format!(
                "{}: {} actions across {} sessions",
                agent.kind.title(),
                agent.actions,
                agent.sessions
            ));
        }
        let mut about_workspaces: Vec<String> = self
            .experience
            .failure_modes()
            .iter()
            .take(6)
            .map(|mode| mode.headline())
            .collect();
        for mode in self.experience.unverified_operations().iter().take(3) {
            about_workspaces.push(format!(
                "{} has never been checked after it ran ({} times)",
                mode.fingerprint, mode.attempts
            ));
        }
        let mut about_machine = self.self_description.clone();
        if !self.latent.is_empty() {
            about_machine.push(format!(
                "It tells {} recurring ways of running itself apart.",
                self.latent.len()
            ));
        }
        let still_uncertain: Vec<String> = self
            .experience
            .procedures()
            .filter(|procedure| !procedure.is_established())
            .map(|procedure| {
                format!(
                    "whether \"{}\" actually helps, or whether it just goes with runs that were \
                     going to work anyway",
                    procedure.name
                )
            })
            .collect();

        view::Knowledge {
            learning_for_ms: now_ms().saturating_sub(self.store.created_ms().unwrap_or(0)),
            machine_states: self.latent.len(),
            workflow_patterns: patterns.len(),
            verified_capabilities: self
                .capabilities
                .values()
                .filter(|c| matches!(c.provenance, Provenance::Learned { .. }))
                .count(),
            open_questions: still_uncertain.len(),
            rejected: self.experience.rejected,
            retired: patterns.iter().filter(|p| !p.is_live()).count() as u64,
            about_agents,
            about_workspaces,
            about_machine,
            still_uncertain,
        }
    }

    /// What this installation has worked out, for the Settings screen.
    ///
    /// This was an upgrade screen: a ledger of what the machine had learned,
    /// followed by prices and a list of what a paid tier would keep doing.
    /// The prices and the list are gone, because CoreScout is bought once at
    /// the door and there is nothing left to sell to somebody already running
    /// it.
    ///
    /// The ledger stayed. It was always the better half: it is the one thing
    /// about this product that only this installation can show, and it is
    /// worth showing to somebody who owns it as much as it ever was to
    /// somebody deciding.
    pub fn worked_out(&self) -> view::WorkedOut {
        let knowledge = self.knowledge();
        let patterns = self.experience.patterns();
        let verified = patterns.iter().filter(|p| p.basis.is_causal()).count();
        let failures = self.experience.failure_modes().len();

        let mut evidence = vec![
            view::Achievement {
                value: knowledge.machine_states.to_string(),
                label: "recurring states of this machine, found rather than configured".into(),
            },
            view::Achievement {
                value: self.experience.observed.to_string(),
                label: "actions watched while your AI worked".into(),
            },
        ];
        if failures > 0 {
            evidence.push(view::Achievement {
                value: failures.to_string(),
                label: "operations that recur and go wrong here".into(),
            });
        }
        if !patterns.is_empty() {
            evidence.push(view::Achievement {
                value: patterns.len().to_string(),
                label: format!(
                    "things noticed, {verified} of them tested under randomised assignment"
                ),
            });
        }

        let licence = self.licence();
        view::WorkedOut {
            headline: licence
                .map(|licence| licence.headline())
                // Not "unlicensed". A build that Windows did not install from
                // the Store cannot ask, and saying so is more honest than
                // implying something is wrong with the licence.
                .unwrap_or_else(|| "This build was not installed from the Microsoft Store.".into()),
            trial: licence.map(|licence| licence.trial).unwrap_or(false),
            trial_days_left: licence.and_then(|licence| licence.trial_days_left),
            worth_mentioning: licence
                .map(|licence| licence.worth_mentioning())
                .unwrap_or(false),
            evidence,
        }
    }

    /// The Privacy page.
    pub fn privacy(&self) -> Result<view::Privacy> {
        let dir = corescout_storage::paths::data_dir();
        let mut holdings = Vec::new();
        for kind in Kind::all() {
            holdings.push(view::Holding {
                kind: kind.as_str().into(),
                describes: describe_kind(kind).into(),
                count: self.store.count(kind)?,
                agent_activity: kind.is_agent_activity(),
            });
        }
        Ok(view::Privacy {
            data_dir: dir.display().to_string(),
            bytes: directory_bytes(&dir),
            telemetry: false,
            holdings,
        })
    }

    /// Diagnostics.
    pub fn diagnostics(&self) -> Result<view::Diagnostics> {
        let mean = if self.observations == 0 {
            0
        } else {
            (self.observe_total_ns / self.observations as u128) as u64
        };
        let mut documents = 0;
        for kind in Kind::all() {
            documents += self.store.count(kind)?;
        }
        let gaps = self
            .latest
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .sensors
                    .iter()
                    .filter(|report| report.errors > 0)
                    .map(|report| {
                        format!(
                            "sensor {} could not read {} times",
                            report.key, report.errors
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(view::Diagnostics {
            mean_observe_ns: mean,
            worst_observe_ns: self.observe_worst_ns,
            observations: self.observations,
            interval_ms: self.interval_ms,
            cpu_share: mean as f64 / (self.interval_ms as f64 * 1e6).max(1.0),
            ring_bytes: self.ring.bytes(),
            ring_samples: self.ring.len(),
            ring_wrapped: self.ring.wrapped(),
            documents,
            events: self.store.event_count()?,
            schema: self.store.schema_version()?,
            gaps,
        })
    }

    /// Forget agent activity, or everything.
    pub fn forget(&mut self, everything: bool) -> Result<usize> {
        let mut removed = 0;
        for kind in Kind::all() {
            if kind.is_agent_activity() || everything {
                removed += self.store.clear(kind)?;
            }
        }
        self.experience.forget();
        self.ingest.forget();
        self.sessions.clear();
        if everything {
            self.agents.clear();
            self.capabilities.clear();
            self.latent = LatentCatalogue::new(NOVELTY, MAX_STATES);
            self.normalizer = None;
            self.fitting.clear();
            let _ = self.ring.clear();
            removed += self.store.clear_events()? as usize;
        }
        self.dirty = true;
        self.persist()?;
        self.note(
            Event::decision(
                if everything {
                    "everything CoreScout had learned was deleted"
                } else {
                    "everything CoreScout knew about your AI activity was deleted"
                },
                "privacy",
                if everything {
                    "no record of this machine or of any AI that worked on it remains"
                } else {
                    "sessions, commands and outcomes go; what was learned about the machine stays"
                },
            )
            .outcome(format!("{removed} records removed")),
        );
        Ok(removed)
    }

    // -------------------------------------------------------------- helpers

    fn today(&self) -> view::Today {
        let cutoff = now_ms().saturating_sub(24 * 60 * 60 * 1000);
        let patterns = self.experience.patterns();
        view::Today {
            learned: patterns
                .iter()
                .filter(|pattern| pattern.last_ms >= cutoff && pattern.is_live())
                .count() as u64,
            capabilities: self
                .capabilities
                .values()
                .filter(|capability| capability.created_ms >= cutoff)
                .count() as u64,
            retired: patterns.iter().filter(|pattern| !pattern.is_live()).count() as u64,
            rejected: self.experience.rejected,
        }
    }

    fn mood(&self) -> view::MachineMood {
        match self.latent.current().and_then(|id| self.latent.get(id)) {
            Some(state) => view::MachineMood {
                plain: if state.entries <= 1 {
                    "Your computer is in a way of running it has not seen before.".into()
                } else {
                    format!(
                        "Your computer is in a familiar state. It has been here {} times.",
                        state.entries
                    )
                },
                state: Some(state.id.0),
                seen: state.entries,
                confidence: Some(state.confidence),
                unfamiliar: state.entries <= 1,
            },
            None => view::MachineMood {
                plain: if self.normalizer.is_none() {
                    "Your computer is still working out what normal looks like.".into()
                } else {
                    "Your computer is not running yet.".into()
                },
                state: None,
                seen: 0,
                confidence: None,
                unfamiliar: false,
            },
        }
    }

    fn nodes(&self) -> Vec<view::Node> {
        let Some(snapshot) = &self.latest else {
            return Vec::new();
        };
        snapshot
            .entities
            .iter()
            .enumerate()
            .map(|(row, entity)| {
                let observed = (0..snapshot.channels.len())
                    .filter(|col| {
                        snapshot.availability(row as u32, corescout_mirror::ChannelId(*col as u16))
                            == corescout_mirror::Availability::Observed
                    })
                    .count();
                view::Node {
                    key: entity.key.clone(),
                    label: entity.key.clone(),
                    class: entity.class_hint.label().into(),
                    activity: activity_of(snapshot, row as u32),
                    clarity: observed as f64 / snapshot.channels.len().max(1) as f64,
                    parent: entity
                        .key
                        .rsplit_once('/')
                        .map(|(head, _)| head.to_string()),
                }
            })
            .collect()
    }
}

/// The status view, re-exported so callers need one import.
pub type Status = view::Status;
/// The home view, re-exported so callers need one import.
pub type Home = view::Home;

/// What a workspace is called, wherever the caller got it from.
///
/// A hook reports the folder it ran in; an agent reports whatever it thinks it
/// is working on; the command line reports its own directory. Identifying in
/// one place is what makes those one workspace instead of three, and getting
/// it wrong means an agent asks about a repository CoreScout has recorded a
/// great deal about and is told it knows nothing.
fn workspace_id(given: &str) -> String {
    let trimmed = given.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    corescout_agent_observation::workspace::identify(std::path::Path::new(trimmed))
}

fn card_from(pattern: &Pattern) -> view::LearnedCard {
    view::LearnedCard {
        id: pattern.id.clone(),
        title: pattern.headline.clone(),
        detail: pattern.detail.clone(),
        basis: pattern.basis.label().into(),
        causal: pattern.basis.is_causal(),
        confidence: pattern.confidence(),
        reliability: match &pattern.basis {
            Basis::Causal { rate_with, .. } => Some(1.0 - rate_with),
            Basis::Association { .. } => None,
        },
        uses: 0,
        last_ms: pattern.last_ms,
        usable_by_ai: false,
        is_capability: false,
        approved: false,
    }
}

/// Turn a learned step, which is a fingerprint, back into something runnable.
///
/// A fingerprint has already had its arguments stripped, so what comes back is
/// the program and its subcommands. That is deliberately less than the agent
/// originally ran: a capability should not carry arguments nobody checked.
fn operation_from(text: &str) -> Option<Operation> {
    let mut parts = text.split_whitespace();
    let command = parts.next()?.to_string();
    let args: Vec<String> = parts.map(str::to_string).collect();
    Some(Operation::Run {
        command,
        args,
        cwd: None,
        timeout_ms: None,
        expect: None,
    })
}

fn describe_state(entries: u64, dwell_ns: f64) -> String {
    let seconds = dwell_ns / 1e9;
    match entries {
        0..=1 => "a way of running that has only been seen once".into(),
        2..=5 => format!("an occasional state, usually lasting {seconds:.0}s"),
        _ => format!("a common state, usually lasting {seconds:.0}s"),
    }
}

fn describe_kind(kind: Kind) -> &'static str {
    match kind {
        Kind::Agent => "which AI systems have connected",
        Kind::Session => "when each AI was working",
        Kind::Task => "what each AI said it was doing",
        Kind::Action => "commands and tools your AI ran, with secrets removed",
        Kind::Workspace => "which folders were worked in",
        Kind::LatentState => "recurring ways this computer runs",
        Kind::Concept => "names this computer gave to things it found",
        Kind::Hypothesis => "claims CoreScout is testing",
        Kind::Evidence => "the counts behind those claims",
        Kind::Pattern => "operational patterns found",
        Kind::Capability => "procedures CoreScout has verified",
        Kind::Proposal => "changes CoreScout suggested",
        Kind::Decision => "what you approved or refused",
        Kind::Setting => "your settings and permissions",
        Kind::Meta => "bookkeeping about CoreScout itself",
    }
}

fn observed_cells(snapshot: &MirrorSnapshot) -> usize {
    let mut count = 0;
    for row in 0..snapshot.entities.len() as u32 {
        for col in 0..snapshot.channels.len() as u16 {
            if snapshot.availability(row, corescout_mirror::ChannelId(col))
                == corescout_mirror::Availability::Observed
            {
                count += 1;
            }
        }
    }
    count
}

/// How busy one entity looks, from whatever channels it exposes.
///
/// A crude aggregate, and named as one. It drives the size of a dot in a
/// visualisation and nothing else; nothing is learned from it.
fn activity_of(snapshot: &MirrorSnapshot, row: u32) -> f64 {
    let mut total = 0.0;
    let mut counted = 0;
    for col in 0..snapshot.channels.len() as u16 {
        if let Some(value) = snapshot.value(row, corescout_mirror::ChannelId(col)) {
            if value.is_finite() {
                total += value.abs();
                counted += 1;
            }
        }
    }
    if counted == 0 {
        return 0.0;
    }
    let mean = total / counted as f64;
    (mean.log10().max(0.0) / 12.0).clamp(0.0, 1.0)
}

/// Aggregate features for a ring sample.
///
/// Six numbers chosen to be readable on a chart rather than to be learned
/// from: the learning uses the full feature row, which is far too wide to
/// store at observation rate.
fn aggregate(snapshot: &MirrorSnapshot) -> [f32; FEATURES] {
    let mut out = [0.0f32; FEATURES];
    let rows = snapshot.entities.len();
    let cols = snapshot.channels.len();
    if rows == 0 || cols == 0 {
        return out;
    }
    let mut activity = 0.0;
    for row in 0..rows {
        activity += activity_of(snapshot, row as u32);
    }
    out[0] = (activity / rows as f64) as f32;
    out[1] = snapshot.coverage() as f32;
    out[2] = observed_cells(snapshot) as f32;
    out[3] = rows as f32;
    out[4] = cols as f32;
    out[5] = snapshot.privilege_gaps() as f32;
    out
}

fn directory_bytes(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.metadata().ok())
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len())
        .sum()
}
