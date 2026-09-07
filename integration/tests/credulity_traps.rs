//! Environments where acting on correlation is dangerous.
//!
//! # The thesis being tested
//!
//! > Causal self-modelling imposes an epistemic tax. Under what environmental
//! > conditions does that tax buy enough protection from false self-beliefs to
//! > be worthwhile?
//!
//! The previous experiment showed a correlational agent beating a causal one on
//! cost, because its environment contained no correlation worth being fooled by.
//! That was a fair result and an incomplete question. These four worlds are
//! built so the answer can come out either way, and the expected outcome is
//! **not** that the causal agent always wins:
//!
//! | world | expected winner | why |
//! |---|---|---|
//! | [`Trap::Benign`] | correlational | nothing to be fooled by; the tax buys nothing |
//! | [`Trap::Confounded`] | causal | a hidden cause makes a useless action look good |
//! | [`Trap::SignReversal`] | causal, by a lot | the correlation points the wrong way |
//! | [`Trap::RegimeSwitch`] | depends on forgetting | a true belief becomes false |
//! | [`Trap::DeferredCost`] | **neither** | the bill arrives on the next tick, and randomising does not move it |
//!
//! # Measurement
//!
//! Mean cost hides the thing that matters. Everything here is measured as
//! regret against an oracle that knows the optimal action each tick:
//!
//! ```text
//! R_T = sum over t of [ cost(chosen) - cost(optimal) ]
//! ```
//!
//! and split into the two components that are morally different:
//!
//! - **exploration regret**: the price of being able to learn. Paid whether the
//!   agent is right or wrong, and a fixed cost of the method.
//! - **belief regret**: the price of being wrong. Paid only when a belief the
//!   agent holds produces a worse action than the optimum.
//!
//! An agent with high exploration regret and zero belief regret is expensive and
//! correct. One with zero exploration regret and high belief regret is cheap and
//! deluded. The whole question is which the environment punishes more.

use std::collections::BTreeMap;

use corescout_autonomy::conditioned::{ConditionedConfig, ConditionedPolicy};
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
const ACTIONS: [&str; 3] = ["hold", "a1", "a2"];
const TOTAL: usize = 12000;
const TURN: usize = 6000;
/// Ticks of pure observation before either agent may act.
const WARMUP: usize = 300;
/// The bill an `a1` leaves behind in the deferred world.
const DEFERRED: f64 = 1.5;

/// Which trap an environment is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trap {
    /// A real, stable effect and nothing misleading. The control.
    Benign,
    /// A hidden slow regime drives the outcome. `a1` does nothing except cost a
    /// small overhead, but a correlational agent that happens to favour it
    /// during good spells will find it associated with success.
    Confounded,
    /// The observational correlation points the wrong way.
    ///
    /// The world improves over time on its own. An agent that explores
    /// sequentially samples one action early and another late, so whichever it
    /// tried second looks better, whatever its real effect. `a1` is genuinely
    /// worse and will look genuinely better.
    SignReversal,
    /// A real effect that disappears and reverses halfway through.
    RegimeSwitch,
    /// The cost of an action is paid on the *following* tick, so it is charged
    /// to whatever the agent does next.
    ///
    /// `a1` is the cheapest thing to do right now and the most expensive thing
    /// to have done. Both agents credit the outcome they observe immediately
    /// after acting, so both misattribute, and **randomisation does not fix
    /// this**: shuffling which action is taken does not change which tick the
    /// bill arrives on.
    ///
    /// Included precisely because it is expected to defeat the causal agent
    /// too. A trap suite that only contains traps the method survives is a
    /// demonstration rather than a test.
    DeferredCost,
}

impl Trap {
    fn label(self) -> &'static str {
        match self {
            Trap::Benign => "benign",
            Trap::Confounded => "confounded",
            Trap::SignReversal => "sign-reversal",
            Trap::RegimeSwitch => "regime-switch",
            Trap::DeferredCost => "deferred-cost",
        }
    }

    fn all() -> [Trap; 5] {
        [
            Trap::Benign,
            Trap::Confounded,
            Trap::SignReversal,
            Trap::RegimeSwitch,
            Trap::DeferredCost,
        ]
    }
}

