//! How sure the model is, and why that has to be reported.
//!
//! # A prediction without a confidence is not usable
//!
//! A controller deciding whether to move a thread needs to distinguish
//! "predicted improvement 12%, and I have been within 2% on this cell for the
//! last hundred ticks" from "predicted improvement 12%, and I have never seen
//! this cell behave this way before". Those are the same number and opposite
//! decisions.
//!
//! # Confidence is per-cell and recent
//!
//! A global "the model is 80% accurate" figure is nearly useless: a mirror has
//! cells that are trivially predictable and cells that are essentially noise,
//! and averaging them describes neither. So confidence is tracked per cell,
//! from a decaying window of recent errors, which also lets it *fall* when the
//! machine changes character.

use serde::{Deserialize, Serialize};

/// A predicted range.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    #[serde(with = "corescout_core::serde_util::maybe_finite")]
    pub point: f64,
    #[serde(with = "corescout_core::serde_util::maybe_finite")]
    pub low: f64,
    #[serde(with = "corescout_core::serde_util::maybe_finite")]
    pub high: f64,
}

impl Interval {
    pub fn new(point: f64, half_width: f64) -> Interval {
        let half_width = half_width.abs();
        Interval {
            point,
            low: point - half_width,
            high: point + half_width,
        }
    }

    /// An interval with no width, for a value known exactly.
    pub fn exact(point: f64) -> Interval {
        Interval {
            point,
            low: point,
            high: point,
        }
    }

    /// An interval expressing complete ignorance.
    pub fn unknown() -> Interval {
        Interval {
            point: f64::NAN,
            low: f64::NEG_INFINITY,
            high: f64::INFINITY,
        }
    }

    pub fn width(&self) -> f64 {
        self.high - self.low
    }

    pub fn contains(&self, value: f64) -> bool {
        value >= self.low && value <= self.high
    }

    pub fn is_known(&self) -> bool {
        self.point.is_finite() && self.width().is_finite()
    }
}

/// A running estimate of how well one cell is being predicted.
///
/// Exponentially weighted, so it adapts when the machine changes rather than
/// averaging over an epoch of behaviour that has ended.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Confidence {
    /// Decaying mean absolute error.
    mean_error: f64,
    /// Decaying mean absolute error of the naive baseline, for comparison.
    baseline_error: f64,
    /// Decaying mean squared error, for the interval width.
    mean_squared_error: f64,
    /// Observations folded in.
    samples: u64,
    /// Weight given to each new observation.
    alpha: f64,
}

impl Default for Confidence {
    fn default() -> Self {
        Confidence::new(0.05)
    }
}

impl Confidence {
    /// `alpha` is the weight of each new sample. 0.05 gives a memory of
    /// roughly the last twenty observations.
    pub fn new(alpha: f64) -> Confidence {
        Confidence {
            mean_error: 0.0,
            baseline_error: 0.0,
            mean_squared_error: 0.0,
            samples: 0,
            alpha: alpha.clamp(1e-4, 1.0),
        }
    }

    /// Fold in one prediction and what actually happened.
    pub fn observe(&mut self, predicted: f64, baseline: f64, actual: f64) {
        if !predicted.is_finite() || !actual.is_finite() {
            return;
        }
        let error = (predicted - actual).abs();
        let baseline_error = (baseline - actual).abs();
        if self.samples == 0 {
            self.mean_error = error;
            self.baseline_error = baseline_error;
            self.mean_squared_error = error * error;
        } else {
            self.mean_error += self.alpha * (error - self.mean_error);
            self.baseline_error += self.alpha * (baseline_error - self.baseline_error);
            self.mean_squared_error += self.alpha * (error * error - self.mean_squared_error);
        }
        self.samples += 1;
    }

    pub fn samples(&self) -> u64 {
        self.samples
    }

    pub fn mean_error(&self) -> f64 {
        self.mean_error
    }

    /// Typical error magnitude, for sizing a prediction interval.
    pub fn sigma(&self) -> f64 {
        self.mean_squared_error.max(0.0).sqrt()
    }

    /// Skill against the naive baseline: `1 - model/baseline`.
    ///
    /// Positive means the model is beating "assume nothing changed"; zero means
    /// it is merely matching it; negative means it is actively worse, which is
    /// worth knowing and worth reporting rather than clamping away.
    pub fn skill(&self) -> f64 {
        // The baseline is perfect, or so nearly perfect that the ratio is an
        // artefact of the divisor rather than a fact about the model.
        //
        // The threshold has to be *relative*. An absolute floor of 1e-12 is
        // meaningless on a channel whose values are around 1e15: a baseline
        // error of 1e-9 there is a perfect prediction in every sense that
        // matters, and dividing by it produced a reported skill of -3.7e11 the
        // first time this ran on real hardware.
        let scale = self.mean_error.abs().max(self.baseline_error.abs());
        if self.baseline_error <= 1e-12 || self.baseline_error <= scale * 1e-9 {
            return 0.0;
        }
        // Bounded below. A model can be arbitrarily worse than the baseline,
        // and letting one cell report -400 would let it swamp any average it
        // appears in. Minus one means "as wrong as the baseline is right",
        // which is as much detail as an aggregate can carry.
        (1.0 - (self.mean_error / self.baseline_error)).max(-1.0)
    }

