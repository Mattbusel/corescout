//! Acting on correlation, on real hardware, where the confounding is physical.
//!
//! # The confounder is a real thing this machine does
//!
//! Simultaneous multithreading. `cpu0` and `cpu1` are the two threads of one
//! physical core and they share its execution resources, so a busy `cpu1` makes
//! `cpu0` genuinely and substantially slower. That is not a model of
//! interference, it is interference.
//!
//! The trap is built from that plus one more real thing: **aliasing**. A sampler
//! whose cadence lines up with a periodic disturbance sees a biased slice of the
//! world. An agent that alternates between two placements while the disturbance
//! alternates on the same period measures one of them only while it is being
//! disturbed and the other only while it is not.
//!
//! ```text
//! trial       1      2      3      4      5      6
//! siblings  BUSY   idle   BUSY   idle   BUSY   idle
//! measured  cpu0   cpu2   cpu0   cpu2   cpu0   cpu2
//!           ^^^^ only while contended  ^^^^ only while not
//! ```
//!
//! Both siblings follow one schedule, so neither core is favoured: each is
//! contended for exactly half the run. The sampler alternates in lockstep, so
//! one core is only ever seen under contention and the other only ever without.
//!
//! The two cores compared are **the same kind**, and the disturbance is
//! symmetric: over a full cycle each is contended for exactly as long. There is
//! no difference between them to find. The sampling manufactures one.
//!
//! That is a stronger trap than narrowing a real gap. A confounder that makes a
//! true difference look smaller is a nuisance; one that invents a difference out
//! of nothing is how a system comes to act confidently on a fact about itself
//! that is not true.
//!
//! # What randomisation does and does not fix
//!
//! Assigning each trial by coin flip breaks the alignment: both placements get
//! measured under both conditions, and the averages come out right. That is the
//! whole claim, and here it is tested against a disturbance made of real
//! contention rather than a term in a formula.
//!
//! # These are ignored by default
//!
//! They spawn real load and burn real CPU. Run with
//! `cargo test -p corescout-integration --test real_credulity -- --ignored --nocapture`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use corescout_core::CpuSet;
use corescout_experiment::benchmarks::workloads::Workload;
use corescout_experiment::trial;
use corescout_science::causal::{Attribution, Exploration};
use corescout_substrate::platform::{self, Platform};
use corescout_substrate::topology::Topology;

const ITERATIONS: u32 = 40;
const WARMUP: u32 = 4;
/// Trials per arm. Enough for the causal estimate to settle.
const TRIALS: usize = 64;

/// Every measurement in this file takes this before touching the machine.
///
/// # Why a mutex and not a flag
///
/// These tests spawn pinned load and then measure how fast a core is. Run two
/// at once and each one's disturbance lands in the other's baseline, which is
/// not a flaky test: it is the apparatus being part of the machine it is
/// measuring. The first run of this file, in parallel, reported SMT contention
/// as 1% and two identical cores as 30% apart, both nonsense.
///
/// `--test-threads=1` would also fix it and can be forgotten. A lock cannot.
static MACHINE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Take the machine, tolerating a previous test having panicked while holding
/// it. A poisoned lock here means an earlier measurement failed, not that this
/// one cannot proceed.
fn exclusive() -> std::sync::MutexGuard<'static, ()> {
    MACHINE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A thread that occupies a CPU on demand.
