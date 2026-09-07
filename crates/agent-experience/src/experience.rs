//! The accumulator: actions in, learned items out.
//!
//! # How "do X before Y" is found
//!
//! For every action, CoreScout looks back over the same session and notes what
//! preceded it. Over time that gives, for each pair, the failure rate of Y
//! when X came first and when it did not. A pair where those differ enough,
//! seen enough times on both sides, becomes an association.
//!
//! That is where a lesser system would stop and start recommending. Here it is
//! only the point at which a [`Procedure`] is created, whose entire purpose is
//! to be *tested*: from then on CoreScout sometimes suggests it by coin flip,
//! and only those trials feed a causal claim.
//!
//! # Everything here is bounded
//!
//! A long-lived service watching a busy machine sees an unbounded stream. Every
//! map in this module has a ceiling and a documented eviction rule, because a
//! product that quietly grows a gigabyte of pair counts is one people uninstall
//! without ever seeing what it learned.

use std::collections::BTreeMap;
use std::collections::VecDeque;

use corescout_agent_observation::Action;
use serde::{Deserialize, Serialize};

use crate::failure::{key_for, FailureMode};
use crate::pattern::{Basis, Pattern, Subject};
use crate::procedure::{Candidate, Procedure, Step};

/// The bars a candidate has to clear.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    /// Attempts before an operation is a failure mode.
    pub min_attempts: u64,
    /// Failures before an operation is a failure mode.
    pub min_failures: u64,
    /// Observations on each side before a precursor is an association.
    pub min_each_side: u64,
    /// How much lower the failure rate must be, absolute.
    pub min_lift: f64,
    /// Reliability a procedure needs before it is offered as a capability.
    pub min_reliability: f64,
    /// Actions of lookback within a session.
    pub lookback: usize,
    /// Milliseconds of lookback within a session.
    pub lookback_ms: u64,
}

impl Default for Thresholds {
    fn default() -> Thresholds {
        Thresholds {
            min_attempts: 5,
            min_failures: 2,
            min_each_side: 3,
            min_lift: 0.25,
            min_reliability: 0.85,
            lookback: 8,
            lookback_ms: 10 * 60 * 1000,
        }
    }
}

/// Distinct operations tracked. Beyond this the least active are dropped.
const MODE_CEILING: usize = 2_000;
/// Distinct ordered pairs tracked.
const PAIR_CEILING: usize = 8_000;
/// Actions of per-session history kept in memory.
const SESSION_HISTORY: usize = 32;
/// How far over a ceiling a map may go before it is trimmed back to it.
///
/// Evicting on every observation past the ceiling turns each action into a
/// sort of the whole map. Trimming in batches makes the cost amortised.
const SLACK: usize = 256;
/// Joins a target key to a precursor fingerprint.
const JOIN: char = '\u{1f}';

/// Counts on one side of a comparison.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Counts {
    attempts: u64,
    failures: u64,
}

impl Counts {
    fn observe(&mut self, failed: bool) {
        self.attempts += 1;
        if failed {
            self.failures += 1;
        }
    }

    fn rate(&self) -> Option<f64> {
        (self.attempts > 0).then(|| self.failures as f64 / self.attempts as f64)
    }
}

/// What was seen before what.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Pair {
    with: Counts,
    last_ms: u64,
    first_ms: u64,
}

/// Everything learned from watching agents work.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Experience {
    /// The bars in force.
    pub thresholds: Thresholds,
    /// Per operation, in a place.
    #[serde(default)]
    modes: BTreeMap<String, FailureMode>,
    /// Per ordered pair, keyed by target and precursor joined by [`JOIN`].
    ///
    /// One string rather than a tuple because a tuple is not a JSON object
    /// key, and everything in this crate has to survive being written to the
    /// document store exactly as it is held here.
    #[serde(default)]
    pairs: BTreeMap<String, Pair>,
    /// Procedures under test, by id.
    #[serde(default)]
    procedures: BTreeMap<String, Procedure>,
    /// Recent actions per session, for lookback. Not stored: after a restart
    /// there is no session in flight to look back over.
    #[serde(skip)]
    history: BTreeMap<String, VecDeque<Seen>>,
    /// Actions observed in total.
    pub observed: u64,
    /// Candidate patterns that failed a threshold. Shown in the interface so
    /// the bars are visible rather than implied.
    pub rejected: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct Seen {
    fingerprint: String,
    at_ms: u64,
}

