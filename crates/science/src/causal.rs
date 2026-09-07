//! Telling "this happened while C held" apart from "C made this happen".
//!
//! # The failure this module exists to make impossible
//!
//! ```text
//! I acted while concept C was present.
//! Things improved.
//! Therefore C caused the improvement.
//! ```
//!
//! Every step there is individually reasonable and the conclusion is
//! unfounded. It is also the single easiest way for this whole project to
//! produce impressive nonsense, because the machinery that coins C and the
//! machinery that observes the improvement are both looking at the same data.
//!
//! The defence is structural rather than disciplinary. [`CausalEstimate`] holds
//! two arms and two counters, and [`CausalEstimate::effect`] returns `None`
//! unless there are **randomised** trials on both. Observational data can only
//! produce [`CausalEstimate::association`], which is a different method with a
//! different name returning a different type, so a caller cannot reach for one
//! and get the other by accident.
//!
//! # Two claim classes, deliberately not interchangeable
//!
//! | claim | evidence that settles it | where |
//! |---|---|---|
//! | `C predicts X` | passive observation | [`crate::Claim::StatePredicts`] |
//! | `under C, action A causes X` | randomised assignment between A and A' | [`crate::Claim::ActionCausesUnder`] |
//!
//! The second is strictly harder to establish, and the type system reflects
//! that: you cannot construct evidence for it by watching.
//!
//! # Why randomisation, and what it costs
//!
//! When the machine is in C and believes A is right, it takes A. Every such
//! trial is confounded: A was chosen *because* of something, and that something
//! may be what produced the outcome.
//!
//! For a small fraction of trials the choice is made by coin flip instead. Those
//! trials cost something, because sometimes the coin says to do the thing the
//! machine believes is worse. That cost is the price of knowing, and it is
//! bounded by [`Exploration::rate`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One side of a comparison: what happened when this action was taken.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Arm {
    /// Trials of any kind.
    pub trials: u32,
    /// Trials where the action was chosen by coin flip rather than by belief.
    ///
    /// The only ones that support a causal claim.
    pub randomised: u32,
    /// Effective count after forgetting. Equal to `trials` when nothing is
    /// forgotten.
    weight: f64,
    sum: f64,
    sum_squares: f64,
    /// Sum and sum of squares over the randomised trials alone, kept separately
    /// so the causal estimate never touches the confounded ones.
    randomised_weight: f64,
    randomised_sum: f64,
    randomised_sum_squares: f64,
}

impl Arm {
    /// Record a trial, forgetting old evidence at `forgetting`.
    ///
    /// # Why the evidence has to be forgetful
    ///
    /// An arm that accumulates forever describes the average of every world the
    /// machine has ever been in. When the world changes, an established effect
    /// stays established: the old trials outnumber the new ones for a long time,
    /// and the machine keeps acting on a fact that has stopped being true.
    ///
    /// Weighting recent trials more heavily means the effective sample size
    /// falls when evidence stops arriving, so a comparison becomes *unsettled*
    /// again and exploration resumes on its own. That is the mechanism by which
    /// a belief can die rather than merely be outvoted.
    pub fn record_with(&mut self, outcome: f64, randomised: bool, forgetting: f64) {
        if !outcome.is_finite() {
            return;
        }
        let keep = forgetting.clamp(0.0, 1.0);
        self.trials += 1;
        self.weight = self.weight * keep + 1.0;
        self.sum = self.sum * keep + outcome;
        self.sum_squares = self.sum_squares * keep + outcome * outcome;
        if randomised {
            self.randomised += 1;
            self.randomised_weight = self.randomised_weight * keep + 1.0;
            self.randomised_sum = self.randomised_sum * keep + outcome;
            self.randomised_sum_squares = self.randomised_sum_squares * keep + outcome * outcome;
        } else {
            // A confounded trial still ages the randomised evidence: time has
            // passed and the world may have moved, whoever chose the action.
            self.randomised_weight *= keep;
            self.randomised_sum *= keep;
            self.randomised_sum_squares *= keep;
        }
    }

    /// Record a trial with no forgetting.
    pub fn record(&mut self, outcome: f64, randomised: bool) {
        self.record_with(outcome, randomised, 1.0);
    }

    /// Age this arm without recording anything.
    pub fn age(&mut self, forgetting: f64) {
        let keep = forgetting.clamp(0.0, 1.0);
        self.weight *= keep;
        self.sum *= keep;
        self.sum_squares *= keep;
        self.randomised_weight *= keep;
        self.randomised_sum *= keep;
        self.randomised_sum_squares *= keep;
    }

