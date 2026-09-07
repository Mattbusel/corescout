//! Does a lineage of self-modified runtimes make more effective compute?
//!
//! # The claim
//!
//! ```text
//! Q(G) = verified useful work / scarce physical inputs
//!
//! Q(G0) < Q(G1) < Q(G2) < ...
//! ```
//!
//! If that holds on **held-out** work, net of what the search cost, and by more
//! than a control lineage that learns nothing, then the process has turned
//! compute into computational capital. Every one of those qualifiers is doing
//! work, and dropping any of them makes the result meaningless.
//!
//! # The substrate
//!
//! A synthetic machine with a configuration space. Each work unit is executed
//! under a configuration, and the cost of executing it depends on:
//!
//! - a **general** structure that holds across all workloads, which is the part
//!   worth learning;
//! - a **workload-specific** quirk, which is the part that looks like learning
//!   and is not;
//! - noise.
//!
//! A lineage that finds the general structure improves on held-out work. One
//! that fits the quirks improves only on what it practised, and the harness
//! says so.
//!
//! # Work is verified
//!
//! Every unit returns a checksum computed from its input. The expected value is
//! held by the harness and never shown to the runtime. A descendant that
//! executes fewer instructions and returns the wrong answer scores **zero** for
//! that unit and is still charged for the silicon, so there is no configuration
//! that is fast because it skips the work.
//!
//! # What this cannot show
//!
//! It is a simulation with a planted structure. It shows the accounting works
//! and that a lineage *can* compound when there is something to find. It says
//! nothing about whether real silicon contains such structure, which is the
//! question that decides whether any of this matters.

use corescout_lineage::productivity::{Inputs, Pool, Weights};
use corescout_lineage::{Comparison, Lineage, Verdict};

/// Knobs a runtime can set. The configuration space it searches.
const KNOBS: usize = 6;
const LEVELS: i32 = 9;
/// Work units each generation executes, per pool.
///
/// Large relative to the search, so a generation is mostly producing. A line
/// that spends most of its silicon searching is not a productive process, and
/// an experiment where it did would be measuring the search.
const UNITS: u64 = 20_000;
/// Configurations a single generation may try.
///
/// Deliberately far too small to solve the space. A generation that could find
/// the optimum on its own would make the lineage pointless: the whole claim is
/// that a descendant starts from where its ancestor left off, and that can only
/// be tested if no single generation gets all the way.
const SEARCH_BUDGET: u32 = 14;

/// The machine. Its cost structure is never shown to any runtime.
struct Substrate {
    seed: u64,
    /// The configuration that is genuinely best, across all workloads.
    ///
    /// The general structure: the thing worth learning.
    optimum: [i32; KNOBS],
}

impl Substrate {
    fn new(seed: u64) -> Substrate {
        Substrate {
            seed: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
            // Far from the configuration a fresh machine starts in, so
            // reaching it takes several generations of partial progress.
            optimum: [7, 2, 8, 3, 6, 1],
        }
    }

