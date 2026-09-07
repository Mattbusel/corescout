//! The whole thing, on the machine running this test. No simulation.
//!
//! # What is real here
//!
//! Every number below comes from this machine:
//!
//! | | source |
//! |---|---|
//! | the machine's structure | `GetLogicalProcessorInformationEx` / sysfs |
//! | placements | real `SetThreadGroupAffinity` |
//! | work | real pointer chasing and arithmetic, result consumed |
//! | verification | a checksum the placement chooser never sees |
//! | cost | `QueryThreadCycleTime`, cycles actually executed |
//! | the hidden structure | whatever this silicon actually is |
//!
//! Nothing is planted. On a hybrid part there is a large real effect to find and
//! the machinery is not told about it; on a uniform part there may be nothing,
//! and these tests are written so that "nothing" is a result rather than a
//! failure.
//!
//! # The one thing that is not measured
//!
//! Energy. Neither backend can read package power without privilege that a test
//! suite should not assume, so `Q` here is work per cycle and says nothing about
//! whether a placement is more *efficient* in the sense an operator pays for.

use std::collections::BTreeMap;

use corescout_core::CpuSet;
use corescout_experiment::benchmarks::workloads::Workload;
use corescout_experiment::trial::{self, Trial};
use corescout_lineage::productivity::{Inputs, Pool, Weights};
use corescout_lineage::Lineage;
use corescout_science::causal::{Attribution, Exploration};
use corescout_substrate::platform::{self, Platform};

/// Iterations per trial. Enough that one scheduler tick does not dominate.
const ITERATIONS: u32 = 60;
const WARMUP: u32 = 6;

/// Every single-CPU placement this process may use.
fn placements(platform: &dyn Platform) -> Vec<(String, CpuSet)> {
    let permitted = platform
        .process_affinity()
        .expect("this process may run somewhere");
    permitted
        .iter()
        .map(|cpu| (format!("cpu{cpu}"), [cpu].into_iter().collect()))
        .collect()
}

/// The three workload families, so a conclusion is not about one kind of work.
fn workloads() -> Vec<Box<dyn Workload>> {
    trial::workload_set()
}

/// Run one trial and return it, insisting the answer was right.
fn measure(
    platform: &dyn Platform,
    cpus: &CpuSet,
    workload: &dyn Workload,
    expected: u64,
) -> Trial {
    let trial = trial::run(platform, cpus, workload, ITERATIONS, WARMUP, Some(expected))
        .expect("a placement this process is permitted");
    assert!(
        trial.verified,
        "a trial produced the wrong answer: the measurement is void"
    );
    trial
}

#[test]
fn this_machine_has_placements_that_differ_and_the_difference_is_measurable() {
    // Before anything can be learned there has to be something to learn. On a
    // uniform part this legitimately finds nothing, and says so.
    let platform = platform::detect();
    let places = placements(platform.as_ref());
    if places.len() < 2 {
        eprintln!(
            "only {} placements available; nothing to compare",
            places.len()
        );
        return;
    }
    let workload = &workloads()[0];
    let expected = trial::reference(platform.as_ref(), workload.as_ref(), ITERATIONS, WARMUP)
        .expect("a reference run");

    // Interleaved rounds, not batches: measuring every placement in turn and
    // repeating confounds placement with the machine's thermal state, and
    // whichever went first would win.
    let mut costs: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for _ in 0..5 {
        for (name, cpus) in &places {
            let trial = measure(platform.as_ref(), cpus, workload.as_ref(), expected);
            costs.entry(name.clone()).or_default().push(trial.cost());
        }
    }

    let medians: Vec<(String, f64)> = costs
        .iter()
        .map(|(name, values)| {
            let mut sorted = values.clone();
            sorted.sort_by(|a, b| a.total_cmp(b));
            (name.clone(), sorted[sorted.len() / 2])
        })
        .collect();
    let best = medians
        .iter()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("at least one");
    let worst = medians
        .iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .expect("at least one");

    let spread = (worst.1 - best.1) / best.1;
    println!(
        "\ncheapest placement {} at {:.0} cycles, dearest {} at {:.0}: {:.1}% apart",
        best.0,
        best.1,
        worst.0,
        worst.1,
        spread * 100.0
    );
    assert!(best.1 > 0.0, "a placement cost no cycles, which cannot be");
}

