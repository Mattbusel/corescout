//! The arrow from concept to action.
//!
//! # The milestone this closes
//!
//! > The machine performs an action that it would not have performed without a
//! > concept it discovered about itself.
//!
//! Not "a concept exists". Not "a concept predicts". Not "a concept happens to
//! correspond to a hidden regime". The claim is that a self-derived concept
//! **changed what the machine did**, and that claim has to be measurable and
//! falsifiable like any other.
//!
//! So [`Choice`] carries [`Choice::counterfactual`]: what this same policy would
//! have chosen at this same moment knowing nothing about concepts.
//! [`Choice::attributable`] is true exactly when the two differ. That flag is
//! the milestone, counted rather than asserted.
//!
//! # Why belief alone is not allowed to establish a preference
//!
//! A preference is installed only from a causal estimate built on randomised
//! trials. The tempting shortcut is to notice that outcomes are better under C
//! when A was taken, and prefer A. That is the reasoning the whole causal module
//! exists to refuse: A was taken *because* of something, and the something may
//! be what produced the outcome.
//!
//! So [`ConditionedPolicy::choose`] consults
//! [`corescout_science::CausalEstimate::worth_acting_on`], which returns false
//! for any amount of observational evidence.
//!
//! # Concept death changes behaviour
//!
//! When a concept is de-coined, [`ConditionedPolicy::forget`] drops the
//! preference and every causal comparison conditioned on it. The machine then
//! chooses as it did before the concept existed. That is the second half of the
//! lifecycle and the easier half to get wrong: a system that learns from a
//! concept and keeps the behaviour after the concept dies has not learned from
//! the concept, it has been trained by it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use corescout_science::causal::{Attribution, Exploration};

/// How much of the usual exploration a settled comparison still gets.
///
/// Not zero. See [`corescout_science::causal::Exploration::decide_scaled`].
const SETTLED_SCALE: f64 = 0.15;

/// The key a comparison is filed under.
///
/// Must match `CausalEstimate::key`, which is why it is written once here and
/// used everywhere rather than formatted inline at each call site.
fn comparison_key(under: &str, family: &str, alternative: &str) -> String {
    format!("{under}/{family}vs{alternative}")
}

/// The action a policy settled on, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    /// The action family chosen.
    pub family: String,
    /// The condition in force, when the machine recognised one.
    pub condition: Option<String>,
    /// What this policy would have chosen knowing nothing about concepts.
    ///
    /// The comparison that makes the milestone measurable rather than asserted.
    pub counterfactual: String,
    /// True when this trial's arm was picked by coin flip rather than belief.
    pub randomised: bool,
    /// The action this trial counts as evidence *for*.
    ///
    /// Carried on the choice rather than recomputed when the outcome arrives.
    /// Recomputing invites the two to disagree, and a trial filed against the
    /// wrong comparison is worse than no trial: it is evidence about something
    /// that never happened.
    pub treatment: String,
    /// What that action is being compared against.
    pub alternative: String,
    /// Why, in one line.
    pub because: String,
}

impl Choice {
    /// Whether a self-derived concept changed what the machine did.
    ///
    /// **The milestone.** True only when a condition was recognised and the
    /// action differs from the concept-blind one. A randomised trial is
    /// excluded: the coin changed the action, not the concept.
    pub fn attributable(&self) -> bool {
        !self.randomised && self.condition.is_some() && self.family != self.counterfactual
    }

    pub fn describe(&self) -> String {
        let mut text = format!("{}: {}", self.family, self.because);
        if self.attributable() {
            text.push_str(&format!(
                "  [would have chosen {} without {}]",
                self.counterfactual,
                self.condition.as_deref().unwrap_or("?")
            ));
        }
        if self.randomised {
            text.push_str("  [randomised trial]");
        }
        text
    }
}

