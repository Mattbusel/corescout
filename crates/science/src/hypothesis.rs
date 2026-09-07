//! A statement about the machine that an observation could kill.
//!
//! # The one rule
//!
//! A [`Hypothesis`] cannot be constructed without a prediction specific enough
//! that some possible reflection would refute it. [`Hypothesis::new`] returns
//! `None` for a statement no evidence could contradict, and that check is the
//! whole difference between this crate and the curiosity module it grew out of.
//!
//! `Curiosity` probes the action it knows least about and records what
//! happened. That is data collection. It has no way to be *wrong*, because it
//! never committed to anything in advance. A hypothesis commits first:
//!
//! ```text
//! curiosity:   "try this and see"          -> cannot be refuted
//! hypothesis:  "this will do X, +/- t"     -> refuted if it does not
//! ```
//!
//! # Why the tolerance is part of the hypothesis
//!
//! "Cell 4 will rise" is unfalsifiable in practice: any noise satisfies it. The
//! prediction carries a magnitude and a tolerance, both fixed before the
//! evidence arrives, so the verdict is a comparison rather than a judgement
//! call made after seeing the answer.
//!
//! A tolerance wide enough to admit any plausible observation is itself an
//! unfalsifiable statement, and [`Hypothesis::new`] rejects it.

use serde::{Deserialize, Serialize};

use corescout_represent::latent::LatentStateId;

/// What a hypothesis is about.
///
/// Each variant names something the machine could be wrong about, and each
/// carries enough detail to compute a numeric expectation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Claim {
    /// Being in this state predicts this cell holds this value.
    ///
    /// The most basic claim a discovered state can make for itself: that
    /// knowing it tells you something.
    StatePredicts {
        state: LatentStateId,
        row: usize,
        col: usize,
    },
    /// Acting in this family changes this cell by this much.
    ActionCauses {
        family: String,
        row: usize,
        col: usize,
    },
    /// These two cells move together.
    Couples {
        row_a: usize,
        col_a: usize,
        row_b: usize,
        col_b: usize,
    },
    /// From this state, the machine next goes to that one.
    StateTransitions {
        from: LatentStateId,
        to: LatentStateId,
    },
    /// Being in this state is worth something for this objective.
    ///
    /// The claim that turns a discovered state from a curiosity into a reason
    /// to act.
    StateFavours {
        state: LatentStateId,
        objective: String,
    },
    /// Under this condition, taking this action rather than that one changes
    /// the outcome.
    ///
    /// A strictly stronger claim than any of the above, and the only one that
    /// justifies acting. It cannot be settled by watching: see
    /// [`crate::causal`], where the evidence for it is required to come from
    /// randomised assignment.
    ///
    /// `under` is a name rather than a typed id so this crate stays below the
    /// concept crate in the dependency graph. It holds whatever coined the
    /// condition: `concept_17`, `latent_state_3`.
    ActionCausesUnder {
        under: String,
        family: String,
        alternative: String,
        objective: String,
    },
}

impl Claim {
    /// A stable key, so the same claim made twice is recognised as the same.
    pub fn key(&self) -> String {
        match self {
            Claim::StatePredicts { state, row, col } => {
                format!("predicts/{state}/{row}/{col}")
            }
            Claim::ActionCauses { family, row, col } => {
                format!("causes/{family}/{row}/{col}")
            }
            Claim::Couples {
                row_a,
                col_a,
                row_b,
                col_b,
            } => format!("couples/{row_a}/{col_a}/{row_b}/{col_b}"),
            Claim::StateTransitions { from, to } => format!("transitions/{from}/{to}"),
            Claim::StateFavours { state, objective } => format!("favours/{state}/{objective}"),
            Claim::ActionCausesUnder {
                under,
                family,
                alternative,
                objective,
            } => format!("causes_under/{under}/{family}vs{alternative}/{objective}"),
        }
    }

