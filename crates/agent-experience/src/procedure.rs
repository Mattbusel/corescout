//! A way of doing something, and the evidence about whether it is better.
//!
//! # Where randomisation actually happens
//!
//! A procedure starts as a hunch built from association. To learn whether it
//! works, CoreScout has to sometimes *not* suggest it when it believes it
//! should, and sometimes suggest it when it does not. [`Procedure::decide`]
//! delegates that coin flip to [`corescout_science::Exploration`], which is
//! the only place in this workspace that produces an honest `randomised` flag.
//!
//! The cost is real: a randomised trial is one where CoreScout may deliberately
//! let a build fail that it could have saved. That is why the exploration rate
//! is small and the budget is finite, and why a settled procedure keeps a
//! residual rate rather than stopping: the world changes, and a procedure that
//! is never re-tested keeps driving behaviour from evidence about a repository
//! that no longer looks like that.

use corescout_science::causal::{CausalEstimate, Insufficient};
use corescout_science::Exploration;
use serde::{Deserialize, Serialize};

use crate::pattern::Basis;

/// The residual exploration rate once a comparison is settled.
///
/// A fifth of the usual rate: enough to notice a repository that has changed,
/// little enough that a procedure people rely on is applied nearly always.
pub const SETTLED_SCALE: f64 = 0.2;

/// Randomised trials needed on each arm before an effect is reported.
pub const MIN_TRIALS: u32 = 8;

/// One step of a procedure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Step {
    /// What to do, as a command or an instruction.
    pub action: String,
    /// Why this step is here.
    pub because: String,
    /// Whether this step checks the world rather than changing it.
    #[serde(default)]
    pub verifies: bool,
}

impl Step {
    /// A step that does something.
    pub fn act(action: impl Into<String>, because: impl Into<String>) -> Step {
        Step {
            action: action.into(),
            because: because.into(),
            verifies: false,
        }
    }

    /// A step that checks something.
    pub fn verify(action: impl Into<String>, because: impl Into<String>) -> Step {
        Step {
            action: action.into(),
            because: because.into(),
            verifies: true,
        }
    }
}

/// A candidate way of doing an operation, and what is known about it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Procedure {
    /// Stable across restarts.
    pub id: String,
    /// The operation this is a way of doing.
    pub target: String,
    /// Where it applies, if it is specific to one place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// A name a person would use.
    pub name: String,
    /// What it does.
    pub steps: Vec<Step>,
    /// The evidence, association and causal kept apart.
    pub estimate: CausalEstimate,
    /// The randomiser.
    pub exploration: Exploration,
    /// Times it has been suggested.
    pub suggested: u64,
    /// Times an agent took the suggestion.
    pub taken: u64,
    /// Wall clock of the first trial.
    pub first_ms: u64,
    /// Wall clock of the most recent.
    pub last_ms: u64,
}

impl Procedure {
    /// Propose a procedure for an operation.
    ///
    /// The outcome measured is the failure rate, so lower is better.
    pub fn new(
        id: impl Into<String>,
        target: impl Into<String>,
        workspace: Option<String>,
        name: impl Into<String>,
        steps: Vec<Step>,
        seed: u64,
    ) -> Procedure {
        let target = target.into();
        let under = workspace.clone().unwrap_or_else(|| "anywhere".into());
        Procedure {
            id: id.into(),
            name: name.into(),
            estimate: CausalEstimate::new(
                under,
                format!("with:{target}"),
                format!("plain:{target}"),
                true,
            )
            .forgetting(0.995),
            exploration: Exploration::new(0.15, MIN_TRIALS, 400, seed),
            target,
            workspace,
            steps,
            suggested: 0,
            taken: 0,
            first_ms: 0,
            last_ms: 0,
        }
    }

