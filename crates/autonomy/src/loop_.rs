//! The decision loop.
//!
//! ```text
//! SEE -> REMEMBER -> INTERPRET -> IMAGINE -> EVALUATE -> CHOOSE -> ACT -> SEE
//! ```
//!
//! # One tick
//!
//! Each [`ControlLoop::tick`] takes one reflection and runs the whole cycle
//! once. The loop owns the memory, the representation, the models and the
//! actuators, and holds the watchdog at arm's length: it asks for a verdict and
//! obeys it, and has no way to overrule it.
//!
//! # Learning happens at the *start* of a tick, not the end
//!
//! The consequence of an action is not visible until the next reflection
//! arrives. So a tick begins by looking at what the previous tick's action
//! actually did, and only then decides what to do next. Attributing the
//! consequence at the moment of acting, which is the obvious way to write this,
//! would mean the effect model learns from a reflection taken before the action
//! had any chance to take effect.
//!
//! # Nothing here is hidden
//!
//! Every tick returns a [`Tick`] describing what was seen, inferred, decided
//! and observed. A controller whose reasoning cannot be reconstructed
//! afterwards cannot be debugged, and cannot be trusted with a machine.

use corescout_agency::{ActionKind, ActuatorSet, Disposition};
use corescout_counterfactual::{EffectModel, Measure, Prediction};
use corescout_intent::{Achievement, Intent};
use corescout_memory::History;
use corescout_mirror::MirrorSnapshot;
use corescout_represent::latent::{LatentCatalogue, LatentStateId};
use corescout_represent::normalize::Normalizer;
use corescout_selfmodel::{AnomalyDetector, SelfModel};

use crate::curiosity::{Curiosity, ExperimentOutcome};
use crate::policy::{Decision, Policy};
use crate::safeguards::{Verdict, Watchdog};

/// How the loop is configured.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopConfig {
    /// Reflections to remember.
    pub history_capacity: usize,
    /// Reflections to gather before the normaliser is fitted and latent states
    /// begin to be recognised. Fitting on too little data produces a scaling
    /// that the next minute invalidates.
    pub warmup_reflections: usize,
    /// How long an experiment observes before concluding.
    pub experiment_duration_ns: u64,
    /// Distance beyond which a reflection founds a new latent state.
    pub novelty_threshold: f64,
    /// Ceiling on discovered states.
    pub max_latent_states: usize,
}

impl Default for LoopConfig {
    fn default() -> Self {
        LoopConfig {
            history_capacity: 4096,
            warmup_reflections: 100,
            experiment_duration_ns: 5_000_000_000,
            novelty_threshold: 0.75,
            max_latent_states: 32,
        }
    }
}

/// One action the loop may consider, as the caller supplies it: what to do, the
/// mirror row it acts on, and the effect family it belongs to.
///
/// The caller builds these because only it knows what workload is running and
/// where it is permitted to go. A loop that generated its own candidates would
/// be choosing the space it searches.
pub type ActionCandidate = (ActionKind, Option<usize>, String);

/// What happened in one pass of the loop.
#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    pub sequence: u64,
    pub monotonic_ns: u64,
    /// The latent state the machine was assigned to, once recognition starts.
    pub latent_state: Option<LatentStateId>,
    /// True when this reflection founded a state the machine had never seen.
    pub novel_state: bool,
    /// How surprising this reflection was to the self-model.
    pub surprise: f64,
    /// Mean prediction skill across the machine.
    pub model_skill: f64,
    /// What the watchdog permitted.
    pub verdict: Verdict,
    /// What the policy chose.
    pub decision: Option<Decision>,
    /// What the actuator did about it.
    pub disposition: Option<Disposition>,
    /// An experiment that concluded this tick.
    pub experiment: Option<ExperimentOutcome>,
    /// Whether the objective improved since the last action.
    pub improved: Option<bool>,
}