    /// A confidence in `0.0 ..= 1.0`, combining skill with evidence.
    ///
    /// A model that has beaten the baseline five times is not as trustworthy as
    /// one that has beaten it five hundred times, so the score is damped by how
    /// much has been seen.
    pub fn score(&self) -> f64 {
        if self.samples == 0 {
            return 0.0;
        }
        let evidence = (self.samples as f64 / 50.0).min(1.0);
        let skill = self.skill().clamp(0.0, 1.0);
        (skill * evidence).clamp(0.0, 1.0)
    }

    /// A prediction interval around a point estimate.
    ///
    /// Two sigma of recent error. Not a rigorous confidence interval, and
    /// documented as such: it is a statement about how wrong this model has
    /// recently been on this cell, which is the useful thing and is not the
    /// same as a probabilistic guarantee.
    pub fn interval(&self, point: f64) -> Interval {
        if self.samples < 3 {
            return Interval::unknown();
        }
        Interval::new(point, 2.0 * self.sigma())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_interval_knows_what_it_contains() {
        let interval = Interval::new(10.0, 2.0);
        assert!(interval.contains(9.0));
        assert!(!interval.contains(13.0));
        assert_eq!(interval.width(), 4.0);
        assert!(interval.is_known());
    }

    #[test]
    fn ignorance_is_representable() {
        let unknown = Interval::unknown();
        assert!(!unknown.is_known());
        assert!(
            unknown.contains(1e300),
            "an unknown interval excludes nothing"
        );
    }

    #[test]
    fn a_model_that_matches_the_baseline_has_no_skill() {
        let mut confidence = Confidence::new(0.2);
        for _ in 0..50 {
            confidence.observe(10.0, 10.0, 12.0);
        }
        assert!(confidence.skill().abs() < 1e-6);
        assert_eq!(confidence.score(), 0.0);
    }

    #[test]
    fn a_model_that_beats_the_baseline_earns_confidence() {
        let mut confidence = Confidence::new(0.2);
        for _ in 0..100 {
            // The model is off by 1, the baseline by 4.
            confidence.observe(11.0, 8.0, 12.0);
        }
        assert!(confidence.skill() > 0.7, "skill was {}", confidence.skill());
        assert!(confidence.score() > 0.7);
    }

    #[test]
    fn a_model_that_is_worse_than_the_baseline_reports_negative_skill() {
        // Clamping this to zero would hide the most important thing a model
        // can tell you about itself.
        let mut confidence = Confidence::new(0.2);
        for _ in 0..50 {
            confidence.observe(50.0, 11.0, 12.0);
        }
        assert!(confidence.skill() < 0.0);
        assert_eq!(confidence.score(), 0.0, "score floors at zero");
    }

    #[test]
    fn little_evidence_means_little_confidence_even_when_right() {
        let mut sparse = Confidence::new(0.2);
        let mut plentiful = Confidence::new(0.2);
        for _ in 0..3 {
            sparse.observe(11.0, 8.0, 12.0);
        }
        for _ in 0..100 {
            plentiful.observe(11.0, 8.0, 12.0);
        }
        assert!(
            sparse.score() < plentiful.score(),
            "three correct predictions is not the same evidence as a hundred"
        );
    }

    #[test]
    fn confidence_falls_when_the_machine_changes_character() {
        let mut confidence = Confidence::new(0.2);
        for _ in 0..100 {
            confidence.observe(11.0, 8.0, 12.0);
        }
        let before = confidence.skill();
        for _ in 0..50 {
            // The model is suddenly wrong and the baseline is right.
            confidence.observe(11.0, 40.0, 40.0);
        }
        assert!(
            confidence.skill() < before,
            "a model must notice when it stops working"
        );
    }

    #[test]
    fn an_interval_needs_evidence_before_it_claims_a_width() {
        let mut confidence = Confidence::new(0.2);
        assert!(!confidence.interval(10.0).is_known());
        for _ in 0..10 {
            confidence.observe(10.0, 10.0, 10.5);
        }
        let interval = confidence.interval(10.0);
        assert!(interval.is_known());
        assert!(interval.contains(10.5));
    }

    #[test]
    fn unobservable_values_are_ignored_rather_than_poisoning_the_estimate() {
        let mut confidence = Confidence::new(0.2);
        confidence.observe(f64::NAN, 1.0, 1.0);
        confidence.observe(1.0, 1.0, f64::NAN);
        assert_eq!(confidence.samples(), 0);
    }
}