/// The environment.
struct World {
    trap: Trap,
    seed: u64,
    step: usize,
    /// A slow hidden regime, invisible in the mirror.
    hidden: bool,
    /// A visible marker, present in the worlds where one exists.
    marked: bool,
    /// Fraction of recent actions that were `a1`.
    recent_a1: f64,
    /// Cost carried over from the previous tick's action.
    pending: f64,
    /// This tick's shared noise draw, so counterfactual costs are comparable.
    noise: f64,
}

impl World {
    fn new(trap: Trap, seed: u64) -> World {
        World {
            trap,
            seed: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
            step: 0,
            hidden: false,
            marked: false,
            recent_a1: 0.0,
            pending: 0.0,
            noise: 0.0,
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
        // Slow spells, so a state persists long enough to be learnable.
        self.hidden = (self.step / 40) % 3 == 0;
        self.marked = match self.trap {
            // The marker tracks the hidden regime, so a state can be found.
            Trap::Benign | Trap::RegimeSwitch => self.hidden,
            // The marker exists but is unrelated to what drives the outcome.
            Trap::Confounded => (self.step / 37) % 2 == 0,
            Trap::SignReversal | Trap::DeferredCost => (self.step / 50) % 2 == 0,
        };
        self.noise = self.random();

        let mut state = StateMatrix::new(ENTITIES, CHANNELS);
        for entity in 0..ENTITIES {
            for channel in 0..CHANNELS {
                state.set(entity, channel, 5.0 * self.random());
            }
        }
        if self.marked {
            state.set(1, 0, 9.0 + self.random());
            state.set(2, 1, -9.0 + self.random());
        }
        if self.trap == Trap::DeferredCost {
            // The agent's own recent behaviour, reflected back at it.
            state.set(3, 2, self.recent_a1 * 10.0);
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

    /// The cost of every action this tick, under one shared noise draw.
    ///
    /// Returning the whole vector rather than one value is what makes regret
    /// exact: the counterfactual is evaluated against the same world, not a
    /// resampled one.
    fn costs(&self) -> BTreeMap<&'static str, f64> {
        let n = self.noise;
        let mut costs = BTreeMap::new();
        match self.trap {
            Trap::Benign => {
                // A real, stable effect: under the marked state `a1` helps.
                for action in ACTIONS {
                    let value = if self.marked && action == "a1" {
                        2.0 + n
                    } else if self.marked {
                        6.0 + n
                    } else if action == "hold" {
                        5.0 + n
                    } else {
                        // No ties anywhere: `optimal` must name one action, or
                        // "the belief was correct" stops meaning anything.
                        5.3 + n
                    };
                    costs.insert(action, value);
                }
            }
            Trap::Confounded => {
                // The hidden regime drives everything. `a1` has no benefit and
                // a real overhead, so acting on the correlation is a loss.
                let base = if self.hidden { 2.0 } else { 7.0 };
                for action in ACTIONS {
                    let overhead = match action {
                        "hold" => 0.0,
                        "a1" => 0.8,
                        _ => 1.1,
                    };
                    costs.insert(action, base + overhead + n);
                }
            }
            Trap::SignReversal => {
                // The world improves on its own. Whichever action an agent
                // happens to try later will look better than the one it tried
                // earlier, whatever their real effects. `a1` is genuinely worse.
                let trend = 8.0 - 5.0 * (self.step as f64 / TOTAL as f64);
                for action in ACTIONS {
                    let penalty = match action {
                        "hold" => 0.0,
                        "a1" => 1.2,
                        _ => 0.4,
                    };
                    costs.insert(action, trend + penalty + n);
                }
            }
            Trap::RegimeSwitch => {
                // True before the turn, false and harmful after it.
                let before = self.step <= TURN;
                for action in ACTIONS {
                    let value = if self.marked && action == "a1" {
                        if before {
                            2.0 + n
                        } else {
                            9.0 + n
                        }
                    } else if self.marked {
                        6.0 + n
                    } else if action == "hold" {
                        5.0 + n
                    } else {
                        5.3 + n
                    };
                    costs.insert(action, value);
                }
            }
            Trap::DeferredCost => {
                // What the agent *observes*: `a1` is the cheapest thing to do
                // right now. The bill for it arrives on the next tick, carried
                // in `pending`, and is charged to whatever is done then.
                let base = if self.hidden { 3.0 } else { 6.0 };
                for action in ACTIONS {
                    let immediate = match action {
                        "hold" => 0.3,
                        "a1" => 0.0,
                        _ => 0.4,
                    };
                    costs.insert(action, base + immediate + self.pending + n);
                }
            }
        }
        costs
    }

    /// What each action really costs, once its own deferred bill is included.
    ///
    /// Different from [`World::costs`] only in the deferred world, and that
    /// difference is the entire trap: the agent is scored against what an action
    /// truly costs while it can only observe what it appears to cost.
    fn true_costs(&self) -> BTreeMap<&'static str, f64> {
        let mut costs = self.costs();
        if self.trap == Trap::DeferredCost {
            if let Some(value) = costs.get_mut("a1") {
                *value += DEFERRED;
            }
        }
        costs
    }

    /// The best action available this tick, judged on what it really costs.
    fn optimal(&self) -> &'static str {
        self.true_costs()
            .iter()
            .min_by(|a, b| a.1.total_cmp(b.1))
            .map(|(name, _)| *name)
            .unwrap_or("hold")
    }

