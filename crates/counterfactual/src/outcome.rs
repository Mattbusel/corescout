//! What a counterfactual answer looks like.

use corescout_intent::{Achievement, Intent};
use serde::{Deserialize, Serialize};

/// How much evidence stands behind a prediction.
///
/// The distinction that keeps a counterfactual model honest: a prediction from
/// forty observed trials and a prediction from none look identical unless the
/// type forces them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    /// This exact action, on this target, has been tried and observed.
    Observed,
    /// A similar action on a different target has been observed, and the
    /// effect is being transferred.
    Transferred,
    /// Nothing comparable has been tried. The prediction is an extrapolation
    /// from the passive model alone.
    Extrapolated,
    /// Not enough is known to say anything.
    None,
}

impl Support {
    pub fn label(self) -> &'static str {
        match self {
            Support::Observed => "observed",
            Support::Transferred => "transferred",
            Support::Extrapolated => "extrapolated",
            Support::None => "unsupported",
        }
    }

    /// A multiplier on confidence, reflecting how much the evidence is worth.
    pub fn weight(self) -> f64 {
        match self {
            Support::Observed => 1.0,
            Support::Transferred => 0.5,
            Support::Extrapolated => 0.15,
            Support::None => 0.0,
        }
    }

    /// Whether a controller should act on this without experimenting first.
    pub fn is_actionable(self) -> bool {
        matches!(self, Support::Observed | Support::Transferred)
    }
}

/// Confidence in a counterfactual, and where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Confidence {
    pub support: Support,
    /// Trials behind it.
    pub trials: u32,
    /// `0.0 ..= 1.0`.
    pub score: f64,
}

impl Confidence {
    pub fn none() -> Confidence {
        Confidence {
            support: Support::None,
            trials: 0,
            score: 0.0,
        }
    }

    /// Combine evidence quality with how consistent the observed effect was.
    pub fn from(support: Support, trials: u32, consistency: f64) -> Confidence {
        let evidence = (trials as f64 / 10.0).min(1.0);
        Confidence {
            support,
            trials,
            score: (support.weight() * evidence * consistency.clamp(0.0, 1.0)).clamp(0.0, 1.0),
        }
    }
}

/// One imagined future.
///
/// Deliberately *not* a `MirrorSnapshot`. A predicted state must never be
/// mistakable for an observed one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    /// Predicted change per `(entity row, channel column)`, in the channel's
    /// own unit. Absent cells are cells the model has nothing to say about,
    /// which is different from cells it predicts will not change.
    pub deltas: Vec<(usize, usize, f64)>,
    /// How far ahead this reaches.
    pub horizon_ns: u64,
    pub confidence: Confidence,
}

impl Outcome {
    pub fn unknown() -> Outcome {
        Outcome {
            deltas: Vec::new(),
            horizon_ns: 0,
            confidence: Confidence::none(),
        }
    }

    pub fn delta(&self, row: usize, col: usize) -> Option<f64> {
        self.deltas
            .iter()
            .find(|(r, c, _)| *r == row && *c == col)
            .map(|(_, _, v)| *v)
    }

    /// Whether this says anything at all.
    pub fn is_informative(&self) -> bool {
        !self.deltas.is_empty() && self.confidence.score > 0.0
    }
}

/// A complete answer to "what if I did this".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prediction {
    /// The action family this concerns.
    pub family: String,
    /// A short description of the action, for audit and reporting.
    pub description: String,
    pub outcome: Outcome,
    /// Predicted achievement against an intent, where one was supplied.
    pub achievement: Option<Achievement>,
    /// Predicted improvement as a fraction, positive meaning better. `None`
    /// when the objectives an intent names cannot be predicted at all.
    pub improvement: Option<f64>,
}

impl Prediction {
    pub fn support(&self) -> Support {
        self.outcome.confidence.support
    }

    /// Whether a controller should act on this now, or experiment first.
    ///
    /// The threshold is not just on predicted gain: an unsupported prediction
    /// of a large gain is exactly the shape of a mistake, and is precisely when
    /// a bounded experiment is the right move instead.
    pub fn worth_acting_on(&self, intent: &Intent) -> bool {
        let Some(improvement) = self.improvement else {
            return false;
        };
        improvement > intent.migration_tolerance.improvement_threshold()
            && self.support().is_actionable()
            && self.outcome.confidence.score > 0.25
    }

    /// Whether this is a good candidate for an experiment: plausibly useful,
    /// and not yet known.
    pub fn worth_investigating(&self) -> bool {
        matches!(self.support(), Support::Extrapolated | Support::Transferred)
            && self.improvement.is_some_and(|i| i > 0.0)
    }
}