impl Experience {
    /// An empty accumulator with the default bars.
    pub fn new() -> Experience {
        Experience {
            thresholds: Thresholds::default(),
            ..Experience::default()
        }
    }

    /// Fold in one action.
    pub fn observe(&mut self, action: &Action) {
        if action.fingerprint.is_empty() {
            return;
        }
        self.observed += 1;
        let key = key_for(&action.fingerprint, action.workspace.as_deref());
        let failed = action.visibly_failed();

        self.modes
            .entry(key.clone())
            .or_insert_with(|| FailureMode::new(&action.fingerprint, action.workspace.as_deref()))
            .observe(action);

        // What came before, in this session, recently enough to be related.
        let floor = action
            .started_ms
            .saturating_sub(self.thresholds.lookback_ms);
        let history = self.history.entry(action.session.clone()).or_default();
        let mut seen_before: Vec<String> = Vec::new();
        for earlier in history.iter().rev().take(self.thresholds.lookback) {
            if earlier.at_ms < floor {
                break;
            }
            // A repeat of the same operation is not a precursor of itself;
            // counting it would make every retried command look like its own
            // remedy.
            if earlier.fingerprint == action.fingerprint {
                continue;
            }
            if !seen_before.contains(&earlier.fingerprint) {
                seen_before.push(earlier.fingerprint.clone());
            }
        }
        for precursor in seen_before {
            let pair = self.pairs.entry(pair_key(&key, &precursor)).or_default();
            if pair.first_ms == 0 {
                pair.first_ms = action.started_ms;
            }
            pair.last_ms = action.started_ms;
            pair.with.observe(failed);
        }

        history.push_back(Seen {
            fingerprint: action.fingerprint.clone(),
            at_ms: action.started_ms,
        });
        while history.len() > SESSION_HISTORY {
            history.pop_front();
        }
        self.evict();
    }

    /// Note that a session is over, so its history can go.
    pub fn close_session(&mut self, session: &str) {
        self.history.remove(session);
    }

    /// Operations that recur and go wrong.
    pub fn failure_modes(&self) -> Vec<&FailureMode> {
        let mut out: Vec<&FailureMode> = self
            .modes
            .values()
            .filter(|mode| {
                mode.is_recurring(self.thresholds.min_attempts, self.thresholds.min_failures)
            })
            .collect();
        out.sort_by(|a, b| b.failures.cmp(&a.failures).then(a.key.cmp(&b.key)));
        out
    }

    /// Every operation seen, whether or not it is a failure mode.
    pub fn operations(&self) -> impl Iterator<Item = &FailureMode> {
        self.modes.values()
    }

    /// One operation.
    pub fn operation(&self, fingerprint: &str, workspace: Option<&str>) -> Option<&FailureMode> {
        self.modes.get(&key_for(fingerprint, workspace))
    }

    /// Procedures under test.
    pub fn procedures(&self) -> impl Iterator<Item = &Procedure> {
        self.procedures.values()
    }

    /// One procedure, mutably, for recording a trial.
    pub fn procedure_mut(&mut self, id: &str) -> Option<&mut Procedure> {
        self.procedures.get_mut(id)
    }