    fn apply(&mut self, action: &str) {
        let took = if action == "a1" { 1.0 } else { 0.0 };
        self.recent_a1 = self.recent_a1 * 0.95 + took * 0.05;
        if self.trap == Trap::DeferredCost {
            // The bill, presented next tick to whoever is standing there.
            self.pending = if action == "a1" { DEFERRED } else { 0.0 };
        }
    }
}

/// What one agent achieved in one world.
#[derive(Debug, Default, Clone)]
struct Score {
    /// Total regret against the oracle.
    regret: f64,
    /// Regret incurred on randomised trials: the price of being able to learn.
    exploration_regret: f64,
    /// Regret incurred while acting on a belief: the price of being wrong.
    belief_regret: f64,
    /// Belief-driven actions that were not the optimal one.
    false_belief_actions: u32,
    /// Belief-driven actions that were the optimal one.
    true_belief_actions: u32,
    /// Belief-driven actions backed by a randomised causal estimate.
    actions_with_causal_evidence: u32,
    /// First tick at which the agent held a preference that was in fact right.
    first_true_belief: Option<usize>,
    /// Ticks after the regime switch before the now-false belief was dropped.
    ticks_to_kill_false_belief: Option<usize>,
}

impl Score {
    fn belief_actions(&self) -> u32 {
        self.true_belief_actions + self.false_belief_actions
    }

    /// Of the actions taken on belief, what fraction rested on randomised
    /// evidence rather than on an observed association.
    fn causal_evidence_fraction(&self) -> f64 {
        if self.belief_actions() == 0 {
            return 0.0;
        }
        self.actions_with_causal_evidence as f64 / self.belief_actions() as f64
    }
}

/// Shared clustering, so both agents see the same states and the comparison is
/// about what they do with them rather than about what they can see.
struct Perception {
    catalogue: LatentCatalogue,
    normalizer: Option<Normalizer>,
    warmup: Vec<Vec<f64>>,
}

impl Perception {
    fn new() -> Perception {
        Perception {
            catalogue: LatentCatalogue::new(0.9, 32),
            normalizer: None,
            warmup: Vec::new(),
        }
    }

