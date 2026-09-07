//! Does a lineage make this machine do more real work per real cycle?
//!
//! # Everything here is measured on the machine running the test
//!
//! No planted structure, no simulated cost function, no synthetic substrate.
//! Generations search real placements by running real work on them, pay real
//! cycles to do it, and are scored on real verified work under the placement
//! they inherited.
//!
//! The structure they are looking for is whatever this silicon is. On the
//! machine this was developed on that is a hybrid part whose P-cores are about
//! twice as fast as its E-cores, and nothing in the lineage is told so. On a
//! uniform part there is nothing to find, and the harness reports
//! `NotImproving`, which is the correct answer there.
//!
//! # Held out means held out
//!
//! Generations search against the compute and cache workloads. They are scored
//! on the latency workload, which they never search against and which stresses
//! the machine differently. A lineage that tuned itself to the search workloads
//! and no others improves on `Trained` and not on `HeldOut`, and the verdict
//! says so.
//!
//! # These are ignored by default
//!
//! Each run spends minutes of real CPU doing real work, which is the point and
//! is also too slow for a suite anyone runs on every change. Run them with
//! `cargo test -p corescout-integration --test real_lineage -- --ignored
//! --nocapture`.
//!
//! # The control is the result
//!
//! A second lineage spends identical cycles searching and throws the answer
//! away. Anything it also gains is the machine drifting: a core warming up, a
//! background process finishing, the room. Only the difference is attributable.

use corescout_core::CpuSet;
use corescout_experiment::benchmarks::workloads::Workload;
use corescout_experiment::trial;
use corescout_lineage::productivity::{Inputs, Pool, Weights};
use corescout_lineage::{Comparison, Lineage, Verdict};
use corescout_substrate::platform::{self, Platform};

const ITERATIONS: u32 = 40;
const WARMUP: u32 = 4;
/// Placements a generation may try. Small, so no single generation can sweep
/// the machine and the line has somewhere to go.
const SEARCH_WIDTH: usize = 3;
/// Work units per generation, counted separately per pool.
///
/// The two pools are not the same size because their units are not the same
/// price: a compute or cache unit costs about ninety times a latency one on
/// this machine. Producing equal counts would spend almost all the time on the
/// trained pool and leave the held-out pool too small to amortise anything,
/// which lets the arithmetic rather than the machine decide the answer.
const TRAINED_UNITS: u32 = 30;
const HELD_OUT_UNITS: u32 = 1200;
const GENERATIONS: u32 = 4;

/// What a generation inherits: where to run, and how much searching it cost to
/// find out.
#[derive(Clone, Debug)]
struct Inheritance {
    placement: CpuSet,
    /// Placements the ancestors already tried, so a descendant starts where its
    /// parent stopped rather than repeating its work.
    tried: Vec<u32>,
}

fn workloads() -> Vec<Box<dyn Workload>> {
    trial::workload_set()
}

/// The workloads a lineage may search against, and the one it may not.
const TRAINED: [usize; 2] = [0, 1];
const HELD_OUT: usize = 2;

/// Try a few placements and keep the best, charging every trial.
///
/// The search is deliberately simple: take the next few untried CPUs, measure
/// each on the trained workloads, keep the cheapest. The experiment is about
/// whether the accounting holds across generations, not about the search being
/// clever.
fn search(
    platform: &dyn Platform,
    permitted: &CpuSet,
    inherited: &Inheritance,
    references: &[u64],
    workloads: &[Box<dyn Workload>],
) -> (CpuSet, Vec<u32>, Inputs) {
    let mut spent = Inputs::default();
    let mut best = inherited.placement.clone();
    let mut tried = inherited.tried.clone();

    // Cost of what we already have, so a candidate has something to beat.
    let mut best_cost = 0.0;
    for index in TRAINED {
        let t = trial::run(
            platform,
            &best,
            workloads[index].as_ref(),
            ITERATIONS,
            WARMUP,
            Some(references[index]),
        )
        .expect("a permitted placement");
        assert!(
            t.verified,
            "the incumbent placement produced a wrong answer"
        );
        spent.add(&Inputs::silicon(t.cost() as u64));
        best_cost += t.cost();
    }

    // Spread across the permitted set rather than taking the first few in
    // order. In order means the early generations only ever see one end of the
    // machine, and on a hybrid part that is one kind of core: a negative result
    // would then be an artefact of where the search looked.
    let untried: Vec<u32> = permitted
        .iter()
        .filter(|cpu| !tried.contains(cpu))
        .collect();
    let stride = (untried.len() / SEARCH_WIDTH).max(1);
    let candidates: Vec<u32> = untried
        .iter()
        .step_by(stride)
        .copied()
        .take(SEARCH_WIDTH)
        .collect();

    for cpu in candidates {
        tried.push(cpu);
        let candidate: CpuSet = [cpu].into_iter().collect();
        let mut cost = 0.0;
        for index in TRAINED {
            let t = trial::run(
                platform,
                &candidate,
                workloads[index].as_ref(),
                ITERATIONS,
                WARMUP,
                Some(references[index]),
            )
            .expect("a permitted placement");
            assert!(t.verified, "a candidate placement produced a wrong answer");
            spent.add(&Inputs::silicon(t.cost() as u64));
            cost += t.cost();
        }
        if cost < best_cost {
            best_cost = cost;
            best = candidate;
        }
    }
    (best, tried, spent)
}