#[test]
fn a_causal_effect_of_placement_is_established_from_randomised_trials_only() {
    // The claim class that needs intervention. Every trial here is assigned by
    // coin flip, so the comparison is between two things the machine was made
    // to do rather than two things it happened to do.
    let platform = platform::detect();
    let places = placements(platform.as_ref());
    if places.len() < 2 {
        return;
    }
    let workload = &workloads()[0];
    let expected = trial::reference(platform.as_ref(), workload.as_ref(), ITERATIONS, WARMUP)
        .expect("a reference run");

    // The two ends of the permitted set. On a hybrid part these are usually a
    // P-core and an E-core, and nothing here knows that.
    let first = places.first().expect("some").clone();
    let last = places.last().expect("some").clone();

    let mut attribution = Attribution::remembering();
    let mut exploration = Exploration::new(1.0, 10, 400, 0xC0FFEE);
    for _ in 0..48 {
        let (take_first, randomised) = exploration.decide(true);
        assert!(
            randomised,
            "every trial in this test is assigned by coin flip"
        );
        let (_, cpus) = if take_first { &first } else { &last };
        let trial = measure(platform.as_ref(), cpus, workload.as_ref(), expected);
        attribution
            .comparison(&first.0, &first.0, &last.0, true)
            .record(take_first, trial.cost(), true);
    }

    let estimate = attribution
        .get(&format!("{}/{}vs{}", first.0, first.0, last.0))
        .expect("the comparison exists");
    println!("\n{}", estimate.describe(10));

    match estimate.effect(10) {
        Ok(effect) => {
            // Whether there *is* an effect is a fact about this machine. That it
            // was measured causally is a fact about the code, and that is what
            // this test asserts.
            assert!(effect.standard_error.is_finite());
            assert!(effect.treatment_trials >= 10 && effect.control_trials >= 10);
            println!("placement effect: {}", effect.describe(true));
        }
        Err(missing) => panic!(
            "48 randomised trials should settle this: {}",
            missing.describe()
        ),
    }
}

#[test]
fn observational_evidence_from_this_machine_still_supports_no_causal_claim() {
    // The rule, on real data. A hundred trials that were all chosen rather than
    // assigned establish nothing, however large the difference between them.
    let platform = platform::detect();
    let places = placements(platform.as_ref());
    if places.len() < 2 {
        return;
    }
    let workload = &workloads()[0];
    let expected = trial::reference(platform.as_ref(), workload.as_ref(), ITERATIONS, WARMUP)
        .expect("a reference run");
    let first = places.first().expect("some").clone();
    let last = places.last().expect("some").clone();

    let mut attribution = Attribution::remembering();
    for round in 0..40 {
        let take_first = round % 2 == 0;
        let (_, cpus) = if take_first { &first } else { &last };
        let trial = measure(platform.as_ref(), cpus, workload.as_ref(), expected);
        // `randomised: false`: chosen, not assigned.
        attribution
            .comparison(&first.0, &first.0, &last.0, true)
            .record(take_first, trial.cost(), false);
    }

    let estimate = attribution
        .get(&format!("{}/{}vs{}", first.0, first.0, last.0))
        .expect("the comparison exists");
    assert!(
        estimate.association().is_some(),
        "there should be an observed difference to be tempted by"
    );
    assert!(
        estimate.effect(10).is_err(),
        "observational trials must not establish a causal effect, however many"
    );
    assert!(!estimate.worth_acting_on(10));
}

