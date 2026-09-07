//! The milestone: an action the machine would not have taken without a concept
//! it coined about itself.
//!
//! # The environment
//!
//! A synthetic substrate in which the best action depends on a **relational
//! hidden regime that is not an input variable**:
//!
//! ```text
//! regime holds when:  thing/1 . x0  is elevated
//!               AND   thing/2 . x1  is depressed
//!
//! under the regime:   action a3 is much better than anything else
//! otherwise:          every action is alike
//! ```
//!
//! There is no channel called `regime`. There is no entity whose value is the
//! conjunction. No single variable separates the two situations: `x0` alone is
//! elevated in half the non-regime frames too, and so is `x1`. The conjunction
//! has to be found.
//!
//! # Four arms against the identical environment
//!
//! | arm | what it is given |
//! |---|---|
//! | baseline | the raw mirror, no state inference |
//! | latent | clustering, no coinage bar and no causal test |
//! | concept | coined concepts, causal attribution, de-coinage |
//! | oracle | the hidden regime, directly |
//!
//! The oracle is the ceiling and the baseline is the floor. The interesting
//! question is where `concept` lands between them, and whether `latent` is
//! distinguishable from it.
//!
//! # Then the regime is removed, and a3 becomes harmful
//!
//! Halfway through, the regime stops occurring and `a3` becomes the *worst*
//! action. An agent that keeps its old belief is now actively punished for it.
//! That is what makes de-coinage measurable rather than merely tidy: the
//! lifecycle has to complete, and completing it has to be worth something.
//!
//! ```text
//! experience -> concept -> belief -> behaviour -> failed prediction -> death
//! ```

use std::collections::BTreeMap;

use corescout_autonomy::conditioned::{Choice, ConditionedConfig, ConditionedPolicy};
use corescout_concept::concept::{signature_for, CoinageRules, Registry};
use corescout_mirror::entity::{Entity, EntityClass};
use corescout_mirror::schema::{Availability, AvailabilityMatrix, SensorId};
use corescout_mirror::state::{ChannelId, ChannelSpec, Semantics, StateMatrix, Unit};
use corescout_mirror::{MirrorSnapshot, FORMAT_VERSION};
use corescout_represent::latent::{LatentCatalogue, LatentStateId};
use corescout_represent::normalize::Normalizer;
use corescout_science::causal::Exploration;

const ENTITIES: usize = 4;
const CHANNELS: usize = 3;
const ACTIONS: [&str; 4] = ["hold", "a1", "a2", "a3"];
/// Ticks before the regime is removed and `a3` turns harmful.
///
/// Long, because causal identification is expensive. The agent runs three
/// candidate actions against a default under each concept it coins, so a run
/// with seven concepts is estimating twenty-one effects at once, and each needs
/// randomised trials on both arms. A shorter run does not fail because the
/// mechanism is wrong; it fails because there is not enough evidence, which is
/// a property of the question rather than of the code.
const TURN: usize = 9000;
const TOTAL: usize = 18000;

/// The environment. Its rules are never shown to any arm but the oracle.
struct World {
    seed: u64,
    step: usize,
    /// True while the hidden regime still occurs at all.
    regime_exists: bool,
    /// Set each tick: whether the conjunction holds right now.
    regime_now: bool,
}

impl World {
    fn new(seed: u64) -> World {
        World {
            seed: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
            step: 0,
            regime_exists: true,
            regime_now: false,
        }
    }

    fn random(&mut self) -> f64 {
        self.seed ^= self.seed >> 12;
        self.seed ^= self.seed << 25;
        self.seed ^= self.seed >> 27;
        let value = self.seed.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (value >> 11) as f64 / (1u64 << 53) as f64
    }

