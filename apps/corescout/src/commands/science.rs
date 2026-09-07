//! The commands that do science: `theory`, `concepts`, `resources`.
//!
//! # What runs here, and what does not
//!
//! These consume reflections and nothing else, so they run against a live plane
//! or a recording without knowing which.
//!
//! What they do **not** do is drive the machine. The autonomous loop can act on
//! a concept once one exists, but the conjecture-and-refutation cycle is run by
//! these commands on demand rather than continuously inside the agent. That is
//! an honest description of where the system currently is: the science is real
//! and the coupling between the science and the acting is thin.

use corescout_concept::concept::{signature_for, CoinageRules, Registry};
use corescout_concept::resource::Catalogue;
use corescout_core::error::{Error, Result};
use corescout_human::Format;
use corescout_mirror::MirrorSnapshot;
use corescout_represent::latent::{LatentCatalogue, LatentStateId};
use corescout_represent::normalize::Normalizer;
use corescout_science::{Theory, TheoryConfig};

use crate::cli::SourceOptions;
use crate::runtime;

/// Conjecture, test, and report what survived.
pub fn theory(options: &SourceOptions, format: Format) -> Result<()> {
    let frames = frames_for(options)?;
    let found = investigate(&frames)?;
    match format {
        Format::Json => println!("{}", runtime::to_json(&found.theory)?),
        Format::Human => {
            print!("{}", found.theory.render());
            if found.theory.conjectured() == 0 {
                println!(
                    "\nnothing was worth conjecturing about: no discovered state recurred \
                     often enough"
                );
            }
        }
    }
    Ok(())
}

/// List the concepts the machine coined about itself.
pub fn concepts(options: &SourceOptions, format: Format) -> Result<()> {
    let frames = frames_for(options)?;
    let found = investigate(&frames)?;
    match format {
        Format::Json => println!("{}", runtime::to_json(&found.registry)?),
        Format::Human => {
            print!("{}", found.registry.render());
            println!(
                "\nmeasured over {} reflections across {} discovered states; knowing which \
                 state the machine is in reduced prediction error by {:.1}%",
                frames.len(),
                found.catalogue.len(),
                found.improvement * 100.0
            );
            if found.registry.is_empty() && found.registry.refused_count() > 0 {
                // The common and correct outcome on a quiet machine.
                println!(
                    "{} candidates were refused. A recurring state is not a concept until \
                     knowing about it improves prediction.",
                    found.registry.refused_count()
                );
            }
        }
    }
    Ok(())
}

/// Show the virtual resources the machine can offer.
pub fn resources(options: &SourceOptions, format: Format) -> Result<()> {
    let frames = frames_for(options)?;
    let found = investigate(&frames)?;
    // A resource needs a recipe, and a recipe comes from having acted. Reading a
    // recording produces concepts but no recipes, so the catalogue is honestly
    // empty here rather than filled with configurations nobody has tried.
    let catalogue = Catalogue::new();
    match format {
        Format::Json => println!("{}", runtime::to_json(&catalogue)?),
        Format::Human => {
            print!("{}", catalogue.render());
            if !found.registry.is_empty() {
                println!(
                    "\n{} concepts exist but none has a tested recipe. A concept becomes a \
                     resource only once the machine has learned to bring it about on purpose, \
                     which requires acting: run `corescout agent start`.",
                    found.registry.len()
                );
            }
        }
    }
    Ok(())
}

/// What an investigation produced.
#[derive(Debug)]
pub struct Investigation {
    pub catalogue: LatentCatalogue,
    pub registry: Registry,
    pub theory: Theory,
    /// Fractional reduction in prediction error from knowing the state.
    pub improvement: f64,
}

