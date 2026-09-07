//! Choosing an action.
//!
//! # What a policy is here
//!
//! A function from (reflection, latent state, counterfactual predictions,
//! intent) to one [`Decision`]. Explicitly *not* a learned controller: the
//! learning lives in the model crates, and this layer applies their output
//! against a stated intent.
//!
//! That split is deliberate. A single learned policy that maps observations
//! straight to actions would be far more powerful and completely opaque: when
//! it made a bad choice there would be no way to tell whether the mirror was
//! incomplete, the model wrong, or the objective misstated. Here, each of those
//! is a separate artefact with its own error measure.
//!
//! # The decision rule
//!
//! ```text
//! reject   candidates that violate a constraint
//! reject   candidates whose support is extrapolation
//! reject   candidates whose predicted gain is inside the migration threshold
//! hold     if nothing survives
//! act      on the best of what remains
//! ```
//!
//! Three of the five outcomes are "do nothing", which is the correct
//! proportion for a controller acting on a machine it does not fully
//! understand.

use corescout_agency::{Action, ActionKind, Target};
use corescout_counterfactual::Prediction;
use corescout_intent::Intent;
use corescout_represent::latent::LatentStateId;
use serde::{Deserialize, Serialize};

/// What the policy decided, and why.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Take this action.
    Act {
        action: Action,
        /// The prediction that justified it.
        expected_improvement: f64,
        confidence: f64,
    },
    /// Do nothing, for this reason.
    Hold { reason: String },
    /// Run an experiment rather than an optimisation. Distinguished from `Act`
    /// so the audit trail can tell the two apart.
    Explore {
        action: Action,
        /// What the experiment is meant to resolve.
        hypothesis: String,
    },
}

impl Decision {
    pub fn action(&self) -> Option<&Action> {
        match self {
            Decision::Act { action, .. } | Decision::Explore { action, .. } => Some(action),
            Decision::Hold { .. } => None,
        }
    }

    pub fn is_action(&self) -> bool {
        !matches!(self, Decision::Hold { .. })
    }

    pub fn is_exploration(&self) -> bool {
        matches!(self, Decision::Explore { .. })
    }

    /// A one-line description, for the audit log.
    pub fn summary(&self) -> String {
        match self {
            Decision::Act {
                action,
                expected_improvement,
                confidence,
            } => format!(
                "act: {} (expected {:+.1}%, confidence {:.2})",
                action.kind,
                expected_improvement * 100.0,
                confidence
            ),
            Decision::Hold { reason } => format!("hold: {reason}"),
            Decision::Explore { action, hypothesis } => {
                format!("explore: {} to test whether {hypothesis}", action.kind)
            }
        }
    }
}

/// How willing the policy is to act.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyConfig {
    /// Minimum confidence in a counterfactual before acting on it.
    pub min_confidence: f64,
    /// Fraction of decisions that may be exploratory when uncertainty is high.
    pub exploration_rate: f64,
    /// Never explore when the machine is already violating a constraint:
    /// experimenting during a breach is the worst possible time for it.
    pub explore_only_when_satisfied: bool,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        PolicyConfig {
            min_confidence: 0.3,
            exploration_rate: 0.1,
            explore_only_when_satisfied: true,
        }
    }
}

/// The decision function.
#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    config: PolicyConfig,
    intent: Intent,
    /// Decisions made, for the exploration budget.
    decisions: u64,
    explorations: u64,
}

impl Policy {
    pub fn new(intent: Intent, config: PolicyConfig) -> Policy {
        Policy {
            config,
            intent,
            decisions: 0,
            explorations: 0,
        }
    }

    pub fn intent(&self) -> &Intent {
        &self.intent
    }

    pub fn decisions(&self) -> u64 {
        self.decisions
    }

    pub fn explorations(&self) -> u64 {
        self.explorations
    }

    /// Whether the exploration budget allows another experiment.
    pub fn may_explore(&self) -> bool {
        if self.decisions == 0 {
            return true;
        }
        (self.explorations as f64 / self.decisions as f64) < self.config.exploration_rate
    }