    /// Effective number of randomised trials, after forgetting.
    pub fn effective_randomised(&self) -> f64 {
        self.randomised_weight
    }

    /// Mean over every trial, confounded ones included.
    pub fn mean(&self) -> Option<f64> {
        (self.weight > 0.0).then(|| self.sum / self.weight)
    }

    /// Mean over randomised trials only.
    pub fn randomised_mean(&self) -> Option<f64> {
        (self.randomised_weight > 0.0).then(|| self.randomised_sum / self.randomised_weight)
    }

    /// Variance over randomised trials only.
    fn randomised_variance(&self) -> Option<f64> {
        if self.randomised_weight < 2.0 {
            return None;
        }
        let n = self.randomised_weight;
        let mean = self.randomised_sum / n;
        let variance = (self.randomised_sum_squares - n * mean * mean) / (n - 1.0);
        Some(variance.max(0.0))
    }
}

/// A measured causal effect, from randomised evidence only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalEffect {
    /// Mean outcome under the action, minus mean outcome under the alternative.
    pub delta: f64,
    /// Standard error of that difference.
    pub standard_error: f64,
    pub treatment_trials: u32,
    pub control_trials: u32,
}

impl CausalEffect {
    /// Whether the effect is distinguishable from zero.
    ///
    /// Two standard errors. Not a formal test, and deliberately conservative:
    /// this gates whether the machine will change its behaviour, so the cost of
    /// a false positive is an action taken on a belief that is not there.
    pub fn is_significant(&self) -> bool {
        self.standard_error > 0.0 && self.delta.abs() > 2.0 * self.standard_error
    }

    /// How many standard errors from zero.
    pub fn strength(&self) -> f64 {
        if self.standard_error <= 0.0 {
            return 0.0;
        }
        self.delta.abs() / self.standard_error
    }

    pub fn describe(&self, lower_is_better: bool) -> String {
        let direction = if (self.delta < 0.0) == lower_is_better {
            "better"
        } else {
            "worse"
        };
        format!(
            "{:.4} {direction} (+/- {:.4}, {} vs {} randomised trials){}",
            self.delta.abs(),
            self.standard_error,
            self.treatment_trials,
            self.control_trials,
            if self.is_significant() {
                ""
            } else {
                ", not distinguishable from no effect"
            }
        )
    }
}

/// What a comparison rests on, when it is not enough for a causal claim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Insufficient {
    /// Nothing has been randomised, so every trial is confounded.
    NoRandomisation { observational_trials: u32 },
    /// Randomised, but not enough of it.
    TooFewTrials {
        treatment: u32,
        control: u32,
        needed: u32,
    },
    /// One arm was never tried under the coin flip.
    OneArmed { treatment: u32, control: u32 },
}

impl Insufficient {
    pub fn describe(&self) -> String {
        match self {
            // Zero is its own case. "0 trials, none randomised: the action was
            // always chosen for a reason" describes trials that never
            // happened, which reads as a broken sentence and, worse, implies
            // CoreScout looked and found confounding where it has not looked.
            Insufficient::NoRandomisation {
                observational_trials: 0,
            } => "not tried under randomised assignment yet, so there is nothing to compare".into(),
            Insufficient::NoRandomisation {
                observational_trials,
            } => format!(
                "{observational_trials} trials, none randomised: the action was always \
                 chosen for a reason, so its outcome cannot be separated from that reason"
            ),
            Insufficient::TooFewTrials {
                treatment,
                control,
                needed,
            } => {
                format!("{treatment} and {control} randomised trials, {needed} needed on each side")
            }
            Insufficient::OneArmed { treatment, control } => format!(
                "{treatment} and {control} randomised trials: a comparison needs both sides"
            ),
        }
    }
}

/// The evidence for and against one intervention, under one condition.
///
/// "Under `under`, does taking `family` change the outcome relative to
/// `alternative`?"
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalEstimate {
    /// The condition this comparison is made under, named by whatever coined
    /// it: `concept_17`, `latent_state_3`.
    pub under: String,
    /// The action family under test.
    pub family: String,
    /// What it is being compared against. A causal claim is always relative to
    /// something; "A works" without an alternative is not a comparison.
    pub alternative: String,
    /// Whether a smaller outcome is a better one.
    pub lower_is_better: bool,
    /// How fast old evidence is forgotten, per trial. 1.0 forgets nothing.
    ///
    /// Each trial ages **both** arms: the one being measured and the one that
    /// was not, because a comparison is between two things as they are now and
    /// only one of them was just observed. So an arm decays twice per pair of
    /// trials, and the effective window is roughly
    /// `forgetting / (1 - forgetting^2)` trials rather than the
    /// `1 / (1 - forgetting)` the factor alone would suggest.
    ///
    /// At the default 0.995 that is about a hundred trials per arm.
    pub forgetting: f64,
    pub treatment: Arm,
    pub control: Arm,
}