/// Discover, measure, conjecture, test, coin.
///
/// Exposed because `demo self` runs the same thing, and two copies would drift.
pub fn investigate(frames: &[MirrorSnapshot]) -> Result<Investigation> {
    if frames.len() < 40 {
        return Err(Error::invalid(format!(
            "science needs more than {} reflections; record a longer trace",
            frames.len()
        )));
    }
    let cols = frames[0].channels.len().max(1);
    let entities = frames[0].entities.len();

    // Fit on an early slice, so nothing from the scored region leaks into the
    // scaling and flatters the result.
    let fit: Vec<Vec<f64>> = frames[..(frames.len() / 5).max(4)]
        .iter()
        .map(|f| f.state.as_slice().to_vec())
        .collect();
    let normalizer = Normalizer::fit(&fit, cols);

    let mut catalogue = LatentCatalogue::new(0.9, 32);
    let mut points = Vec::with_capacity(frames.len());
    let mut assignments = Vec::with_capacity(frames.len());
    for frame in frames {
        let (point, _) = normalizer.apply_filled(frame.state.as_slice(), cols);
        let (state, _) = catalogue.observe(&point, frame.monotonic_ns);
        assignments.push(state);
        points.push(point);
    }

    // Measure what knowing the state is worth, per state and overall. The
    // registry requires this number and must never compute it for itself.
    let (utilities, improvement) = measure_utility(&assignments, &points, cols);

    let median_dwell = median_dwell_ns(&catalogue);
    let mut registry = Registry::new(CoinageRules::default());
    let now = frames.last().map(|f| f.monotonic_ns).unwrap_or(0);
    for (state_id, utility) in &utilities {
        if let Some(state) = catalogue.get(*state_id) {
            let signature = signature_for(state, &catalogue, entities, median_dwell, 0.0);
            // A refusal is the expected outcome and is not an error.
            let _ = registry.coin(state, *utility, signature, now);
        }
    }

    let mut theory = Theory::new(TheoryConfig {
        min_trials: 10,
        min_state_evidence: 8,
        ..TheoryConfig::default()
    });
    if let Some(last) = frames.last() {
        theory.conjecture(&catalogue, last, now);
    }
    for ((frame, point), state) in frames.iter().zip(&points).zip(&assignments) {
        theory.confront(frame, Some(*state), point, frame.monotonic_ns);
    }

    Ok(Investigation {
        catalogue,
        registry,
        theory,
        improvement,
    })
}

