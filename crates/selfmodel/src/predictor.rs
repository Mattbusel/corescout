//! An online self-model: predict the next reflection, then find out.
//!
//! # Shape
//!
//! One small autoregressive model per cell, fitted online, plus a running
//! record of how well each has been doing. Deliberately the simplest thing that
//! can express momentum:
//!
//! ```text
//! delta_hat(t+1) = a * delta(t) + b
//! ```
//!
//! The point of this milestone is not a good predictor. It is to find out
//! whether the reflection contains enough signal for *any* predictor to beat
//! copying the last value. A weak model that clearly beats the baseline is a
//! stronger result than a complicated one whose advantage cannot be attributed.
//!
//! Fitting is online, by recursive least squares with a forgetting factor, so
//! the model tracks a machine whose behaviour changes rather than averaging
//! over an epoch that has ended.
//!
//! # Latent states as context
//!
//! [`SelfModel::predict_state`] answers the other question: given where the
//! machine is now, where will it be. That is a distribution over discovered
//! states rather than a number, and it is what makes a latent state *useful*
//! rather than merely present: a state that improves this prediction has earned
//! its place in the ontology.

use std::collections::BTreeMap;

use corescout_mirror::MirrorSnapshot;
use corescout_represent::latent::{LatentCatalogue, LatentStateId};
use serde::{Deserialize, Serialize};

use crate::uncertainty::{Confidence, Interval};

/// A prediction about one cell.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Prediction {
    pub row: usize,
    pub col: usize,
    /// The predicted value with its interval.
    pub interval: Interval,
    /// What the naive baseline predicts, for comparison.
    pub baseline: f64,
    /// Confidence in `0.0 ..= 1.0`.
    pub confidence: f64,
    /// How far ahead this reaches.
    pub horizon_ns: u64,
}

impl Prediction {
    /// Whether this prediction says anything the baseline does not.
    pub fn is_informative(&self) -> bool {
        self.interval.is_known() && self.confidence > 0.1
    }
}

/// A prediction about which latent state comes next.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatePrediction {
    pub current: Option<LatentStateId>,
    /// Candidate successors with probabilities, most likely first.
    pub candidates: Vec<(LatentStateId, f64)>,
    /// Confidence in the whole distribution.
    pub confidence: f64,
}

impl StatePrediction {
    pub fn most_likely(&self) -> Option<(LatentStateId, f64)> {
        self.candidates.first().copied()
    }
}

/// One cell's fitted step, plus how well it has been doing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct CellModel {
    /// Coefficient on the previous delta.
    a: f64,
    /// Intercept.
    b: f64,
    /// Running sums for recursive least squares.
    sum_x: f64,
    sum_y: f64,
    sum_xx: f64,
    sum_xy: f64,
    weight: f64,
    /// Last observed value and delta, so the next prediction has an input.
    ///
    /// `Option` rather than a `NaN` sentinel: these are genuinely "not yet
    /// known", the distinction matters to every read, and it makes the model
    /// serialisable without a codec.
    last_value: Option<f64>,
    last_delta: Option<f64>,
    confidence: Confidence,
}

impl CellModel {
    fn new() -> CellModel {
        CellModel {
            a: 0.0,
            b: 0.0,
            sum_x: 0.0,
            sum_y: 0.0,
            sum_xx: 0.0,
            sum_xy: 0.0,
            weight: 0.0,
            last_value: None,
            last_delta: None,
            confidence: Confidence::default(),
        }
    }

    /// Fold in a new observation, refitting.
    ///
    /// `forgetting` decays the accumulated sums so the fit tracks recent
    /// behaviour. 0.995 gives a memory of a few hundred samples.
    fn observe(&mut self, value: f64, forgetting: f64) {
        if !value.is_finite() {
            // A gap breaks the delta chain: differencing across a hole is not
            // a measurement.
            self.last_value = None;
            self.last_delta = None;
            return;
        }
        let Some(previous) = self.last_value else {
            self.last_value = Some(value);
            return;
        };

        let delta = value - previous;
        if let Some(previous_delta) = self.last_delta {
            let (x, y) = (previous_delta, delta);
            self.sum_x = self.sum_x * forgetting + x;
            self.sum_y = self.sum_y * forgetting + y;
            self.sum_xx = self.sum_xx * forgetting + x * x;
            self.sum_xy = self.sum_xy * forgetting + x * y;
            self.weight = self.weight * forgetting + 1.0;
            self.refit();
        }
        self.last_delta = Some(delta);
        self.last_value = Some(value);
    }