impl CausalEstimate {
    pub fn new(
        under: impl Into<String>,
        family: impl Into<String>,
        alternative: impl Into<String>,
        lower_is_better: bool,
    ) -> CausalEstimate {
        CausalEstimate {
            under: under.into(),
            family: family.into(),
            alternative: alternative.into(),
            lower_is_better,
            forgetting: 1.0,
            treatment: Arm::default(),
            control: Arm::default(),
        }
    }

    /// Set how fast this comparison forgets. See [`Arm::record_with`].
    pub fn forgetting(mut self, forgetting: f64) -> CausalEstimate {
        self.forgetting = forgetting.clamp(0.0, 1.0);
        self
    }

    /// A stable key, so the same comparison is recognised across runs.
    pub fn key(&self) -> String {
        format!("{}/{}vs{}", self.under, self.family, self.alternative)
    }

    /// Record a trial.
    ///
    /// `randomised` must be true only when the action was chosen by coin flip
    /// rather than by belief. Passing true for a chosen action is not a bug the
    /// type system can catch, and it would silently destroy every guarantee in
    /// this module, so the one caller that sets it lives in
    /// [`Exploration::decide`] and returns the flag alongside the choice.
    pub fn record(&mut self, took_treatment: bool, outcome: f64, randomised: bool) {
        let forgetting = self.forgetting;
        if took_treatment {
            self.treatment.record_with(outcome, randomised, forgetting);
            // The other arm ages too: a comparison is between two things as
            // they are now, and only one of them was measured just now.
            self.control.age(forgetting);
        } else {
            self.control.record_with(outcome, randomised, forgetting);
            self.treatment.age(forgetting);
        }
    }

    /// The difference in means over **all** trials.
    ///
    /// Deliberately named "association". This number is what a naive
    /// implementation would call the effect, and it is exactly the number that
    /// is wrong when the action was chosen for a reason.
    pub fn association(&self) -> Option<f64> {
        Some(self.treatment.mean()? - self.control.mean()?)
    }

    /// The causal effect, from randomised trials only.
    ///
    /// `Err` says what is missing rather than returning a number with a caveat
    /// attached, because a number with a caveat gets used and the caveat gets
    /// dropped.
    pub fn effect(&self, min_trials: u32) -> Result<CausalEffect, Insufficient> {
        if self.treatment.randomised == 0 && self.control.randomised == 0 {
            return Err(Insufficient::NoRandomisation {
                observational_trials: self.treatment.trials + self.control.trials,
            });
        }
        if self.treatment.randomised == 0 || self.control.randomised == 0 {
            return Err(Insufficient::OneArmed {
                treatment: self.treatment.randomised,
                control: self.control.randomised,
            });
        }
        // Effective, not raw: evidence that has aged out no longer settles
        // anything, which is what lets a comparison reopen when the world moves.
        if self.treatment.effective_randomised() < min_trials as f64
            || self.control.effective_randomised() < min_trials as f64
        {
            return Err(Insufficient::TooFewTrials {
                treatment: self.treatment.effective_randomised().round() as u32,
                control: self.control.effective_randomised().round() as u32,
                needed: min_trials,
            });
        }

        let treatment_mean = self.treatment.randomised_mean().unwrap_or(f64::NAN);
        let control_mean = self.control.randomised_mean().unwrap_or(f64::NAN);
        let treatment_var = self.treatment.randomised_variance().unwrap_or(0.0);
        let control_var = self.control.randomised_variance().unwrap_or(0.0);
        let standard_error = (treatment_var / self.treatment.effective_randomised()
            + control_var / self.control.effective_randomised())
        .sqrt();

        Ok(CausalEffect {
            delta: treatment_mean - control_mean,
            standard_error,
            treatment_trials: self.treatment.effective_randomised().round() as u32,
            control_trials: self.control.effective_randomised().round() as u32,
        })
    }

    /// Whether enough randomised evidence exists to reach a verdict either way.
    ///
    /// Exploration exists to gather evidence. Once there is enough to settle the
    /// question, continuing to randomise buys nothing and costs a fraction of
    /// every decision, so a policy should stop paying for it here rather than
    /// exploring at a fixed rate forever.
    ///
    /// True for a settled null as well as a settled effect: "this action makes
    /// no difference here" is knowledge, and re-testing it indefinitely is the
    /// same waste as re-testing a known win.
    pub fn is_settled(&self, min_trials: u32) -> bool {
        self.effect(min_trials).is_ok()
    }