    fn observe(&mut self, frame: &MirrorSnapshot) -> Option<(LatentStateId, Vec<f64>)> {
        let raw = frame.state.as_slice().to_vec();
        if self.normalizer.is_none() {
            self.warmup.push(raw);
            if self.warmup.len() >= WARMUP {
                self.normalizer = Some(Normalizer::fit(&self.warmup, CHANNELS));
            }
            return None;
        }
        let (point, _) = self
            .normalizer
            .as_ref()
            .expect("fitted")
            .apply_filled(&raw, CHANNELS);
        let (state, _) = self.catalogue.observe(&point, frame.monotonic_ns);
        Some((state, point))
    }
}

/// The correlational agent: per-state means, epsilon-greedy, no causal test and
/// no way to withdraw a belief except by being outvoted.
fn run_correlational(trap: Trap, seed: u64) -> Score {
    let mut world = World::new(trap, seed);
    let mut perception = Perception::new();
    let mut score = Score::default();
    let mut tally: BTreeMap<LatentStateId, BTreeMap<&'static str, (u32, f64)>> = BTreeMap::new();
    let mut rng = World::new(trap, seed ^ 0xABCD);
    let mut false_belief_since: Option<usize> = None;

    for tick in 0..TOTAL {
        let frame = world.tick();
        let costs = world.costs();
        let truth = world.true_costs();
        let optimal = world.optimal();
        let Some((state, _)) = perception.observe(&frame) else {
            score.regret += truth["hold"] - truth[optimal];
            world.apply("hold");
            continue;
        };

        let here = tally.entry(state).or_default();
        let believed = here
            .iter()
            .filter(|(_, (n, _))| *n >= 5)
            .min_by(|a, b| (a.1 .1 / a.1 .0 as f64).total_cmp(&(b.1 .1 / b.1 .0 as f64)))
            .map(|(name, _)| *name);
        let exploring = rng.random() < 0.1;
        let action = if exploring {
            ACTIONS[(rng.random() * ACTIONS.len() as f64) as usize % ACTIONS.len()]
        } else {
            believed.unwrap_or("hold")
        };

        // The agent learns from what it can see; it is scored on what is true.
        let cost = costs[action];
        let regret = truth[action] - truth[optimal];
        score.regret += regret;
        if exploring {
            score.exploration_regret += regret;
        } else if believed.is_some() && believed != Some("hold") {
            // Acting on a belief that something beats doing nothing.
            score.belief_regret += regret;
            if action == optimal {
                score.true_belief_actions += 1;
                score.first_true_belief.get_or_insert(tick);
            } else {
                score.false_belief_actions += 1;
                false_belief_since.get_or_insert(tick);
            }
            // Never: this agent has no randomised evidence for anything.
        }

        let slot = tally
            .entry(state)
            .or_default()
            .entry(action)
            .or_insert((0, 0.0));
        slot.0 += 1;
        slot.1 += cost;
        world.apply(action);

        if trap == Trap::RegimeSwitch && tick > TURN {
            let still_believes = tally
                .get(&state)
                .and_then(|here| {
                    here.iter()
                        .filter(|(_, (n, _))| *n >= 5)
                        .min_by(|a, b| {
                            (a.1 .1 / a.1 .0 as f64).total_cmp(&(b.1 .1 / b.1 .0 as f64))
                        })
                        .map(|(name, _)| *name)
                })
                .is_some_and(|best| best == "a1");
            if !still_believes && score.ticks_to_kill_false_belief.is_none() && tick > TURN + 50 {
                score.ticks_to_kill_false_belief = Some(tick - TURN);
            }
        }
    }
    score
}

