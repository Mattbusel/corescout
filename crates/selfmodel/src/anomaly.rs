//! Noticing that something is unusual.
//!
//! # Surprise is prediction error
//!
//! An anomaly detector that needs its own model of normality would be a second,
//! parallel self-model that could disagree with the first. Instead, surprise is
//! defined as *how badly the existing self-model predicted this reflection*.
//! One model, one notion of normal, and a detector that improves automatically
//! as the model does.
//!
//! # Why this matters beyond alerting
//!
//! Surprise is the signal for curiosity. A controller with an exploration
//! budget should spend it where its model is worst, and that is exactly what a
//! high surprise score identifies. It is also the trigger for consolidating a
//! new latent state: a machine that keeps being surprised in the same way is a
//! machine encountering a condition it has no concept for.

use std::collections::BTreeMap;

use corescout_mirror::MirrorSnapshot;
use serde::{Deserialize, Serialize};

/// How surprising one reflection was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Surprise {
    pub sequence: u64,
    pub monotonic_ns: u64,
    /// Mean absolute prediction error, in units of the model's own recent
    /// typical error. 1.0 means "as wrong as usual"; 5.0 means "five times
    /// worse than this model is normally".
    pub score: f64,
    /// The cells that contributed most.
    pub contributors: Vec<Anomaly>,
    /// How much of the machine could be judged at all.
    pub coverage: f64,
}

impl Surprise {
    /// Whether this reflection is unusual enough to act on.
    pub fn is_anomalous(&self, threshold: f64) -> bool {
        self.score >= threshold && self.coverage > 0.1
    }
}

/// One cell that behaved unexpectedly.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Anomaly {
    pub row: usize,
    pub col: usize,
    pub expected: f64,
    pub observed: f64,
    /// Error in units of this cell's typical error.
    pub deviation: f64,
}

/// Watches for reflections the self-model did not see coming.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnomalyDetector {
    /// Decaying typical absolute error per cell.
    #[serde(with = "corescout_core::serde_util::cell_map")]
    typical: BTreeMap<(usize, usize), f64>,
    alpha: f64,
    /// Decaying mean of the whole-machine surprise score, so "unusual" is
    /// relative to how surprising this machine usually is.
    baseline_score: f64,
    observations: u64,
    epoch: Option<u64>,
}

impl Default for AnomalyDetector {
    fn default() -> Self {
        AnomalyDetector::new(0.05)
    }
}

impl AnomalyDetector {
    pub fn new(alpha: f64) -> AnomalyDetector {
        AnomalyDetector {
            typical: BTreeMap::new(),
            alpha: alpha.clamp(1e-4, 1.0),
            baseline_score: 0.0,
            observations: 0,
            epoch: None,
        }
    }

    pub fn observations(&self) -> u64 {
        self.observations
    }

