//! The threshold experiment.
//!
//! > The system constructs a concept that was not explicitly supplied by its
//! > designers, uses that concept to predict its own behavior, and acts
//! > successfully because of it.
//!
//! Three clauses, tested separately, each able to fail on its own.
//!
//! # How the experiment is arranged so it can fail
//!
//! A synthetic machine is given a **hidden regime**: a conjunction of
//! conditions across several entities and channels under which one channel
//! behaves very differently. The conjunction is deliberately not any single
//! variable, is never named, is not a channel, is not an entity, and appears
//! nowhere in the mirror's vocabulary. Nothing in the pipeline is told it
//! exists.
//!
//! Then the same pipeline is run on a machine with **no hidden regime at all**,
//! where the same channels vary independently. If concepts are coined there
//! too, the coinage rule is measuring noise and the positive result means
//! nothing. That negative control is the reason to believe the positive one.
//!
//! # What a synthetic machine can and cannot establish
//!
//! It can establish that the mechanism works: that a conjunction which exists
//! is found, earns promotion, and is used; and that one which does not exist is
//! not invented. That is a real result about the code.
//!
//! It cannot establish that real hardware contains such regimes, or that they
//! survive into the mirror at 10 Hz. Those are the open questions in
//! `RESEARCH.md` and no test in this file speaks to them.

use corescout_concept::concept::{signature_for, CoinageRules, Origin, Refusal, Registry};
use corescout_mirror::entity::{Entity, EntityClass};
use corescout_mirror::schema::AvailabilityMatrix;
use corescout_mirror::schema::SensorId;
use corescout_mirror::state::{ChannelId, ChannelSpec, Semantics, StateMatrix, Unit};
use corescout_mirror::{MirrorSnapshot, FORMAT_VERSION};
use corescout_represent::latent::LatentCatalogue;
use corescout_represent::normalize::Normalizer;

const ENTITIES: usize = 6;
const CHANNELS: usize = 4;
/// The channel whose behaviour the hidden regime governs. The pipeline is never
/// told this is special.
const OUTCOME: usize = 3;

/// A machine with a regularity nobody mentioned.
///
/// The regime is a conjunction: entities 2 and 5 both elevated on channel 0
/// *and* both depressed on channel 1. When it holds, channel 3 is quiet. When
/// it does not, channel 3 is noisy. Nothing names this, and no single channel
/// reveals it: conditioning on channel 0 alone, or on either entity alone,
/// leaves the outcome as variable as before.
struct Machine {
    seed: u64,
    /// When false, the same channels vary independently and there is nothing to
    /// find. This is the negative control.
    has_regime: bool,
    step: u64,
}

impl Machine {
    fn new(seed: u64, has_regime: bool) -> Machine {
        Machine {
            seed: seed | 1,
            has_regime,
            step: 0,
        }
    }