    /// What CoreScout has noticed, whatever the grounds.
    ///
    /// Associations from precursor counts, and causal findings from procedures
    /// that have accumulated randomised trials. The two are labelled, never
    /// merged.
    pub fn patterns(&self) -> Vec<Pattern> {
        let mut out = Vec::new();

        for procedure in self.procedures.values() {
            if let Some(basis) = procedure.basis() {
                if !basis.is_causal() {
                    continue;
                }
                out.push(Pattern {
                    id: format!("pat-{}", procedure.id),
                    subject: subject_for(&procedure.target, procedure.workspace.as_deref()),
                    headline: procedure.name.clone(),
                    detail: format!(
                        "{} is more reliable here when this runs first.",
                        procedure.target
                    ),
                    basis,
                    first_ms: procedure.first_ms,
                    last_ms: procedure.last_ms,
                    retired: None,
                });
            }
        }

        for (pattern, _) in self.associations() {
            out.push(pattern);
        }

        out.sort_by(|a, b| {
            b.confidence()
                .partial_cmp(&a.confidence())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.id.cmp(&b.id))
        });
        out
    }

    /// Associations strong enough to be worth testing, with the pair that
    /// produced each.
    fn associations(&self) -> Vec<(Pattern, (String, String))> {
        let mut out = Vec::new();
        for (joined, pair) in &self.pairs {
            let Some((target, precursor)) = joined.split_once(JOIN) else {
                continue;
            };
            let Some(mode) = self.modes.get(target) else {
                continue;
            };
            if !mode.is_recurring(self.thresholds.min_attempts, self.thresholds.min_failures) {
                continue;
            }
            let without = Counts {
                attempts: mode.attempts.saturating_sub(pair.with.attempts),
                failures: mode.failures.saturating_sub(pair.with.failures),
            };
            if pair.with.attempts < self.thresholds.min_each_side
                || without.attempts < self.thresholds.min_each_side
            {
                continue;
            }
            let (Some(rate_with), Some(rate_without)) = (pair.with.rate(), without.rate()) else {
                continue;
            };
            if rate_without - rate_with < self.thresholds.min_lift {
                continue;
            }
            let pattern = Pattern {
                id: format!("pat-assoc-{}", stable_id(target, precursor)),
                subject: subject_for(&mode.fingerprint, mode.workspace.as_deref()),
                headline: format!("{} works better after {precursor}", mode.fingerprint),
                detail: format!(
                    "When {precursor} ran first, {} failed {}% of the time instead of {}%.",
                    mode.fingerprint,
                    (rate_with * 100.0).round() as u64,
                    (rate_without * 100.0).round() as u64
                ),
                basis: Basis::Association {
                    together: pair.with.attempts,
                    occurrences: mode.attempts,
                    rate_with,
                    rate_without,
                },
                first_ms: pair.first_ms,
                last_ms: pair.last_ms,
                retired: None,
            };
            out.push((pattern, (target.to_string(), precursor.to_string())));
        }
        out
    }

    /// Turn associations that have cleared the bars into procedures to test.
    ///
    /// Returns the ids of procedures created. This is the step that converts a
    /// correlation into an experiment, and it is the only way a procedure
    /// comes into existence.
    pub fn propose(&mut self, seed: u64) -> Vec<String> {
        let candidates: Vec<(String, String)> = self
            .associations()
            .into_iter()
            .map(|(_, pair)| pair)
            .collect();
        let mut created = Vec::new();
        for (target, precursor) in candidates {
            let id = format!("proc-{}", stable_id(&target, &precursor));
            if self.procedures.contains_key(&id) {
                continue;
            }
            let Some(mode) = self.modes.get(&target) else {
                continue;
            };
            let procedure = Procedure::new(
                id.clone(),
                mode.fingerprint.clone(),
                mode.workspace.clone(),
                format!("{precursor}, then {}", mode.fingerprint),
                vec![
                    Step::act(
                        precursor.clone(),
                        format!("{} fails less often when this ran first", mode.fingerprint),
                    ),
                    Step::act(mode.fingerprint.clone(), "the operation itself"),
                ],
                seed ^ hash(&id),
            );
            self.procedures.insert(id.clone(), procedure);
            created.push(id);
        }
        created
    }

    /// Procedures ready to be offered to the user as capabilities.
    pub fn candidates(&self) -> Vec<Candidate> {
        self.procedures
            .values()
            .filter(|procedure| procedure.is_promotable(self.thresholds.min_reliability))
            .filter_map(|procedure| {
                let basis = procedure.basis()?;
                Some(Candidate {
                    name: procedure.name.clone(),
                    because: basis.explain(),
                    basis,
                    procedure: procedure.clone(),
                })
            })
            .collect()
    }

    /// Operations that keep reporting success without anyone checking.
    ///
    /// Not a failure and not nothing: it is the gap where a silent failure
    /// would hide, and naming it is how CoreScout proposes adding a
    /// verification step.
    pub fn unverified_operations(&self) -> Vec<&FailureMode> {
        let mut out: Vec<&FailureMode> = self
            .modes
            .values()
            .filter(|mode| {
                mode.attempts >= self.thresholds.min_attempts
                    && mode.unverified == mode.attempts
                    && mode.silent_failures == 0
            })
            .collect();
        out.sort_by(|a, b| b.attempts.cmp(&a.attempts).then(a.key.cmp(&b.key)));
        out
    }

    /// How many operations are tracked.
    pub fn tracked_operations(&self) -> usize {
        self.modes.len()
    }

    /// How many ordered pairs are tracked.
    pub fn tracked_pairs(&self) -> usize {
        self.pairs.len()
    }

    /// Forget everything learned from agent activity.
    pub fn forget(&mut self) {
        let thresholds = self.thresholds;
        *self = Experience::default();
        self.thresholds = thresholds;
    }

    /// Drop the least useful entries when the maps get too big.
    ///
    /// Operations go by how little has been seen of them; pairs go by how long
    /// ago they were last seen. Both are the entries least likely to become a
    /// pattern, and dropping them is preferable to a service that grows
    /// forever.
    fn evict(&mut self) {
        if self.modes.len() > MODE_CEILING + SLACK {
            let mut by_activity: Vec<(u64, String)> = self
                .modes
                .values()
                .map(|mode| (mode.attempts, mode.key.clone()))
                .collect();
            by_activity.sort();
            let doomed: Vec<String> = by_activity
                .into_iter()
                .take(self.modes.len() - MODE_CEILING)
                .map(|(_, key)| key)
                .collect();
            for key in &doomed {
                self.modes.remove(key);
            }
            // One pass over the pairs for the whole batch. Doing it per
            // evicted operation is the same work multiplied by the batch size,
            // and it is enough to make a background service visible in a
            // profiler.
            let prefixes: Vec<String> = doomed.iter().map(|key| format!("{key}{JOIN}")).collect();
            self.pairs.retain(|joined, _| {
                !prefixes
                    .iter()
                    .any(|prefix| joined.starts_with(prefix.as_str()))
            });
            self.rejected += doomed.len() as u64;
        }
        if self.pairs.len() > PAIR_CEILING + SLACK {
            let mut by_age: Vec<(u64, String)> = self
                .pairs
                .iter()
                .map(|(key, pair)| (pair.last_ms, key.clone()))
                .collect();
            by_age.sort();
            for (_, key) in by_age.into_iter().take(self.pairs.len() - PAIR_CEILING) {
                self.pairs.remove(&key);
                self.rejected += 1;
            }
        }
    }
}