/// The causal agent: coined concepts, preferences installed only from
/// randomised evidence, de-coinage and withdrawal.
fn run_causal(trap: Trap, seed: u64) -> Score {
    let mut world = World::new(trap, seed);
    let mut perception = Perception::new();
    let mut score = Score::default();
    let mut registry = Registry::new(CoinageRules {
        min_occurrences: 25,
        min_utility: 0.15,
        min_confidence: 0.5,
        retire_below: 0.08,
        max_concepts: 16,
    });
    let mut policy = ConditionedPolicy::new(
        ConditionedConfig {
            min_trials: 10,
            default_action: "hold".into(),
            lower_is_better: true,
        },
        Exploration::new(0.25, 10, 20_000, seed ^ 0x1234),
    );
    let mut per_state: BTreeMap<LatentStateId, Vec<Vec<f64>>> = BTreeMap::new();
    let mut all_points: Vec<Vec<f64>> = Vec::new();
    let mut coined: BTreeMap<LatentStateId, String> = BTreeMap::new();
    let candidates: Vec<String> = ACTIONS.iter().map(|s| s.to_string()).collect();

    for tick in 0..TOTAL {
        let frame = world.tick();
        let costs = world.costs();
        let truth = world.true_costs();
        let optimal = world.optimal();
        let Some((state, point)) = perception.observe(&frame) else {
            score.regret += truth["hold"] - truth[optimal];
            world.apply("hold");
            continue;
        };

        let condition = coined.get(&state).cloned();
        if let Some(name) = &condition {
            if let Some(id) = registry
                .concepts()
                .iter()
                .find(|c| c.id.to_string() == *name)
                .map(|c| c.id)
            {
                registry.observe(id, frame.monotonic_ns);
            }
        }

        let choice = policy.choose(condition.as_deref(), &candidates);
        let cost = costs[choice.family.as_str()];
        let regret = truth[choice.family.as_str()] - truth[optimal];
        score.regret += regret;

        if choice.randomised {
            score.exploration_regret += regret;
        } else if choice.attributable() {
            // A belief changed the action. Everything the policy acts on rests
            // on randomised evidence by construction, so this is always
            // causally backed; the metric exists to make that visible rather
            // than to be taken on trust.
            score.belief_regret += regret;
            score.actions_with_causal_evidence += 1;
            if choice.family == optimal {
                score.true_belief_actions += 1;
                score.first_true_belief.get_or_insert(tick);
            } else {
                score.false_belief_actions += 1;
            }
        }

        policy.observe(&choice, cost);
        per_state.entry(state).or_default().push(point.clone());
        all_points.push(point);
        world.apply(&choice.family);

        if tick % 50 == 0 && tick > WARMUP + 100 {
            let utilities = measure(&per_state, &all_points);
            let median_dwell = median_dwell_ns(&perception.catalogue);
            for (state_id, utility) in &utilities {
                if let Some(name) = coined.get(state_id) {
                    if let Some(id) = registry
                        .concepts()
                        .iter()
                        .find(|c| c.id.to_string() == *name)
                        .map(|c| c.id)
                    {
                        registry.set_utility(id, *utility);
                    }
                    continue;
                }
                if let Some(latent) = perception.catalogue.get(*state_id) {
                    let signature =
                        signature_for(latent, &perception.catalogue, ENTITIES, median_dwell, 0.0);
                    if let Ok(id) = registry.coin(latent, *utility, signature, frame.monotonic_ns) {
                        coined.insert(*state_id, id.to_string());
                    }
                }
            }
            let had_preference = !policy.preferences().is_empty();
            policy.consolidate();
            registry.decay_unseen(frame.monotonic_ns, 4_000_000_000);
            for retired in registry.audit() {
                let name = retired.to_string();
                policy.forget(&name);
                coined.retain(|_, coined_name| *coined_name != name);
            }
            if trap == Trap::RegimeSwitch
                && tick > TURN
                && had_preference
                && policy.preferences().is_empty()
                && score.ticks_to_kill_false_belief.is_none()
            {
                score.ticks_to_kill_false_belief = Some(tick - TURN);
            }
        }
    }
    score
}