/// How the conditioned policy is run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionedConfig {
    /// Randomised trials needed on each arm before a preference is installed.
    pub min_trials: u32,
    /// What to do when no condition is recognised and nothing is established.
    ///
    /// Holding is the right default: a policy with no evidence should not act.
    pub default_action: String,
    /// Whether a smaller outcome is a better one.
    pub lower_is_better: bool,
}

impl Default for ConditionedConfig {
    fn default() -> Self {
        ConditionedConfig {
            min_trials: 12,
            default_action: "hold".to_string(),
            lower_is_better: true,
        }
    }
}

/// A policy whose choices depend on concepts the machine coined itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionedPolicy {
    config: ConditionedConfig,
    attribution: Attribution,
    exploration: Exploration,
    /// The action preferred under each condition, once randomised evidence
    /// established one. Never written from observational data.
    #[serde(with = "corescout_core::serde_util::pairs")]
    preferences: BTreeMap<String, String>,
    /// Decisions where a concept changed the action.
    attributable_decisions: u64,
    decisions: u64,
}

impl ConditionedPolicy {
    pub fn new(config: ConditionedConfig, exploration: Exploration) -> ConditionedPolicy {
        ConditionedPolicy {
            config,
            attribution: Attribution::new(),
            exploration,
            preferences: BTreeMap::new(),
            attributable_decisions: 0,
            decisions: 0,
        }
    }

    pub fn config(&self) -> &ConditionedConfig {
        &self.config
    }

    pub fn attribution(&self) -> &Attribution {
        &self.attribution
    }

    pub fn exploration(&self) -> &Exploration {
        &self.exploration
    }

    pub fn preferences(&self) -> &BTreeMap<String, String> {
        &self.preferences
    }

    pub fn decisions(&self) -> u64 {
        self.decisions
    }

    /// How many times a self-derived concept changed the chosen action.
    pub fn attributable_decisions(&self) -> u64 {
        self.attributable_decisions
    }

    /// Choose an action.
    ///
    /// `condition` is the concept the machine currently recognises itself to be
    /// in, if any. `candidates` are the action families available.
    pub fn choose(&mut self, condition: Option<&str>, candidates: &[String]) -> Choice {
        self.decisions += 1;

        // What this policy would do knowing nothing about concepts. Computed
        // first and unconditionally, so it cannot be quietly derived from the
        // concept-aware answer.
        let counterfactual = self.blind_choice(candidates);

        let Some(condition) = condition else {
            return Choice {
                family: counterfactual.clone(),
                condition: None,
                treatment: counterfactual.clone(),
                alternative: counterfactual.clone(),
                counterfactual,
                randomised: false,
                because: "no condition recognised".into(),
            };
        };

        // The comparison this trial belongs to. When a preference is
        // established it is the one under test; otherwise the policy probes,
        // because a causal claim cannot be bootstrapped by watching.
        //
        // Not probing here was a deadlock: a preference needed randomised
        // evidence, and randomised evidence was only gathered once a preference
        // existed, so nothing was ever established.
        let established = self
            .preferences
            .get(condition)
            .cloned()
            .filter(|f| candidates.contains(f));
        let treatment = match &established {
            Some(preferred) => preferred.clone(),
            None => self.probe_for(condition, candidates),
        };
        let alternative = self.alternative_to(&treatment, candidates);

        if treatment == alternative {
            return Choice {
                family: counterfactual.clone(),
                condition: Some(condition.to_string()),
                treatment: counterfactual.clone(),
                alternative: counterfactual.clone(),
                counterfactual,
                randomised: false,
                because: format!("in {condition}, but there is only one action available"),
            };
        }

        // Once a comparison is settled, stop paying for it. Exploration exists
        // to gather evidence; continuing at a fixed rate after the question is
        // answered is a tax on every decision that buys nothing.
        let settled = self
            .attribution
            .get(&comparison_key(condition, &treatment, &alternative))
            .is_some_and(|estimate| estimate.is_settled(self.config.min_trials));

        // Belief drives behaviour except on the fraction of trials given over to
        // finding out. With no belief yet the un-randomised choice is the
        // concept-blind one, so an unproven condition still changes nothing.
        // A settled comparison is revisited at a fraction of the usual rate:
        // cheap enough not to be a tax, frequent enough that a change in the
        // world eventually shows up rather than being invisible forever.
        let scale = if settled { SETTLED_SCALE } else { 1.0 };
        let (take_treatment, randomised) =
            self.exploration.decide_scaled(established.is_some(), scale);
        let family = if randomised {
            if take_treatment {
                treatment.clone()
            } else {
                alternative.clone()
            }
        } else if established.is_some() {
            treatment.clone()
        } else {
            counterfactual.clone()
        };

        let choice = Choice {
            because: match (&established, randomised) {
                (_, true) => {
                    format!("randomised trial under {condition}: {treatment} vs {alternative}")
                }
                (Some(preferred), false) => {
                    format!("established under {condition}: {preferred} beats {alternative}")
                }
                (None, false) => format!("in {condition}, but nothing is established there yet"),
            },
            family,
            condition: Some(condition.to_string()),
            treatment,
            alternative,
            counterfactual,
            randomised,
        };
        if choice.attributable() {
            self.attributable_decisions += 1;
        }
        choice
    }

