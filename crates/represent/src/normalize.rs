//! Putting incommensurable numbers on a common scale.
//!
//! # Why this cannot be skipped
//!
//! One reflection contains frequencies near `4_000_000`, temperatures near
//! `50`, and cumulative cycle counts near `10^13`. Any distance measure applied
//! to that raw vector is a distance measure on the cycle counter and nothing
//! else. Every clustering, every regime, every latent state would be a
//! restatement of "the machine has been up for a while".
//!
//! # Robust by default
//!
//! The default scaling is median and median-absolute-deviation rather than mean
//! and standard deviation. Machine telemetry is full of one-sample excursions:
//! a scheduling hiccup, an interrupt storm, a counter wrapping. A single such
//! sample moves a mean and a standard deviation enough to compress everything
//! else into a narrow band, which destroys exactly the structure being looked
//! for.
//!
//! # Fitted once, applied everywhere
//!
//! A [`Normalizer`] is fitted on a window and then applied to later reflections
//! unchanged. Refitting per window would make two windows incomparable, and
//! a latent state discovered in one would not be recognisable in the next.
//! That stability is what allows a discovered concept to persist at all.

use serde::{Deserialize, Serialize};

/// How one column is scaled.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Scaling {
    pub centre: f64,
    pub spread: f64,
    /// True when the column never varied in the fitting window, so scaling it
    /// would divide by zero. Such a column contributes nothing to distance and
    /// is passed through as zero.
    pub degenerate: bool,
}

impl Scaling {
    pub fn apply(&self, value: f64) -> f64 {
        if !value.is_finite() {
            return f64::NAN;
        }
        if self.degenerate {
            return 0.0;
        }
        (value - self.centre) / self.spread
    }

    pub fn invert(&self, scaled: f64) -> f64 {
        if self.degenerate {
            return self.centre;
        }
        scaled * self.spread + self.centre
    }
}

/// A fitted per-column scaling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Normalizer {
    scalings: Vec<Scaling>,
    /// Whether the fit used the robust statistics.
    robust: bool,
}

impl Normalizer {
    /// Fit on a window of flat row-major frames, each `cols` wide.
    ///
    /// Every entity's value for a column contributes to that column's scaling,
    /// so two entities reporting the same quantity end up comparable, which is
    /// the whole point of scaling per column rather than per cell.
    pub fn fit(frames: &[Vec<f64>], cols: usize) -> Normalizer {
        Normalizer::fit_with(frames, cols, true)
    }

    pub fn fit_with(frames: &[Vec<f64>], cols: usize, robust: bool) -> Normalizer {
        let mut scalings = Vec::with_capacity(cols);
        for col in 0..cols {
            let values: Vec<f64> = frames
                .iter()
                .flat_map(|frame| {
                    frame
                        .iter()
                        .skip(col)
                        .step_by(cols.max(1))
                        .copied()
                        .filter(|v| v.is_finite())
                })
                .collect();
            scalings.push(fit_column(&values, robust));
        }
        Normalizer { scalings, robust }
    }

    /// Fit from per-cell series, which is the shape the discovery code holds.
    pub fn fit_series(
        series: &std::collections::BTreeMap<(usize, usize), Vec<f64>>,
        cols: usize,
    ) -> Normalizer {
        let mut scalings = Vec::with_capacity(cols);
        for col in 0..cols {
            let values: Vec<f64> = series
                .iter()
                .filter(|((_, c), _)| *c == col)
                .flat_map(|(_, v)| v.iter().copied())
                .filter(|v| v.is_finite())
                .collect();
            scalings.push(fit_column(&values, true));
        }
        Normalizer {
            scalings,
            robust: true,
        }
    }

    pub fn cols(&self) -> usize {
        self.scalings.len()
    }

    pub fn is_robust(&self) -> bool {
        self.robust
    }

    pub fn scaling(&self, col: usize) -> Option<Scaling> {
        self.scalings.get(col).copied()
    }

    /// Columns that never varied, and so carry no information for distance.
    pub fn degenerate_columns(&self) -> Vec<usize> {
        self.scalings
            .iter()
            .enumerate()
            .filter(|(_, s)| s.degenerate)
            .map(|(i, _)| i)
            .collect()
    }

    /// Scale one flat row-major frame.
    pub fn apply(&self, frame: &[f64], cols: usize) -> Vec<f64> {
        frame
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let col = if cols == 0 { 0 } else { index % cols };
                match self.scalings.get(col) {
                    Some(scaling) => scaling.apply(*value),
                    None => f64::NAN,
                }
            })
            .collect()
    }

    /// Scale a frame and replace unobserved cells with zero.
    ///
    /// Zero is the *centre* of a scaled column, so an unobserved cell becomes
    /// "typical" rather than extreme. That is the least-wrong filler for a
    /// distance computation: it neither pulls a point toward an edge nor
    /// silently drops a dimension. Which cells were filled is worth knowing, so
    /// the count is returned rather than hidden.
    pub fn apply_filled(&self, frame: &[f64], cols: usize) -> (Vec<f64>, usize) {
        let scaled = self.apply(frame, cols);
        let mut filled = 0;
        let out = scaled
            .into_iter()
            .map(|v| {
                if v.is_finite() {
                    v
                } else {
                    filled += 1;
                    0.0
                }
            })
            .collect();
        (out, filled)
    }
}

