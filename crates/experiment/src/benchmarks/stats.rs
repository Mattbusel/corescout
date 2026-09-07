//! Robust statistics for benchmark samples.
//!
//! # Why robust statistics
//!
//! A benchmark sample set on a live machine is not a nice distribution around a
//! true value. It is a tight cluster of *good* measurements plus a long right
//! tail of measurements that were interrupted: a timer tick, an interrupt, a
//! migration, an SMT sibling waking up. The mean and standard deviation are
//! both dominated by that tail, so a single 40x outlier can make the fastest
//! core in the machine look mediocre.
//!
//! CoreScout therefore centres on the **median**, measures spread with the
//! **median absolute deviation** (MAD), and reports the tail explicitly as p95
//! and p99 rather than folding it into the headline number. The tail is not
//! noise to be discarded, it is the thing a latency-sensitive user came for,
//! so it gets its own ranking category instead of being averaged away.

use serde::{Deserialize, Serialize};

/// Scale factor making the MAD a consistent estimator of the standard
/// deviation for normally distributed data: `1 / Phi^-1(3/4)`.
pub const MAD_TO_SIGMA: f64 = 1.482_602_218_505_602;

/// Summary of one set of samples.
///
/// All fields are in the sample's own unit (nanoseconds for timings, arbitrary
/// units for throughput).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub count: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub median: f64,
    pub p95: f64,
    pub p99: f64,
    /// Median absolute deviation, the robust spread measure.
    pub mad: f64,
    /// Classical standard deviation, kept for comparison with other tools.
    pub stddev: f64,
    /// `p99 - median`: how much worse the tail is than the typical case. This
    /// is the number that matters for a latency budget.
    pub tail_excess: f64,
}

impl Summary {
    /// Summarise a slice of samples.
    ///
    /// Returns `None` for an empty slice rather than producing NaNs that would
    /// silently poison downstream ranking.
    pub fn from_samples(samples: &[f64]) -> Option<Summary> {
        if samples.is_empty() {
            return None;
        }
        let mut sorted: Vec<f64> = samples.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).expect("benchmark samples are never NaN"));

        let count = sorted.len();
        let mean = sorted.iter().sum::<f64>() / count as f64;
        let median = percentile_sorted(&sorted, 50.0);
        let p95 = percentile_sorted(&sorted, 95.0);
        let p99 = percentile_sorted(&sorted, 99.0);

        let variance = sorted.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / count as f64;

        let mut deviations: Vec<f64> = sorted.iter().map(|v| (v - median).abs()).collect();
        deviations.sort_by(|a, b| a.partial_cmp(b).expect("deviations are never NaN"));
        let mad = percentile_sorted(&deviations, 50.0);

        Some(Summary {
            count,
            min: sorted[0],
            max: sorted[count - 1],
            mean,
            median,
            p95,
            p99,
            mad,
            stddev: variance.sqrt(),
            tail_excess: p99 - median,
        })
    }

    /// Relative spread of the *typical* case, as a fraction of the median.
    ///
    /// Uses MAD rather than stddev so one preemption does not dominate.
    pub fn relative_jitter(&self) -> f64 {
        if self.median == 0.0 {
            0.0
        } else {
            (self.mad * MAD_TO_SIGMA) / self.median
        }
    }
}

/// Linear-interpolated percentile of an already sorted slice.
///
/// Interpolation matters at the small sample counts CoreScout uses: with 30
/// samples, a nearest-rank p99 is just "the maximum", which throws away
/// information and makes p95 and p99 identical.
pub fn percentile_sorted(sorted: &[f64], pct: f64) -> f64 {
    debug_assert!(!sorted.is_empty());
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = (pct / 100.0) * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        sorted[lower]
    } else {
        let frac = rank - lower as f64;
        sorted[lower] * (1.0 - frac) + sorted[upper] * frac
    }
}

/// Classification of a single sample relative to its cohort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleClass {
    Clean,
    /// Statistically distant from the median.
    Outlier,
}

/// Flag samples that are statistically implausible given the rest of the set.
///
/// The test is a one-sided robust z-score: `(x - median) / (MAD * 1.4826) >
/// threshold`. One-sided because a benchmark iteration can be arbitrarily
/// *slower* than the truth (something stole the CPU) but not meaningfully
/// faster than the hardware allows, so a fast sample is evidence about the
/// core, while a slow one is usually evidence about the machine.
///
/// When the MAD is zero (a very quiet machine producing identical timings) the
/// z-score is undefined; we fall back to a proportional threshold so a genuinely
/// stalled iteration is still caught.
pub fn classify_outliers(samples: &[f64], threshold: f64) -> Vec<SampleClass> {
    let Some(summary) = Summary::from_samples(samples) else {
        return Vec::new();
    };
    let scale = summary.mad * MAD_TO_SIGMA;
    samples
        .iter()
        .map(|&x| {
            let is_outlier = if scale > 0.0 {
                (x - summary.median) / scale > threshold
            } else {
                // Degenerate spread: anything more than 25% above the median.
                x > summary.median * 1.25
            };
            if is_outlier {
                SampleClass::Outlier
            } else {
                SampleClass::Clean
            }
        })
        .collect()
}