    /// Record what happened after a choice.
    ///
    /// `outcome` is the objective value observed afterwards. The estimate this
    /// feeds is only allowed to become causal when the trial was randomised,
    /// which is carried on the [`Choice`] rather than supplied by the caller.
    pub fn observe(&mut self, choice: &Choice, outcome: f64) {
        let Some(condition) = &choice.condition else {
            return;
        };
        if choice.treatment == choice.alternative {
            return;
        }
        // The pair comes from the choice, so a trial is always filed against
        // exactly the comparison the decision was made under.
        let took_treatment = choice.family == choice.treatment;
        let lower_is_better = self.config.lower_is_better;
        self.attribution
            .comparison(
                condition,
                &choice.treatment,
                &choice.alternative,
                lower_is_better,
            )
            .record(took_treatment, outcome, choice.randomised);
    }

    /// Record a trial for an explicit comparison, outside the normal loop.
    ///
    /// Used when the caller is deliberately sweeping a pair of actions rather
    /// than following the policy's own preference.
    pub fn record_trial(
        &mut self,
        condition: &str,
        family: &str,
        alternative: &str,
        took_family: bool,
        outcome: f64,
        randomised: bool,
    ) {
        let lower_is_better = self.config.lower_is_better;
        self.attribution
            .comparison(condition, family, alternative, lower_is_better)
            .record(took_family, outcome, randomised);
    }

    /// Install preferences wherever randomised evidence now supports one.
    ///
    /// Returns the conditions whose preference changed. Nothing here reads
    /// observational data: a preference that cannot be justified causally is
    /// not installed, however suggestive the association.
    pub fn consolidate(&mut self) -> Vec<String> {
        let min_trials = self.config.min_trials;
        let mut changed = Vec::new();
        let established: Vec<(String, String)> = self
            .attribution
            .established(min_trials)
            .into_iter()
            .map(|estimate| (estimate.under.clone(), estimate.family.clone()))
            .collect();
        for (condition, family) in &established {
            if self.preferences.get(condition) != Some(family) {
                self.preferences.insert(condition.clone(), family.clone());
                changed.push(condition.clone());
            }
        }

        // Withdraw preferences the evidence no longer supports.
        //
        // Installing without withdrawing was a one-way door: a preference
        // established under conditions that have since changed would drive
        // behaviour forever, and the machine would look like it had learned
        // something when it had merely been trained.
        let supported: Vec<&String> = established.iter().map(|(c, _)| c).collect();
        let withdrawn: Vec<String> = self
            .preferences
            .keys()
            .filter(|condition| !supported.contains(condition))
            .cloned()
            .collect();
        for condition in withdrawn {
            self.preferences.remove(&condition);
            changed.push(condition);
        }
        changed
    }