    /// Decide whether to apply this procedure on this occasion.
    ///
    /// Returns `(apply, randomised)`. `randomised` is true only when the coin
    /// decided, and it is the flag every causal claim downstream rests on.
    pub fn decide(&mut self, believed_better: bool) -> (bool, bool) {
        let scale = if self.estimate.is_settled(MIN_TRIALS) {
            SETTLED_SCALE
        } else {
            1.0
        };
        self.suggested += 1;
        let (apply, randomised) = self.exploration.decide_scaled(believed_better, scale);
        if apply {
            self.taken += 1;
        }
        (apply, randomised)
    }

    /// Record how a trial went.
    ///
    /// `failed` is the outcome, so the estimate is over failure rates and
    /// lower is better.
    pub fn record(&mut self, applied: bool, failed: bool, randomised: bool, now_ms: u64) {
        if self.first_ms == 0 {
            self.first_ms = now_ms;
        }
        self.last_ms = now_ms;
        self.estimate
            .record(applied, if failed { 1.0 } else { 0.0 }, randomised);
    }

    /// The grounds for believing this procedure helps, if there are any.
    ///
    /// Prefers randomised evidence and falls back to association, which is
    /// what the interface then labels differently.
    pub fn basis(&self) -> Option<Basis> {
        let with = self.estimate.treatment.mean();
        let without = self.estimate.control.mean();
        match self.estimate.effect(MIN_TRIALS) {
            Ok(effect) => Some(Basis::Causal {
                delta: effect.delta,
                standard_error: effect.standard_error,
                treatment_trials: effect.treatment_trials,
                control_trials: effect.control_trials,
                rate_with: self.estimate.treatment.randomised_mean().or(with)?,
                rate_without: self.estimate.control.randomised_mean().or(without)?,
            }),
            Err(_) => Some(Basis::Association {
                together: self.estimate.treatment.trials as u64,
                occurrences: (self.estimate.treatment.trials + self.estimate.control.trials) as u64,
                rate_with: with?,
                rate_without: without?,
            }),
        }
    }

    /// Why CoreScout cannot yet make a causal claim, if it cannot.
    pub fn missing(&self) -> Option<Insufficient> {
        self.estimate.effect(MIN_TRIALS).err()
    }

    /// Whether this has enough randomised evidence to be worth acting on.
    pub fn is_established(&self) -> bool {
        self.estimate.worth_acting_on(MIN_TRIALS)
    }

    /// The share of trials with the procedure that did not fail.
    pub fn reliability(&self) -> Option<f64> {
        self.estimate.treatment.mean().map(|rate| 1.0 - rate)
    }

    /// Whether this is ready to be offered to the user as a capability.
    pub fn is_promotable(&self, min_reliability: f64) -> bool {
        self.is_established() && self.reliability().is_some_and(|r| r >= min_reliability)
    }
}

