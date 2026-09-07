//! Predicting the next reflection.
//!
//! ```text
//! F(M(t)) -> M_hat(t+1)
//! ```
//!
//! # Why the baselines matter more than the model
//!
//! Predicting a mirror snapshot is easy to do impressively badly. Most cells
//! barely move between consecutive reflections, so *copying the last value*
//! scores extremely well on almost every channel. Any model that does not
//! clearly beat that has learned nothing, however small its absolute error.
//!
//! So every prediction here is scored as **skill relative to the best naive
//! baseline**, and the baselines are chosen to be genuinely hard to beat:
//!
//! - **Persistence.** `x(t+1) = x(t)`. Very strong for slow-moving readings.
//! - **Drift.** `x(t+1) = x(t) + mean recent delta`. Near-perfect for
//!   accumulators, which otherwise make a model look brilliant for free.
//!
//! The drift baseline is the reason the accumulator question in
//! `corescout_represent::discover` is not academic: an observer that has not worked out
//! which variables accumulate cannot construct this baseline, and will mistake
//! trivially predictable counters for evidence that it understands the machine.
//!
//! # The model
//!
//! A per-cell autoregressive step fitted by least squares:
//!
//! ```text
//! delta(t+1) = a * delta(t) + b
//! ```
//!
//! Deliberately the simplest thing that can express momentum. The point of this
//! milestone is not a good predictor; it is to find out whether the reflection
//! contains enough signal for *any* predictor to beat copying the last value.
//! A weak model that clearly beats the baseline is a stronger result than a
//! complicated one whose advantage cannot be attributed.

use std::collections::BTreeMap;

use corescout_represent::discover::{differences, mean};

/// How well something predicted one cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellScore {
    pub row: usize,
    pub col: usize,
    /// Mean absolute error of the model.
    pub model_mae: f64,
    /// Mean absolute error of the best naive baseline.
    pub baseline_mae: f64,
    /// `1 - model/baseline`. Positive means the model beat the baseline;
    /// 0.0 means it merely matched it.
    pub skill: f64,
    pub samples: usize,
}

/// Prediction performance over a whole machine.
#[derive(Debug, Clone, PartialEq)]
pub struct PredictionReport {
    /// Cells that could be scored at all.
    pub cells: usize,
    /// Cells where the model beat the baseline by a margin worth reporting.
    pub cells_with_skill: usize,
    /// Median skill across scored cells.
    pub median_skill: f64,
    /// Mean skill, which the tails move and the median does not.
    pub mean_skill: f64,
    /// The cells the model predicted best.
    pub best: Vec<CellScore>,
    /// Cells where the model was clearly worse than doing nothing clever.
    pub worst: Vec<CellScore>,
    /// Columns the predictor treated as accumulators.
    pub accumulator_columns: Vec<usize>,
}