impl Tick {
    /// A one-line rendering.
    pub fn summary(&self) -> String {
        let mut line = format!("t{:>7}", self.sequence);
        if let Some(state) = self.latent_state {
            line.push_str(&format!("  {state}"));
            if self.novel_state {
                line.push_str(" (new)");
            }
        }
        line.push_str(&format!("  surprise {:.2}", self.surprise));
        match (&self.decision, &self.disposition) {
            (Some(decision), Some(disposition)) => {
                line.push_str(&format!(
                    "  {} [{}]",
                    decision.summary(),
                    disposition.label()
                ));
            }
            (Some(decision), None) => line.push_str(&format!("  {}", decision.summary())),
            _ => {}
        }
        if let Some(reason) = self.verdict.reason() {
            line.push_str(&format!("  watchdog: {reason}"));
        }
        line
    }
}

/// Owns everything the loop needs, and runs it.
pub struct ControlLoop {
    config: LoopConfig,
    history: History,
    normalizer: Option<Normalizer>,
    catalogue: LatentCatalogue,
    model: SelfModel,
    anomaly: AnomalyDetector,
    effects: EffectModel,
    policy: Policy,
    curiosity: Curiosity,
    watchdog: Watchdog,
    actuators: ActuatorSet,
    /// The reflection and action from the previous tick, so the consequence can
    /// be attributed when the next reflection arrives.
    pending: Option<PendingAction>,
    /// Achievement measured on the previous tick, for detecting regression.
    previous_achievement: Option<Achievement>,
    ticks: u64,
}

struct PendingAction {
    before: MirrorSnapshot,
    family: String,
    target_row: Option<usize>,
    predicted_effect: f64,
    was_experiment: bool,
    latent_state: Option<LatentStateId>,
}

impl std::fmt::Debug for ControlLoop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlLoop")
            .field("ticks", &self.ticks)
            .field("history", &self.history.len())
            .field("latent_states", &self.catalogue.len())
            .field("experiments", &self.curiosity.completed().len())
            .field("stopped", &self.watchdog.is_stopped())
            .finish()
    }
}