    /// Whether the intervention is worth preferring under this condition.
    ///
    /// Requires a significant effect in the right direction, established from
    /// randomised evidence. Nothing else counts.
    pub fn worth_acting_on(&self, min_trials: u32) -> bool {
        match self.effect(min_trials) {
            Ok(effect) => effect.is_significant() && (effect.delta < 0.0) == self.lower_is_better,
            Err(_) => false,
        }
    }

    /// How the association and the effect compare.
    ///
    /// Worth printing. When they disagree badly, the confounding was real and
    /// the randomisation earned its cost.
    pub fn describe(&self, min_trials: u32) -> String {
        let association = match self.association() {
            Some(value) => format!("{value:+.4}"),
            None => "none".into(),
        };
        match self.effect(min_trials) {
            Ok(effect) => format!(
                "under {}: {} vs {} -> association {association}, causal effect {}",
                self.under,
                self.family,
                self.alternative,
                effect.describe(self.lower_is_better)
            ),
            Err(missing) => format!(
                "under {}: {} vs {} -> association {association}, but no causal claim: {}",
                self.under,
                self.family,
                self.alternative,
                missing.describe()
            ),
        }
    }
}

/// How much of the machine's behaviour is given over to finding out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exploration {
    /// Fraction of eligible decisions made by coin flip.
    ///
    /// Small. Every randomised trial is one where the machine may deliberately
    /// do what it believes is worse, and that cost is paid on a real machine
    /// running someone's real work.
    pub rate: f64,
    /// Randomised trials needed on each arm before an effect is reported.
    pub min_trials: u32,
    /// Total randomised trials permitted, so exploration cannot run forever.
    pub budget: u32,
    spent: u32,
    seed: u64,
}

impl Default for Exploration {
    fn default() -> Self {
        Exploration::new(0.1, 12, 500, 0x5EED)
    }
}