///
/// Real load: a dependent arithmetic chain that the optimiser cannot remove and
/// that keeps the core's execution units busy. It is pinned, so the contention
/// lands where it is meant to.
struct Interference {
    running: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Interference {
    /// Start a thread pinned to `cpu`, idle until switched on.
    fn on(cpu: u32) -> Interference {
        let running = Arc::new(AtomicBool::new(true));
        let active = Arc::new(AtomicBool::new(false));
        let thread_running = Arc::clone(&running);
        let thread_active = Arc::clone(&active);
        let handle = std::thread::spawn(move || {
            let platform = platform::detect();
            // If it cannot be pinned there is no controlled interference, and a
            // test that carried on regardless would be measuring noise.
            if platform.pin_current_thread(cpu).is_err() {
                return;
            }
            let mut sink = 0u64;
            while thread_running.load(Ordering::Relaxed) {
                if thread_active.load(Ordering::Relaxed) {
                    // A dependent chain: each step needs the previous result, so
                    // it occupies the core rather than filling a pipeline.
                    for i in 0..20_000u64 {
                        sink = sink
                            .wrapping_mul(6364136223846793005)
                            .wrapping_add(i ^ sink);
                    }
                    std::hint::black_box(sink);
                } else {
                    std::thread::yield_now();
                    std::thread::sleep(std::time::Duration::from_micros(200));
                }
            }
            std::hint::black_box(sink);
        });
        Interference {
            running,
            active,
            handle: Some(handle),
        }
    }

    fn set(&self, busy: bool) {
        self.active.store(busy, Ordering::Relaxed);
        // Let the load thread notice before the measurement begins, so the
        // disturbance is actually present for the trial it belongs to.
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
}

impl Drop for Interference {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        self.active.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Two P-cores that are as alike as this machine has, and each one's SMT
/// sibling.
///
/// Two of the same kind on purpose. Comparing a P-core with an E-core would
/// leave a real 2x difference for the disturbance to fight against, and a
/// confounder that merely narrows a true gap is a much weaker thing than one
/// that manufactures a false one.
///
/// `None` when this machine has fewer than two SMT-capable performance cores,
/// in which case the trap cannot be built and the tests say so rather than
/// measuring something else.
fn arrangement(topology: &Topology) -> Option<(u32, u32, u32, u32)> {
    use corescout_substrate::topology::CoreType;

    let mut cores = topology
        .physical_cores
        .iter()
        .filter(|core| core.logical_cpus.len() >= 2)
        .filter(|core| {
            // On a hybrid part, performance cores. On a uniform one, any two.
            !topology.hybrid || core.core_type == CoreType::Performance
        });
    let first = cores.next()?;
    let second = cores.next()?;
    Some((
        first.logical_cpus[0],
        first.logical_cpus[1],
        second.logical_cpus[0],
        second.logical_cpus[1],
    ))
}

fn workload() -> Box<dyn Workload> {
    Box::new(corescout_experiment::benchmarks::workloads::compute::IntegerAlu)
}

/// Measure one placement, insisting the answer was right.
fn measure(platform: &dyn Platform, cpu: u32, expected: u64) -> f64 {
    let cpus: CpuSet = [cpu].into_iter().collect();
    let t = trial::run(
        platform,
        &cpus,
        workload().as_ref(),
        ITERATIONS,
        WARMUP,
        Some(expected),
    )
    .expect("a permitted placement");
    assert!(t.verified, "a trial produced the wrong answer");
    t.cost()
}

/// What the two placements really cost, with nothing disturbing either.
///
/// Not available to either agent. It exists so a test can say what was true
/// rather than what looked true.
fn undisturbed_truth(platform: &dyn Platform, a: u32, b: u32, expected: u64) -> (f64, f64) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    // Interleaved, so drift hits both equally.
    for _ in 0..10 {
        left.push(measure(platform, a, expected));
        right.push(measure(platform, b, expected));
    }
    let median = |mut v: Vec<f64>| {
        v.sort_by(|x, y| x.total_cmp(y));
        v[v.len() / 2]
    };
    (median(left), median(right))
}

#[test]
#[ignore = "spawns real load and burns real CPU; run with --ignored"]
fn smt_contention_is_real_and_large_on_this_machine() {
    let _machine = exclusive();
    // Before a trap can be built from it, the confounder has to exist.
    let platform = platform::detect();
    let topology = platform.discover_topology().expect("discoverable");
    let Some((core, sibling, _, _)) = arrangement(&topology) else {
        eprintln!("this machine has no SMT pair; the trap cannot be built");
        return;
    };
    let expected = trial::reference(platform.as_ref(), workload().as_ref(), ITERATIONS, WARMUP)
        .expect("a reference");

    let load = Interference::on(sibling);
    load.set(false);
    let quiet: Vec<f64> = (0..6)
        .map(|_| measure(platform.as_ref(), core, expected))
        .collect();
    load.set(true);
    let busy: Vec<f64> = (0..6)
        .map(|_| measure(platform.as_ref(), core, expected))
        .collect();
    drop(load);

    let median = |mut v: Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2]
    };
    let quiet = median(quiet);
    let busy = median(busy);
    println!(
        "\ncpu{core} costs {quiet:.0} cycles with its sibling cpu{sibling} idle, \
         {busy:.0} with it busy: {:.0}% slower",
        (busy / quiet - 1.0) * 100.0
    );
    assert!(
        busy > quiet * 1.15,
        "SMT contention moved the cost by less than 15%; there is no confounder here"
    );
}