/// Fit and evaluate a one-step predictor over remembered series.
///
/// `accumulator_columns` is what the observer believes accumulates: either
/// told by the mirror, or worked out from behaviour. It changes which baseline
/// each cell is scored against, so getting it wrong is penalised.
///
/// The series are split in time: the first `train_fraction` fits, the rest
/// scores. No cell is ever scored on a sample used to fit it.
pub fn evaluate(
    series: &BTreeMap<(usize, usize), Vec<f64>>,
    accumulator_columns: &[usize],
    train_fraction: f64,
) -> PredictionReport {
    let mut scores: Vec<CellScore> = Vec::new();

    for ((row, col), values) in series {
        let is_accumulator = accumulator_columns.contains(col);
        if let Some(score) = score_cell(*row, *col, values, is_accumulator, train_fraction) {
            scores.push(score);
        }
    }

    let skills: Vec<f64> = scores.iter().map(|s| s.skill).collect();
    let median_skill = median(&skills);
    let mean_skill = mean(&skills);

    let mut ranked = scores.clone();
    ranked.sort_by(|a, b| {
        b.skill
            .partial_cmp(&a.skill)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let best: Vec<CellScore> = ranked.iter().take(5).copied().collect();
    let worst: Vec<CellScore> = ranked.iter().rev().take(5).copied().collect();

    let mut accumulator_columns = accumulator_columns.to_vec();
    accumulator_columns.sort_unstable();

    PredictionReport {
        cells: scores.len(),
        // A 1% margin: below that the difference is noise, and claiming skill
        // for it would be the same kind of overreach the baselines exist to
        // prevent.
        cells_with_skill: scores.iter().filter(|s| s.skill > 0.01).count(),
        median_skill,
        mean_skill,
        best,
        worst,
        accumulator_columns,
    }
}

/// Score one cell, or `None` when there is not enough usable data.
fn score_cell(
    row: usize,
    col: usize,
    values: &[f64],
    is_accumulator: bool,
    train_fraction: f64,
) -> Option<CellScore> {
    // Work in deltas: it is the same information, and it makes the
    // accumulator and reading cases the same shape of problem.
    let deltas = differences(values);
    if deltas.len() < 12 {
        return None;
    }
    let split = ((deltas.len() as f64) * train_fraction) as usize;
    if split < 6 || deltas.len() - split < 4 {
        return None;
    }

    let train = &deltas[..split];
    let test = &deltas[split..];

    // Baseline. For an accumulator, the sensible naive guess is that it keeps
    // going at its recent rate; for a reading, that it does not move at all.
    let baseline_delta = if is_accumulator {
        mean(
            &train
                .iter()
                .copied()
                .filter(|d| d.is_finite())
                .collect::<Vec<f64>>(),
        )
    } else {
        0.0
    };

    let (a, b) = fit_ar1(train)?;

    let mut model_error = 0.0;
    let mut baseline_error = 0.0;
    let mut samples = 0usize;
    let mut previous = train.last().copied().unwrap_or(0.0);

    for actual in test {
        if !actual.is_finite() {
            previous = f64::NAN;
            continue;
        }
        if previous.is_finite() {
            let predicted = a * previous + b;
            model_error += (predicted - actual).abs();
            baseline_error += (baseline_delta - actual).abs();
            samples += 1;
        }
        previous = *actual;
    }

    if samples < 4 {
        return None;
    }
    let model_mae = model_error / samples as f64;
    let baseline_mae = baseline_error / samples as f64;

    // A cell that never moves is perfectly predicted by everything and
    // distinguishes nothing, so it is not scored.
    if baseline_mae <= 1e-12 && model_mae <= 1e-12 {
        return None;
    }
    let skill = if baseline_mae > 1e-12 {
        1.0 - (model_mae / baseline_mae)
    } else {
        // The baseline was perfect and the model was not.
        -1.0
    };

    Some(CellScore {
        row,
        col,
        model_mae,
        baseline_mae,
        skill: skill.clamp(-1.0, 1.0),
        samples,
    })
}

/// Least-squares fit of `y(t+1) = a*y(t) + b`.
fn fit_ar1(series: &[f64]) -> Option<(f64, f64)> {
    let pairs: Vec<(f64, f64)> = series
        .windows(2)
        .filter(|w| w[0].is_finite() && w[1].is_finite())
        .map(|w| (w[0], w[1]))
        .collect();
    if pairs.len() < 4 {
        return None;
    }
    let n = pairs.len() as f64;
    let mean_x = pairs.iter().map(|(x, _)| *x).sum::<f64>() / n;
    let mean_y = pairs.iter().map(|(_, y)| *y).sum::<f64>() / n;

    let mut cov = 0.0;
    let mut var = 0.0;
    for (x, y) in &pairs {
        cov += (x - mean_x) * (y - mean_y);
        var += (x - mean_x).powi(2);
    }
    // No variation to regress on: fall back to predicting the mean, which is
    // the correct answer for a constant series rather than a failure.
    if var <= 1e-12 {
        return Some((0.0, mean_y));
    }
    let a = cov / var;
    Some((a, mean_y - a * mean_x))
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
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

    fn series(values: Vec<f64>) -> BTreeMap<(usize, usize), Vec<f64>> {
        [((0usize, 0usize), values)].into_iter().collect()
    }

    #[test]
    fn a_pure_counter_is_predicted_by_the_drift_baseline_not_by_skill() {
        // The trap this whole module is arranged around. A counter rising by
        // exactly 10 every tick is trivially predictable, and a model that
        // "predicts" it has demonstrated nothing.
        let values: Vec<f64> = (0..60).map(|i| (i as f64) * 10.0).collect();
        let report = evaluate(&series(values), &[0], 0.7);
        assert_eq!(
            report.cells, 0,
            "a perfectly steady counter is not scorable"
        );
    }

    #[test]
    fn momentum_is_learnable_and_shows_as_skill() {
        // An oscillation the AR(1) step can follow but persistence cannot.
        let values: Vec<f64> = (0..120)
            .map(|i| ((i as f64) * 0.35).sin() * 100.0)
            .collect();
        let report = evaluate(&series(values), &[], 0.7);
        assert_eq!(report.cells, 1);
        assert!(
            report.median_skill > 0.2,
            "expected real skill on a smooth signal, got {}",
            report.median_skill
        );
    }

    #[test]
    fn pure_noise_yields_no_skill() {
        // The honesty check. On an unpredictable series the model must not
        // appear to beat the baseline.
        let mut state = 12345u64;
        let values: Vec<f64> = (0..120)
            .map(|_| {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                ((state >> 33) as f64 / (1u64 << 31) as f64) - 0.5
            })
            .collect();
        let report = evaluate(&series(values), &[], 0.7);
        assert!(
            report.median_skill < 0.15,
            "claimed skill {} on noise",
            report.median_skill
        );
    }

    #[test]
    fn mislabelling_an_accumulator_costs_measurable_skill() {
        // The direct measurement of what the `Cumulative` label is worth: a
        // rising counter with a wobble, scored with and without knowing that
        // it accumulates.
        let values: Vec<f64> = (0..120)
            .map(|i| (i as f64) * 10.0 + ((i as f64) * 0.7).sin() * 3.0)
            .collect();

        let knowing = evaluate(&series(values.clone()), &[0], 0.7);
        let not_knowing = evaluate(&series(values), &[], 0.7);

        assert_eq!(knowing.cells, 1);
        assert_eq!(not_knowing.cells, 1);
        // Not knowing means being scored against a much weaker baseline, so the
        // model looks better while predicting exactly as well.
        assert!(
            not_knowing.median_skill > knowing.median_skill,
            "an unknown accumulator should flatter the model: {} vs {}",
            not_knowing.median_skill,
            knowing.median_skill
        );
    }

    #[test]
    fn training_and_scoring_windows_do_not_overlap() {
        // A cell whose behaviour changes halfway must not be scored using the
        // half it was fitted on.
        let mut values: Vec<f64> = (0..60).map(|i| ((i as f64) * 0.3).sin()).collect();
        values.extend((0..60).map(|_| 0.0));
        let report = evaluate(&series(values), &[], 0.5);
        assert_eq!(report.cells, 1);
        // 120 values give 119 deltas; half are fitted, so at most 60 can be
        // scored, and the fitting half must not appear among them.
        assert!(report.best[0].samples > 0);
        assert!(report.best[0].samples <= 60);
        assert!(report.best[0].samples < 119);
    }

    #[test]
    fn short_series_are_not_scored() {
        let report = evaluate(&series(vec![1.0, 2.0, 3.0]), &[], 0.7);
        assert_eq!(report.cells, 0);
    }

    #[test]
    fn ar1_recovers_a_known_coefficient() {
        // y(t+1) = 0.5*y(t) + 2
        let mut values = vec![1.0];
        for _ in 0..50 {
            let last = *values.last().unwrap();
            values.push(0.5 * last + 2.0);
        }
        let (a, b) = fit_ar1(&values).unwrap();
        assert!((a - 0.5).abs() < 1e-6, "a = {a}");
        assert!((b - 2.0).abs() < 1e-6, "b = {b}");
    }
}