    fn random(&mut self) -> f64 {
        self.seed ^= self.seed >> 12;
        self.seed ^= self.seed << 25;
        self.seed ^= self.seed >> 27;
        let value = self.seed.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (value >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Silicon nanoseconds to execute one unit of `workload` under `config`.
    ///
    /// Three terms, and telling them apart is the whole experiment.
    fn cost_ns(&mut self, config: &[i32; KNOBS], workload: u32) -> u64 {
        // The general term: distance from the true optimum. Falls for any
        // workload, so learning it transfers.
        let general: i32 = config
            .iter()
            .zip(self.optimum.iter())
            .map(|(a, b)| (a - b).abs())
            .sum();

        // The workload-specific term: a quirk that rewards a different
        // configuration for each workload. Fitting this looks exactly like
        // learning and transfers to nothing.
        let quirk_knob = (workload as usize) % KNOBS;
        let quirk_target = (workload as i32 * 7) % LEVELS;
        let quirk = (config[quirk_knob] - quirk_target).abs();

        let base = 1000.0;
        let noise = 0.94 + 0.12 * self.random();
        ((base + general as f64 * 90.0 + quirk as f64 * 40.0) * noise) as u64
    }

    /// Execute one unit. The result must match what the harness expects.
    ///
    /// The checksum depends only on the unit's input, so no configuration can
    /// change it. A runtime that returns something else did not do the work.
    fn execute(&mut self, config: &[i32; KNOBS], workload: u32, unit: u64) -> (u64, u64) {
        let cost = self.cost_ns(config, workload);
        (checksum(workload, unit), cost)
    }
}

/// What a correct execution must return.
fn checksum(workload: u32, unit: u64) -> u64 {
    let mut value = unit.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (workload as u64).wrapping_mul(31);
    value ^= value >> 29;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^ (value >> 32)
}

/// Workloads a lineage may explore against.
const TRAINED: [u32; 3] = [1, 2, 3];
/// Workloads it never sees until it is measured on them.
const HELD_OUT: [u32; 3] = [11, 12, 13];

/// A runtime's inherited knowledge: the configuration it starts from.
///
/// This is what a descendant is *born with*, and it is the only thing carried
/// across a generation boundary.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Inheritance {
    config: [i32; KNOBS],
    /// How many configurations the ancestors tried, for the edge count.
    explored: usize,
}

impl Default for Inheritance {
    fn default() -> Self {
        // The configuration a fresh machine starts in: all knobs at zero.
        Inheritance {
            config: [0; KNOBS],
            explored: 0,
        }
    }
}

/// Search for a better configuration, and record what the search cost.
///
/// Hill climbing from the inherited configuration. Deliberately unsophisticated:
/// the experiment is about whether the *accounting* holds when a lineage
/// compounds, not about the search being clever.
fn search(
    substrate: &mut Substrate,
    start: [i32; KNOBS],
    budget: u32,
    on_trained_only: bool,
) -> ([i32; KNOBS], Inputs) {
    let mut best = start;
    let mut spent = Inputs::default();

    // Evaluate a configuration on the workloads this lineage is allowed to see.
    let evaluate = |substrate: &mut Substrate, config: &[i32; KNOBS], spent: &mut Inputs| {
        let workloads: &[u32] = if on_trained_only { &TRAINED } else { &HELD_OUT };
        let mut total = 0u64;
        for workload in workloads {
            for unit in 0..4u64 {
                let cost = substrate.cost_ns(config, *workload);
                spent.add(&Inputs::silicon(cost));
                total += cost;
                let _ = unit;
            }
        }
        total
    };

    let mut best_cost = evaluate(substrate, &best, &mut spent);
    let mut tried = 0;
    while tried < budget {
        let mut improved = false;
        for knob in 0..KNOBS {
            for delta in [-1i32, 1] {
                if tried >= budget {
                    break;
                }
                let mut candidate = best;
                candidate[knob] = (candidate[knob] + delta).clamp(0, LEVELS - 1);
                if candidate == best {
                    continue;
                }
                tried += 1;
                let cost = evaluate(substrate, &candidate, &mut spent);
                if cost < best_cost {
                    best = candidate;
                    best_cost = cost;
                    improved = true;
                }
            }
        }
        if !improved {
            break;
        }
    }
    (best, spent)
}

/// Run one generation: optionally search, then produce.
fn run_generation(
    lineage: &mut Lineage,
    substrate: &mut Substrate,
    inheritance: Inheritance,
    learning: bool,
    fit_the_benchmark: bool,
) -> Inheritance {
    lineage.begin(
        inheritance.explored,
        if inheritance.explored > 0 { 1 } else { 0 },
    );

    let (config, search_cost) = if learning {
        search(substrate, inheritance.config, SEARCH_BUDGET, true)
    } else {
        // The control spends the same silicon and keeps nothing: it searches
        // and then throws the answer away. Without this it would be a cheaper
        // lineage rather than a comparable one.
        let (_, spent) = search(substrate, inheritance.config, SEARCH_BUDGET, true);
        (inheritance.config, spent)
    };

    // A lineage that fits the benchmark tunes the quirk knob of a trained
    // workload, which cannot transfer.
    let config = if fit_the_benchmark {
        let mut tuned = inheritance.config;
        let workload = TRAINED[0];
        tuned[(workload as usize) % KNOBS] = (workload as i32 * 7) % LEVELS;
        tuned
    } else {
        config
    };

    {
        let generation = lineage.current().expect("a generation");
        generation.ledger.explore(search_cost);

        for (pool, workloads) in [(Pool::Trained, &TRAINED), (Pool::HeldOut, &HELD_OUT)] {
            for unit in 0..UNITS {
                let workload = workloads[(unit as usize) % workloads.len()];
                let (result, cost) = substrate.execute(&config, workload, unit);
                generation.ledger.record(
                    pool,
                    result,
                    checksum(workload, unit),
                    Inputs::silicon(cost),
                );
            }
        }
    }

    Inheritance {
        config,
        explored: inheritance.explored + SEARCH_BUDGET as usize,
    }
}

fn run_lineage(name: &str, seed: u64, generations: u32, learning: bool, fit: bool) -> Lineage {
    let mut lineage = Lineage::new(name, Weights::default());
    let mut substrate = Substrate::new(seed);
    let mut inheritance = Inheritance::default();
    for _ in 0..generations {
        inheritance = run_generation(&mut lineage, &mut substrate, inheritance, learning, fit);
    }
    lineage
}

#[test]
fn a_learning_lineage_raises_held_out_productivity() {
    // The claim: Q(G0) < Q(G1) < Q(G2), on work it never practised on, net of
    // what the search cost.
    let lineage = run_lineage("learned", 0xC0FFEE, 4, true, false);
    let verdict = lineage.verdict();
    assert!(
        verdict.is_improving(),
        "{}\n{}",
        verdict.describe(),
        lineage.render()
    );
}

#[test]
fn the_gain_is_attributable_to_learning_and_not_to_drift() {
    // The only comparison worth quoting. The control spends identical silicon
    // searching and throws the answer away, so anything it also gained was the
    // machine rather than the runtime.
    let learned = run_lineage("learned", 0xC0FFEE, 4, true, false);
    let control = run_lineage("control", 0xC0FFEE, 4, false, false);
    let comparison = Comparison::of(&learned, &control);
    assert!(
        comparison.is_attributable(0.05),
        "{}",
        comparison.describe()
    );
}

#[test]
fn the_control_does_not_improve() {
    // If it did, the substrate would be drifting and every result above would
    // be measuring the drift.
    let control = run_lineage("control", 0xC0FFEE, 4, false, false);
    assert!(
        !control.verdict().is_improving(),
        "the control improved, so the experiment measures drift: {}",
        control.verdict().describe()
    );
}

#[test]
fn a_lineage_that_fits_the_benchmark_is_caught() {
    // The most likely false positive: a descendant tuned to the workloads it
    // practised on and no better at anything else.
    let lineage = run_lineage("overfit", 0xC0FFEE, 3, true, true);
    let verdict = lineage.verdict();
    assert!(!verdict.is_improving(), "{}", verdict.describe());
    assert!(
        matches!(verdict, Verdict::LearnedTheBenchmark { .. })
            || matches!(verdict, Verdict::NotImproving { .. }),
        "{}",
        verdict.describe()
    );
}

#[test]
fn a_descendant_that_skips_the_work_cannot_win() {
    // The anti-fraud property. A runtime returning wrong answers in a tenth of
    // the time must score zero, not ten times better.
    let mut lineage = Lineage::new("cheat", Weights::default());
    lineage.begin(0, 0);
    {
        let generation = lineage.current().expect("a generation");
        for unit in 0..UNITS {
            let workload = HELD_OUT[0];
            generation.ledger.record(
                Pool::HeldOut,
                0,
                checksum(workload, unit),
                Inputs::silicon(50),
            );
        }
    }
    lineage.begin(0, 0);
    {
        let generation = lineage.current().expect("a generation");
        for unit in 0..UNITS {
            let workload = HELD_OUT[0];
            generation.ledger.record(
                Pool::HeldOut,
                0,
                checksum(workload, unit),
                Inputs::silicon(5),
            );
        }
    }
    let verdict = lineage.verdict();
    assert!(matches!(verdict, Verdict::Untrustworthy { .. }));
    assert!(verdict.describe().contains("should be believed"));
}

#[test]
fn the_search_is_charged_and_a_single_generation_may_not_repay_it() {
    // A lineage that searches hard and produces little should show the cost.
    let mut lineage = Lineage::new("expensive", Weights::default());
    let mut substrate = Substrate::new(0xC0FFEE);
    let inheritance = Inheritance::default();

    lineage.begin(0, 0);
    let (config, cost) = search(&mut substrate, inheritance.config, SEARCH_BUDGET, true);
    {
        let generation = lineage.current().expect("a generation");
        generation.ledger.explore(cost);
        // Very little production: too little to amortise the search.
        for unit in 0..20u64 {
            let workload = HELD_OUT[0];
            let (result, unit_cost) = substrate.execute(&config, workload, unit);
            generation.ledger.record(
                Pool::HeldOut,
                result,
                checksum(workload, unit),
                Inputs::silicon(unit_cost),
            );
        }
    }
    let generation = lineage.latest().expect("a generation");
    assert!(
        generation.ledger.exploration().silicon_ns > 0,
        "the search must cost something"
    );

    lineage.begin(0, 1);
    let inherited = lineage.latest().expect("a generation").ledger.inherited();
    assert!(
        inherited.silicon_ns > 0,
        "the descendant must inherit the bill"
    );
}

#[test]
fn the_experiment_is_reproducible() {
    let a = run_lineage("a", 31, 3, true, false);
    let b = run_lineage("b", 31, 3, true, false);
    assert_eq!(a.verdict(), b.verdict());
}

#[test]
fn report_the_lineage() {
    // Printed rather than asserted. Run with `-- --nocapture`.
    for (name, learning, fit) in [
        ("learned", true, false),
        ("control", false, false),
        ("overfit", true, true),
    ] {
        let lineage = run_lineage(name, 0xC0FFEE, 4, learning, fit);
        println!("\n{}", lineage.render());
    }
    let learned = run_lineage("learned", 0xC0FFEE, 4, true, false);
    let control = run_lineage("control", 0xC0FFEE, 4, false, false);
    println!("\n{}", Comparison::of(&learned, &control).describe());
}