    /// The claim in words, using the machine's own names for what it found.
    ///
    /// `latent_state_13` stays `latent_state_13`. It is not translated into
    /// whatever we suspect it corresponds to; that correspondence, if it turns
    /// out to hold, is a finding rather than a naming convention.
    pub fn describe(&self) -> String {
        match self {
            Claim::StatePredicts { state, row, col } => {
                format!("being in {state} predicts the value of cell ({row},{col})")
            }
            Claim::ActionCauses { family, row, col } => {
                format!("acting on {family} changes cell ({row},{col})")
            }
            Claim::Couples {
                row_a,
                col_a,
                row_b,
                col_b,
            } => format!("cells ({row_a},{col_a}) and ({row_b},{col_b}) move together"),
            Claim::StateTransitions { from, to } => format!("{from} is followed by {to}"),
            Claim::StateFavours { state, objective } => {
                format!("being in {state} is good for {objective}")
            }
            Claim::ActionCausesUnder {
                under,
                family,
                alternative,
                objective,
            } => format!(
                "while in {under}, doing {family} rather than {alternative} changes {objective}"
            ),
        }
    }
}

impl Claim {
    /// Whether settling this claim requires intervening rather than watching.
    ///
    /// The distinction the whole causal module exists to enforce. A predictive
    /// claim can be tested against reflections as they arrive; a causal one
    /// cannot be tested at all without sometimes doing the other thing.
    pub fn needs_intervention(&self) -> bool {
        matches!(self, Claim::ActionCausesUnder { .. })
    }

    /// The condition this claim is conditional on, where it has one.
    pub fn condition(&self) -> Option<String> {
        match self {
            Claim::ActionCausesUnder { under, .. } => Some(under.clone()),
            Claim::StatePredicts { state, .. } | Claim::StateFavours { state, .. } => {
                Some(state.to_string())
            }
            _ => None,
        }
    }
}

/// A committed numeric expectation.
///
/// Fixed before the evidence arrives. Everything about the verdict follows from
/// comparing an observation to this, which is what stops a hypothesis from
/// being quietly reinterpreted once the answer is known.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expectation {
    /// The value the hypothesis commits to.
    pub value: f64,
    /// How far off the observation may be and still count as agreement.
    pub tolerance: f64,
    /// The spread of observations that would be normal if the hypothesis were
    /// *false*. A prediction inside the noise is not a prediction.
    pub null_spread: f64,
}

impl Expectation {
    pub fn new(value: f64, tolerance: f64, null_spread: f64) -> Expectation {
        Expectation {
            value,
            tolerance,
            null_spread,
        }
    }

    /// Whether an observation agrees.
    pub fn agrees_with(&self, observed: f64) -> bool {
        if !observed.is_finite() {
            return false;
        }
        (observed - self.value).abs() <= self.tolerance
    }

    /// Whether this expectation could be contradicted by any observation.
    ///
    /// Three ways to fail:
    ///
    /// - a non-finite value or tolerance, which cannot be compared to anything;
    /// - a tolerance so wide that the ordinary variation of the quantity fits
    ///   inside it, so no plausible observation lands outside;
    /// - a tolerance of zero on a continuous quantity, which is refuted by
    ///   arithmetic rather than by evidence.
    pub fn is_falsifiable(&self) -> bool {
        if !self.value.is_finite() || !self.tolerance.is_finite() {
            return false;
        }
        if self.tolerance <= 0.0 {
            return false;
        }
        // The prediction must be sharper than the noise it is competing with.
        // Equal is not enough: a hypothesis that predicts exactly the spread of
        // doing nothing has said nothing.
        self.null_spread.is_finite() && self.tolerance < self.null_spread
    }

    /// How much sharper this prediction is than the null spread.
    ///
    /// 1.0 means "no sharper than noise". Larger is a bolder claim, and a bold
    /// claim that survives is worth more than a timid one that survives.
    pub fn boldness(&self) -> f64 {
        if self.tolerance <= 0.0 || !self.null_spread.is_finite() {
            return 0.0;
        }
        self.null_spread / self.tolerance
    }
}