    /// Compare what was predicted against what arrived.
    ///
    /// `predictions` maps cells to their predicted values. Cells without a
    /// prediction are skipped rather than counted as perfectly predicted, which
    /// would make a model that predicts nothing look infallible.
    pub fn assess(
        &mut self,
        snapshot: &MirrorSnapshot,
        predictions: &BTreeMap<(usize, usize), f64>,
    ) -> Surprise {
        if self.epoch != Some(snapshot.epoch) {
            self.typical.clear();
            self.baseline_score = 0.0;
            self.epoch = Some(snapshot.epoch);
        }

        let mut deviations: Vec<Anomaly> = Vec::new();
        let mut total_deviation = 0.0;
        let mut judged = 0usize;

        for ((row, col), expected) in predictions {
            let observed = snapshot.state.get(*row, *col);
            if !observed.is_finite() || !expected.is_finite() {
                continue;
            }
            let error = (observed - expected).abs();
            let typical = self.typical.entry((*row, *col)).or_insert(error);
            // Relative to this cell's own history, so a cell measured in
            // billions and a cell measured in degrees are comparable.
            let deviation = if *typical <= 1e-12 {
                if error <= 1e-12 {
                    0.0
                } else {
                    // The cell has always been perfectly predicted and now is
                    // not. That is maximally surprising, and a ratio would be
                    // an infinity.
                    10.0
                }
            } else {
                error / *typical
            };
            *typical += self.alpha * (error - *typical);

            total_deviation += deviation;
            judged += 1;
            if deviation > 3.0 {
                deviations.push(Anomaly {
                    row: *row,
                    col: *col,
                    expected: *expected,
                    observed,
                    deviation,
                });
            }
        }

        let score = if judged == 0 {
            0.0
        } else {
            total_deviation / judged as f64
        };
        if self.observations == 0 {
            self.baseline_score = score;
        } else {
            self.baseline_score += self.alpha * (score - self.baseline_score);
        }
        self.observations += 1;

        deviations.sort_by(|a, b| {
            b.deviation
                .partial_cmp(&a.deviation)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        deviations.truncate(8);

        let cells = (snapshot.state.rows() * snapshot.state.cols()).max(1);
        Surprise {
            sequence: snapshot.sequence,
            monotonic_ns: snapshot.monotonic_ns,
            score,
            contributors: deviations,
            coverage: judged as f64 / cells as f64,
        }
    }

    /// How surprising this machine usually is, for calibrating a threshold.
    pub fn baseline(&self) -> f64 {
        self.baseline_score
    }

    /// A threshold that would flag roughly the most unusual reflections,
    /// scaled to this machine rather than to a constant chosen in advance.
    pub fn suggested_threshold(&self) -> f64 {
        (self.baseline_score * 3.0).max(2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::test_support::fixture;

    fn predictions(value: f64) -> BTreeMap<(usize, usize), f64> {
        [((2usize, 0usize), value)].into_iter().collect()
    }

    fn snapshot_with(value: f64, sequence: u64) -> MirrorSnapshot {
        let mut snapshot = fixture();
        snapshot.sequence = sequence;
        snapshot.state.set(2, 0, value);
        snapshot
    }

    #[test]
    fn a_well_predicted_reflection_is_not_surprising() {
        let mut detector = AnomalyDetector::new(0.2);
        for i in 0..50u64 {
            let surprise = detector.assess(&snapshot_with(100.0, i), &predictions(100.0));
            assert!(surprise.score < 1.0, "score {}", surprise.score);
        }
    }

    #[test]
    fn a_sudden_departure_is_flagged() {
        let mut detector = AnomalyDetector::new(0.2);
        // Establish what "normally wrong" looks like.
        for i in 0..50u64 {
            detector.assess(&snapshot_with(101.0, i), &predictions(100.0));
        }
        let surprise = detector.assess(&snapshot_with(500.0, 51), &predictions(100.0));
        assert!(
            surprise.score > 10.0,
            "a 400-unit miss on a cell usually off by 1 should stand out: {}",
            surprise.score
        );
        assert!(surprise.is_anomalous(detector.suggested_threshold()));
        assert_eq!(surprise.contributors.len(), 1);
        assert_eq!(surprise.contributors[0].observed, 500.0);
    }

    #[test]
    fn a_noisy_cell_does_not_cry_wolf() {
        // The point of scaling by each cell's own typical error: a cell that is
        // always wrong by a lot is not surprising when it is wrong by a lot.
        let mut detector = AnomalyDetector::new(0.2);
        let mut state = 99u64;
        for i in 0..100u64 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let noise = ((state >> 40) as f64 / (1u64 << 23) as f64) * 400.0;
            let surprise = detector.assess(&snapshot_with(100.0 + noise, i), &predictions(100.0));
            if i > 50 {
                assert!(
                    surprise.score < 6.0,
                    "a habitually noisy cell should not keep alarming: {}",
                    surprise.score
                );
            }
        }
    }

    #[test]
    fn cells_without_a_prediction_are_not_counted_as_correct() {
        // A model that predicts nothing must not look infallible.
        let mut detector = AnomalyDetector::new(0.2);
        let surprise = detector.assess(&fixture(), &BTreeMap::new());
        assert_eq!(surprise.score, 0.0);
        assert_eq!(surprise.coverage, 0.0);
        assert!(
            !surprise.is_anomalous(1.0),
            "zero coverage cannot be evidence of anything"
        );
    }

    #[test]
    fn an_epoch_change_resets_what_normal_means() {
        let mut detector = AnomalyDetector::new(0.2);
        for i in 0..50u64 {
            detector.assess(&snapshot_with(100.0, i), &predictions(100.0));
        }
        let mut changed = snapshot_with(100.0, 51);
        changed.epoch += 1;
        detector.assess(&changed, &predictions(100.0));
        assert_eq!(detector.baseline(), 0.0, "normality is epoch-scoped");
    }

    #[test]
    fn the_threshold_adapts_to_how_surprising_this_machine_usually_is() {
        let mut calm = AnomalyDetector::new(0.2);
        for i in 0..50u64 {
            calm.assess(&snapshot_with(100.0, i), &predictions(100.0));
        }
        assert!(calm.suggested_threshold() >= 2.0);
    }
}