/// A procedure CoreScout would like to turn into a capability.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    /// The procedure.
    pub procedure: Procedure,
    /// The grounds.
    pub basis: Basis,
    /// What CoreScout would call it.
    pub name: String,
    /// Why it thinks this is worth having, in one sentence.
    pub because: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn procedure() -> Procedure {
        Procedure::new(
            "p1",
            "cargo build",
            Some("app".into()),
            "Regenerate schema, then build",
            vec![
                Step::act(
                    "cargo run --bin gen-schema",
                    "the build reads a generated file",
                ),
                Step::act("cargo build", "the operation being replaced"),
                Step::verify(
                    "check the binary was produced",
                    "so success is not just an exit code",
                ),
            ],
            0x51EED,
        )
    }

    #[test]
    fn observation_alone_never_produces_a_causal_basis() {
        // The rule this whole workspace is built on, applied to agent
        // operations. Two hundred chosen trials establish nothing.
        let mut procedure = procedure();
        for n in 0..200 {
            procedure.record(true, n % 20 == 0, false, 1000);
            procedure.record(false, n % 2 == 0, false, 1000);
        }
        assert!(!procedure.is_established());
        assert!(matches!(procedure.basis(), Some(Basis::Association { .. })));
        assert!(procedure.missing().is_some());
    }

    #[test]
    fn randomised_trials_on_both_arms_produce_a_causal_basis() {
        let mut procedure = procedure();
        for n in 0..40 {
            procedure.record(true, n % 14 == 0, true, 1000);
            procedure.record(false, n % 2 == 0, true, 1000);
        }
        let basis = procedure.basis().expect("a basis");
        assert!(basis.is_causal(), "{basis:?}");
        assert!(procedure.is_established());
    }

    #[test]
    fn randomised_trials_on_one_arm_only_are_not_enough() {
        let mut procedure = procedure();
        for _ in 0..40 {
            procedure.record(true, false, true, 1000);
            procedure.record(false, true, false, 1000);
        }
        assert!(!procedure.is_established());
        assert!(matches!(
            procedure.missing(),
            Some(Insufficient::OneArmed { .. })
        ));
    }

    #[test]
    fn the_coin_is_the_only_source_of_the_randomised_flag() {
        // If a procedure ever reports randomised trials it did not get from
        // the exploration policy, every causal number downstream is wrong.
        let mut procedure = procedure();
        let mut randomised = 0;
        for _ in 0..2000 {
            let (_, was_random) = procedure.decide(true);
            if was_random {
                randomised += 1;
            }
        }
        assert!(randomised > 0, "some trials must be randomised");
        assert!(
            randomised < 600,
            "{randomised} of 2000 is too much of a user's real work"
        );
    }

    #[test]
    fn a_settled_procedure_keeps_exploring_at_a_lower_rate() {
        // Stopping entirely would be cheaper and wrong: a repository changes,
        // and a procedure that is never re-tested keeps acting on evidence
        // about a repository that no longer exists.
        let mut procedure = procedure();
        for n in 0..40 {
            procedure.record(true, n % 14 == 0, true, 1000);
            procedure.record(false, n % 2 == 0, true, 1000);
        }
        assert!(procedure.estimate.is_settled(MIN_TRIALS));
        let before = procedure.exploration.spent();
        for _ in 0..2000 {
            procedure.decide(true);
        }
        let after = procedure.exploration.spent();
        assert!(after > before, "a settled procedure still explores");
    }

    #[test]
    fn reliability_is_the_share_of_applications_that_did_not_fail() {
        let mut procedure = procedure();
        for n in 0..20 {
            procedure.record(true, n < 2, true, 1000);
        }
        let reliability = procedure.reliability().expect("a reliability");
        assert!((reliability - 0.9).abs() < 0.05, "{reliability}");
    }

    #[test]
    fn promotion_needs_both_randomised_evidence_and_a_high_success_rate() {
        let mut procedure = procedure();
        // Randomised on both arms, and it genuinely helps, but a third of the
        // runs still fail: a real improvement is not automatically a
        // capability anyone should be offered.
        for n in 0..40 {
            procedure.record(true, n % 3 == 0, true, 1000);
            procedure.record(false, true, true, 1000);
        }
        assert!(procedure.is_established());
        assert!(!procedure.is_promotable(0.9), "67% is not a capability");
        assert!(procedure.is_promotable(0.5));
    }

    #[test]
    fn a_procedure_with_no_trials_claims_nothing() {
        let procedure = procedure();
        assert_eq!(procedure.basis(), None);
        assert_eq!(procedure.reliability(), None);
        assert!(!procedure.is_established());
        assert!(!procedure.is_promotable(0.0));
    }

    #[test]
    fn steps_distinguish_doing_from_checking() {
        // The verification step is what turns "it exited zero" into "it
        // worked", which is the difference the whole product turns on.
        let procedure = procedure();
        assert_eq!(procedure.steps.iter().filter(|s| s.verifies).count(), 1);
    }

    #[test]
    fn a_procedure_survives_storage() {
        let mut procedure = procedure();
        procedure.record(true, false, true, 1000);
        let json = serde_json::to_string(&procedure).expect("serialise");
        assert_eq!(
            serde_json::from_str::<Procedure>(&json).expect("deserialise"),
            procedure
        );
    }
}