    fn tick(&mut self) -> MirrorSnapshot {
        self.step += 1;
        if self.step == TURN {
            self.regime_exists = false;
        }
        // The regime comes and goes in runs, so it is a state rather than a
        // coin flip, and long enough to be worth learning about.
        self.regime_now = self.regime_exists && ((self.step / 15) % 3 == 0);

        let mut state = StateMatrix::new(ENTITIES, CHANNELS);
        for entity in 0..ENTITIES {
            for channel in 0..CHANNELS {
                // Baseline noise everywhere.
                state.set(entity, channel, 5.0 * self.random());
            }
        }

        // The conjunction. Note that each half is *also* produced on its own in
        // roughly half the other frames, below, so neither variable alone
        // separates the regime from anything.
        if self.regime_now {
            state.set(1, 0, 9.0 + self.random());
            state.set(2, 1, -9.0 + self.random());
        } else {
            // Decoys: one half of the conjunction without the other.
            let decoy = self.random();
            if decoy < 0.35 {
                state.set(1, 0, 9.0 + self.random());
            } else if decoy < 0.70 {
                state.set(2, 1, -9.0 + self.random());
            }
        }

        let mut availability = AvailabilityMatrix::new(ENTITIES, CHANNELS);
        for row in 0..ENTITIES {
            for col in 0..CHANNELS {
                availability.set(row, col, Availability::Observed);
            }
        }

        MirrorSnapshot {
            format_version: FORMAT_VERSION,
            epoch: 1,
            sequence: self.step as u64,
            monotonic_ns: self.step as u64 * 10_000_000,
            realtime_ns: 1_700_000_000_000_000_000 + self.step as u64 * 10_000_000,
            entities: (0..ENTITIES)
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

    /// What an action costs right now. Lower is better.
    ///
    /// Before the turn: `a3` is much better under the regime, and everything is
    /// alike otherwise. After the turn: `a3` is the worst thing available, so
    /// an agent clinging to a dead belief pays for it.
    fn cost(&mut self, action: &str) -> f64 {
        let noise = self.random();
        if self.regime_now {
            if action == "a3" {
                1.0 + noise
            } else {
                8.0 + noise
            }
        } else if !self.regime_exists && action == "a3" {
            // The belief has become harmful.
            9.0 + noise
        } else {
            5.0 + noise
        }
    }
}

/// What one arm achieved.
#[derive(Debug, Default)]
struct Run {
    cost_before: f64,
    cost_after: f64,
    /// Decisions where a self-derived concept changed the action.
    attributable: u64,
    /// Times `a3` was chosen while the regime held, for any reason.
    right_action_in_regime: u32,
    /// Times `a3` was chosen while the regime held *because the agent believed
    /// in it*, as opposed to while probing.
    ///
    /// The measurement that matters, and the mirror of
    /// `stale_belief_after_turn`. Probe-driven correct actions are luck.
    right_belief_in_regime: u32,
    /// Times `a3` was chosen after it became harmful, for any reason.
    stale_action_after_turn: u32,
    /// Times `a3` was chosen after the turn *because the agent still believed
    /// in it*, as opposed to while probing.
    ///
    /// The distinction matters: exploration cost is the price of being able to
    /// learn at all, and is paid whether or not the agent is wrong. Stale
    /// belief is the agent being wrong.
    stale_belief_after_turn: u32,
    concepts_coined: usize,
    concepts_retired: usize,
}

impl Run {
    fn mean_before(&self, ticks: usize) -> f64 {
        self.cost_before / ticks as f64
    }
    fn mean_after(&self, ticks: usize) -> f64 {
        self.cost_after / ticks as f64
    }
}

fn record_choice(run: &mut Run, world: &World, choice: &Choice, cost: f64) {
    record(run, world, &choice.family, cost);
    if choice.family == "a3" && !choice.randomised {
        if world.step > TURN {
            run.stale_belief_after_turn += 1;
        } else if world.regime_now {
            run.right_belief_in_regime += 1;
        }
    }
}

fn record(run: &mut Run, world: &World, action: &str, cost: f64) {
    if world.step <= TURN {
        run.cost_before += cost;
        if world.regime_now && action == "a3" {
            run.right_action_in_regime += 1;
        }
    } else {
        run.cost_after += cost;
        if action == "a3" {
            run.stale_action_after_turn += 1;
        }
    }
}

/// Never acts. The floor.
fn run_baseline(seed: u64) -> Run {
    let mut world = World::new(seed);
    let mut run = Run::default();
    for _ in 0..TOTAL {
        world.tick();
        let cost = world.cost("hold");
        record(&mut run, &world, "hold", cost);
    }
    run
}

/// Told the hidden regime directly. The ceiling.
fn run_oracle(seed: u64) -> Run {
    let mut world = World::new(seed);
    let mut run = Run::default();
    for _ in 0..TOTAL {
        world.tick();
        let action = if world.regime_now { "a3" } else { "hold" };
        let cost = world.cost(action);
        record(&mut run, &world, action, cost);
    }
    run
}

/// Clustering with no coinage bar and no causal test.
///
/// Conditions on the raw latent state id, and installs a preference from
/// whichever action has had the better mean outcome there. This is the arm that
/// shows what the bars are worth: it is the same pipeline with the two rules
/// this project added taken out.
fn run_latent(seed: u64) -> Run {
    let mut world = World::new(seed);
    let mut run = Run::default();
    let mut catalogue = LatentCatalogue::new(0.9, 32);
    let mut normalizer: Option<Normalizer> = None;
    let mut warmup: Vec<Vec<f64>> = Vec::new();
    // condition -> action -> (count, total cost)
    let mut tally: BTreeMap<LatentStateId, BTreeMap<String, (u32, f64)>> = BTreeMap::new();
    let mut rng = World::new(seed ^ 0xABCD);

    for _ in 0..TOTAL {
        let frame = world.tick();
        let raw = frame.state.as_slice().to_vec();
        if normalizer.is_none() {
            warmup.push(raw.clone());
            if warmup.len() >= 120 {
                normalizer = Some(Normalizer::fit(&warmup, CHANNELS));
            }
            let cost = world.cost("hold");
            record(&mut run, &world, "hold", cost);
            continue;
        }
        let (point, _) = normalizer
            .as_ref()
            .expect("fitted")
            .apply_filled(&raw, CHANNELS);
        let (state, _) = catalogue.observe(&point, frame.monotonic_ns);

        // Pick the best-so-far action here, exploring a little.
        let here = tally.entry(state).or_default();
        let action: String = if rng.random() < 0.1 {
            ACTIONS[(rng.random() * ACTIONS.len() as f64) as usize % ACTIONS.len()].to_string()
        } else {
            here.iter()
                .filter(|(_, (n, _))| *n >= 3)
                .min_by(|a, b| (a.1 .1 / a.1 .0 as f64).total_cmp(&(b.1 .1 / b.1 .0 as f64)))
                .map(|(name, _)| name.clone())
                .unwrap_or_else(|| "hold".to_string())
        };
        let cost = world.cost(&action);
        let here = tally.entry(state).or_default();
        let slot = here.entry(action.clone()).or_insert((0, 0.0));
        slot.0 += 1;
        slot.1 += cost;
        record(&mut run, &world, &action, cost);
    }
    run
}

/// The full pipeline: coin concepts that pay their way, establish causally that
/// an action helps under them, act on that, and de-coin when it stops holding.
fn run_concept(seed: u64) -> Run {
    let mut world = World::new(seed);
    let mut run = Run::default();

    let mut catalogue = LatentCatalogue::new(0.9, 32);
    let mut normalizer: Option<Normalizer> = None;
    let mut warmup: Vec<Vec<f64>> = Vec::new();
    let mut registry = Registry::new(CoinageRules {
        min_occurrences: 25,
        min_utility: 0.20,
        min_confidence: 0.5,
        retire_below: 0.10,
        max_concepts: 16,
    });
    let mut policy = ConditionedPolicy::new(
        ConditionedConfig {
            min_trials: 10,
            default_action: "hold".into(),
            lower_is_better: true,
        },
        // A quarter of trials randomised: this is a learning experiment, and
        // the cost of that exploration shows up in the reported means.
        Exploration::new(0.25, 10, 4000, seed ^ 0x1234),
    );
    // state -> costs seen, for measuring what knowing the state is worth.
    let mut per_state: BTreeMap<LatentStateId, Vec<f64>> = BTreeMap::new();
    let mut all_costs: Vec<f64> = Vec::new();
    let mut coined: BTreeMap<LatentStateId, String> = BTreeMap::new();
    let candidates: Vec<String> = ACTIONS.iter().map(|s| s.to_string()).collect();

    for tick in 0..TOTAL {
        let frame = world.tick();
        let raw = frame.state.as_slice().to_vec();
        if normalizer.is_none() {
            warmup.push(raw.clone());
            if warmup.len() >= 120 {
                normalizer = Some(Normalizer::fit(&warmup, CHANNELS));
            }
            let cost = world.cost("hold");
            record(&mut run, &world, "hold", cost);
            continue;
        }
        let (point, _) = normalizer
            .as_ref()
            .expect("fitted")
            .apply_filled(&raw, CHANNELS);
        let (state, _) = catalogue.observe(&point, frame.monotonic_ns);

        // The condition is the *concept*, when one has been coined for this
        // state. A bare latent state is not a reason to do anything.
        let condition = coined.get(&state).cloned();
        if let Some(name) = &condition {
            // Mark the concept as still occurring, so absence is distinguishable
            // from presence when its standing is decayed below.
            if let Some(id) = registry_id(&registry, name).copied() {
                registry.observe(id, frame.monotonic_ns);
            }
        }
        let choice = policy.choose(condition.as_deref(), &candidates);
        let cost = world.cost(&choice.family);
        policy.observe(&choice, cost);
        record_choice(&mut run, &world, &choice, cost);

        per_state.entry(state).or_default().push(cost);
        all_costs.push(cost);

        // Periodically: measure what states are worth, coin, consolidate,
        // and audit.
        if tick % 50 == 0 && tick > 150 {
            let utilities = measure(&per_state, &all_costs);
            let median_dwell = median_dwell_ns(&catalogue);
            for (state_id, utility) in &utilities {
                if coined.contains_key(state_id) {
                    registry.set_utility(
                        *registry_id(&registry, &coined[state_id]).expect("coined"),
                        *utility,
                    );
                    continue;
                }
                if let Some(latent) = catalogue.get(*state_id) {
                    let signature = signature_for(latent, &catalogue, ENTITIES, median_dwell, 0.0);
                    if let Ok(id) = registry.coin(latent, *utility, signature, frame.monotonic_ns) {
                        coined.insert(*state_id, id.to_string());
                        run.concepts_coined += 1;
                    }
                }
            }
            policy.consolidate();

            // A concept whose conditions have stopped occurring loses standing.
            // Without this its measured utility never falls, because utility is
            // computed over evidence that only ever accumulates, and a dead
            // belief would drive behaviour forever.
            registry.decay_unseen(frame.monotonic_ns, 3_000_000_000);

            // The lifecycle's second half: a concept that stopped paying is
            // retired, and everything conditioned on it goes with it.
            for retired in registry.audit() {
                let name = retired.to_string();
                policy.forget(&name);
                coined.retain(|_, coined_name| *coined_name != name);
                run.concepts_retired += 1;
            }
        }
    }

    run.attributable = policy.attributable_decisions();
    run
}

/// How much knowing the state reduces the spread of the outcome.
fn measure(
    per_state: &BTreeMap<LatentStateId, Vec<f64>>,
    all: &[f64],
) -> Vec<(LatentStateId, f64)> {
    if all.is_empty() {
        return Vec::new();
    }
    let grand = all.iter().sum::<f64>() / all.len() as f64;
    let baseline = all.iter().map(|c| (c - grand).abs()).sum::<f64>() / all.len() as f64;
    if baseline <= 0.0 {
        return Vec::new();
    }
    per_state
        .iter()
        .filter(|(_, costs)| costs.len() >= 8)
        .map(|(state, costs)| {
            let mean = costs.iter().sum::<f64>() / costs.len() as f64;
            let error = costs.iter().map(|c| (c - mean).abs()).sum::<f64>() / costs.len() as f64;
            (*state, ((baseline - error) / baseline).clamp(0.0, 1.0))
        })
        .collect()
}

fn registry_id<'a>(
    registry: &'a Registry,
    name: &str,
) -> Option<&'a corescout_concept::concept::ConceptId> {
    registry
        .concepts()
        .iter()
        .find(|c| c.id.to_string() == name)
        .map(|c| &c.id)
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

#[test]
fn the_oracle_beats_the_baseline_so_the_environment_is_learnable() {
    // If this fails, nothing else in the file means anything: there would be no
    // advantage available for any arm to find.
    let oracle = run_oracle(0xC0FFEE);
    let baseline = run_baseline(0xC0FFEE);
    assert!(
        oracle.mean_before(TURN) < baseline.mean_before(TURN) - 0.5,
        "oracle {:.3} vs baseline {:.3}",
        oracle.mean_before(TURN),
        baseline.mean_before(TURN)
    );
    assert!(oracle.right_action_in_regime > 100);
}

#[test]
fn a_concept_the_machine_coined_changes_what_it_does() {
    // The milestone. Not "a concept exists" and not "a concept predicts", but
    // that a self-derived concept changed the action taken.
    let run = run_concept(0xC0FFEE);
    assert!(
        run.concepts_coined > 0,
        "nothing was coined, so nothing could have changed the action"
    );
    assert!(
        run.attributable > 0,
        "no decision was changed by a concept: coined {} concepts but acted on none",
        run.concepts_coined
    );
}

#[test]
fn the_concept_agent_finds_the_action_the_oracle_knows() {
    // It will not match the oracle: the oracle is told the regime, and this
    // agent has to establish it causally, which is slow and expensive. What has
    // to be true is that its *belief* produces the right action, repeatedly,
    // and far more often than that belief produces the wrong one.
    let concept = run_concept(0xC0FFEE);
    let baseline = run_baseline(0xC0FFEE);
    assert_eq!(
        baseline.right_belief_in_regime, 0,
        "the floor takes no action"
    );
    assert!(
        concept.right_belief_in_regime > 50,
        "belief produced the right action only {} times",
        concept.right_belief_in_regime
    );
    assert!(
        concept.right_belief_in_regime > concept.stale_belief_after_turn * 10,
        "belief produced {} right actions and {} wrong ones; that is not a working          belief, it is a coin",
        concept.right_belief_in_regime,
        concept.stale_belief_after_turn
    );
}

#[test]
fn the_concept_agent_beats_the_baseline_while_the_regime_exists() {
    let concept = run_concept(0xC0FFEE);
    let baseline = run_baseline(0xC0FFEE);
    assert!(
        concept.mean_before(TURN) < baseline.mean_before(TURN),
        "concept {:.3} vs baseline {:.3}; the exploration cost is included in both",
        concept.mean_before(TURN),
        baseline.mean_before(TURN)
    );
}

#[test]
fn when_the_regime_dies_the_concept_dies_and_the_behaviour_reverts() {
    // The full lifecycle:
    //   experience -> concept -> belief -> behaviour -> failed prediction -> death
    //
    // After the turn, `a3` is the worst action available. The measurement that
    // matters is `stale_belief_after_turn`: how often the agent took the dead
    // action *while still believing in it*. Raw `stale_action_after_turn`
    // includes randomised probing, which is the price of being able to learn at
    // all and is paid whether the agent is right or wrong.
    let concept = run_concept(0xC0FFEE);
    assert!(
        concept.concepts_retired > 0,
        "no concept was retired after its regime stopped existing"
    );
    assert!(
        concept.stale_belief_after_turn * 20 < concept.right_action_in_regime,
        "the agent took the dead action from belief {} times against {} correct          actions while the regime held; the belief did not die",
        concept.stale_belief_after_turn,
        concept.right_action_in_regime
    );
}

#[test]
fn exploration_is_the_cost_and_it_is_reported_rather_than_hidden() {
    // The concept agent is *worse* than the baseline after the turn, and that
    // is honest: it is still paying to keep its beliefs testable on a machine
    // where there is currently nothing to find. Recording the size of that cost
    // stops it from being quietly explained away later.
    let concept = run_concept(0xC0FFEE);
    let baseline = run_baseline(0xC0FFEE);
    let after = TOTAL - TURN;
    let overhead = concept.mean_after(after) - baseline.mean_after(after);
    assert!(
        overhead > 0.0,
        "exploration should cost something; if it costs nothing it is not happening"
    );
    assert!(
        overhead < 0.5,
        "exploration overhead of {overhead:.3} per decision is too high to be worth it"
    );
}

#[test]
fn the_experiment_is_reproducible() {
    let a = run_concept(11);
    let b = run_concept(11);
    assert_eq!(a.attributable, b.attributable);
    assert_eq!(a.concepts_coined, b.concepts_coined);
    assert!((a.cost_before - b.cost_before).abs() < 1e-9);
}

#[test]
fn report_the_four_arms() {
    // Printed rather than asserted, so the comparison can be read. Run with
    // `-- --nocapture`.
    let after = TOTAL - TURN;
    println!(
        "\n{:<10} {:>11} {:>11} {:>7} {:>7} {:>7} {:>7} {:>7}",
        "arm", "cost bef", "cost aft", "right", "r-blf", "stale", "s-blf", "attrib"
    );
    for (name, run) in [
        ("baseline", run_baseline(0xC0FFEE)),
        ("latent", run_latent(0xC0FFEE)),
        ("concept", run_concept(0xC0FFEE)),
        ("oracle", run_oracle(0xC0FFEE)),
    ] {
        println!(
            "{:<10} {:>11.3} {:>11.3} {:>7} {:>7} {:>7} {:>7} {:>7}",
            name,
            run.mean_before(TURN),
            run.mean_after(after),
            run.right_action_in_regime,
            run.right_belief_in_regime,
            run.stale_action_after_turn,
            run.stale_belief_after_turn,
            run.attributable
        );
    }
    println!(
        "\nright  = took a3 while the regime held, for any reason\n\
         r-blf  = ...because it believed in it (higher is better)\n\
         stale  = took a3 after it became harmful, for any reason\n\
         s-blf  = ...because it still believed in it (lower is better)\n\
         attrib = decisions a self-coined concept changed\n\
         \n\
         right minus r-blf is the cost of exploring.\n\
         s-blf above zero is the cost of being wrong."
    );
}

#[test]
fn diagnose_the_concept_arm() {
    // Instrumentation, not a claim. Prints where the evidence goes, so a
    // failure to establish a preference can be diagnosed rather than tuned
    // around.
    let mut world = World::new(0xC0FFEE);
    let mut catalogue = LatentCatalogue::new(0.9, 32);
    let mut normalizer: Option<Normalizer> = None;
    let mut warmup: Vec<Vec<f64>> = Vec::new();
    let mut registry = Registry::new(CoinageRules {
        min_occurrences: 25,
        min_utility: 0.20,
        min_confidence: 0.5,
        retire_below: 0.10,
        max_concepts: 16,
    });
    let mut policy = ConditionedPolicy::new(
        ConditionedConfig {
            min_trials: 10,
            default_action: "hold".into(),
            lower_is_better: true,
        },
        Exploration::new(0.25, 10, 4000, 0x1234),
    );
    let mut per_state: BTreeMap<LatentStateId, Vec<f64>> = BTreeMap::new();
    let mut all_costs: Vec<f64> = Vec::new();
    let mut coined: BTreeMap<LatentStateId, String> = BTreeMap::new();
    let candidates: Vec<String> = ACTIONS.iter().map(|s| s.to_string()).collect();
    let mut regime_states: BTreeMap<LatentStateId, u32> = BTreeMap::new();
    let mut conditions_seen = 0u32;

    for tick in 0..TURN {
        let frame = world.tick();
        let raw = frame.state.as_slice().to_vec();
        if normalizer.is_none() {
            warmup.push(raw.clone());
            if warmup.len() >= 120 {
                normalizer = Some(Normalizer::fit(&warmup, CHANNELS));
            }
            world.cost("hold");
            continue;
        }
        let (point, _) = normalizer.as_ref().unwrap().apply_filled(&raw, CHANNELS);
        let (state, _) = catalogue.observe(&point, frame.monotonic_ns);
        if world.regime_now {
            *regime_states.entry(state).or_default() += 1;
        }
        let condition = coined.get(&state).cloned();
        if condition.is_some() {
            conditions_seen += 1;
        }
        let choice = policy.choose(condition.as_deref(), &candidates);
        let cost = world.cost(&choice.family);
        policy.observe(&choice, cost);
        per_state.entry(state).or_default().push(cost);
        all_costs.push(cost);

        if tick % 50 == 0 && tick > 150 {
            let utilities = measure(&per_state, &all_costs);
            let median_dwell = median_dwell_ns(&catalogue);
            for (state_id, utility) in &utilities {
                if coined.contains_key(state_id) {
                    continue;
                }
                if let Some(latent) = catalogue.get(*state_id) {
                    let signature = signature_for(latent, &catalogue, ENTITIES, median_dwell, 0.0);
                    if let Ok(id) = registry.coin(latent, *utility, signature, frame.monotonic_ns) {
                        coined.insert(*state_id, id.to_string());
                    }
                }
            }
            policy.consolidate();
        }
    }

    println!(
        "\nlatent states covering the regime: {}",
        regime_states.len()
    );
    for (state, count) in regime_states.iter().take(6) {
        println!(
            "  {state}: {count} regime frames, coined as {:?}",
            coined.get(state)
        );
    }
    println!("concepts coined: {}", registry.len());
    println!("ticks with a recognised condition: {conditions_seen}");
    println!("exploration spent: {}", policy.exploration().spent());
    println!("comparisons: {}", policy.attribution().len());
    for estimate in policy.attribution().estimates() {
        println!("  {}", estimate.describe(10));
    }
    println!("preferences: {:?}", policy.preferences());
}
