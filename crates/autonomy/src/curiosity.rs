//! Self-experimentation: acting in order to find out.
//!
//! # Separated from optimisation on purpose
//!
//! An experiment's purpose is to reduce uncertainty, not to improve the
//! objective. Keeping the two apart matters for a reason that is easy to
//! overlook: if exploration and exploitation share a code path, then every
//! regression can be excused as "that was exploration", and the safety layer
//! loses the ability to tell a bad optimisation from a deliberate probe.
//!
//! # The loop
//!
//! ```text
//! belief       entity 12 may suit this workload         confidence 0.42
//! experiment   route a bounded share of it there
//! observe      the reflections that follow
//! update       confidence 0.42 -> 0.81, or the belief is dropped
//! ```
//!
//! # Every experiment is bounded before it starts
//!
//! It has a hypothesis, a duration, a revert plan, and a stated cost ceiling.
//! An experiment without a pre-declared end is just an unexplained change to
//! the machine.

use corescout_agency::{Action, ActionKind};
use corescout_counterfactual::Prediction;
use corescout_represent::latent::LatentStateId;
use serde::{Deserialize, Serialize};

/// A bounded, reversible thing to try.
#[derive(Debug, Clone, PartialEq)]
pub struct Experiment {
    pub id: u64,
    /// What this is meant to resolve, in one line.
    pub hypothesis: String,
    pub action: ActionKind,
    /// Confidence before running it.
    pub prior_confidence: f64,
    /// How long to observe before deciding.
    pub duration_ns: u64,
    /// The latent state the machine was in when this was proposed, so the
    /// result can be attributed to a context rather than to the machine at
    /// large.
    pub context: Option<LatentStateId>,
    /// Monotonic time it started.
    pub started_ns: u64,
}

impl Experiment {
    /// Whether the observation window has elapsed.
    pub fn is_complete(&self, now_ns: u64) -> bool {
        now_ns.saturating_sub(self.started_ns) >= self.duration_ns
    }

    pub fn as_action(&self) -> Action {
        Action::new(
            self.action.clone(),
            format!("experiment {}: {}", self.id, self.hypothesis),
        )
        .expecting(format!(
            "resolves a belief currently held at confidence {:.2}",
            self.prior_confidence
        ))
    }
}

/// What an experiment established.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperimentOutcome {
    pub id: u64,
    pub hypothesis: String,
    /// Measured change in the objective, positive meaning better.
    pub measured_effect: f64,
    /// What was predicted before running it.
    pub predicted_effect: f64,
    pub prior_confidence: f64,
    pub posterior_confidence: f64,
    /// Whether the hypothesis survived.
    pub supported: bool,
    pub context: Option<LatentStateId>,
}

impl ExperimentOutcome {
    /// How wrong the prediction was, as a fraction of what was predicted.
    ///
    /// The number that actually improves the model: a confirmed hypothesis
    /// with a badly wrong magnitude is still a lesson.
    pub fn prediction_error(&self) -> f64 {
        if self.predicted_effect.abs() <= 1e-12 {
            return self.measured_effect.abs();
        }
        (self.measured_effect - self.predicted_effect).abs() / self.predicted_effect.abs()
    }

    pub fn summary(&self) -> String {
        format!(
            "{}: predicted {:+.1}%, observed {:+.1}%, confidence {:.2} -> {:.2} ({})",
            self.hypothesis,
            self.predicted_effect * 100.0,
            self.measured_effect * 100.0,
            self.prior_confidence,
            self.posterior_confidence,
            if self.supported {
                "supported"
            } else {
                "refuted"
            }
        )
    }
}

/// Proposes and tracks experiments.
#[derive(Debug, Clone, Default)]
pub struct Curiosity {
    next_id: u64,
    running: Option<Experiment>,
    completed: Vec<ExperimentOutcome>,
    /// Total experiments proposed.
    proposed: u64,
    /// Minimum uncertainty before an experiment is worth running.
    min_uncertainty: f64,
    /// Ceiling on how many may run in one session.
    budget: u64,
}

impl Curiosity {
    pub fn new(min_uncertainty: f64, budget: u64) -> Curiosity {
        Curiosity {
            next_id: 1,
            running: None,
            completed: Vec::new(),
            proposed: 0,
            min_uncertainty,
            budget,
        }
    }

    pub fn running(&self) -> Option<&Experiment> {
        self.running.as_ref()
    }

    pub fn completed(&self) -> &[ExperimentOutcome] {
        &self.completed
    }

    pub fn proposed(&self) -> u64 {
        self.proposed
    }