/// Return only the samples that are not statistical outliers.
pub fn clean_samples(samples: &[f64], threshold: f64) -> Vec<f64> {
    classify_outliers(samples, threshold)
        .into_iter()
        .zip(samples)
        .filter(|(class, _)| *class == SampleClass::Clean)
        .map(|(_, v)| *v)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    #[test]
    fn empty_input_yields_no_summary() {
        assert!(Summary::from_samples(&[]).is_none());
        assert!(classify_outliers(&[], 5.0).is_empty());
    }

    #[test]
    fn basic_statistics() {
        let s = Summary::from_samples(&[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
        approx(s.min, 1.0);
        approx(s.max, 5.0);
        approx(s.mean, 3.0);
        approx(s.median, 3.0);
        assert_eq!(s.count, 5);
    }

    #[test]
    fn percentiles_interpolate() {
        let sorted: Vec<f64> = (1..=100).map(|v| v as f64).collect();
        approx(percentile_sorted(&sorted, 0.0), 1.0);
        approx(percentile_sorted(&sorted, 100.0), 100.0);
        approx(percentile_sorted(&sorted, 50.0), 50.5);
    }

    #[test]
    fn single_sample_is_its_own_percentile() {
        approx(percentile_sorted(&[7.0], 99.0), 7.0);
    }

    #[test]
    fn median_survives_a_catastrophic_outlier() {
        // This is the whole reason for using the median: one interrupted
        // iteration must not move the headline number.
        let mut samples = vec![100.0; 30];
        samples.push(40_000.0);
        let s = Summary::from_samples(&samples).unwrap();
        approx(s.median, 100.0);
        assert!(s.mean > 1_000.0, "the mean should be wrecked, and is");
    }

    #[test]
    fn outlier_detection_finds_the_interrupted_iteration() {
        let mut samples: Vec<f64> = (0..40).map(|i| 100.0 + (i % 5) as f64).collect();
        samples.push(5_000.0);
        let classes = classify_outliers(&samples, 5.0);
        assert_eq!(classes[classes.len() - 1], SampleClass::Outlier);
        assert_eq!(
            classes
                .iter()
                .filter(|c| **c == SampleClass::Outlier)
                .count(),
            1
        );
    }

    #[test]
    fn fast_samples_are_never_outliers() {
        // One-sided: a suspiciously quick iteration is kept, because the
        // hardware floor is real information.
        let mut samples = vec![100.0; 30];
        samples.push(60.0);
        let classes = classify_outliers(&samples, 3.0);
        assert!(classes.iter().all(|c| *c == SampleClass::Clean));
    }

    #[test]
    fn zero_spread_falls_back_to_a_proportional_rule() {
        let mut samples = vec![100.0; 30];
        samples.push(200.0);
        let classes = classify_outliers(&samples, 3.0);
        assert_eq!(classes[30], SampleClass::Outlier);
    }

    #[test]
    fn cleaning_removes_only_outliers() {
        let mut samples = vec![100.0; 20];
        samples.push(9_000.0);
        let cleaned = clean_samples(&samples, 3.0);
        assert_eq!(cleaned.len(), 20);
        assert!(cleaned.iter().all(|v| *v == 100.0));
    }

    #[test]
    fn tail_excess_measures_the_gap_that_matters() {
        // 5% of iterations are slow: the median is untouched and the p99 sits
        // out in the tail, which is exactly the shape a latency user cares
        // about.
        let mut samples = vec![100.0; 95];
        samples.extend(std::iter::repeat(900.0).take(5));
        let s = Summary::from_samples(&samples).unwrap();
        assert_eq!(s.median, 100.0);
        assert!(s.tail_excess > 400.0, "p99 tail should be visible");
    }

    #[test]
    fn a_single_outlier_in_a_hundred_barely_moves_p99() {
        // The flip side, and the reason p99 is interpolated: one bad sample in
        // a hundred is a p99 event only marginally, and pretending otherwise
        // would make every core look like it has a terrible tail.
        let mut samples = vec![100.0; 99];
        samples.push(900.0);
        let s = Summary::from_samples(&samples).unwrap();
        assert!(s.max == 900.0);
        assert!(s.tail_excess < 50.0, "p99 was {}", s.p99);
    }
}