/// A falsifiable statement, with the evidence gathered so far.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hypothesis {
    pub claim: Claim,
    pub expectation: Expectation,
    /// Reflections consistent with it.
    pub corroborations: u32,
    /// Reflections that contradicted it.
    pub refutations: u32,
    /// Sum of absolute errors, for reporting how wrong it was when wrong.
    error_total: f64,
    /// When it was first stated, so a young hypothesis is not mistaken for a
    /// well-tested one.
    pub proposed_ns: u64,
    /// Set when the hypothesis is retired, with the reason.
    pub retired: Option<String>,
}

impl Hypothesis {
    /// State a hypothesis, or decline to.
    ///
    /// Returns `None` when the expectation could not be contradicted by any
    /// observation. That is the Popper criterion, made mechanical: a statement
    /// that forbids nothing is not admitted, however true it may be.
    pub fn new(claim: Claim, expectation: Expectation, now_ns: u64) -> Option<Hypothesis> {
        if !expectation.is_falsifiable() {
            return None;
        }
        Some(Hypothesis {
            claim,
            expectation,
            corroborations: 0,
            refutations: 0,
            error_total: 0.0,
            proposed_ns: now_ns,
            retired: None,
        })
    }

    /// Test one observation against the commitment.
    pub fn test(&mut self, observed: f64) -> bool {
        let agrees = self.expectation.agrees_with(observed);
        if agrees {
            self.corroborations += 1;
        } else {
            self.refutations += 1;
            if observed.is_finite() {
                self.error_total += (observed - self.expectation.value).abs();
            }
        }
        agrees
    }

    pub fn trials(&self) -> u32 {
        self.corroborations + self.refutations
    }

    /// Fraction of trials that agreed.
    pub fn support(&self) -> f64 {
        if self.trials() == 0 {
            return 0.0;
        }
        self.corroborations as f64 / self.trials() as f64
    }

    /// Mean size of the error on the trials that disagreed.
    pub fn mean_error(&self) -> Option<f64> {
        (self.refutations > 0).then(|| self.error_total / self.refutations as f64)
    }

    pub fn is_retired(&self) -> bool {
        self.retired.is_some()
    }

    /// Where the hypothesis currently stands.
    ///
    /// The thresholds are deliberately asymmetric. A hypothesis needs a good
    /// deal of agreement to be called supported, and rather little
    /// disagreement to be called refuted, because a single reliable
    /// contradiction is worth more than many agreements: agreement is also what
    /// a vacuous statement produces.
    pub fn verdict(&self, min_trials: u32) -> Verdict {
        if let Some(reason) = &self.retired {
            return Verdict::Retired(reason.clone());
        }
        if self.trials() < min_trials {
            return Verdict::Untested {
                trials: self.trials(),
                needed: min_trials,
            };
        }
        let support = self.support();
        if support >= 0.85 {
            Verdict::Supported {
                support,
                trials: self.trials(),
            }
        } else if support <= 0.5 {
            Verdict::Refuted {
                support,
                trials: self.trials(),
                mean_error: self.mean_error().unwrap_or(f64::NAN),
            }
        } else {
            Verdict::Inconclusive {
                support,
                trials: self.trials(),
            }
        }
    }

    /// Withdraw the hypothesis, with a reason.
    pub fn retire(&mut self, reason: impl Into<String>) {
        self.retired = Some(reason.into());
    }

    /// A one-line rendering, evidence included.
    pub fn summary(&self, min_trials: u32) -> String {
        format!(
            "{}  [{}]",
            self.claim.describe(),
            self.verdict(min_trials).describe()
        )
    }
}

/// Where a hypothesis stands against the evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Not enough trials to say anything.
    Untested { trials: u32, needed: u32 },
    /// The evidence agrees.
    Supported { support: f64, trials: u32 },
    /// The evidence disagrees. **This is the point of the crate.**
    Refuted {
        support: f64,
        trials: u32,
        mean_error: f64,
    },
    /// Tested, and the evidence does not settle it.
    Inconclusive { support: f64, trials: u32 },
    /// Withdrawn.
    Retired(String),
}