    fn random(&mut self) -> f64 {
        // xorshift64*, so a run is reproducible and two policies see the same
        // rolls.
        self.seed ^= self.seed >> 12;
        self.seed ^= self.seed << 25;
        self.seed ^= self.seed >> 27;
        let value = self.seed.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (value >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Whether the hidden regime holds this step.
    ///
    /// Driven by a slow oscillation so it persists for a while rather than
    /// flickering, which is what makes it a *state* rather than a coincidence.
    fn in_regime(&self) -> bool {
        self.has_regime && ((self.step / 12) % 3 == 0)
    }

    fn tick(&mut self) -> MirrorSnapshot {
        self.step += 1;
        let regime = self.in_regime();
        let mut state = StateMatrix::new(ENTITIES, CHANNELS);

        for entity in 0..ENTITIES {
            let participates = entity == 2 || entity == 5;
            for channel in 0..CHANNELS {
                let value = match (channel, participates && regime) {
                    // The conjunction: participants elevated on 0, depressed on 1.
                    (0, true) => 8.0 + self.random(),
                    (1, true) => -8.0 + self.random(),
                    // Everything else, and everyone else, is noise.
                    (0, false) | (1, false) => 4.0 * self.random(),
                    (OUTCOME, _) => {
                        if regime {
                            // Quiet in the regime.
                            0.5 * self.random()
                        } else {
                            10.0 * self.random()
                        }
                    }
                    _ => 5.0 * self.random(),
                };
                state.set(entity, channel, value);
            }
        }

        let mut availability = AvailabilityMatrix::new(ENTITIES, CHANNELS);
        for row in 0..ENTITIES {
            for col in 0..CHANNELS {
                availability.set(row, col, corescout_mirror::schema::Availability::Observed);
            }
        }

        MirrorSnapshot {
            format_version: FORMAT_VERSION,
            epoch: 1,
            sequence: self.step,
            monotonic_ns: self.step * 10_000_000,
            realtime_ns: 1_700_000_000_000_000_000 + self.step * 10_000_000,
            entities: (0..ENTITIES)
                // Opaque keys and no class hint: the pipeline is given no
                // vocabulary it could lean on instead of finding the structure.
                .map(|i| {
                    Entity::new(
                        format!("thing/{i}"),
                        EntityClass::Unclassified,
                        Some(i as u32),
                    )
                })
                .collect(),
            channels: (0..CHANNELS)
                .map(|i| ChannelSpec {
                    id: ChannelId(i as u16),
                    key: format!("x{i}"),
                    unit: Unit::Dimensionless,
                    semantics: Semantics::Instant,
                    sensor: SensorId(0),
                })
                .collect(),
            relations: Vec::new(),
            state,
            availability,
            sensors: Vec::new(),
        }
    }

    fn run(&mut self, frames: usize) -> Vec<MirrorSnapshot> {
        (0..frames).map(|_| self.tick()).collect()
    }
}

/// What one pass of the pipeline found.
struct Outcome {
    catalogue: LatentCatalogue,
    /// Predictive utility per discovered state, measured rather than assumed.
    utilities: Vec<(corescout_represent::latent::LatentStateId, f64)>,
    registry: Registry,
    /// Error predicting the outcome channel while ignoring discovered states.
    baseline_error: f64,
    /// Error predicting it while conditioning on them.
    informed_error: f64,
}

/// Run discovery, measure whether the discovered states predict, and coin.
fn pipeline(frames: &[MirrorSnapshot]) -> Outcome {
    let cols = CHANNELS;
    // Fit the scaling on an early slice only, so nothing from the scored region
    // leaks into the normalisation.
    let fit: Vec<Vec<f64>> = frames[..frames.len() / 5]
        .iter()
        .map(|f| f.state.as_slice().to_vec())
        .collect();
    let normalizer = Normalizer::fit(&fit, cols);

    let mut catalogue = LatentCatalogue::new(0.9, 16);
    let mut assignments = Vec::with_capacity(frames.len());
    for frame in frames {
        let (point, _) = normalizer.apply_filled(frame.state.as_slice(), cols);
        let (state, _) = catalogue.observe(&point, frame.monotonic_ns);
        assignments.push(state);
    }

    // Measure predictive utility honestly: how much better is predicting the
    // outcome channel when you know which discovered state you are in?
    //
    // Baseline is the grand mean; informed is the per-state mean. Both are
    // scored on the same reflections, so the comparison is like for like.
    let outcome_values: Vec<f64> = frames
        .iter()
        .map(|f| {
            // Mean over participating entities, so the measure does not depend
            // on which row happens to be interesting.
            (0..ENTITIES)
                .map(|row| f.state.get(row, OUTCOME))
                .filter(|value| value.is_finite())
                .sum::<f64>()
                / ENTITIES as f64
        })
        .collect();

    let grand_mean = outcome_values.iter().sum::<f64>() / outcome_values.len() as f64;
    let baseline_error = outcome_values
        .iter()
        .map(|v| (v - grand_mean).abs())
        .sum::<f64>()
        / outcome_values.len() as f64;

    let mut per_state: std::collections::BTreeMap<_, Vec<f64>> = Default::default();
    for (state, value) in assignments.iter().zip(&outcome_values) {
        per_state.entry(*state).or_default().push(*value);
    }
    let informed_error = assignments
        .iter()
        .zip(&outcome_values)
        .map(|(state, value)| {
            let group = &per_state[state];
            let mean = group.iter().sum::<f64>() / group.len() as f64;
            (value - mean).abs()
        })
        .sum::<f64>()
        / outcome_values.len() as f64;

    // Utility per state: how much conditioning on *this* state reduces error,
    // relative to the baseline. This is the number the registry requires and
    // does not compute for itself.
    let mut utilities = Vec::new();
    for (state, values) in &per_state {
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let error = values.iter().map(|v| (v - mean).abs()).sum::<f64>() / values.len() as f64;
        let utility = if baseline_error > 0.0 {
            ((baseline_error - error) / baseline_error).clamp(0.0, 1.0)
        } else {
            0.0
        };
        utilities.push((*state, utility));
    }

    let median_dwell = {
        let mut dwells: Vec<f64> = catalogue
            .states()
            .iter()
            .map(|s| s.mean_dwell_ns())
            .filter(|d| d.is_finite() && *d > 0.0)
            .collect();
        dwells.sort_by(|a, b| a.total_cmp(b));
        dwells.get(dwells.len() / 2).copied().unwrap_or(1.0)
    };

    let mut registry = Registry::new(CoinageRules {
        min_occurrences: 20,
        min_utility: 0.20,
        min_confidence: 0.5,
        retire_below: 0.05,
        max_concepts: 16,
    });
    for (state_id, utility) in &utilities {
        let Some(state) = catalogue.get(*state_id) else {
            continue;
        };
        let signature = signature_for(state, &catalogue, ENTITIES, median_dwell, 0.0);
        match registry.coin(
            state,
            *utility,
            signature,
            frames.last().unwrap().monotonic_ns,
        ) {
            Ok(_) | Err(Refusal::DoesNotPredict { .. }) | Err(Refusal::TooRare { .. }) => {}
            Err(Refusal::NotConfident { .. }) | Err(Refusal::AlreadyCoined(_)) => {}
            Err(other) => panic!("unexpected refusal: {}", other.describe()),
        }
    }

    Outcome {
        catalogue,
        utilities,
        registry,
        baseline_error,
        informed_error,
    }
}

#[test]
fn clause_one_the_concept_was_not_supplied_by_its_designers() {
    let frames = Machine::new(0xC0FFEE, true).run(600);
    let outcome = pipeline(&frames);

    assert!(
        !outcome.registry.is_empty(),
        "nothing was coined from a machine that has a regime to find"
    );

    for concept in outcome.registry.live() {
        // Every concept must trace to something discovered, never to a name we
        // wrote down.
        assert!(
            matches!(concept.origin, Origin::LatentState(_)),
            "{} did not come from a discovery: {:?}",
            concept.id,
            concept.origin
        );
        // And it must carry the machine's own name.
        assert!(concept.id.to_string().starts_with("concept_"));
        assert!(concept.resembles.is_none(), "nobody annotated it");
    }

    // The regime is a conjunction across entities and channels. It is not any
    // channel key, so no concept can have been read off the vocabulary.
    let channel_keys: Vec<String> = frames[0].channels.iter().map(|c| c.key.clone()).collect();
    for concept in outcome.registry.live() {
        assert!(
            !channel_keys.contains(&concept.id.to_string()),
            "a concept was named after a channel"
        );
    }
}

#[test]
fn clause_two_the_concept_predicts_the_machines_own_behaviour() {
    let frames = Machine::new(0xC0FFEE, true).run(600);
    let outcome = pipeline(&frames);

    // Knowing which discovered state you are in must beat not knowing.
    assert!(
        outcome.informed_error < outcome.baseline_error,
        "discovered states did not help: informed {:.4} vs baseline {:.4}",
        outcome.informed_error,
        outcome.baseline_error
    );

    let improvement = (outcome.baseline_error - outcome.informed_error) / outcome.baseline_error;
    assert!(
        improvement > 0.15,
        "improvement of {:.1}% is too small to call a discovery",
        improvement * 100.0
    );

    // And the coined concepts specifically must be the useful ones: coinage is
    // gated on utility, so nothing below the bar can have been promoted.
    let bar = outcome.registry.rules().min_utility;
    for concept in outcome.registry.live() {
        assert!(
            concept.predictive_utility >= bar,
            "{} was coined with utility {:.3}, below the bar {bar:.3}",
            concept.id,
            concept.predictive_utility
        );
    }
}

#[test]
fn the_negative_control_coins_nothing_from_a_machine_with_nothing_to_find() {
    // The result that makes the positive one worth believing. Same pipeline,
    // same channels, same number of frames, no hidden regime.
    let frames = Machine::new(0xC0FFEE, false).run(600);
    let outcome = pipeline(&frames);

    let improvement = (outcome.baseline_error - outcome.informed_error) / outcome.baseline_error;
    assert!(
        improvement < 0.15,
        "the control machine showed {:.1}% improvement; the pipeline is finding \
         structure in noise",
        improvement * 100.0
    );

    assert!(
        outcome.registry.is_empty(),
        "coined {} concepts from a machine with no regime: {}",
        outcome.registry.len(),
        outcome.registry.render()
    );
    assert!(
        outcome.registry.refused_count() > 0,
        "the control should have refused at least one candidate"
    );
}

#[test]
fn the_experiment_is_reproducible() {
    // Two runs of the same seed must reach the same conclusions, or the result
    // is about the dice rather than the machine.
    let first = pipeline(&Machine::new(7, true).run(400));
    let second = pipeline(&Machine::new(7, true).run(400));
    assert_eq!(first.registry.len(), second.registry.len());
    assert_eq!(first.catalogue.len(), second.catalogue.len());
    assert!((first.informed_error - second.informed_error).abs() < 1e-9);
}

#[test]
fn a_concept_that_stops_predicting_is_retired_rather_than_kept() {
    // The ontology must be able to shrink, or it will eventually contain a
    // category per reflection and explain nothing while appearing to explain
    // everything.
    let frames = Machine::new(0xC0FFEE, true).run(600);
    let mut outcome = pipeline(&frames);
    let live_before = outcome.registry.len();
    assert!(live_before > 0);

    for concept in outcome
        .registry
        .concepts()
        .iter()
        .map(|c| c.id)
        .collect::<Vec<_>>()
    {
        outcome.registry.set_utility(concept, 0.0);
    }
    let retired = outcome.registry.audit();
    assert_eq!(retired.len(), live_before);
    assert!(outcome.registry.is_empty());
}

#[test]
fn utility_is_supplied_by_measurement_not_invented_by_the_registry() {
    // A registry that could compute its own justification would coin
    // everything. The measurement lives in the test, above; the registry only
    // enforces the bar.
    let frames = Machine::new(0xC0FFEE, true).run(600);
    let outcome = pipeline(&frames);
    assert!(
        !outcome.utilities.is_empty(),
        "utilities must be measured before coinage is possible"
    );
    // Some states are useful and some are not, on the same machine. If every
    // state scored the same, the measure is not discriminating.
    let values: Vec<f64> = outcome.utilities.iter().map(|(_, u)| *u).collect();
    let max = values.iter().cloned().fold(f64::MIN, f64::max);
    let min = values.iter().cloned().fold(f64::MAX, f64::min);
    assert!(
        max - min > 0.05,
        "every state scored alike ({min:.3} to {max:.3}); the measure does not discriminate"
    );
}

#[test]
fn report_the_numbers() {
    // Not an assertion. This prints what the experiment actually measured, so
    // the result can be read rather than inferred from a green tick. Run with
    // `cargo test -p corescout-integration --test concept_threshold -- --nocapture`.
    for (label, regime) in [
        ("with a hidden regime", true),
        ("control, no regime", false),
    ] {
        let frames = Machine::new(0xC0FFEE, regime).run(600);
        let outcome = pipeline(&frames);
        let improvement =
            (outcome.baseline_error - outcome.informed_error) / outcome.baseline_error;
        println!(
            "\n{label}:\n  states discovered: {}\n  baseline error: {:.4}\n  \
             informed error: {:.4}\n  improvement: {:.1}%\n  concepts coined: {}\n  \
             candidates refused: {}",
            outcome.catalogue.len(),
            outcome.baseline_error,
            outcome.informed_error,
            improvement * 100.0,
            outcome.registry.len(),
            outcome.registry.refused_count()
        );
        for concept in outcome.registry.live() {
            println!("    {}", concept.describe());
        }
    }
}