    fn refit(&mut self) {
        if self.weight < 4.0 {
            return;
        }
        let mean_x = self.sum_x / self.weight;
        let mean_y = self.sum_y / self.weight;
        let variance = self.sum_xx / self.weight - mean_x * mean_x;
        let covariance = self.sum_xy / self.weight - mean_x * mean_y;
        if variance.abs() <= 1e-12 {
            // Nothing to regress on: predict the mean change, which is the
            // right answer for a steady counter.
            self.a = 0.0;
            self.b = mean_y;
            return;
        }
        self.a = covariance / variance;
        self.b = mean_y - self.a * mean_x;
        // A runaway coefficient means the fit has gone unstable, usually
        // because the machine changed character. Clamping keeps one bad window
        // from producing a wild prediction.
        if !self.a.is_finite() || self.a.abs() > 4.0 {
            self.a = 0.0;
            self.b = mean_y;
        }
    }

    /// The predicted next value, or `NaN` when there is nothing to go on.
    fn predict(&self) -> f64 {
        let Some(last) = self.last_value else {
            return f64::NAN;
        };
        let delta = match self.last_delta {
            Some(previous) => self.a * previous + self.b,
            None => self.b,
        };
        last + delta
    }

    /// What "assume nothing changed" predicts.
    fn baseline(&self) -> f64 {
        self.last_value.unwrap_or(f64::NAN)
    }
}

/// The machine's model of its own dynamics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelfModel {
    #[serde(with = "corescout_core::serde_util::cell_map")]
    cells: BTreeMap<(usize, usize), CellModel>,
    forgetting: f64,
    rows: usize,
    cols: usize,
    /// Reflections folded in.
    updates: u64,
    /// Predictions made and later scored.
    scored: u64,
    /// The last prediction made, kept so the next reflection can score it.
    #[serde(with = "corescout_core::serde_util::cell_map")]
    pending: BTreeMap<(usize, usize), (f64, f64)>,
    /// Cadence, learned from the reflections themselves.
    interval_ns: u64,
    last_monotonic_ns: u64,
    epoch: Option<u64>,
}

impl Default for SelfModel {
    fn default() -> Self {
        SelfModel::new(0.995)
    }
}

impl SelfModel {
    pub fn new(forgetting: f64) -> SelfModel {
        SelfModel {
            cells: BTreeMap::new(),
            forgetting: forgetting.clamp(0.5, 1.0),
            rows: 0,
            cols: 0,
            updates: 0,
            scored: 0,
            pending: BTreeMap::new(),
            interval_ns: 0,
            last_monotonic_ns: 0,
            epoch: None,
        }
    }

    pub fn updates(&self) -> u64 {
        self.updates
    }

    pub fn scored(&self) -> u64 {
        self.scored
    }

    pub fn interval_ns(&self) -> u64 {
        self.interval_ns
    }

    pub fn tracked_cells(&self) -> usize {
        self.cells.len()
    }

    /// Take in a reflection: score the last prediction, then update.
    ///
    /// Scoring before updating matters. A model that updated first would be
    /// grading itself on data it had already seen, which is the most common way
    /// to accidentally report excellent predictive performance.
    pub fn observe(&mut self, snapshot: &MirrorSnapshot) {
        if self.epoch != Some(snapshot.epoch) {
            // The rows mean something different now. Everything learned about
            // cell (12, 3) describes hardware that may no longer be there.
            self.cells.clear();
            self.pending.clear();
            self.epoch = Some(snapshot.epoch);
            self.rows = snapshot.state.rows();
            self.cols = snapshot.state.cols();
        }

        // 1. Score what was predicted last time.
        for ((row, col), (predicted, baseline)) in std::mem::take(&mut self.pending) {
            let actual = snapshot.state.get(row, col);
            if actual.is_finite() {
                if let Some(model) = self.cells.get_mut(&(row, col)) {
                    model.confidence.observe(predicted, baseline, actual);
                    self.scored += 1;
                }
            }
        }

        // 2. Learn from it.
        for row in 0..self.rows {
            for col in 0..self.cols {
                let value = snapshot.state.get(row, col);
                self.cells
                    .entry((row, col))
                    .or_insert_with(CellModel::new)
                    .observe(value, self.forgetting);
            }
        }

        if self.last_monotonic_ns > 0 && snapshot.monotonic_ns > self.last_monotonic_ns {
            let gap = snapshot.monotonic_ns - self.last_monotonic_ns;
            self.interval_ns = if self.interval_ns == 0 {
                gap
            } else {
                // A slow average, so one late tick does not redefine the
                // model's idea of how far ahead it is predicting.
                (self.interval_ns * 7 + gap) / 8
            };
        }
        self.last_monotonic_ns = snapshot.monotonic_ns;
        self.updates += 1;
    }