    /// Forget everything conditioned on a concept that no longer exists.
    ///
    /// The second half of the lifecycle. A machine that keeps the behaviour
    /// after the concept dies was trained by the concept rather than reasoning
    /// from it.
    pub fn forget(&mut self, condition: &str) -> bool {
        let had_preference = self.preferences.remove(condition).is_some();
        let dropped = self.attribution.forget(condition);
        had_preference || dropped > 0
    }

    /// Which action to probe under a condition with nothing established.
    ///
    /// Rotates through the non-default candidates, so every action gets tried
    /// rather than the first one being probed forever, and rotates slowly so a
    /// probe gets a run of trials rather than one each.
    fn probe_for(&self, condition: &str, candidates: &[String]) -> String {
        let options: Vec<&String> = candidates
            .iter()
            .filter(|f| **f != self.config.default_action)
            .collect();
        if options.is_empty() {
            return self.config.default_action.clone();
        }
        // Prefer an action whose comparison is not yet settled, so exploration
        // goes where the evidence is missing rather than being spread evenly
        // over questions that are already answered.
        let unsettled: Vec<&&String> = options
            .iter()
            .filter(|family| {
                let alternative = self.alternative_to(family, candidates);
                !self
                    .attribution
                    .get(&comparison_key(condition, family, &alternative))
                    .is_some_and(|estimate| estimate.is_settled(self.config.min_trials))
            })
            .collect();
        let pool: &[&String] = if unsettled.is_empty() {
            &options
        } else {
            // Reborrow through the filtered set.
            return unsettled[(self.decisions / 40) as usize % unsettled.len()].to_string();
        };
        let hash = condition
            .bytes()
            .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
        let index = (hash.wrapping_add(self.decisions / 40)) as usize % pool.len();
        pool[index].clone()
    }

    /// What the policy would choose with no concept knowledge at all.
    ///
    /// Deliberately trivial: the configured default. A concept-blind policy has
    /// no basis for preferring one action over another, and giving it one would
    /// make the comparison measure something other than the concept.
    fn blind_choice(&self, candidates: &[String]) -> String {
        if candidates.contains(&self.config.default_action) {
            return self.config.default_action.clone();
        }
        candidates
            .first()
            .cloned()
            .unwrap_or_else(|| self.config.default_action.clone())
    }

    /// The action a preference is measured against.
    fn alternative_to(&self, preferred: &str, candidates: &[String]) -> String {
        if self.config.default_action != preferred
            && candidates.contains(&self.config.default_action)
        {
            return self.config.default_action.clone();
        }
        candidates
            .iter()
            .find(|f| f.as_str() != preferred)
            .cloned()
            .unwrap_or_else(|| preferred.to_string())
    }