impl Exploration {
    pub fn new(rate: f64, min_trials: u32, budget: u32, seed: u64) -> Exploration {
        Exploration {
            rate: rate.clamp(0.0, 1.0),
            min_trials,
            budget,
            spent: 0,
            // Only zero needs replacing: it is a fixed point of xorshift.
            // Forcing the low bit instead would collapse every adjacent pair of
            // seeds onto one generator, so seeds 42 and 43 would produce
            // identical runs and two "independent" arms would not be.
            seed: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    pub fn spent(&self) -> u32 {
        self.spent
    }

    pub fn remaining(&self) -> u32 {
        self.budget.saturating_sub(self.spent)
    }

    pub fn exhausted(&self) -> bool {
        self.remaining() == 0
    }

    /// Decide whether this trial is randomised, and if so which arm.
    ///
    /// Returns `(take_treatment, randomised)`. When not randomising, the
    /// caller's `preferred` is returned untouched, so belief drives behaviour
    /// except on the fraction of trials given over to finding out.
    ///
    /// The single place `randomised = true` originates. Everything in this
    /// module depends on that flag being honest, so it is produced here and
    /// nowhere else.
    pub fn decide(&mut self, preferred: bool) -> (bool, bool) {
        self.decide_scaled(preferred, 1.0)
    }

    /// Decide, with the exploration rate scaled.
    ///
    /// A settled comparison passes a small scale rather than zero. Stopping
    /// entirely would be cheaper and is wrong: the world can change while the
    /// condition persists, and a comparison that is never revisited would keep
    /// driving behaviour from evidence gathered about a machine that no longer
    /// exists. A residual rate is the cost of staying able to notice.
    pub fn decide_scaled(&mut self, preferred: bool, scale: f64) -> (bool, bool) {
        let rate = self.rate * scale.clamp(0.0, 1.0);
        if self.exhausted() || rate <= 0.0 {
            return (preferred, false);
        }
        if self.next_uniform() >= rate {
            return (preferred, false);
        }
        self.spent += 1;
        // A fair coin between the arms, not "the opposite of what I believe":
        // always flipping to the alternative would make the randomised arms
        // unbalanced and the estimate biased.
        (self.next_uniform() < 0.5, true)
    }

    fn next_uniform(&mut self) -> f64 {
        // xorshift64*, so a run is reproducible and two policies see the same
        // coin flips. Reproducibility matters more than cryptographic quality:
        // an experiment that cannot be repeated is not an experiment.
        self.seed ^= self.seed >> 12;
        self.seed ^= self.seed << 25;
        self.seed ^= self.seed >> 27;
        let value = self.seed.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (value >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Every causal comparison the machine is running.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attribution {
    #[serde(with = "corescout_core::serde_util::pairs")]
    estimates: BTreeMap<String, CausalEstimate>,
    /// Forgetting applied to every comparison started here.
    forgetting: f64,
}

impl Default for Attribution {
    fn default() -> Self {
        Attribution {
            estimates: BTreeMap::new(),
            // Roughly a two-hundred-trial window. Long enough to establish an
            // effect, short enough that a world which changed shows through.
            forgetting: 0.995,
        }
    }
}

impl Attribution {
    pub fn new() -> Attribution {
        Attribution::default()
    }

    /// How fast comparisons started here forget. 1.0 forgets nothing.
    pub fn forgetting_factor(&self) -> f64 {
        self.forgetting
    }

    /// An attribution that never forgets. For tests and for offline analysis of
    /// a recording, where the world is fixed by definition.
    pub fn remembering() -> Attribution {
        Attribution {
            estimates: BTreeMap::new(),
            forgetting: 1.0,
        }
    }

    pub fn len(&self) -> usize {
        self.estimates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.estimates.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&CausalEstimate> {
        self.estimates.get(key)
    }

    pub fn estimates(&self) -> impl Iterator<Item = &CausalEstimate> {
        self.estimates.values()
    }

    /// Start or fetch a comparison.
    pub fn comparison(
        &mut self,
        under: &str,
        family: &str,
        alternative: &str,
        lower_is_better: bool,
    ) -> &mut CausalEstimate {
        let estimate = CausalEstimate::new(under, family, alternative, lower_is_better)
            .forgetting(self.forgetting);
        self.estimates.entry(estimate.key()).or_insert(estimate)
    }

    /// Comparisons that have established a real effect.
    pub fn established(&self, min_trials: u32) -> Vec<&CausalEstimate> {
        self.estimates
            .values()
            .filter(|e| e.worth_acting_on(min_trials))
            .collect()
    }

    /// Forget everything learned under a condition that no longer exists.
    ///
    /// A concept that has been de-coined takes its causal claims with it. They
    /// are not false, they are meaningless: there is no longer a condition for
    /// them to be conditional on.
    pub fn forget(&mut self, under: &str) -> usize {
        let before = self.estimates.len();
        self.estimates.retain(|_, e| e.under != under);
        before - self.estimates.len()
    }

    /// Where association and causation disagree most.
    ///
    /// The most interesting output of this module. A large gap means the naive
    /// reading of the data would have been wrong, which is the whole reason for
    /// paying the cost of randomisation.
    pub fn most_confounded(&self, min_trials: u32) -> Option<(&CausalEstimate, f64)> {
        self.estimates
            .values()
            .filter_map(|estimate| {
                let association = estimate.association()?;
                let effect = estimate.effect(min_trials).ok()?;
                Some((estimate, (association - effect.delta).abs()))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
    }

    pub fn render(&self, min_trials: u32) -> String {
        if self.estimates.is_empty() {
            return "no causal comparisons are running\n".into();
        }
        let mut out = format!("{} comparisons:\n", self.estimates.len());
        for estimate in self.estimates.values() {
            out.push_str(&format!("  {}\n", estimate.describe(min_trials)));
        }
        if let Some((estimate, gap)) = self.most_confounded(min_trials) {
            if gap > 1e-9 {
                out.push_str(&format!(
                    "\nlargest gap between association and effect: {gap:.4} on {}. \
                     That gap is what randomisation bought.\n",
                    estimate.key()
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn estimate() -> CausalEstimate {
        CausalEstimate::new("concept_17", "affinity", "hold", true)
    }

    #[test]
    fn observational_data_never_produces_a_causal_effect() {
        // The central rule. A thousand confounded trials with a huge difference
        // still support no causal claim.
        let mut estimate = estimate();
        for _ in 0..500 {
            estimate.record(true, 1.0, false);
            estimate.record(false, 100.0, false);
        }
        assert_eq!(estimate.association(), Some(-99.0));
        let error = estimate.effect(5).unwrap_err();
        assert!(matches!(error, Insufficient::NoRandomisation { .. }));
        assert!(error.describe().contains("cannot be separated"));
        assert!(!estimate.worth_acting_on(5));
    }

    #[test]
    fn randomised_data_produces_an_effect_with_the_right_sign() {
        let mut estimate = estimate();
        for i in 0..40 {
            // Treatment is better (lower), with a little spread so the standard
            // error is not zero.
            estimate.record(true, 1.0 + (i % 3) as f64 * 0.1, true);
            estimate.record(false, 5.0 + (i % 3) as f64 * 0.1, true);
        }
        let effect = estimate.effect(5).expect("established");
        assert!((effect.delta + 4.0).abs() < 0.2, "delta {}", effect.delta);
        assert!(effect.is_significant());
        assert!(estimate.worth_acting_on(5));
        assert!(effect.describe(true).contains("better"));
    }

    #[test]
    fn an_effect_in_the_wrong_direction_is_not_worth_acting_on() {
        let mut estimate = estimate();
        for i in 0..40 {
            estimate.record(true, 5.0 + (i % 3) as f64 * 0.1, true);
            estimate.record(false, 1.0 + (i % 3) as f64 * 0.1, true);
        }
        assert!(estimate.effect(5).unwrap().is_significant());
        assert!(
            !estimate.worth_acting_on(5),
            "a significant effect in the wrong direction is still a reason not to act"
        );
    }

    #[test]
    fn one_randomised_arm_is_not_a_comparison() {
        let mut estimate = estimate();
        for _ in 0..50 {
            estimate.record(true, 1.0, true);
            estimate.record(false, 5.0, false);
        }
        let error = estimate.effect(5).unwrap_err();
        assert!(matches!(error, Insufficient::OneArmed { .. }));
    }

    #[test]
    fn too_few_randomised_trials_says_how_many_are_needed() {
        let mut estimate = estimate();
        for _ in 0..3 {
            estimate.record(true, 1.0, true);
            estimate.record(false, 5.0, true);
        }
        let error = estimate.effect(12).unwrap_err();
        assert!(matches!(
            error,
            Insufficient::TooFewTrials { needed: 12, .. }
        ));
        assert!(error.describe().contains("12 needed"));
    }

    #[test]
    fn confounding_is_visible_as_a_gap_between_association_and_effect() {
        // The scenario the module exists for: the machine chose the action when
        // conditions were already good, so the observational difference is huge
        // and the real effect is nil.
        let mut estimate = estimate();
        // Confounded trials: treatment taken only when things were going well.
        for _ in 0..200 {
            estimate.record(true, 1.0, false);
            estimate.record(false, 50.0, false);
        }
        // Randomised trials: no real difference.
        for i in 0..40 {
            let noise = (i % 5) as f64 * 0.1;
            estimate.record(true, 10.0 + noise, true);
            estimate.record(false, 10.0 + noise, true);
        }
        let association = estimate.association().unwrap();
        let effect = estimate.effect(5).unwrap();
        assert!(association < -20.0, "association was {association}");
        assert!(effect.delta.abs() < 0.2, "effect was {}", effect.delta);
        assert!(!effect.is_significant());
        assert!(
            !estimate.worth_acting_on(5),
            "the machine must not act on a purely observational difference"
        );
    }

    #[test]
    fn a_non_finite_outcome_is_dropped_rather_than_poisoning_the_mean() {
        let mut estimate = estimate();
        estimate.record(true, f64::NAN, true);
        estimate.record(true, 1.0, true);
        assert_eq!(estimate.treatment.trials, 1);
        assert_eq!(estimate.treatment.randomised_mean(), Some(1.0));
    }

    #[test]
    fn exploration_returns_the_preferred_action_most_of_the_time() {
        let mut exploration = Exploration::new(0.1, 5, 1000, 7);
        let mut randomised = 0;
        for _ in 0..1000 {
            let (_, was_random) = exploration.decide(true);
            if was_random {
                randomised += 1;
            }
        }
        // Around 10%, with room for the sampler.
        assert!(
            (60..160).contains(&randomised),
            "randomised {randomised} of 1000"
        );
    }

    #[test]
    fn exploration_flips_a_fair_coin_rather_than_always_contradicting_belief() {
        // Always taking the alternative would unbalance the arms and bias the
        // estimate.
        let mut exploration = Exploration::new(1.0, 5, 10_000, 11);
        let mut treatment = 0;
        let mut total = 0;
        for _ in 0..2000 {
            let (took, randomised) = exploration.decide(true);
            if randomised {
                total += 1;
                if took {
                    treatment += 1;
                }
            }
        }
        let share = treatment as f64 / total as f64;
        assert!((0.4..0.6).contains(&share), "treatment share {share}");
    }

    #[test]
    fn a_scaled_rate_explores_less_but_never_stops_entirely() {
        // A settled comparison should cost less, not become unrevisable.
        let mut exploration = Exploration::new(0.5, 5, 100_000, 17);
        let mut randomised = 0;
        for _ in 0..4000 {
            if exploration.decide_scaled(true, 0.1).1 {
                randomised += 1;
            }
        }
        // Around 5% of 4000, and definitely not zero.
        assert!(randomised > 0, "a scaled rate must still explore sometimes");
        assert!(
            (100..320).contains(&randomised),
            "randomised {randomised} of 4000 at a scaled rate of 0.05"
        );
    }

    #[test]
    fn a_zero_scale_stops_exploring() {
        let mut exploration = Exploration::new(1.0, 5, 1000, 19);
        for _ in 0..100 {
            assert_eq!(exploration.decide_scaled(true, 0.0), (true, false));
        }
        assert_eq!(exploration.spent(), 0);
    }

    #[test]
    fn exploration_stops_when_its_budget_is_gone() {
        let mut exploration = Exploration::new(1.0, 5, 10, 3);
        for _ in 0..100 {
            exploration.decide(true);
        }
        assert_eq!(exploration.spent(), 10);
        assert!(exploration.exhausted());
        let (took, randomised) = exploration.decide(true);
        assert!(took, "belief must drive behaviour once exploring is over");
        assert!(!randomised);
    }

    #[test]
    fn a_zero_rate_never_randomises() {
        let mut exploration = Exploration::new(0.0, 5, 1000, 3);
        for _ in 0..100 {
            assert_eq!(exploration.decide(true), (true, false));
        }
        assert_eq!(exploration.spent(), 0);
    }

    #[test]
    fn exploration_is_reproducible() {
        let sequence = |seed| {
            let mut exploration = Exploration::new(0.3, 5, 1000, seed);
            (0..200)
                .map(|_| exploration.decide(true))
                .collect::<Vec<_>>()
        };
        assert_eq!(sequence(42), sequence(42));
        assert_ne!(sequence(42), sequence(43));
    }

    #[test]
    fn forgetting_a_condition_drops_its_comparisons() {
        // A de-coined concept takes its causal claims with it: they are not
        // false, they are meaningless.
        let mut attribution = Attribution::new();
        attribution.comparison("concept_1", "affinity", "hold", true);
        attribution.comparison("concept_1", "priority", "hold", true);
        attribution.comparison("concept_2", "affinity", "hold", true);
        assert_eq!(attribution.len(), 3);
        assert_eq!(attribution.forget("concept_1"), 2);
        assert_eq!(attribution.len(), 1);
    }

    #[test]
    fn the_same_comparison_is_not_started_twice() {
        let mut attribution = Attribution::new();
        attribution
            .comparison("concept_1", "affinity", "hold", true)
            .record(true, 1.0, true);
        attribution
            .comparison("concept_1", "affinity", "hold", true)
            .record(true, 1.0, true);
        assert_eq!(attribution.len(), 1);
        assert_eq!(
            attribution
                .get("concept_1/affinityvshold")
                .unwrap()
                .treatment
                .trials,
            2
        );
    }

    #[test]
    fn the_report_names_the_gap_randomisation_bought() {
        let mut attribution = Attribution::new();
        {
            let estimate = attribution.comparison("concept_1", "affinity", "hold", true);
            for _ in 0..100 {
                estimate.record(true, 1.0, false);
                estimate.record(false, 50.0, false);
            }
            for i in 0..30 {
                let noise = (i % 4) as f64 * 0.1;
                estimate.record(true, 10.0 + noise, true);
                estimate.record(false, 10.0 + noise, true);
            }
        }
        let text = attribution.render(5);
        assert!(text.contains("randomisation bought"), "{text}");
    }

    #[test]
    fn an_empty_attribution_says_so() {
        assert!(Attribution::new()
            .render(5)
            .contains("no causal comparisons"));
    }

    #[test]
    fn a_comparison_is_unsettled_until_both_arms_have_evidence() {
        let mut estimate = estimate();
        assert!(!estimate.is_settled(5));
        for _ in 0..20 {
            estimate.record(true, 1.0, true);
        }
        assert!(!estimate.is_settled(5), "one arm is not a comparison");
        for _ in 0..20 {
            estimate.record(false, 5.0, true);
        }
        assert!(estimate.is_settled(5));
    }

    #[test]
    fn a_settled_null_counts_as_settled() {
        // "This makes no difference here" is knowledge. Re-testing it forever
        // is the same waste as re-testing a known win.
        let mut estimate = estimate();
        for i in 0..40 {
            let noise = (i % 5) as f64 * 0.1;
            estimate.record(true, 3.0 + noise, true);
            estimate.record(false, 3.0 + noise, true);
        }
        assert!(estimate.is_settled(5));
        assert!(!estimate.worth_acting_on(5));
    }

    #[test]
    fn an_estimate_round_trips_through_json() {
        let mut attribution = Attribution::new();
        attribution
            .comparison("concept_1", "affinity", "hold", true)
            .record(true, 1.0, true);
        let text = serde_json::to_string(&attribution).expect("serialises");
        let back: Attribution = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back, attribution);
    }
}

#[cfg(test)]
mod forgetting_tests {
    use super::*;

    fn forgetful() -> CausalEstimate {
        CausalEstimate::new("concept_17", "affinity", "hold", true).forgetting(0.98)
    }

    #[test]
    fn a_comparison_that_stops_being_fed_becomes_unsettled_again() {
        // The mechanism by which a belief can die rather than merely be
        // outvoted: evidence that has aged out no longer settles anything, so
        // exploration resumes on its own.
        let mut estimate = forgetful();
        for i in 0..200 {
            let noise = (i % 4) as f64 * 0.1;
            estimate.record(true, 1.0 + noise, true);
            estimate.record(false, 5.0 + noise, true);
        }
        assert!(estimate.is_settled(10));

        // Only the treatment arm keeps being measured; the control ages out.
        for _ in 0..200 {
            estimate.record(true, 1.0, true);
        }
        assert!(
            !estimate.is_settled(10),
            "an arm nobody has measured lately cannot still be settling anything"
        );
    }

    #[test]
    fn a_reversed_world_reverses_the_conclusion() {
        // The experiment's turn, in miniature: the action that was better
        // becomes worse, and the estimate has to follow rather than being
        // outvoted by history.
        let mut estimate = forgetful();
        for i in 0..300 {
            let noise = (i % 4) as f64 * 0.1;
            estimate.record(true, 1.0 + noise, true);
            estimate.record(false, 5.0 + noise, true);
        }
        assert!(
            estimate.worth_acting_on(10),
            "the effect should be established"
        );

        for i in 0..300 {
            let noise = (i % 4) as f64 * 0.1;
            estimate.record(true, 9.0 + noise, true);
            estimate.record(false, 5.0 + noise, true);
        }
        assert!(
            !estimate.worth_acting_on(10),
            "after the world reversed, the old preference must not survive"
        );
        let effect = estimate.effect(10).expect("still enough evidence");
        assert!(
            effect.delta > 0.0,
            "delta {} should now favour hold",
            effect.delta
        );
    }

    #[test]
    fn without_forgetting_history_outvotes_the_present() {
        // Why forgetting is not optional. The same reversal, remembered
        // forever, keeps the dead belief alive.
        let mut estimate = CausalEstimate::new("concept_17", "affinity", "hold", true);
        for i in 0..2000 {
            let noise = (i % 4) as f64 * 0.1;
            estimate.record(true, 1.0 + noise, true);
            estimate.record(false, 5.0 + noise, true);
        }
        for i in 0..200 {
            let noise = (i % 4) as f64 * 0.1;
            estimate.record(true, 9.0 + noise, true);
            estimate.record(false, 5.0 + noise, true);
        }
        assert!(
            estimate.worth_acting_on(10),
            "with no forgetting the stale belief survives, which is the point"
        );
    }

    #[test]
    fn forgetting_does_not_prevent_an_effect_being_established() {
        let mut estimate = forgetful();
        for i in 0..100 {
            let noise = (i % 4) as f64 * 0.1;
            estimate.record(true, 1.0 + noise, true);
            estimate.record(false, 5.0 + noise, true);
        }
        assert!(estimate.worth_acting_on(10));
    }

    #[test]
    fn an_attribution_forgets_by_default_and_can_be_told_not_to() {
        assert!(Attribution::new().forgetting_factor() < 1.0);
        assert_eq!(Attribution::remembering().forgetting_factor(), 1.0);
    }
}

#[cfg(test)]
mod zero_trials {
    use super::*;

    /// With nothing tried, CoreScout has not discovered confounding; it has
    /// not looked. Saying "the action was always chosen for a reason" about
    /// zero trials claims a finding it does not have.
    #[test]
    fn nothing_tried_is_not_the_same_as_tried_and_confounded() {
        let nothing = Insufficient::NoRandomisation {
            observational_trials: 0,
        }
        .describe();
        assert!(!nothing.starts_with('0'), "{nothing}");
        assert!(!nothing.contains("chosen for a reason"), "{nothing}");
        assert!(nothing.contains("yet"), "{nothing}");

        // One trial still gets the confounding explanation, because there it
        // is true: something was tried, and it was not randomised.
        let some = Insufficient::NoRandomisation {
            observational_trials: 7,
        }
        .describe();
        assert!(some.contains("chosen for a reason"), "{some}");
    }
}