#[test]
#[ignore = "spawns real load and burns real CPU; run with --ignored"]
fn the_two_placements_are_genuinely_alike() {
    let _machine = exclusive();
    // The trap only means something if there is no real difference to find. If
    // these two cores differ undisturbed, a later "false" conclusion might be
    // true.
    let platform = platform::detect();
    let topology = platform.discover_topology().expect("discoverable");
    let Some((a, _, b, _)) = arrangement(&topology) else {
        return;
    };
    let expected = trial::reference(platform.as_ref(), workload().as_ref(), ITERATIONS, WARMUP)
        .expect("a reference");
    let (left, right) = undisturbed_truth(platform.as_ref(), a, b, expected);
    let gap = (left.max(right) / left.min(right)) - 1.0;
    println!(
        "\nundisturbed: cpu{a} {left:.0}, cpu{b} {right:.0}: {:.1}% apart",
        gap * 100.0
    );
    assert!(
        gap < 0.10,
        "these two cores differ by {:.1}% undisturbed, so they are not interchangeable",
        gap * 100.0
    );
}

#[test]
#[ignore = "spawns real load and burns real CPU; run with --ignored"]
fn an_aliased_sampler_invents_a_difference_that_does_not_exist() {
    let _machine = exclusive();
    // The disturbance is symmetric: over a full cycle both cores are contended
    // for exactly as long. The *sampling* is not: each core is measured only
    // during the half of the cycle when it is the one being disturbed.
    //
    // Nothing about the machine favours either core. The conclusion comes
    // entirely from when the measurements were taken.
    let platform = platform::detect();
    let topology = platform.discover_topology().expect("discoverable");
    let Some((a, a_sibling, b, b_sibling)) = arrangement(&topology) else {
        return;
    };
    let expected = trial::reference(platform.as_ref(), workload().as_ref(), ITERATIONS, WARMUP)
        .expect("a reference");
    let (truth_a, truth_b) = undisturbed_truth(platform.as_ref(), a, b, expected);

    let load_a = Interference::on(a_sibling);
    let load_b = Interference::on(b_sibling);
    let mut costs_a = Vec::new();
    let mut costs_b = Vec::new();
    for round in 0..TRIALS {
        // Symmetric disturbance, aliased sampling: `a` is only ever measured
        // while `a` is contended, and `b` only while `b` is not.
        // Both siblings share one schedule, so neither core is favoured: over
        // the run each is contended for exactly half the time.
        let contended = round % 2 == 0;
        load_a.set(contended);
        load_b.set(contended);
        // The sampler alternates in lockstep with it. `a` is therefore only
        // ever measured while the machine is contended and `b` only ever while
        // it is not. Nothing about the cores differs; only when they were
        // looked at.
        if contended {
            costs_a.push(measure(platform.as_ref(), a, expected));
        } else {
            costs_b.push(measure(platform.as_ref(), b, expected));
        }
    }
    drop(load_a);
    drop(load_b);

    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let observed_a = mean(&costs_a);
    let observed_b = mean(&costs_b);
    println!(
        "\nundisturbed: cpu{a} {truth_a:.0}, cpu{b} {truth_b:.0}\n\
         as sampled:  cpu{a} {observed_a:.0}, cpu{b} {observed_b:.0}"
    );
    println!(
        "the correlation says cpu{b} is {:.0}% cheaper. The machine says they are \
         {:.1}% apart.",
        (observed_a / observed_b - 1.0) * 100.0,
        ((truth_a.max(truth_b) / truth_a.min(truth_b)) - 1.0) * 100.0
    );
    assert!(
        observed_a > observed_b * 1.10,
        "the aliasing did not manufacture a difference: {observed_a:.0} vs {observed_b:.0}"
    );
}