    /// A report a person can read.
    pub fn render(&self) -> String {
        let mut out = format!(
            "{} decisions, {} changed by a concept the machine coined itself\n",
            self.decisions, self.attributable_decisions
        );
        if self.preferences.is_empty() {
            out.push_str(
                "no preferences are established: no comparison has enough randomised \
                 evidence to justify acting on it\n",
            );
        } else {
            for (condition, family) in &self.preferences {
                out.push_str(&format!("  under {condition}: prefer {family}\n"));
            }
        }
        out.push('\n');
        out.push_str(&self.attribution.render(self.config.min_trials));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates() -> Vec<String> {
        vec!["hold".into(), "affinity".into(), "priority".into()]
    }

    fn policy() -> ConditionedPolicy {
        ConditionedPolicy::new(
            ConditionedConfig {
                min_trials: 5,
                ..ConditionedConfig::default()
            },
            Exploration::new(0.0, 5, 1000, 7),
        )
    }

    /// Feed randomised evidence that `family` beats `hold` under `condition`.
    fn establish(policy: &mut ConditionedPolicy, condition: &str, family: &str) {
        for i in 0..30 {
            let noise = (i % 4) as f64 * 0.05;
            policy.record_trial(condition, family, "hold", true, 1.0 + noise, true);
            policy.record_trial(condition, family, "hold", false, 6.0 + noise, true);
        }
        policy.consolidate();
    }

    #[test]
    fn with_nothing_established_the_policy_holds() {
        let mut policy = policy();
        let choice = policy.choose(Some("concept_1"), &candidates());
        assert_eq!(choice.family, "hold");
        assert!(!choice.attributable());
        assert!(choice.because.contains("nothing is established"));
    }

    #[test]
    fn recognising_a_condition_is_not_by_itself_a_reason_to_act_differently() {
        // The failure mode: "I know what state I am in, therefore I should do
        // something about it."
        let mut policy = policy();
        let with = policy.choose(Some("concept_1"), &candidates());
        let without = policy.choose(None, &candidates());
        assert_eq!(with.family, without.family);
    }

    #[test]
    fn observational_evidence_never_installs_a_preference() {
        // The central rule, at the policy level.
        let mut policy = policy();
        for _ in 0..500 {
            policy.record_trial("concept_1", "affinity", "hold", true, 1.0, false);
            policy.record_trial("concept_1", "affinity", "hold", false, 99.0, false);
        }
        assert!(policy.consolidate().is_empty());
        assert!(policy.preferences().is_empty());
        let choice = policy.choose(Some("concept_1"), &candidates());
        assert_eq!(choice.family, "hold");
        assert!(!choice.attributable());
    }

    #[test]
    fn randomised_evidence_installs_a_preference_and_changes_the_action() {
        // The milestone: an action the machine would not have taken without a
        // concept it coined itself.
        let mut policy = policy();
        establish(&mut policy, "concept_17", "affinity");
        assert_eq!(
            policy.preferences().get("concept_17"),
            Some(&"affinity".to_string())
        );

        let choice = policy.choose(Some("concept_17"), &candidates());
        assert_eq!(choice.family, "affinity");
        assert_eq!(choice.counterfactual, "hold");
        assert!(
            choice.attributable(),
            "the concept must have changed the action: {}",
            choice.describe()
        );
        assert_eq!(policy.attributable_decisions(), 1);
        assert!(choice.describe().contains("would have chosen hold"));
    }

    #[test]
    fn the_preference_applies_only_under_its_own_condition() {
        let mut policy = policy();
        establish(&mut policy, "concept_17", "affinity");
        assert_eq!(
            policy.choose(Some("concept_17"), &candidates()).family,
            "affinity"
        );
        assert_eq!(
            policy.choose(Some("concept_99"), &candidates()).family,
            "hold"
        );
        assert_eq!(policy.choose(None, &candidates()).family, "hold");
    }

    #[test]
    fn a_concepts_death_changes_behaviour_back() {
        // The second half of the lifecycle. Keeping the behaviour after the
        // concept dies means the machine was trained by it, not reasoning from
        // it.
        let mut policy = policy();
        establish(&mut policy, "concept_17", "affinity");
        assert_eq!(
            policy.choose(Some("concept_17"), &candidates()).family,
            "affinity"
        );

        assert!(policy.forget("concept_17"));
        let after = policy.choose(Some("concept_17"), &candidates());
        assert_eq!(after.family, "hold");
        assert!(!after.attributable());
        assert!(policy.preferences().is_empty());
        assert!(policy.attribution().is_empty());
    }

    #[test]
    fn forgetting_a_condition_that_was_never_known_does_nothing() {
        let mut policy = policy();
        assert!(!policy.forget("concept_404"));
    }

    #[test]
    fn a_randomised_trial_is_not_counted_as_concept_attributable() {
        // The coin changed the action, not the concept.
        let mut policy = ConditionedPolicy::new(
            ConditionedConfig {
                min_trials: 5,
                ..ConditionedConfig::default()
            },
            // Always randomise.
            Exploration::new(1.0, 5, 1000, 3),
        );
        // An unsettled condition, so exploration runs at full rate.
        let mut randomised_choices = 0;
        for _ in 0..50 {
            let choice = policy.choose(Some("concept_new"), &candidates());
            if choice.randomised {
                randomised_choices += 1;
                // The coin changed the action, not the concept.
                assert!(!choice.attributable());
            }
        }
        assert!(randomised_choices > 0);
        assert_eq!(policy.attributable_decisions(), 0);
    }

    #[test]
    fn a_preference_keeps_being_tested_rather_than_becoming_self_confirming() {
        let mut policy = ConditionedPolicy::new(
            ConditionedConfig {
                min_trials: 5,
                ..ConditionedConfig::default()
            },
            Exploration::new(0.2, 5, 1000, 11),
        );
        establish(&mut policy, "concept_17", "affinity");
        // The comparison is settled by `establish`, so the rate drops to a
        // residual. It must not drop to zero: a preference that is never
        // revisited cannot be revised when the world changes.
        let mut randomised = 0;
        for _ in 0..3000 {
            if policy.choose(Some("concept_17"), &candidates()).randomised {
                randomised += 1;
            }
        }
        assert!(
            randomised > 0,
            "a settled preference must still be tested occasionally, or it \
             becomes unrevisable"
        );
    }

    #[test]
    fn a_preference_naming_an_unavailable_action_is_not_used() {
        let mut policy = policy();
        establish(&mut policy, "concept_17", "affinity");
        let narrowed = vec!["hold".to_string(), "priority".to_string()];
        let choice = policy.choose(Some("concept_17"), &narrowed);
        assert_eq!(choice.family, "hold");
        assert!(!choice.attributable());
    }

    #[test]
    fn observing_outcomes_through_the_policy_reaches_the_estimate() {
        let mut policy = policy();
        establish(&mut policy, "concept_17", "affinity");
        let choice = policy.choose(Some("concept_17"), &candidates());
        policy.observe(&choice, 1.0);
        let estimate = policy
            .attribution()
            .get("concept_17/affinityvshold")
            .expect("the comparison exists");
        assert!(estimate.treatment.trials > 0);
    }

    #[test]
    fn an_outcome_with_no_condition_is_not_attributed_to_anything() {
        let mut policy = policy();
        let choice = policy.choose(None, &candidates());
        policy.observe(&choice, 1.0);
        assert!(policy.attribution().is_empty());
    }

    #[test]
    fn the_report_says_when_nothing_is_established() {
        let policy = policy();
        assert!(policy.render().contains("no preferences are established"));
    }

    #[test]
    fn a_policy_round_trips_through_json() {
        let mut policy = policy();
        establish(&mut policy, "concept_17", "affinity");
        policy.choose(Some("concept_17"), &candidates());
        let text = serde_json::to_string(&policy).expect("serialises");
        let back: ConditionedPolicy = serde_json::from_str(&text).expect("deserialises");
        // Structural equality rather than byte equality: the arms carry
        // accumulated f64 sums, and serde_json can lose an ulp on those. What
        // has to survive is everything a decision depends on.
        assert_eq!(back.preferences(), policy.preferences());
        assert_eq!(back.decisions(), policy.decisions());
        assert_eq!(
            back.attributable_decisions(),
            policy.attributable_decisions()
        );
        assert_eq!(back.attribution().len(), policy.attribution().len());
        for estimate in policy.attribution().estimates() {
            let restored = back
                .attribution()
                .get(&estimate.key())
                .expect("every comparison survives");
            assert_eq!(
                restored.worth_acting_on(policy.config().min_trials),
                estimate.worth_acting_on(policy.config().min_trials),
                "{} changed its verdict across a round trip",
                estimate.key()
            );
        }
    }
}