    pub fn budget_remaining(&self) -> u64 {
        self.budget.saturating_sub(self.proposed)
    }

    /// Propose an experiment, if one is worth running.
    ///
    /// Returns `None` when: one is already running, the budget is spent, or
    /// nothing is uncertain enough to be worth the disturbance. All three are
    /// normal and none is an error.
    pub fn propose(
        &mut self,
        candidates: &[(Prediction, ActionKind)],
        context: Option<LatentStateId>,
        now_ns: u64,
        duration_ns: u64,
    ) -> Option<&Experiment> {
        if self.running.is_some() || self.budget_remaining() == 0 {
            return None;
        }

        // The most informative candidate: plausibly useful, and least known.
        // Ranking by *uncertainty* rather than by predicted gain is what makes
        // this exploration rather than optimisation with extra steps.
        let (prediction, action) = candidates
            .iter()
            .filter(|(prediction, _)| prediction.worth_investigating())
            .filter(|(prediction, _)| {
                1.0 - prediction.outcome.confidence.score >= self.min_uncertainty
            })
            .min_by(|a, b| {
                a.0.outcome
                    .confidence
                    .score
                    .partial_cmp(&b.0.outcome.confidence.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })?;

        self.proposed += 1;
        let id = self.next_id;
        self.next_id += 1;
        self.running = Some(Experiment {
            id,
            hypothesis: format!(
                "{} improves the objective (currently {})",
                prediction.description,
                prediction.support().label()
            ),
            action: action.clone(),
            prior_confidence: prediction.outcome.confidence.score,
            duration_ns,
            context,
            started_ns: now_ns,
        });
        self.running.as_ref()
    }

    /// Conclude the running experiment with what was measured.
    ///
    /// `measured_effect` and `predicted_effect` are fractions where positive is
    /// better.
    pub fn conclude(
        &mut self,
        measured_effect: f64,
        predicted_effect: f64,
    ) -> Option<ExperimentOutcome> {
        let experiment = self.running.take()?;
        // A hypothesis is supported when the effect is real and in the
        // predicted direction. Getting the sign right is the claim; getting the
        // magnitude right is a separate, softer question tracked as prediction
        // error.
        let supported = measured_effect > 0.0
            && (predicted_effect <= 0.0 || measured_effect.signum() == predicted_effect.signum());

        let posterior = if supported {
            // Confidence rises toward certainty, never reaching it on one trial.
            (experiment.prior_confidence + 0.5 * (1.0 - experiment.prior_confidence)).min(0.95)
        } else {
            experiment.prior_confidence * 0.4
        };

        let outcome = ExperimentOutcome {
            id: experiment.id,
            hypothesis: experiment.hypothesis,
            measured_effect,
            predicted_effect,
            prior_confidence: experiment.prior_confidence,
            posterior_confidence: posterior,
            supported,
            context: experiment.context,
        };
        self.completed.push(outcome.clone());
        Some(outcome)
    }

    /// Abandon the running experiment without concluding anything.
    ///
    /// For when the watchdog intervenes or the machine's shape changes: an
    /// experiment interrupted partway through has established nothing, and
    /// recording a result would be worse than recording none.
    pub fn abandon(&mut self) -> Option<Experiment> {
        self.running.take()
    }

    /// Experiments whose hypothesis survived.
    pub fn supported(&self) -> Vec<&ExperimentOutcome> {
        self.completed.iter().filter(|o| o.supported).collect()
    }

    /// How well calibrated the predictions behind experiments have been.
    ///
    /// Mean prediction error across concluded experiments. A system that is
    /// always right about the sign and always wrong about the magnitude has a
    /// specific, fixable problem, and this is what surfaces it.
    pub fn calibration(&self) -> Option<f64> {
        if self.completed.is_empty() {
            return None;
        }
        let total: f64 = self.completed.iter().map(|o| o.prediction_error()).sum();
        Some(total / self.completed.len() as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_agency::Target;
    use corescout_counterfactual::outcome::{Confidence, Outcome};
    use corescout_counterfactual::Support;

    fn candidate(
        description: &str,
        improvement: f64,
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
                improvement: Some(improvement),
            },
            ActionKind::SetAffinity {
                target: Target::CurrentThread,
                cpus: [1u32].into_iter().collect(),
            },
        )
    }

    fn curiosity() -> Curiosity {
        Curiosity::new(0.3, 10)
    }

    #[test]
    fn an_experiment_targets_the_least_known_candidate() {
        // Not the most promising one: this is exploration.
        let mut curiosity = curiosity();
        let candidates = vec![
            candidate("well understood", 0.5, Support::Transferred, 30),
            candidate("barely known", 0.2, Support::Extrapolated, 0),
        ];
        let experiment = curiosity
            .propose(&candidates, Some(LatentStateId(2)), 1000, 5_000_000_000)
            .expect("an experiment");
        assert!(experiment.hypothesis.contains("barely known"));
        assert_eq!(experiment.context, Some(LatentStateId(2)));
    }

    #[test]
    fn only_one_experiment_runs_at_a_time() {
        // Two concurrent interventions cannot be attributed.
        let mut curiosity = curiosity();
        let candidates = vec![candidate("a", 0.5, Support::Extrapolated, 0)];
        assert!(curiosity.propose(&candidates, None, 0, 1000).is_some());
        assert!(curiosity.propose(&candidates, None, 0, 1000).is_none());
    }

    #[test]
    fn nothing_sufficiently_certain_is_worth_experimenting_on() {
        let mut curiosity = Curiosity::new(0.9, 10);
        let candidates = vec![candidate("known", 0.5, Support::Transferred, 100)];
        assert!(curiosity.propose(&candidates, None, 0, 1000).is_none());
    }

    #[test]
    fn the_budget_is_finite() {
        let mut curiosity = Curiosity::new(0.3, 2);
        let candidates = vec![candidate("unknown", 0.5, Support::Extrapolated, 0)];
        for _ in 0..2 {
            assert!(curiosity.propose(&candidates, None, 0, 1000).is_some());
            curiosity.conclude(0.1, 0.5);
        }
        assert_eq!(curiosity.budget_remaining(), 0);
        assert!(curiosity.propose(&candidates, None, 0, 1000).is_none());
    }

    #[test]
    fn a_confirmed_hypothesis_raises_confidence_without_reaching_certainty() {
        let mut curiosity = curiosity();
        let candidates = vec![candidate("try it", 0.4, Support::Extrapolated, 0)];
        let prior = curiosity
            .propose(&candidates, None, 0, 1000)
            .unwrap()
            .prior_confidence;
        let outcome = curiosity.conclude(0.35, 0.4).unwrap();
        assert!(outcome.supported);
        assert!(outcome.posterior_confidence > prior);
        assert!(outcome.posterior_confidence < 1.0, "one trial is not proof");
    }

    #[test]
    fn a_refuted_hypothesis_loses_confidence() {
        let mut curiosity = curiosity();
        let candidates = vec![candidate("try it", 0.4, Support::Transferred, 5)];
        let prior = curiosity
            .propose(&candidates, None, 0, 1000)
            .unwrap()
            .prior_confidence;
        // Predicted an improvement, measured a regression.
        let outcome = curiosity.conclude(-0.2, 0.4).unwrap();
        assert!(!outcome.supported);
        assert!(outcome.posterior_confidence < prior);
    }

    #[test]
    fn the_right_sign_with_the_wrong_magnitude_is_supported_but_miscalibrated() {
        let mut curiosity = curiosity();
        let candidates = vec![candidate("try it", 0.5, Support::Extrapolated, 0)];
        curiosity.propose(&candidates, None, 0, 1000);
        let outcome = curiosity.conclude(0.05, 0.5).unwrap();
        assert!(outcome.supported, "the direction was right");
        assert!(
            outcome.prediction_error() > 0.8,
            "and the magnitude was badly wrong: {}",
            outcome.prediction_error()
        );
        assert!(curiosity.calibration().unwrap() > 0.8);
    }

    #[test]
    fn an_abandoned_experiment_records_nothing() {
        // An interrupted experiment established nothing, and a recorded result
        // would be worse than none.
        let mut curiosity = curiosity();
        let candidates = vec![candidate("try it", 0.4, Support::Extrapolated, 0)];
        curiosity.propose(&candidates, None, 0, 1000);
        assert!(curiosity.abandon().is_some());
        assert!(curiosity.completed().is_empty());
        assert!(curiosity.conclude(0.1, 0.4).is_none());
    }

    #[test]
    fn an_experiment_knows_when_its_window_has_elapsed() {
        let mut curiosity = curiosity();
        let candidates = vec![candidate("try it", 0.4, Support::Extrapolated, 0)];
        let experiment = curiosity
            .propose(&candidates, None, 1_000_000, 5_000_000_000)
            .unwrap();
        assert!(!experiment.is_complete(2_000_000));
        assert!(experiment.is_complete(6_000_000_000));
    }
}