#[test]
fn work_that_is_not_done_is_caught_on_real_hardware() {
    // The anti-fraud property, live. A trial checked against the wrong reference
    // must report itself unverified rather than fast.
    let platform = platform::detect();
    let permitted = platform.process_affinity().expect("permitted");
    let workload = &workloads()[0];
    let honest = trial::reference(platform.as_ref(), workload.as_ref(), ITERATIONS, WARMUP)
        .expect("a reference");

    let trial = trial::run(
        platform.as_ref(),
        &permitted,
        workload.as_ref(),
        // Fewer iterations: less work, and a different answer.
        ITERATIONS / 2,
        WARMUP,
        Some(honest),
    )
    .expect("runnable");
    assert!(
        !trial.verified,
        "half the work produced the same checksum, so verification proves nothing"
    );
    assert!(trial.cost() > 0.0, "and it still cost something");
}

#[test]
fn productivity_of_this_machine_is_measured_with_verified_work() {
    // Q on real silicon: verified iterations per cycle, with unverified work
    // worth nothing and still charged.
    let platform = platform::detect();
    let permitted = platform.process_affinity().expect("permitted");
    let workload = &workloads()[0];
    let expected = trial::reference(platform.as_ref(), workload.as_ref(), ITERATIONS, WARMUP)
        .expect("a reference");

    let mut lineage = Lineage::new("this machine", Weights::default());
    lineage.begin(0, 0);
    {
        let generation = lineage.current().expect("a generation");
        for _ in 0..10 {
            let t = measure(platform.as_ref(), &permitted, workload.as_ref(), expected);
            generation.ledger.record(
                Pool::HeldOut,
                if t.verified { 1 } else { 0 },
                1,
                Inputs::silicon(t.cost() as u64),
            );
        }
    }
    let generation = lineage.latest().expect("a generation");
    let q = generation
        .ledger
        .productivity(Pool::HeldOut, &Weights::default())
        .expect("measurable");
    println!("\nQ on this machine: {q:.9} verified trials per cycle");
    assert!(q > 0.0);
    assert!(
        generation.ledger.is_trustworthy(),
        "some work did not verify"
    );
}

#[test]
fn report_the_real_machine() {
    // The headline. Printed rather than asserted, because what this machine
    // turns out to be is a fact about it and not something to require.
    let platform = platform::detect();
    let topology = platform.discover_topology().expect("discoverable");
    let places = placements(platform.as_ref());
    println!(
        "\n{} :: {} physical cores, {} logical, hybrid: {}",
        topology.model_name,
        topology.physical_count(),
        topology.logical_count(),
        topology.hybrid
    );

    let expected: Vec<u64> = workloads()
        .iter()
        .map(|w| {
            trial::reference(platform.as_ref(), w.as_ref(), ITERATIONS, WARMUP)
                .expect("a reference")
        })
        .collect();

    println!(
        "\n{:<8} {:>14} {:>14} {:>14}",
        "cpu", "compute", "cache", "latency"
    );
    // Interleaved rounds so drift hits every placement equally.
    let mut costs: BTreeMap<u32, Vec<Vec<f64>>> = BTreeMap::new();
    for _ in 0..5 {
        for (index, workload) in workloads().iter().enumerate() {
            for (name, cpus) in &places {
                let cpu: u32 = name.trim_start_matches("cpu").parse().unwrap_or(0);
                let t = measure(platform.as_ref(), cpus, workload.as_ref(), expected[index]);
                let entry = costs.entry(cpu).or_insert_with(|| vec![Vec::new(); 3]);
                entry[index].push(t.cost());
            }
        }
    }

    let median = |values: &[f64]| -> f64 {
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.total_cmp(b));
        sorted.get(sorted.len() / 2).copied().unwrap_or(f64::NAN)
    };
    for (cpu, families) in &costs {
        let core = topology.core_of_cpu(*cpu);
        let kind = core.map(|c| c.core_type.short()).unwrap_or("?");
        println!(
            "cpu{:<5} {:>14.0} {:>14.0} {:>14.0}   {}",
            cpu,
            median(&families[0]),
            median(&families[1]),
            median(&families[2]),
            kind
        );
    }
    println!("\ncycles per {ITERATIONS} iterations, median of 5 interleaved rounds");
    println!("the core type column is from the topology and was NOT shown to anything measuring");
}