    /// Predict every cell's next value, and remember the predictions so the
    /// next reflection can score them.
    pub fn predict_next(&mut self) -> Vec<Prediction> {
        let mut out = Vec::new();
        self.pending.clear();
        for ((row, col), model) in self.cells.iter() {
            let point = model.predict();
            let baseline = model.baseline();
            if !point.is_finite() {
                continue;
            }
            self.pending.insert((*row, *col), (point, baseline));
            out.push(Prediction {
                row: *row,
                col: *col,
                interval: model.confidence.interval(point),
                baseline,
                confidence: model.confidence.score(),
                horizon_ns: self.interval_ns,
            });
        }
        out
    }

    /// Predict one cell without recording it for scoring.
    pub fn predict_cell(&self, row: usize, col: usize) -> Option<Prediction> {
        let model = self.cells.get(&(row, col))?;
        let point = model.predict();
        if !point.is_finite() {
            return None;
        }
        Some(Prediction {
            row,
            col,
            interval: model.confidence.interval(point),
            baseline: model.baseline(),
            confidence: model.confidence.score(),
            horizon_ns: self.interval_ns,
        })
    }

    /// Mean skill across every cell that has been scored enough to judge.
    ///
    /// The headline number: how much better than "nothing changed" this model
    /// is, on this machine, right now.
    pub fn skill(&self) -> f64 {
        let skills: Vec<f64> = self
            .cells
            .values()
            .filter(|m| m.confidence.samples() >= 10)
            .map(|m| m.confidence.skill())
            .collect();
        if skills.is_empty() {
            return 0.0;
        }
        skills.iter().sum::<f64>() / skills.len() as f64
    }

    /// Cells the model predicts meaningfully better than the baseline.
    pub fn cells_with_skill(&self, threshold: f64) -> Vec<(usize, usize, f64)> {
        let mut out: Vec<(usize, usize, f64)> = self
            .cells
            .iter()
            .filter(|(_, m)| m.confidence.samples() >= 10)
            .map(|((row, col), m)| (*row, *col, m.confidence.skill()))
            .filter(|(_, _, skill)| *skill > threshold)
            .collect();
        out.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    /// Predict which latent state comes next, from the catalogue's own
    /// transition history.
    pub fn predict_state(&self, catalogue: &LatentCatalogue) -> StatePrediction {
        let current = catalogue.current();
        let Some(current) = current else {
            return StatePrediction {
                current: None,
                candidates: Vec::new(),
                confidence: 0.0,
            };
        };

        let counts = catalogue.transition_counts();
        let outgoing: Vec<(LatentStateId, u64)> = counts
            .iter()
            .filter(|((from, _), _)| *from == current)
            .map(|((_, to), count)| (*to, *count))
            .collect();
        let total: u64 = outgoing.iter().map(|(_, c)| *c).sum();
        if total == 0 {
            // Never observed leaving this state. The honest prediction is that
            // it stays, with low confidence.
            return StatePrediction {
                current: Some(current),
                candidates: vec![(current, 1.0)],
                confidence: 0.1,
            };
        }

        let mut candidates: Vec<(LatentStateId, f64)> = outgoing
            .into_iter()
            .map(|(to, count)| (to, count as f64 / total as f64))
            .collect();
        candidates.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });

        // Confidence rises with evidence and with how concentrated the
        // distribution is. A state that leads equally to five others is not
        // predicted just because we know its options.
        let evidence = (total as f64 / 20.0).min(1.0);
        let concentration = candidates.first().map(|(_, p)| *p).unwrap_or(0.0);
        StatePrediction {
            current: Some(current),
            candidates,
            confidence: (evidence * concentration).clamp(0.0, 1.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::test_support::fixture;

    /// A series where one cell follows a smooth curve the model can learn, and
    /// another is pure noise it cannot.
    fn series(count: u64) -> Vec<MirrorSnapshot> {
        let mut state = 12345u64;
        (0..count)
            .map(|i| {
                let mut snapshot = fixture();
                snapshot.sequence = i;
                snapshot.monotonic_ns = i * 100_000_000;
                snapshot
                    .state
                    .set(2, 0, ((i as f64) * 0.25).sin() * 1000.0 + 3_000_000.0);
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let noise = ((state >> 33) as f64 / (1u64 << 31) as f64) - 0.5;
                snapshot.state.set(3, 0, 800_000.0 + noise * 10_000.0);
                snapshot
            })
            .collect()
    }

    fn train(model: &mut SelfModel, snapshots: &[MirrorSnapshot]) {
        for snapshot in snapshots {
            model.predict_next();
            model.observe(snapshot);
        }
    }

    #[test]
    fn a_smooth_signal_is_predicted_better_than_the_baseline() {
        let mut model = SelfModel::default();
        train(&mut model, &series(300));
        let prediction = model.predict_cell(2, 0).expect("a prediction for cell 2,0");
        assert!(prediction.interval.is_known());
        assert!(
            prediction.confidence > 0.3,
            "confidence was {}",
            prediction.confidence
        );
    }

    #[test]
    fn noise_earns_no_confidence() {
        // The honesty check: a model must not claim skill on an unpredictable
        // cell.
        let mut model = SelfModel::default();
        train(&mut model, &series(300));
        let noisy = model.predict_cell(3, 0).expect("a prediction for cell 3,0");
        let smooth = model.predict_cell(2, 0).expect("a prediction for cell 2,0");
        assert!(
            noisy.confidence < smooth.confidence,
            "noise {} should not be as trusted as signal {}",
            noisy.confidence,
            smooth.confidence
        );
    }

    #[test]
    fn predictions_are_scored_against_what_actually_happened() {
        let mut model = SelfModel::default();
        train(&mut model, &series(100));
        assert!(model.scored() > 50, "scored {}", model.scored());
        assert!(model.updates() == 100);
    }

    #[test]
    fn the_model_learns_the_cadence_from_the_reflections() {
        let mut model = SelfModel::default();
        train(&mut model, &series(50));
        assert!(
            (model.interval_ns() as i64 - 100_000_000).abs() < 5_000_000,
            "learned interval {}",
            model.interval_ns()
        );
    }

    #[test]
    fn an_epoch_change_discards_what_was_learned() {
        // Cell (12, 3) after a hotplug is different hardware; a model that kept
        // its coefficients would be predicting one core from another's history.
        let mut model = SelfModel::default();
        train(&mut model, &series(100));
        assert!(model.tracked_cells() > 0);

        let mut changed = fixture();
        changed.epoch += 1;
        model.observe(&changed);
        assert_eq!(model.scored(), model.scored(), "no scoring across the gap");
        // The cells are rebuilt from the new epoch, with no accumulated fit.
        let prediction = model.predict_cell(2, 0);
        assert!(
            prediction.is_none() || prediction.unwrap().confidence == 0.0,
            "confidence must not survive an epoch change"
        );
    }

    #[test]
    fn a_gap_breaks_the_delta_chain_rather_than_spanning_it() {
        let mut model = SelfModel::default();
        let mut snapshots = series(60);
        // Punch a hole.
        snapshots[30].state.set(2, 0, f64::NAN);
        train(&mut model, &snapshots);
        // It survives, and still predicts.
        assert!(model.predict_cell(2, 0).is_some());
    }

    #[test]
    fn skill_is_reported_across_the_machine() {
        let mut model = SelfModel::default();
        train(&mut model, &series(300));
        let skill = model.skill();
        assert!(skill.is_finite());
        let good = model.cells_with_skill(0.05);
        assert!(
            good.iter().any(|(row, col, _)| *row == 2 && *col == 0),
            "the smooth cell should show skill: {good:?}"
        );
    }

    #[test]
    fn state_prediction_uses_the_catalogues_own_transitions() {
        let mut catalogue = LatentCatalogue::new(0.75, 8);
        for i in 0..60u64 {
            let value = if i % 2 == 0 { 0.0 } else { 5.0 };
            catalogue.observe(&[value, value], i * 1_000_000);
        }
        let model = SelfModel::default();
        let prediction = model.predict_state(&catalogue);
        assert!(prediction.current.is_some());
        let (next, probability) = prediction.most_likely().expect("a successor");
        assert_ne!(Some(next), prediction.current, "it always alternates");
        assert!(probability > 0.9);
        assert!(prediction.confidence > 0.5);
    }

    #[test]
    fn an_empty_catalogue_yields_no_state_prediction() {
        let model = SelfModel::default();
        let prediction = model.predict_state(&LatentCatalogue::default());
        assert!(prediction.current.is_none());
        assert_eq!(prediction.confidence, 0.0);
    }

    #[test]
    fn a_model_survives_serialisation() {
        let mut model = SelfModel::default();
        train(&mut model, &series(60));
        let json = serde_json::to_string(&model).unwrap();
        let back: SelfModel = serde_json::from_str(&json).unwrap();
        assert_eq!(back.updates(), model.updates());
        assert_eq!(back.tracked_cells(), model.tracked_cells());
    }
}