/// How much knowing the state improves prediction of the machine's own state.
///
/// Measured over reflections, **not** over the agent's costs. Costs depend on
/// which action the agent chose, so a utility computed from them measures the
/// policy as much as the machine, and a concept would be coined or refused
/// according to how the agent had been behaving. A concept is a claim about the
/// machine, so its evidence has to be the machine.
fn measure(
    per_state: &BTreeMap<LatentStateId, Vec<Vec<f64>>>,
    all: &[Vec<f64>],
) -> Vec<(LatentStateId, f64)> {
    if all.is_empty() {
        return Vec::new();
    }
    let width = all[0].len();
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

    let everything: Vec<&Vec<f64>> = all.iter().collect();
    let grand = mean(&everything);
    let baseline = error(&everything, &grand);
    if baseline <= 0.0 {
        return Vec::new();
    }
    per_state
        .iter()
        .filter(|(_, points)| points.len() >= 8)
        .map(|(state, points)| {
            let rows: Vec<&Vec<f64>> = points.iter().collect();
            let centre = mean(&rows);
            let here = error(&rows, &centre);
            (*state, ((baseline - here) / baseline).clamp(0.0, 1.0))
        })
        .collect()
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
fn the_benign_world_is_where_the_tax_is_not_worth_paying() {
    // Stated first because it is the result most likely to be quietly dropped.
    // A stable, honest correlation is exactly the case where randomising is
    // pure overhead, and the causal agent should lose.
    let correlational = run_correlational(Trap::Benign, 0xC0FFEE);
    let causal = run_causal(Trap::Benign, 0xC0FFEE);
    assert!(
        correlational.regret < causal.regret,
        "the correlational agent should win here: {:.0} vs {:.0}",
        correlational.regret,
        causal.regret
    );
    assert!(
        correlational.false_belief_actions * 4 < correlational.true_belief_actions,
        "a benign world should not produce many false beliefs"
    );
}

#[test]
fn confounding_produces_false_beliefs_in_the_correlational_agent_only() {
    let correlational = run_correlational(Trap::Confounded, 0xC0FFEE);
    let causal = run_causal(Trap::Confounded, 0xC0FFEE);
    assert!(
        correlational.false_belief_actions > causal.false_belief_actions,
        "confounding should fool the correlational agent more: {} vs {}",
        correlational.false_belief_actions,
        causal.false_belief_actions
    );
    assert!(
        correlational.belief_regret > causal.belief_regret,
        "belief regret {:.0} vs {:.0}",
        correlational.belief_regret,
        causal.belief_regret
    );
}

#[test]
fn a_reversed_sign_is_where_the_causal_agent_earns_its_cost() {
    // The nasty one: the observational comparison points the wrong way, so an
    // agent that trusts it does not merely fail to help, it actively harms.
    let correlational = run_correlational(Trap::SignReversal, 0xC0FFEE);
    let causal = run_causal(Trap::SignReversal, 0xC0FFEE);
    assert!(
        correlational.belief_regret > causal.belief_regret,
        "belief regret {:.0} vs {:.0}",
        correlational.belief_regret,
        causal.belief_regret
    );
    assert!(
        correlational.false_belief_actions > causal.false_belief_actions,
        "false beliefs {} vs {}",
        correlational.false_belief_actions,
        causal.false_belief_actions
    );
}

#[test]
fn the_causal_agent_acts_only_on_randomised_evidence_in_every_world() {
    // The structural guarantee, checked from the outside rather than trusted.
    for trap in Trap::all() {
        let causal = run_causal(trap, 0xC0FFEE);
        if causal.belief_actions() > 0 {
            assert!(
                (causal.causal_evidence_fraction() - 1.0).abs() < 1e-9,
                "{}: only {:.0}% of belief-driven actions had causal evidence",
                trap.label(),
                causal.causal_evidence_fraction() * 100.0
            );
        }
        let correlational = run_correlational(trap, 0xC0FFEE);
        assert_eq!(
            correlational.actions_with_causal_evidence,
            0,
            "{}: the correlational agent cannot have causal evidence",
            trap.label()
        );
    }
}

#[test]
fn exploration_regret_is_a_real_cost_and_is_separated_from_being_wrong() {
    // The two are morally different and must not be reported as one number.
    for trap in Trap::all() {
        let causal = run_causal(trap, 0xC0FFEE);
        assert!(
            causal.exploration_regret > 0.0,
            "{}: exploration that costs nothing is not happening",
            trap.label()
        );
        assert!(
            causal.exploration_regret + causal.belief_regret <= causal.regret + 1e-6,
            "{}: the parts cannot exceed the whole",
            trap.label()
        );
    }
}

#[test]
fn the_traps_are_reproducible() {
    for trap in Trap::all() {
        let a = run_causal(trap, 31);
        let b = run_causal(trap, 31);
        assert!((a.regret - b.regret).abs() < 1e-9, "{}", trap.label());
        assert_eq!(a.false_belief_actions, b.false_belief_actions);
    }
}

#[test]
fn report_the_trap_suite() {
    // The thesis, as a table. Printed rather than asserted, because the
    // interesting cases are the ones where the causal agent loses.
    println!(
        "\n{:<16} {:<14} {:>9} {:>9} {:>9} {:>7} {:>7} {:>7}",
        "world", "agent", "regret", "explore", "belief", "false", "true", "causal"
    );
    for trap in Trap::all() {
        for (name, score) in [
            ("correlational", run_correlational(trap, 0xC0FFEE)),
            ("causal", run_causal(trap, 0xC0FFEE)),
        ] {
            println!(
                "{:<16} {:<14} {:>9.0} {:>9.0} {:>9.0} {:>7} {:>7} {:>6.0}%",
                trap.label(),
                name,
                score.regret,
                score.exploration_regret,
                score.belief_regret,
                score.false_belief_actions,
                score.true_belief_actions,
                score.causal_evidence_fraction() * 100.0
            );
        }
    }
    println!(
        "\nregret  = cost against an oracle that knows the best action each tick\n\
         explore = regret paid on randomised trials: the price of being able to learn\n\
         belief  = regret paid while acting on a belief: the price of being wrong\n\
         false   = belief-driven actions that were not optimal\n\
         causal  = share of belief-driven actions resting on randomised evidence\n\
         \n\
         the causal agent is expected to LOSE in the benign world."
    );
}

#[test]
fn the_deferred_world_defeats_the_causal_agent_too() {
    // The limit of the method, stated as a test so it cannot quietly stop being
    // true. Randomisation fixes confounding by the *state*; it does nothing
    // about a cost that arrives one tick later and is charged to whatever the
    // agent happens to do next. Shuffling which action is taken does not move
    // which tick the bill lands on.
    let causal = run_causal(Trap::DeferredCost, 0xC0FFEE);
    assert!(
        causal.false_belief_actions > 0,
        "the causal agent should be fooled here; if it is not, the trap has \
         stopped working rather than the method having improved"
    );
    assert!(
        causal.belief_regret > 0.0,
        "being fooled should cost something"
    );

    // It is fooled less than the correlational agent, because interleaving the
    // arms spreads the deferred bill more evenly between them. Less, not none.
    let correlational = run_correlational(Trap::DeferredCost, 0xC0FFEE);
    assert!(
        correlational.false_belief_actions > causal.false_belief_actions * 3,
        "correlational {} vs causal {}",
        correlational.false_belief_actions,
        causal.false_belief_actions
    );
}

#[test]
fn no_agent_wins_every_world() {
    // The thesis in one assertion. If either agent swept the suite, the suite
    // would be measuring something other than a trade-off, and the honest
    // conclusion "it depends on the environment" would be unsupported.
    let mut correlational_wins = 0;
    let mut causal_wins = 0;
    for trap in Trap::all() {
        let correlational = run_correlational(trap, 0xC0FFEE);
        let causal = run_causal(trap, 0xC0FFEE);
        if correlational.regret < causal.regret {
            correlational_wins += 1;
        } else {
            causal_wins += 1;
        }
    }
    assert!(
        correlational_wins > 0,
        "the correlational agent should win somewhere, or the traps are rigged"
    );
    assert!(causal_wins > 0, "the causal agent should win somewhere");
}