#[test]
#[ignore = "spawns real load and burns real CPU; run with --ignored"]
fn randomised_assignment_refuses_the_invented_difference() {
    let _machine = exclusive();
    // Same machine, same disturbance, same period. The only change is that each
    // trial's arm is decided by a coin flip, so both cores are measured under
    // both halves of the cycle.
    let platform = platform::detect();
    let topology = platform.discover_topology().expect("discoverable");
    let Some((a, a_sibling, b, b_sibling)) = arrangement(&topology) else {
        return;
    };
    let expected = trial::reference(platform.as_ref(), workload().as_ref(), ITERATIONS, WARMUP)
        .expect("a reference");

    let load_a = Interference::on(a_sibling);
    let load_b = Interference::on(b_sibling);
    let mut attribution = Attribution::remembering();
    let mut exploration = Exploration::new(1.0, 12, 1000, 0xC0FFEE);
    for round in 0..TRIALS {
        // The same schedule and the same period as the aliased sampler above.
        let contended = round % 2 == 0;
        load_a.set(contended);
        load_b.set(contended);
        // The only difference: which arm is measured is decided by a coin flip
        // rather than by the round, so arm and phase are no longer locked
        // together and each core is seen under both.
        let (take_a, randomised) = exploration.decide(true);
        assert!(randomised);
        let cpu = if take_a { a } else { b };
        let cost = measure(platform.as_ref(), cpu, expected);
        attribution
            .comparison("smt", "a", "b", true)
            .record(take_a, cost, true);
    }
    drop(load_a);
    drop(load_b);

    let estimate = attribution.get("smt/avsb").expect("the comparison exists");
    println!("\n{}", estimate.describe(12));
    let effect = estimate.effect(12).unwrap_or_else(|e| {
        panic!(
            "{TRIALS} randomised trials should settle this: {}",
            e.describe()
        )
    });
    println!("randomised effect: {}", effect.describe(true));

    // The claim. Two interchangeable cores under a symmetric disturbance have
    // no causal difference, and randomised assignment should decline to invent
    // one. It may still report a small effect, so the bar is that it must not
    // reproduce the large invented one.
    assert!(
        !estimate.worth_acting_on(12)
            || effect.delta.abs() < 0.10 * estimate.treatment.mean().unwrap_or(1.0),
        "randomised assignment reproduced an invented difference of {:.0} cycles",
        effect.delta
    );
}

#[test]
#[ignore = "spawns real load and burns real CPU; run with --ignored"]
fn report_the_real_trap() {
    let _machine = exclusive();
    // Both agents, same machine, same disturbance, side by side.
    let platform = platform::detect();
    let topology = platform.discover_topology().expect("discoverable");
    let Some((a, a_sibling, b, b_sibling)) = arrangement(&topology) else {
        println!("this machine cannot host the trap");
        return;
    };
    let expected = trial::reference(platform.as_ref(), workload().as_ref(), ITERATIONS, WARMUP)
        .expect("a reference");
    let (truth_a, truth_b) = undisturbed_truth(platform.as_ref(), a, b, expected);

    let load_a = Interference::on(a_sibling);
    let load_b = Interference::on(b_sibling);

    // Correlational: alternates, aliased with the disturbance.
    let (mut ca, mut cb) = (Vec::new(), Vec::new());
    for round in 0..TRIALS {
        let contended = round % 2 == 0;
        load_a.set(contended);
        load_b.set(contended);
        if contended {
            ca.push(measure(platform.as_ref(), a, expected));
        } else {
            cb.push(measure(platform.as_ref(), b, expected));
        }
    }

    // Causal: coin flip, same disturbance.
    let mut attribution = Attribution::remembering();
    let mut exploration = Exploration::new(1.0, 12, 1000, 0xC0FFEE);
    for round in 0..TRIALS {
        let contended = round % 2 == 0;
        load_a.set(contended);
        load_b.set(contended);
        let (take_a, _) = exploration.decide(true);
        let cpu = if take_a { a } else { b };
        let cost = measure(platform.as_ref(), cpu, expected);
        attribution
            .comparison("smt", "a", "b", true)
            .record(take_a, cost, true);
    }
    drop(load_a);
    drop(load_b);

    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let estimate = attribution.get("smt/avsb").expect("exists");
    let effect = estimate.effect(12).expect("settled");
    let scale = estimate.treatment.mean().unwrap_or(1.0);

    println!(
        "\n{:<26} {:>14} {:>14}",
        "",
        format!("cpu{a}"),
        format!("cpu{b}")
    );
    println!(
        "{:<26} {:>14.0} {:>14.0}",
        "undisturbed truth", truth_a, truth_b
    );
    println!(
        "{:<26} {:>14.0} {:>14.0}",
        "correlational (aliased)",
        mean(&ca),
        mean(&cb)
    );
    println!(
        "\ncorrelational concludes: cpu{b} is {:.0}% better  <-- INVENTED, they are \
         {:.1}% apart",
        (mean(&ca) / mean(&cb) - 1.0) * 100.0,
        ((truth_a.max(truth_b) / truth_a.min(truth_b)) - 1.0) * 100.0
    );
    println!(
        "causal concludes:        {:.1}% of the mean, {}",
        effect.delta.abs() / scale * 100.0,
        if estimate.worth_acting_on(12) {
            "and calls it real"
        } else {
            "and declines to act on it"
        }
    );
    println!(
        "\nthe disturbance is real SMT contention from threads pinned to cpu{a_sibling}\n\
         and cpu{b_sibling}, alternating. Both cores are contended for exactly as\n\
         long. Nothing here is simulated."
    );
}