fn subject_for(fingerprint: &str, workspace: Option<&str>) -> Subject {
    match workspace {
        Some(workspace) => Subject::OperationHere {
            operation: fingerprint.to_string(),
            workspace: workspace.to_string(),
        },
        None => Subject::Operation(fingerprint.to_string()),
    }
}

/// A short, stable identifier for a pair.
///
/// Stable matters: the same association found again after a restart has to be
/// recognised as the same one, or every restart creates a fresh procedure and
/// no evidence ever accumulates.
fn pair_key(target: &str, precursor: &str) -> String {
    format!("{target}{JOIN}{precursor}")
}

fn stable_id(target: &str, precursor: &str) -> String {
    format!("{:08x}", hash(&format!("{target}|{precursor}")) as u32)
}

fn hash(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_agent_observation::{ActionKind, Report, Verification};

    fn act(session: &str, name: &str, at_ms: u64, failed: bool) -> Action {
        Action {
            id: format!("{session}-{at_ms}-{name}"),
            session: session.into(),
            task: None,
            workspace: Some("app".into()),
            started_ms: at_ms,
            duration_ms: 2000,
            kind: ActionKind::Build,
            name: name.into(),
            fingerprint: name.into(),
            reported: if failed {
                Report::Failure("boom".into())
            } else {
                Report::Success
            },
            verified: Some(if failed {
                Verification::Contradicted("boom".into())
            } else {
                Verification::Confirmed
            }),
            exit_code: None,
            files: Vec::new(),
            retry_of: None,
            machine_state: None,
        }
    }

    /// Twelve builds: the six that ran after the generator mostly worked, the
    /// six that did not mostly failed.
    fn planted() -> Experience {
        let mut experience = Experience::new();
        let mut at = 1_000_000u64;
        for n in 0..6 {
            experience.observe(&act("s1", "gen schema", at, false));
            at += 1000;
            experience.observe(&act("s1", "cargo build", at, n == 5));
            at += 1000;
        }
        for n in 0..6 {
            experience.observe(&act("s2", "cargo build", at, n != 5));
            at += 1000;
        }
        experience
    }

    #[test]
    fn a_recurring_failure_is_found() {
        let experience = planted();
        let modes = experience.failure_modes();
        assert_eq!(modes.len(), 1);
        assert_eq!(modes[0].fingerprint, "cargo build");
        assert_eq!(modes[0].attempts, 12);
        assert_eq!(modes[0].failures, 6);
    }

    #[test]
    fn the_precursor_is_found_as_an_association_and_not_more() {
        // Everything observed here was chosen by the agent, so the honest
        // reading is that the two go together, not that one causes the other.
        let experience = planted();
        let patterns = experience.patterns();
        assert!(!patterns.is_empty(), "nothing was noticed");
        let found = patterns
            .iter()
            .find(|p| p.headline.contains("gen schema"))
            .expect("the precursor pattern");
        assert!(!found.basis.is_causal());
        assert_eq!(found.basis.label(), "Seen together");
        assert!(found.detail.contains('%'));
    }

    #[test]
    fn a_weak_association_does_not_clear_the_bar() {
        // Half the point of this crate is what it declines to report.
        let mut experience = Experience::new();
        let mut at = 1_000_000u64;
        for n in 0..12 {
            experience.observe(&act("s1", "gen schema", at, false));
            at += 1000;
            experience.observe(&act("s1", "cargo build", at, n % 2 == 0));
            at += 1000;
            experience.observe(&act("s2", "cargo build", at, n % 2 == 0));
            at += 1000;
        }
        let noticed: Vec<&Pattern> = experience
            .patterns()
            .iter()
            .filter(|p| p.headline.contains("gen schema"))
            .map(|_| unreachable!("nothing should clear the bar"))
            .collect();
        assert!(noticed.is_empty());
    }

    #[test]
    fn an_operation_is_not_a_precursor_of_itself() {
        // A retried command would otherwise look like its own remedy, which is
        // both wrong and the most common shape in the data.
        let mut experience = Experience::new();
        let mut at = 1_000_000u64;
        for n in 0..20 {
            experience.observe(&act("s1", "cargo build", at, n < 10));
            at += 1000;
        }
        assert!(experience.patterns().iter().all(|p| !p
            .headline
            .contains("cargo build works better after cargo build")));
    }

    #[test]
    fn an_association_becomes_a_procedure_to_be_tested() {
        // The step that turns a correlation into an experiment. Nothing else
        // creates a procedure.
        let mut experience = planted();
        assert_eq!(experience.procedures().count(), 0);
        let created = experience.propose(7);
        assert_eq!(created.len(), 1);
        assert_eq!(experience.procedures().count(), 1);
        // Proposing twice does not create it twice, so evidence accumulates
        // against one procedure rather than being split across restarts.
        assert!(experience.propose(7).is_empty());
        assert_eq!(experience.procedures().count(), 1);
    }

    #[test]
    fn a_proposed_procedure_starts_with_no_causal_claim() {
        let mut experience = planted();
        experience.propose(7);
        let procedure = experience.procedures().next().expect("a procedure");
        assert!(!procedure.is_established());
        assert!(experience.candidates().is_empty());
    }

    #[test]
    fn a_procedure_becomes_a_candidate_only_after_randomised_trials() {
        let mut experience = planted();
        let ids = experience.propose(7);
        let id = ids[0].clone();
        {
            let procedure = experience.procedure_mut(&id).expect("a procedure");
            for n in 0..40 {
                procedure.record(true, n % 20 == 0, true, 2_000_000);
                procedure.record(false, n % 2 == 0, true, 2_000_000);
            }
        }
        let candidates = experience.candidates();
        assert_eq!(candidates.len(), 1);
        assert!(candidates[0].basis.is_causal());
        assert!(candidates[0].because.contains("randomised trials"));
    }

    #[test]
    fn a_procedure_with_chosen_trials_alone_never_becomes_a_candidate() {
        let mut experience = planted();
        let ids = experience.propose(7);
        let id = ids[0].clone();
        let procedure = experience.procedure_mut(&id).expect("a procedure");
        for _ in 0..500 {
            procedure.record(true, false, false, 2_000_000);
            procedure.record(false, true, false, 2_000_000);
        }
        assert!(experience.candidates().is_empty());
    }

    #[test]
    fn operations_that_nobody_ever_checks_are_named() {
        let mut experience = Experience::new();
        for n in 0..8u64 {
            experience.observe(&Action {
                verified: None,
                ..act("s1", "deploy", 1000 + n * 1000, false)
            });
        }
        let unverified = experience.unverified_operations();
        assert_eq!(unverified.len(), 1);
        assert_eq!(unverified[0].fingerprint, "deploy");
    }

    #[test]
    fn an_operation_that_is_sometimes_checked_is_not_flagged_as_unchecked() {
        let mut experience = Experience::new();
        for n in 0..8u64 {
            let action = act("s1", "deploy", 1000 + n * 1000, false);
            experience.observe(&Action {
                verified: if n == 3 {
                    Some(Verification::Confirmed)
                } else {
                    None
                },
                ..action
            });
        }
        assert!(experience.unverified_operations().is_empty());
    }

    #[test]
    fn a_precursor_in_another_session_does_not_count() {
        // Two agents working at the same time would otherwise contaminate each
        // other's evidence.
        let mut experience = Experience::new();
        let mut at = 1_000_000u64;
        for n in 0..8 {
            experience.observe(&act("other", "gen schema", at, false));
            at += 500;
            experience.observe(&act("s1", "cargo build", at, n < 6));
            at += 500;
        }
        assert!(experience
            .patterns()
            .iter()
            .all(|p| !p.headline.contains("gen schema")));
    }

    #[test]
    fn a_precursor_long_ago_does_not_count() {
        let mut experience = Experience::new();
        let mut at = 1_000_000u64;
        for n in 0..8 {
            experience.observe(&act("s1", "gen schema", at, false));
            at += experience.thresholds.lookback_ms + 1000;
            experience.observe(&act("s1", "cargo build", at, n < 6));
            at += 1000;
        }
        assert!(experience
            .patterns()
            .iter()
            .all(|p| !p.headline.contains("gen schema")));
    }

    #[test]
    fn the_maps_do_not_grow_without_bound() {
        // A busy machine produces an unbounded stream. This is the difference
        // between a background service and a leak.
        let mut experience = Experience::new();
        for n in 0..12_000u64 {
            experience.observe(&act("s1", &format!("op-{n}"), n * 10, n % 3 == 0));
        }
        assert!(experience.tracked_operations() <= MODE_CEILING + SLACK);
        assert!(experience.tracked_pairs() <= PAIR_CEILING + SLACK);
    }

    #[test]
    fn everything_learned_survives_storage() {
        let mut experience = planted();
        experience.propose(7);
        let json = serde_json::to_string(&experience).expect("serialise");
        let back: Experience = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back.observed, experience.observed);
        assert_eq!(back.failure_modes().len(), 1);
        assert_eq!(back.procedures().count(), 1);
    }

    #[test]
    fn a_procedure_keeps_its_identity_across_a_restart() {
        // Otherwise every restart creates a fresh procedure and no evidence
        // ever accumulates past one session.
        let mut first = planted();
        let created = first.propose(7);
        let json = serde_json::to_string(&first).expect("serialise");
        let mut back: Experience = serde_json::from_str(&json).expect("deserialise");
        assert!(
            back.propose(7).is_empty(),
            "it was recognised, not recreated"
        );
        assert_eq!(
            back.procedures().next().expect("a procedure").id,
            created[0]
        );
    }

    #[test]
    fn forgetting_leaves_nothing_behind() {
        let mut experience = planted();
        experience.propose(7);
        experience.forget();
        assert_eq!(experience.observed, 0);
        assert_eq!(experience.tracked_operations(), 0);
        assert_eq!(experience.procedures().count(), 0);
        assert!(experience.patterns().is_empty());
    }
}