/// How much knowing the discovered state reduces prediction error.
///
/// Returns per-state utility and the overall improvement. The baseline is the
/// grand mean of each cell; the informed prediction is the per-state mean. Both
/// are scored on the same reflections, so the comparison is like for like.
fn measure_utility(
    assignments: &[LatentStateId],
    points: &[Vec<f64>],
    _cols: usize,
) -> (Vec<(LatentStateId, f64)>, f64) {
    if points.is_empty() {
        return (Vec::new(), 0.0);
    }
    let width = points[0].len();
    let mean = |rows: &[&Vec<f64>]| -> Vec<f64> {
        let mut sums = vec![0.0; width];
        let mut counts = vec![0.0; width];
        for row in rows {
            for (i, value) in row.iter().enumerate() {
                if value.is_finite() {
                    sums[i] += value;
                    counts[i] += 1.0;
                }
            }
        }
        sums.iter()
            .zip(&counts)
            .map(|(s, c)| if *c > 0.0 { s / c } else { 0.0 })
            .collect()
    };
    let error = |rows: &[&Vec<f64>], centre: &[f64]| -> f64 {
        let mut total = 0.0;
        let mut count = 0.0;
        for row in rows {
            for (i, value) in row.iter().enumerate() {
                if value.is_finite() {
                    total += (value - centre[i]).abs();
                    count += 1.0;
                }
            }
        }
        if count > 0.0 {
            total / count
        } else {
            0.0
        }
    };

    let all: Vec<&Vec<f64>> = points.iter().collect();
    let grand = mean(&all);
    let baseline = error(&all, &grand);

    let mut grouped: std::collections::BTreeMap<LatentStateId, Vec<&Vec<f64>>> = Default::default();
    for (state, point) in assignments.iter().zip(points) {
        grouped.entry(*state).or_default().push(point);
    }

    let mut utilities = Vec::new();
    let mut informed_total = 0.0;
    let mut informed_count = 0.0;
    for (state, rows) in &grouped {
        let centre = mean(rows);
        let state_error = error(rows, &centre);
        let utility = if baseline > 0.0 {
            ((baseline - state_error) / baseline).clamp(0.0, 1.0)
        } else {
            0.0
        };
        utilities.push((*state, utility));
        informed_total += state_error * rows.len() as f64;
        informed_count += rows.len() as f64;
    }

    let informed = if informed_count > 0.0 {
        informed_total / informed_count
    } else {
        baseline
    };
    let improvement = if baseline > 0.0 {
        ((baseline - informed) / baseline).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    (utilities, improvement)
}

fn median_dwell_ns(catalogue: &LatentCatalogue) -> f64 {
    let mut dwells: Vec<f64> = catalogue
        .states()
        .iter()
        .map(|s| s.mean_dwell_ns())
        .filter(|d| d.is_finite() && *d > 0.0)
        .collect();
    dwells.sort_by(|a, b| a.total_cmp(b));
    dwells.get(dwells.len() / 2).copied().unwrap_or(1.0)
}

fn frames_for(options: &SourceOptions) -> Result<Vec<MirrorSnapshot>> {
    let mut source = runtime::open_source(options)?;
    runtime::take_frames(&mut source, options.frames.or(Some(2000)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::test_support::series;

    #[test]
    fn science_refuses_a_trace_too_short_to_learn_from() {
        // A number derived from twenty reflections would be meaningless, and
        // producing one anyway is how a system starts lying quietly.
        let error = match investigate(&series(10)) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("a ten-frame trace must not yield a result"),
        };
        assert!(error.contains("needs more than"), "{error}");
    }

    #[test]
    fn a_periodic_trace_yields_states_and_a_measured_improvement() {
        let investigation = investigate(&series(400)).expect("investigated");
        assert!(!investigation.catalogue.is_empty());
        assert!(investigation.improvement.is_finite());
        // Whether anything is coined depends on whether the states pay their
        // way, which is the point; the mechanism must simply run.
        assert!(investigation.registry.coined() + investigation.registry.refused_count() > 0);
    }

    #[test]
    fn every_conjecture_the_theory_admits_is_falsifiable() {
        let investigation = investigate(&series(400)).expect("investigated");
        for hypothesis in investigation.theory.open() {
            assert!(hypothesis.expectation.is_falsifiable());
        }
    }

    #[test]
    fn utility_is_zero_when_the_states_explain_nothing() {
        // Identical points: every state has the same mean as the grand mean, so
        // knowing the state is worth nothing and nothing should be coined.
        let points = vec![vec![1.0, 2.0]; 50];
        let assignments = vec![LatentStateId(0); 50];
        let (utilities, improvement) = measure_utility(&assignments, &points, 2);
        assert!(improvement.abs() < 1e-9);
        assert!(utilities.iter().all(|(_, u)| u.abs() < 1e-9));
    }

    #[test]
    fn utility_is_high_when_the_states_separate_the_data() {
        let mut points = vec![vec![0.0, 0.0]; 25];
        points.extend(vec![vec![100.0, 100.0]; 25]);
        let mut assignments = vec![LatentStateId(0); 25];
        assignments.extend(vec![LatentStateId(1); 25]);
        let (utilities, improvement) = measure_utility(&assignments, &points, 2);
        assert!(improvement > 0.9, "improvement was {improvement}");
        assert!(utilities.iter().all(|(_, u)| *u > 0.9));
    }

    #[test]
    fn measuring_an_empty_trace_does_not_panic() {
        let (utilities, improvement) = measure_utility(&[], &[], 2);
        assert!(utilities.is_empty());
        assert_eq!(improvement, 0.0);
    }
}