impl Verdict {
    pub fn describe(&self) -> String {
        match self {
            Verdict::Untested { trials, needed } => {
                format!("untested: {trials} of {needed} trials")
            }
            Verdict::Supported { support, trials } => {
                format!("supported: {:.0}% of {trials} trials", support * 100.0)
            }
            Verdict::Refuted {
                support,
                trials,
                mean_error,
            } => format!(
                "REFUTED: only {:.0}% of {trials} trials, off by {mean_error:.3} on average",
                support * 100.0
            ),
            Verdict::Inconclusive { support, trials } => {
                format!("inconclusive: {:.0}% of {trials} trials", support * 100.0)
            }
            Verdict::Retired(reason) => format!("retired: {reason}"),
        }
    }

    pub fn is_supported(&self) -> bool {
        matches!(self, Verdict::Supported { .. })
    }

    pub fn is_refuted(&self) -> bool {
        matches!(self, Verdict::Refuted { .. })
    }

    /// Whether more evidence could still change this.
    pub fn is_open(&self) -> bool {
        matches!(
            self,
            Verdict::Untested { .. } | Verdict::Inconclusive { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim() -> Claim {
        Claim::StatePredicts {
            state: LatentStateId(3),
            row: 1,
            col: 2,
        }
    }

    #[test]
    fn a_statement_no_observation_could_contradict_is_not_admitted() {
        // The central rule. A tolerance as wide as the noise forbids nothing.
        let vacuous = Expectation::new(10.0, 5.0, 5.0);
        assert!(!vacuous.is_falsifiable());
        assert!(Hypothesis::new(claim(), vacuous, 0).is_none());

        let wider_than_noise = Expectation::new(10.0, 50.0, 5.0);
        assert!(Hypothesis::new(claim(), wider_than_noise, 0).is_none());
    }

    #[test]
    fn an_infinitely_precise_claim_is_also_not_admitted() {
        // Refuted by arithmetic rather than by evidence, which is not science
        // either.
        let exact = Expectation::new(10.0, 0.0, 5.0);
        assert!(!exact.is_falsifiable());
        assert!(Hypothesis::new(claim(), exact, 0).is_none());
    }

    #[test]
    fn a_non_finite_expectation_is_not_admitted() {
        for bad in [
            Expectation::new(f64::NAN, 1.0, 5.0),
            Expectation::new(1.0, f64::NAN, 5.0),
            Expectation::new(1.0, 1.0, f64::NAN),
            Expectation::new(f64::INFINITY, 1.0, 5.0),
        ] {
            assert!(!bad.is_falsifiable(), "{bad:?} should not be falsifiable");
        }
    }

    #[test]
    fn a_sharp_claim_is_admitted_and_records_its_boldness() {
        let sharp = Expectation::new(10.0, 1.0, 8.0);
        assert!(sharp.is_falsifiable());
        assert!((sharp.boldness() - 8.0).abs() < 1e-9);
        assert!(Hypothesis::new(claim(), sharp, 0).is_some());
    }

    #[test]
    fn evidence_that_disagrees_produces_a_refutation() {
        // The verdict this whole crate exists to be able to reach.
        let mut hypothesis =
            Hypothesis::new(claim(), Expectation::new(10.0, 1.0, 8.0), 0).expect("admitted");
        for _ in 0..10 {
            hypothesis.test(50.0);
        }
        let verdict = hypothesis.verdict(5);
        assert!(verdict.is_refuted(), "{}", verdict.describe());
        assert!(verdict.describe().contains("REFUTED"));
        assert!((hypothesis.mean_error().unwrap() - 40.0).abs() < 1e-9);
    }

    #[test]
    fn evidence_that_agrees_produces_support() {
        let mut hypothesis =
            Hypothesis::new(claim(), Expectation::new(10.0, 1.0, 8.0), 0).expect("admitted");
        for _ in 0..10 {
            hypothesis.test(10.3);
        }
        assert!(hypothesis.verdict(5).is_supported());
        assert_eq!(hypothesis.mean_error(), None);
    }

    #[test]
    fn a_hypothesis_is_untested_until_it_has_been_tried_enough() {
        let mut hypothesis =
            Hypothesis::new(claim(), Expectation::new(10.0, 1.0, 8.0), 0).expect("admitted");
        hypothesis.test(10.0);
        let verdict = hypothesis.verdict(20);
        assert!(verdict.is_open());
        assert!(!verdict.is_supported(), "one trial is not support");
    }

    #[test]
    fn mixed_evidence_is_inconclusive_rather_than_rounded_to_a_side() {
        let mut hypothesis =
            Hypothesis::new(claim(), Expectation::new(10.0, 1.0, 8.0), 0).expect("admitted");
        for _ in 0..7 {
            hypothesis.test(10.0);
        }
        for _ in 0..3 {
            hypothesis.test(90.0);
        }
        let verdict = hypothesis.verdict(5);
        assert!(verdict.is_open());
        assert!(!verdict.is_supported() && !verdict.is_refuted());
    }

    #[test]
    fn refutation_needs_less_evidence_than_support() {
        // Asymmetric on purpose: agreement is also what a vacuous statement
        // produces, so it is worth less per trial than disagreement.
        let mut weak =
            Hypothesis::new(claim(), Expectation::new(10.0, 1.0, 8.0), 0).expect("admitted");
        for _ in 0..8 {
            weak.test(10.0);
        }
        for _ in 0..2 {
            weak.test(99.0);
        }
        // 80% agreement: not enough to be called supported.
        assert!(!weak.verdict(5).is_supported());

        let mut bad =
            Hypothesis::new(claim(), Expectation::new(10.0, 1.0, 8.0), 0).expect("admitted");
        for _ in 0..5 {
            bad.test(10.0);
        }
        for _ in 0..5 {
            bad.test(99.0);
        }
        // 50% agreement: refuted.
        assert!(bad.verdict(5).is_refuted());
    }

    #[test]
    fn an_unobservable_trial_counts_against_rather_than_being_skipped() {
        // A prediction about a cell the machine cannot see has not been
        // confirmed, and silently skipping it would let a hypothesis survive on
        // the strength of never being tested.
        let mut hypothesis =
            Hypothesis::new(claim(), Expectation::new(10.0, 1.0, 8.0), 0).expect("admitted");
        assert!(!hypothesis.test(f64::NAN));
        assert_eq!(hypothesis.refutations, 1);
    }

    #[test]
    fn a_retired_hypothesis_says_so_whatever_its_evidence() {
        let mut hypothesis =
            Hypothesis::new(claim(), Expectation::new(10.0, 1.0, 8.0), 0).expect("admitted");
        for _ in 0..10 {
            hypothesis.test(10.0);
        }
        hypothesis.retire("the state it referred to was consolidated away");
        assert!(hypothesis.is_retired());
        assert!(matches!(hypothesis.verdict(5), Verdict::Retired(_)));
    }

    #[test]
    fn claim_keys_distinguish_different_claims_and_match_identical_ones() {
        let a = claim();
        let b = Claim::StatePredicts {
            state: LatentStateId(3),
            row: 1,
            col: 2,
        };
        let c = Claim::StatePredicts {
            state: LatentStateId(4),
            row: 1,
            col: 2,
        };
        assert_eq!(a.key(), b.key());
        assert_ne!(a.key(), c.key());
    }

    #[test]
    fn a_claim_describes_itself_using_the_machines_own_names() {
        // `latent_state_3`, not "the thermal state". The correspondence, if
        // there is one, is a finding rather than a naming convention.
        let text = claim().describe();
        assert!(text.contains("latent_state_3"), "{text}");
    }

    #[test]
    fn a_hypothesis_round_trips_through_json() {
        let mut hypothesis =
            Hypothesis::new(claim(), Expectation::new(10.0, 1.0, 8.0), 42).expect("admitted");
        hypothesis.test(10.0);
        hypothesis.test(99.0);
        let text = serde_json::to_string(&hypothesis).expect("serialises");
        let back: Hypothesis = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back, hypothesis);
    }
}