    /// Choose.
    ///
    /// `candidates` are counterfactual predictions, each paired with the action
    /// that would realise it. `constraints_satisfied` says whether the machine
    /// is currently meeting the intent.
    pub fn decide(
        &mut self,
        candidates: &[(Prediction, ActionKind)],
        constraints_satisfied: bool,
        latent_state: Option<LatentStateId>,
    ) -> Decision {
        self.decisions += 1;

        if candidates.is_empty() {
            return Decision::Hold {
                reason: "no candidate actions were available".into(),
            };
        }

        // Exploitation first: the best well-supported improvement.
        let best = candidates
            .iter()
            .filter(|(prediction, _)| prediction.worth_acting_on(&self.intent))
            .filter(|(prediction, _)| {
                prediction.outcome.confidence.score >= self.config.min_confidence
            })
            .max_by(|a, b| {
                a.0.improvement
                    .unwrap_or(f64::NEG_INFINITY)
                    .partial_cmp(&b.0.improvement.unwrap_or(f64::NEG_INFINITY))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

        if let Some((prediction, kind)) = best {
            let context = match latent_state {
                Some(state) => format!(" while in {state}"),
                None => String::new(),
            };
            return Decision::Act {
                action: Action::new(
                    kind.clone(),
                    format!(
                        "{} predicted to improve {} by {:+.1}%{context}",
                        prediction.description,
                        self.intent.name,
                        prediction.improvement.unwrap_or(0.0) * 100.0
                    ),
                )
                .expecting(format!(
                    "{:+.1}% on {}",
                    prediction.improvement.unwrap_or(0.0) * 100.0,
                    self.intent.name
                )),
                expected_improvement: prediction.improvement.unwrap_or(0.0),
                confidence: prediction.outcome.confidence.score,
            };
        }

        // Nothing worth exploiting. Is anything worth learning about?
        let may_explore = self.may_explore()
            && (constraints_satisfied || !self.config.explore_only_when_satisfied);
        if may_explore {
            if let Some((prediction, kind)) = candidates
                .iter()
                .filter(|(prediction, _)| prediction.worth_investigating())
                .max_by(|a, b| {
                    a.0.improvement
                        .unwrap_or(f64::NEG_INFINITY)
                        .partial_cmp(&b.0.improvement.unwrap_or(f64::NEG_INFINITY))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            {
                self.explorations += 1;
                return Decision::Explore {
                    action: Action::new(
                        kind.clone(),
                        format!("experiment: {}", prediction.description),
                    )
                    .expecting(format!(
                        "{:+.1}% on {}, unverified",
                        prediction.improvement.unwrap_or(0.0) * 100.0,
                        self.intent.name
                    )),
                    hypothesis: format!(
                        "{} improves {} (support: {})",
                        prediction.description,
                        self.intent.name,
                        prediction.support().label()
                    ),
                };
            }
        }

        Decision::Hold {
            reason: if !constraints_satisfied {
                "the intent is not being met, and no action is well enough supported to try".into()
            } else {
                "no candidate improves on the current placement by more than the migration \
                 threshold"
                    .into()
            },
        }
    }

    /// The do-nothing action, for when the watchdog has paused acting.
    pub fn hold(reason: impl Into<String>) -> Action {
        Action::new(ActionKind::Hold, reason)
    }
}

/// Candidate actions for placing a target on each of a set of CPUs.
///
/// The most common candidate generator: one action per possible placement.
pub fn placement_candidates(
    target: Target,
    cpu_sets: &[(corescout_core::CpuSet, usize)],
) -> Vec<(ActionKind, usize)> {
    cpu_sets
        .iter()
        .map(|(cpus, row)| {
            (
                ActionKind::SetAffinity {
                    target,
                    cpus: cpus.clone(),
                },
                *row,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_counterfactual::outcome::{Confidence, Outcome, Support};
    use corescout_intent::{Constraint, MigrationTolerance, Objective};

    fn intent() -> Intent {
        Intent::new("latency")
            .with_constraint(Constraint::latency_p99_us(30.0))
            .preferring(Objective::Latency, 1.0)
            .migration(MigrationTolerance::Medium)
    }

    fn candidate(
        description: &str,
        improvement: Option<f64>,
        support: Support,
        trials: u32,
    ) -> (Prediction, ActionKind) {
        (
            Prediction {
                family: "affinity".into(),
                description: description.into(),
                outcome: Outcome {
                    deltas: vec![(0, 0, -1.0)],
                    horizon_ns: 100_000_000,
                    confidence: Confidence::from(support, trials, 0.9),
                },
                achievement: None,
                improvement,
            },
            ActionKind::SetAffinity {
                target: Target::CurrentThread,
                cpus: [1u32].into_iter().collect(),
            },
        )
    }

    #[test]
    fn with_no_candidates_the_policy_holds() {
        let mut policy = Policy::new(intent(), PolicyConfig::default());
        let decision = policy.decide(&[], true, None);
        assert!(matches!(decision, Decision::Hold { .. }));
        assert!(!decision.is_action());
    }

    #[test]
    fn a_well_supported_improvement_is_acted_on() {
        let mut policy = Policy::new(intent(), PolicyConfig::default());
        let candidates = vec![candidate("move to cpu 1", Some(0.4), Support::Observed, 40)];
        let decision = policy.decide(&candidates, true, Some(LatentStateId(3)));
        match decision {
            Decision::Act {
                expected_improvement,
                ..
            } => assert!((expected_improvement - 0.4).abs() < 1e-9),
            other => panic!("expected an action, got {other:?}"),
        }
        // The reason names the latent state it was in, so the decision can be
        // reconstructed later.
        assert!(decision.summary().contains("act:"));
        assert!(decision.action().unwrap().reason.contains("latent_state_3"));
    }

    #[test]
    fn a_small_gain_is_not_worth_moving_for() {
        let mut policy = Policy::new(intent(), PolicyConfig::default());
        // Medium tolerance wants 8%.
        let candidates = vec![candidate("marginal", Some(0.03), Support::Observed, 40)];
        let decision = policy.decide(&candidates, true, None);
        assert!(matches!(decision, Decision::Hold { .. }));
    }

    #[test]
    fn an_unsupported_prediction_becomes_an_experiment_not_an_action() {
        // The core of the exploration/exploitation split: a large predicted
        // gain with no evidence is a hypothesis, not a plan.
        let mut policy = Policy::new(intent(), PolicyConfig::default());
        let candidates = vec![candidate(
            "untried placement",
            Some(0.9),
            Support::Extrapolated,
            0,
        )];
        let decision = policy.decide(&candidates, true, None);
        assert!(decision.is_exploration(), "got {decision:?}");
        assert!(decision.summary().contains("explore:"));
        assert_eq!(policy.explorations(), 1);
    }

    #[test]
    fn exploration_is_budgeted() {
        let mut policy = Policy::new(
            intent(),
            PolicyConfig {
                exploration_rate: 0.2,
                ..PolicyConfig::default()
            },
        );
        let candidates = vec![candidate("untried", Some(0.9), Support::Extrapolated, 0)];
        let mut explorations = 0;
        for _ in 0..20 {
            if policy.decide(&candidates, true, None).is_exploration() {
                explorations += 1;
            }
        }
        assert!(
            explorations <= 5,
            "the budget should hold exploration near 20%, got {explorations}/20"
        );
        assert!(explorations >= 1);
    }

    #[test]
    fn the_policy_does_not_experiment_during_a_constraint_breach() {
        // The worst possible time to try something new.
        let mut policy = Policy::new(intent(), PolicyConfig::default());
        let candidates = vec![candidate("untried", Some(0.9), Support::Extrapolated, 0)];
        let decision = policy.decide(&candidates, false, None);
        assert!(matches!(decision, Decision::Hold { .. }));
        assert!(decision.summary().contains("not being met"));
    }

    #[test]
    fn low_confidence_blocks_action_even_with_observed_support() {
        let mut policy = Policy::new(
            intent(),
            PolicyConfig {
                min_confidence: 0.9,
                ..PolicyConfig::default()
            },
        );
        let candidates = vec![candidate("weak evidence", Some(0.4), Support::Observed, 3)];
        assert!(matches!(
            policy.decide(&candidates, true, None),
            Decision::Hold { .. }
        ));
    }

    #[test]
    fn the_best_candidate_wins() {
        let mut policy = Policy::new(intent(), PolicyConfig::default());
        let candidates = vec![
            candidate("modest", Some(0.15), Support::Observed, 40),
            candidate("large", Some(0.55), Support::Observed, 40),
            candidate("negative", Some(-0.3), Support::Observed, 40),
        ];
        match policy.decide(&candidates, true, None) {
            Decision::Act {
                expected_improvement,
                ..
            } => assert!((expected_improvement - 0.55).abs() < 1e-9),
            other => panic!("expected the best action, got {other:?}"),
        }
    }
}