/// Run one generation: search (or pretend to), then produce verified work.
fn generation(
    lineage: &mut Lineage,
    platform: &dyn Platform,
    permitted: &CpuSet,
    inherited: Inheritance,
    references: &[u64],
    learning: bool,
) -> Inheritance {
    lineage.begin(inherited.tried.len(), 0);
    let workloads = workloads();

    let (placement, tried, spent) = search(platform, permitted, &inherited, references, &workloads);
    // The control searches identically and keeps nothing, so it pays the same
    // cycles. Without that it would be a cheaper lineage rather than a
    // comparable one.
    let (placement, tried) = if learning {
        (placement, tried)
    } else {
        (inherited.placement.clone(), tried)
    };

    {
        let ledger = &mut lineage.current().expect("a generation").ledger;
        ledger.explore(spent);

        // Trained pool: the workloads it was allowed to search against.
        for _ in 0..TRAINED_UNITS {
            for index in TRAINED {
                let t = trial::run(
                    platform,
                    &placement,
                    workloads[index].as_ref(),
                    ITERATIONS,
                    WARMUP,
                    Some(references[index]),
                )
                .expect("a permitted placement");
                ledger.record(
                    Pool::Trained,
                    if t.verified { 1 } else { 0 },
                    1,
                    Inputs::silicon(t.cost() as u64),
                );
            }
        }
        // Held out: never searched against.
        for _ in 0..HELD_OUT_UNITS {
            let t = trial::run(
                platform,
                &placement,
                workloads[HELD_OUT].as_ref(),
                ITERATIONS,
                WARMUP,
                Some(references[HELD_OUT]),
            )
            .expect("a permitted placement");
            ledger.record(
                Pool::HeldOut,
                if t.verified { 1 } else { 0 },
                1,
                Inputs::silicon(t.cost() as u64),
            );
        }
    }

    Inheritance { placement, tried }
}

fn run_lineage(name: &str, learning: bool) -> Lineage {
    let platform = platform::detect();
    let permitted = platform
        .process_affinity()
        .expect("this process may run somewhere");
    let workloads = workloads();
    let references: Vec<u64> = workloads
        .iter()
        .map(|w| {
            trial::reference(platform.as_ref(), w.as_ref(), ITERATIONS, WARMUP)
                .expect("a reference run")
        })
        .collect();

    let mut lineage = Lineage::new(name, Weights::default());
    // G0 starts where a fresh machine starts: no affinity of its own, running
    // wherever the process is permitted, which is what the OS scheduler does.
    let mut inherited = Inheritance {
        placement: permitted.clone(),
        tried: Vec::new(),
    };
    for _ in 0..GENERATIONS {
        inherited = generation(
            &mut lineage,
            platform.as_ref(),
            &permitted,
            inherited,
            &references,
            learning,
        );
    }
    lineage
}

#[test]
#[ignore = "burns a minute of real CPU; run with --ignored"]
fn a_lineage_on_this_machine_is_measured_honestly() {
    // Whatever the answer, the accounting has to hold: verified work only, the
    // search charged to whoever inherits it, held-out work kept separate.
    let lineage = run_lineage("learned", true);
    assert_eq!(lineage.len(), GENERATIONS as usize);
    for generation in lineage.generations() {
        assert!(
            generation.ledger.is_trustworthy(),
            "{} produced work that did not verify",
            generation.name()
        );
        assert!(
            generation.ledger.work(Pool::HeldOut).verified > 0,
            "{} did no held-out work",
            generation.name()
        );
    }
    // Every generation after the first inherits a bill.
    for generation in lineage.generations().iter().skip(1) {
        assert!(
            generation.ledger.inherited().silicon_ns > 0,
            "{} inherited no search cost",
            generation.name()
        );
    }
}

#[test]
#[ignore = "burns two minutes of real CPU; run with --ignored"]
fn the_control_pays_the_same_cycles_and_keeps_nothing() {
    // Without this the comparison is between a lineage that searched and one
    // that did not, which measures the cost of searching rather than the value
    // of what it found.
    let learned = run_lineage("learned", true);
    let control = run_lineage("control", false);

    let spent = |lineage: &Lineage| -> u64 {
        lineage
            .generations()
            .iter()
            .map(|g| g.ledger.exploration().silicon_ns)
            .sum()
    };
    let a = spent(&learned) as f64;
    let b = spent(&control) as f64;
    let ratio = a.max(b) / a.min(b);
    assert!(
        ratio < 1.5,
        "the two lines spent {a:.0} and {b:.0} cycles searching, which is not comparable"
    );
}

#[test]
#[ignore = "burns two minutes of real CPU; run with --ignored -- --nocapture"]
fn report_the_real_lineage() {
    // The headline, printed rather than asserted. Whether this machine has an
    // improvement to find is a fact about it.
    let learned = run_lineage("learned", true);
    let control = run_lineage("control", false);

    for lineage in [&learned, &control] {
        println!("\n{}", lineage.render());
    }
    let comparison = Comparison::of(&learned, &control);
    println!("\n{}", comparison.describe());

    match learned.verdict() {
        Verdict::Improving { gain, .. } => {
            println!(
                "\nthis machine's lineage improved held-out productivity by {:.1}%",
                gain * 100.0
            );
        }
        other => {
            println!("\nno improvement on this machine: {}", other.describe());
        }
    }
}