impl ControlLoop {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: LoopConfig,
        policy: Policy,
        actuators: ActuatorSet,
        watchdog: Watchdog,
        curiosity: Curiosity,
    ) -> ControlLoop {
        let history = History::new(config.history_capacity);
        let catalogue = LatentCatalogue::new(config.novelty_threshold, config.max_latent_states);
        ControlLoop {
            config,
            history,
            normalizer: None,
            catalogue,
            model: SelfModel::default(),
            anomaly: AnomalyDetector::default(),
            effects: EffectModel::new(),
            policy,
            curiosity,
            watchdog,
            actuators,
            pending: None,
            previous_achievement: None,
            ticks: 0,
        }
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn catalogue(&self) -> &LatentCatalogue {
        &self.catalogue
    }

    pub fn model(&self) -> &SelfModel {
        &self.model
    }

    pub fn effects(&self) -> &EffectModel {
        &self.effects
    }

    pub fn curiosity(&self) -> &Curiosity {
        &self.curiosity
    }

    pub fn watchdog(&self) -> &Watchdog {
        &self.watchdog
    }

    pub fn actuators(&self) -> &ActuatorSet {
        &self.actuators
    }

    /// Mutable access, for the one thing a caller outside the loop legitimately
    /// does to the actuators: revert on the way out. The loop itself never
    /// reaches for this.
    pub fn actuators_mut(&mut self) -> &mut ActuatorSet {
        &mut self.actuators
    }

    pub fn intent(&self) -> &Intent {
        self.policy.intent()
    }

    /// Run one pass.
    ///
    /// `candidates` is the set of actions worth considering, produced by the
    /// caller because only it knows what the workload is and where it could go.
    /// `measure` maps a reflection to the objectives the intent names.
    pub fn tick(
        &mut self,
        snapshot: &MirrorSnapshot,
        candidates: &[ActionCandidate],
        measure: Measure<'_>,
    ) -> Tick {
        let started = corescout_core::clock::now_ns();
        self.ticks += 1;

        // ---- 1. Learn from the previous tick's action, now that its effect is
        // visible. Doing this first is what makes attribution honest.
        let mut experiment_outcome = None;
        let achievement = measure(snapshot, &[]);
        let mut improved = None;

        if let Some(pending) = self.pending.take() {
            self.effects.learn(
                &pending.family,
                pending.target_row,
                &pending.before,
                snapshot,
            );

            let before = measure(&pending.before, &[]);
            let measured = corescout_counterfactual::outcome::score(
                self.policy.intent(),
                &achievement,
                &before,
            )
            .unwrap_or(0.0);
            improved = Some(measured > 0.0);
            self.watchdog.outcome_observed(measured > 0.0);

            if pending.was_experiment {
                experiment_outcome = self.curiosity.conclude(measured, pending.predicted_effect);
            }
            if let (Some(state), Some(outcome)) =
                (pending.latent_state, experiment_outcome.as_ref())
            {
                self.catalogue.record_action(
                    state,
                    &pending.family,
                    false,
                    outcome.measured_effect,
                );
            }
        }

        // ---- 2. SEE and REMEMBER.
        self.history.record(snapshot);
        let predictions: std::collections::BTreeMap<(usize, usize), f64> = self
            .model
            .predict_next()
            .into_iter()
            .filter(|p| p.interval.point.is_finite())
            .map(|p| ((p.row, p.col), p.interval.point))
            .collect();
        let surprise = self.anomaly.assess(snapshot, &predictions);
        self.model.observe(snapshot);
        let model_skill = self.model.skill();

        // ---- 3. INTERPRET: which latent state is this?
        let (latent_state, novel_state) = self.recognise(snapshot);

        // ---- 4. Ask the watchdog before considering any action at all.
        let verdict = self.watchdog.assess(snapshot.monotonic_ns, model_skill);
        if !verdict.permits_action() {
            if verdict.is_terminal() {
                // Hand the machine back.
                self.actuators.scope_mut().freeze();
                if let Some(experiment) = self.curiosity.abandon() {
                    let _ = experiment;
                }
                let _ = self.actuators.revert_last();
            }
            self.watchdog
                .tick_completed(corescout_core::clock::now_ns().saturating_sub(started));
            return Tick {
                sequence: snapshot.sequence,
                monotonic_ns: snapshot.monotonic_ns,
                latent_state,
                novel_state,
                surprise: surprise.score,
                model_skill,
                verdict,
                decision: None,
                disposition: None,
                experiment: experiment_outcome,
                improved,
            };
        }

        // ---- 5. IMAGINE: what would each candidate do?
        let horizon = self.model.interval_ns().max(1);
        let imagined: Vec<(Prediction, ActionKind)> = candidates
            .iter()
            .map(|(kind, row, description)| {
                // An `ActionKind` is what to do; a counterfactual `Candidate` is
                // the effect family it belongs to. Translating here keeps the
                // effect model ignorant of syscalls.
                let candidate = (kind.family().to_string(), *row, description.clone());
                let prediction = self.effects.predict(
                    &candidate,
                    snapshot,
                    horizon,
                    self.policy.intent(),
                    measure,
                );
                (prediction, kind.clone())
            })
            .collect();

        // ---- 6. EVALUATE and CHOOSE.
        let satisfied = achievement.satisfies(self.policy.intent());
        let decision = self.policy.decide(&imagined, satisfied, latent_state);

        // ---- 7. ACT.
        self.actuators.observing(snapshot.sequence);
        let disposition = match &decision {
            Decision::Hold { reason } => Some(
                self.actuators
                    .perform(corescout_agency::Action::hold(reason.clone())),
            ),
            Decision::Act { action, .. } | Decision::Explore { action, .. } => {
                let family = action.kind.family().to_string();
                let target_row = imagined
                    .iter()
                    .find(|(_, kind)| *kind == action.kind)
                    .and_then(|(prediction, _)| {
                        prediction.outcome.deltas.first().map(|(row, _, _)| *row)
                    });
                let predicted_effect = imagined
                    .iter()
                    .find(|(_, kind)| *kind == action.kind)
                    .and_then(|(prediction, _)| prediction.improvement)
                    .unwrap_or(0.0);

                let disposition = self.actuators.perform(action.clone());
                self.watchdog
                    .action_taken(snapshot.monotonic_ns, disposition.changed_the_machine());
                if disposition.changed_the_machine() {
                    // Remember what was done, so the *next* reflection can say
                    // what it did.
                    self.pending = Some(PendingAction {
                        before: snapshot.clone(),
                        family,
                        target_row,
                        predicted_effect,
                        was_experiment: decision.is_exploration(),
                        latent_state,
                    });
                }
                Some(disposition)
            }
        };

        self.previous_achievement = Some(achievement);
        self.watchdog
            .tick_completed(corescout_core::clock::now_ns().saturating_sub(started));

        Tick {
            sequence: snapshot.sequence,
            monotonic_ns: snapshot.monotonic_ns,
            latent_state,
            novel_state,
            surprise: surprise.score,
            model_skill,
            verdict,
            decision: Some(decision),
            disposition,
            experiment: experiment_outcome,
            improved,
        }
    }

    /// Fit the normaliser once enough has been seen, then assign the reflection
    /// to a latent state.
    fn recognise(&mut self, snapshot: &MirrorSnapshot) -> (Option<LatentStateId>, bool) {
        if self.normalizer.is_none() {
            if self.history.len() < self.config.warmup_reflections {
                return (None, false);
            }
            let frames: Vec<Vec<f64>> = self
                .history
                .frames()
                .map(|frame| frame.values.clone())
                .collect();
            self.normalizer = Some(Normalizer::fit(&frames, self.history.cols()));
        }
        let Some(normalizer) = &self.normalizer else {
            return (None, false);
        };
        let (point, _) = normalizer.apply_filled(snapshot.state.as_slice(), snapshot.state.cols());
        let (id, novel) = self.catalogue.observe(&point, snapshot.monotonic_ns);
        (Some(id), novel)
    }

    /// Whether the loop has been stopped and the machine handed back.
    pub fn is_stopped(&self) -> bool {
        self.watchdog.is_stopped()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_agency::{Bounds, Scope, Target};
    use corescout_counterfactual::effects::channel_measure;
    use corescout_intent::{Constraint, MigrationTolerance, Objective};
    use corescout_mirror::test_support::fixture;
    use corescout_represent::latent::LatentStateId;

    fn intent() -> Intent {
        Intent::new("latency")
            .with_constraint(Constraint::latency_p99_us(1e12))
            .preferring(Objective::Latency, 1.0)
            .migration(MigrationTolerance::High)
    }

    fn control_loop() -> ControlLoop {
        ControlLoop::new(
            LoopConfig {
                warmup_reflections: 20,
                ..LoopConfig::default()
            },
            Policy::new(intent(), crate::policy::PolicyConfig::default()),
            // A dry run so the tests exercise the whole loop without needing
            // Linux: the scope, bounds, rate limiting and audit trail are all
            // real, and only the syscall is skipped.
            ActuatorSet::dry_run(Scope::own_process(), Bounds::within((0u32..8).collect())),
            Watchdog::new(crate::safeguards::WatchdogConfig {
                warmup_ticks: 10_000,
                ..Default::default()
            }),
            Curiosity::new(0.3, 10),
        )
    }

    fn candidates() -> Vec<(ActionKind, Option<usize>, String)> {
        vec![(
            ActionKind::SetAffinity {
                target: Target::CurrentThread,
                cpus: [1u32].into_iter().collect(),
            },
            Some(2),
            "move to cpu 1".into(),
        )]
    }

    fn series(count: u64) -> Vec<MirrorSnapshot> {
        (0..count)
            .map(|i| {
                let mut snapshot = fixture();
                snapshot.sequence = i;
                snapshot.monotonic_ns = i * 100_000_000;
                snapshot
                    .state
                    .set(2, 0, 3_600_000.0 + ((i as f64) * 0.3).sin() * 50_000.0);
                snapshot
            })
            .collect()
    }

    #[test]
    fn the_loop_runs_and_reports_every_tick() {
        let mut control = control_loop();
        let measure = channel_measure(vec![(Objective::Latency, 2, 0)]);
        let mut ticks = Vec::new();
        for snapshot in series(60) {
            ticks.push(control.tick(&snapshot, &candidates(), &measure));
        }
        assert_eq!(ticks.len(), 60);
        assert_eq!(control.ticks(), 60);
        assert!(ticks.iter().all(|t| t.decision.is_some()));
        assert!(!control.is_stopped());
    }

    #[test]
    fn latent_states_appear_only_after_warmup() {
        let mut control = control_loop();
        let measure = channel_measure(vec![(Objective::Latency, 2, 0)]);
        let ticks: Vec<Tick> = series(60)
            .iter()
            .map(|s| control.tick(s, &candidates(), &measure))
            .collect();
        assert!(
            ticks[0].latent_state.is_none(),
            "recognition must wait for a fitted normaliser"
        );
        assert!(
            ticks.last().unwrap().latent_state.is_some(),
            "and must start once there is one"
        );
        assert!(!control.catalogue().is_empty());
    }

    #[test]
    fn with_no_evidence_the_loop_holds_or_explores_but_never_confidently_acts() {
        // At the start the effect model knows nothing, so a confident
        // optimisation would be unfounded.
        let mut control = control_loop();
        let measure = channel_measure(vec![(Objective::Latency, 2, 0)]);
        for snapshot in series(30) {
            let tick = control.tick(&snapshot, &candidates(), &measure);
            if let Some(Decision::Act { confidence, .. }) = &tick.decision {
                panic!("acted with confidence {confidence} on no evidence");
            }
        }
    }

    #[test]
    fn holding_is_recorded_in_the_audit_trail() {
        let mut control = control_loop();
        let measure = channel_measure(vec![(Objective::Latency, 2, 0)]);
        for snapshot in series(10) {
            control.tick(&snapshot, &candidates(), &measure);
        }
        assert!(!control.actuators().log().is_empty());
        assert!(control
            .actuators()
            .log()
            .events()
            .any(|e| e.action.kind.family() == "hold"));
    }

    #[test]
    fn a_stopped_watchdog_freezes_actuation_and_the_loop_keeps_observing() {
        let mut control = ControlLoop::new(
            LoopConfig {
                warmup_reflections: 5,
                ..LoopConfig::default()
            },
            Policy::new(intent(), crate::policy::PolicyConfig::default()),
            ActuatorSet::dry_run(Scope::own_process(), Bounds::within((0u32..8).collect())),
            // A watchdog that trips immediately on a collapsed model.
            Watchdog::new(crate::safeguards::WatchdogConfig {
                warmup_ticks: 0,
                min_model_skill: 10.0,
                ..Default::default()
            }),
            Curiosity::new(0.3, 10),
        );
        let measure = channel_measure(vec![(Objective::Latency, 2, 0)]);
        let ticks: Vec<Tick> = series(20)
            .iter()
            .map(|s| control.tick(s, &candidates(), &measure))
            .collect();

        assert!(control.is_stopped());
        assert!(
            ticks.iter().all(|t| t.decision.is_none()),
            "a stopped loop must not decide anything"
        );
        // Observation continues regardless: a stopped controller still watches.
        assert!(control.history().len() > 1);
        assert!(control.actuators().scope().is_frozen());
    }

    #[test]
    fn an_epoch_change_does_not_crash_the_loop() {
        let mut control = control_loop();
        let measure = channel_measure(vec![(Objective::Latency, 2, 0)]);
        for snapshot in series(30) {
            control.tick(&snapshot, &candidates(), &measure);
        }
        let mut changed = fixture();
        changed.epoch += 1;
        changed.sequence = 999;
        let tick = control.tick(&changed, &candidates(), &measure);
        assert_eq!(tick.sequence, 999);
        assert_eq!(control.history().len(), 1, "the series restarted");
    }

    #[test]
    fn every_tick_can_be_rendered_for_an_operator() {
        let mut control = control_loop();
        let measure = channel_measure(vec![(Objective::Latency, 2, 0)]);
        for snapshot in series(30) {
            let tick = control.tick(&snapshot, &candidates(), &measure);
            let summary = tick.summary();
            assert!(summary.starts_with('t'));
            assert!(summary.contains("surprise"));
        }
    }

    #[test]
    fn the_context_of_a_decision_is_the_latent_state_it_was_made_in() {
        let mut control = control_loop();
        let measure = channel_measure(vec![(Objective::Latency, 2, 0)]);
        let mut with_state = 0;
        for snapshot in series(60) {
            let tick = control.tick(&snapshot, &candidates(), &measure);
            if tick.latent_state.is_some() && tick.decision.is_some() {
                with_state += 1;
            }
        }
        assert!(
            with_state > 20,
            "decisions should be attributable to a state once one is known"
        );
        assert!(control.catalogue().current().is_some());
        let _ = LatentStateId(0);
    }
}