/// Score a predicted achievement against an intent.
///
/// Returns a fraction where positive is better, or `None` when nothing the
/// intent cares about could be predicted. Constraint violations are absolute:
/// a candidate that breaks one is not merely penalised.
pub fn score(intent: &Intent, predicted: &Achievement, current: &Achievement) -> Option<f64> {
    if !predicted.violations(intent).is_empty() {
        return Some(f64::NEG_INFINITY);
    }
    let mut total = 0.0;
    let mut weight_used = 0.0;
    for objective in intent.objectives() {
        let weight = intent.weight(objective);
        if weight <= 0.0 {
            continue;
        }
        let (Some(before), Some(after)) = (current.get(objective), predicted.get(objective)) else {
            continue;
        };
        if !before.is_finite() || !after.is_finite() || before == 0.0 {
            continue;
        }
        // Fractional change, oriented so positive always means better.
        let change = if objective.lower_is_better() {
            (before - after) / before.abs()
        } else {
            (after - before) / before.abs()
        };
        total += weight * change;
        weight_used += weight;
    }
    if weight_used <= 0.0 {
        return None;
    }
    Some(total / weight_used)
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_intent::Objective;
    use corescout_intent::{Constraint, MigrationTolerance};

    fn intent() -> Intent {
        Intent::new("test")
            .with_constraint(Constraint::latency_p99_us(30.0))
            .preferring(Objective::Latency, 1.0)
            .migration(MigrationTolerance::Medium)
    }

    #[test]
    fn support_distinguishes_evidence_from_extrapolation() {
        assert!(Support::Observed.is_actionable());
        assert!(Support::Transferred.is_actionable());
        assert!(!Support::Extrapolated.is_actionable());
        assert!(Support::Observed.weight() > Support::Extrapolated.weight());
    }

    #[test]
    fn confidence_needs_both_evidence_and_consistency() {
        let plenty = Confidence::from(Support::Observed, 50, 1.0);
        let few = Confidence::from(Support::Observed, 2, 1.0);
        let inconsistent = Confidence::from(Support::Observed, 50, 0.1);
        assert!(plenty.score > few.score);
        assert!(plenty.score > inconsistent.score);
        assert_eq!(Confidence::none().score, 0.0);
    }

    #[test]
    fn a_constraint_violation_is_absolute_not_penalised() {
        let current = Achievement::new().with(Objective::Latency, 25.0);
        let predicted = Achievement::new().with(Objective::Latency, 45.0);
        let result = score(&intent(), &predicted, &current).unwrap();
        assert_eq!(result, f64::NEG_INFINITY);
    }

    #[test]
    fn improvement_is_oriented_so_positive_is_always_better() {
        let current = Achievement::new().with(Objective::Latency, 20.0);
        let better = Achievement::new().with(Objective::Latency, 10.0);
        assert!(score(&intent(), &better, &current).unwrap() > 0.0);

        let throughput = Intent::new("t").preferring(Objective::Throughput, 1.0);
        let current = Achievement::new().with(Objective::Throughput, 100.0);
        let more = Achievement::new().with(Objective::Throughput, 200.0);
        assert!(score(&throughput, &more, &current).unwrap() > 0.0);
    }

    #[test]
    fn an_unpredictable_objective_yields_no_score_rather_than_zero() {
        let current = Achievement::new();
        let predicted = Achievement::new();
        // Nothing measurable, so no claim. Zero would read as "no change",
        // which is a much stronger statement.
        let bare = Intent::new("t").preferring(Objective::Energy, 1.0);
        assert_eq!(score(&bare, &predicted, &current), None);
    }

    #[test]
    fn an_unsupported_prediction_is_not_acted_on_however_good_it_looks() {
        // The failure mode this guards: a large predicted gain with no evidence
        // is the exact shape of a mistake.
        let prediction = Prediction {
            family: "affinity".into(),
            description: "move to entity 13".into(),
            outcome: Outcome {
                deltas: vec![(0, 0, -10.0)],
                horizon_ns: 100_000_000,
                confidence: Confidence::from(Support::Extrapolated, 0, 1.0),
            },
            achievement: None,
            improvement: Some(0.9),
        };
        assert!(!prediction.worth_acting_on(&intent()));
        assert!(
            prediction.worth_investigating(),
            "but it is exactly what an experiment is for"
        );
    }

    #[test]
    fn a_well_supported_improvement_is_actionable() {
        let prediction = Prediction {
            family: "affinity".into(),
            description: "move to entity 13".into(),
            outcome: Outcome {
                deltas: vec![(0, 0, -10.0)],
                horizon_ns: 100_000_000,
                confidence: Confidence::from(Support::Observed, 40, 0.9),
            },
            achievement: None,
            improvement: Some(0.4),
        };
        assert!(prediction.worth_acting_on(&intent()));
        assert!(!prediction.worth_investigating(), "already known");
    }

    #[test]
    fn migration_tolerance_gates_action() {
        let prediction = Prediction {
            family: "affinity".into(),
            description: "small gain".into(),
            outcome: Outcome {
                deltas: vec![(0, 0, -1.0)],
                horizon_ns: 100_000_000,
                confidence: Confidence::from(Support::Observed, 40, 0.9),
            },
            achievement: None,
            improvement: Some(0.05),
        };
        // Medium tolerance wants 8%; a 5% gain is not worth the move.
        assert!(!prediction.worth_acting_on(&intent()));
        let tolerant = intent().migration(MigrationTolerance::High);
        assert!(prediction.worth_acting_on(&tolerant));
    }

    #[test]
    fn an_unknown_outcome_says_nothing() {
        let outcome = Outcome::unknown();
        assert!(!outcome.is_informative());
        assert_eq!(outcome.delta(0, 0), None);
    }
}