fn fit_column(values: &[f64], robust: bool) -> Scaling {
    if values.len() < 2 {
        return Scaling {
            centre: values.first().copied().unwrap_or(0.0),
            spread: 1.0,
            degenerate: true,
        };
    }
    if robust {
        let mut sorted: Vec<f64> = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        let centre = median_of_sorted(&sorted);
        let mut deviations: Vec<f64> = sorted.iter().map(|v| (v - centre).abs()).collect();
        deviations.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        // 1.4826 makes the MAD a consistent estimator of the standard
        // deviation for normally distributed data, so robust and classical
        // scalings produce comparable magnitudes.
        let spread = median_of_sorted(&deviations) * 1.482_602_218_505_602;
        if spread <= 1e-12 {
            return Scaling {
                centre,
                spread: 1.0,
                degenerate: true,
            };
        }
        Scaling {
            centre,
            spread,
            degenerate: false,
        }
    } else {
        let n = values.len() as f64;
        let centre = values.iter().sum::<f64>() / n;
        let variance = values.iter().map(|v| (v - centre).powi(2)).sum::<f64>() / n;
        let spread = variance.sqrt();
        if spread <= 1e-12 {
            return Scaling {
                centre,
                spread: 1.0,
                degenerate: true,
            };
        }
        Scaling {
            centre,
            spread,
            degenerate: false,
        }
    }
}

fn median_of_sorted(sorted: &[f64]) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(rows: usize, cols: usize, f: impl Fn(usize, usize, usize) -> f64) -> Vec<Vec<f64>> {
        (0..20)
            .map(|t| (0..rows * cols).map(|i| f(t, i / cols, i % cols)).collect())
            .collect()
    }

    #[test]
    fn a_huge_column_stops_dominating_after_scaling() {
        // The failure this module exists to prevent: a cumulative counter in
        // the trillions and a temperature near 50 in the same distance metric.
        let data = frames(2, 2, |t, _, col| {
            if col == 0 {
                1e13 + t as f64 * 1e9
            } else {
                50.0 + (t % 3) as f64
            }
        });
        let normalizer = Normalizer::fit(&data, 2);
        let scaled = normalizer.apply(&data[10], 2);
        // Both columns now live on the same order of magnitude.
        assert!(scaled[0].abs() < 10.0, "counter column: {}", scaled[0]);
        assert!(scaled[1].abs() < 10.0, "temperature column: {}", scaled[1]);
    }

    #[test]
    fn a_constant_column_is_marked_degenerate_rather_than_dividing_by_zero() {
        let data = frames(1, 2, |t, _, col| if col == 0 { 5.0 } else { t as f64 });
        let normalizer = Normalizer::fit(&data, 2);
        assert!(normalizer.scaling(0).unwrap().degenerate);
        assert!(!normalizer.scaling(1).unwrap().degenerate);
        assert_eq!(normalizer.degenerate_columns(), vec![0]);
        // And it scales to zero, contributing nothing to distance.
        assert_eq!(normalizer.apply(&data[3], 2)[0], 0.0);
    }

    #[test]
    fn the_robust_fit_is_not_moved_by_one_excursion() {
        let mut values: Vec<f64> = (0..40).map(|i| 100.0 + (i % 4) as f64).collect();
        values.push(1_000_000.0);
        let robust = fit_column(&values, true);
        let classical = fit_column(&values, false);
        assert!(
            (robust.centre - 101.0).abs() < 2.0,
            "robust centre moved to {}",
            robust.centre
        );
        assert!(
            classical.centre > 1_000.0,
            "the classical centre should be wrecked, and is: {}",
            classical.centre
        );
    }

    #[test]
    fn scaling_inverts() {
        let data = frames(1, 1, |t, _, _| t as f64 * 3.0);
        let normalizer = Normalizer::fit(&data, 1);
        let scaling = normalizer.scaling(0).unwrap();
        let original = 27.0;
        assert!((scaling.invert(scaling.apply(original)) - original).abs() < 1e-9);
    }

    #[test]
    fn unobserved_cells_fill_to_the_centre_and_are_counted() {
        let data = frames(1, 2, |t, _, _| t as f64);
        let normalizer = Normalizer::fit(&data, 2);
        let (filled, count) = normalizer.apply_filled(&[f64::NAN, 5.0], 2);
        assert_eq!(count, 1);
        assert_eq!(filled[0], 0.0, "an unobserved cell becomes typical");
        assert!(filled[1].is_finite());
    }

    #[test]
    fn a_fitted_normalizer_applies_unchanged_to_later_data() {
        // Two windows must stay comparable, or a latent state found in one is
        // not recognisable in the next.
        let early = frames(1, 1, |t, _, _| t as f64);
        let normalizer = Normalizer::fit(&early, 1);
        let a = normalizer.apply(&[10.0], 1);
        let b = normalizer.apply(&[10.0], 1);
        assert_eq!(a, b);
    }

    #[test]
    fn an_empty_fit_does_not_panic() {
        let normalizer = Normalizer::fit(&[], 3);
        assert_eq!(normalizer.cols(), 3);
        assert!(normalizer.scaling(0).unwrap().degenerate);
    }
}
