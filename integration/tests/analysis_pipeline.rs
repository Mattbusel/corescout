//! End-to-end test of the analysis pipeline with synthetic measurements.
//!
//! Topology parsing, scoring, ranking, recommendation, caching and rendering
//! are exercised together, with the benchmark engine replaced by hand-built
//! measurements. That substitution is what makes the test deterministic: real
//! timings on a CI box would make any assertion about *which* core wins a
//! coin flip.
//!
//! The scenario is the one CoreScout exists for. The firmware's preferred core
//! is not the fastest core, and the fastest core is not the quietest one.

mod common;

use common::FakeMachine;
use corescout_analysis::placement::{self as launch, Profile};
use corescout_analysis::Analysis;
use corescout_experiment::benchmarks::stats::Summary;
use corescout_experiment::benchmarks::workloads::{Direction, WorkloadKind};
use corescout_experiment::benchmarks::{BenchmarkResults, Measurement, RunConfig};
use corescout_substrate::platform::linux::sysfs::Sysfs;
use corescout_substrate::topology::{LogicalId, PhysicalId, Topology};

fn summary(median: f64, p99: f64) -> Summary {
    Summary {
        count: 30,
        min: median * 0.98,
        max: p99,
        mean: median,
        median,
        p95: median + (p99 - median) * 0.5,
        p99,
        mad: median * 0.01,
        stddev: median * 0.03,
        tail_excess: p99 - median,
    }
}

fn measurement(
    core: PhysicalId,
    cpu: LogicalId,
    workload: &str,
    kind: WorkloadKind,
    median_ns: f64,
    p99_ns: f64,
) -> Measurement {
    const OPS: f64 = 1_000.0;
    Measurement {
        core,
        cpu,
        workload: workload.to_string(),
        kind,
        direction: if kind == WorkloadKind::Latency {
            Direction::LowerIsBetter
        } else {
            Direction::HigherIsBetter
        },
        clean: summary(median_ns, p99_ns),
        raw: summary(median_ns, p99_ns),
        ops_per_second: OPS / (median_ns / 1e9),
        ns_per_op: median_ns / OPS,
        samples_collected: 30,
        samples_used: 30,
        outliers: 0,
        preempted: 0,
    }
}

/// Build the scenario:
///
/// - core 1 is the fastest at compute (and the firmware ranks it only second),
/// - core 2 is the firmware's favourite but merely average,
/// - core 3 has by far the lowest tail latency,
/// - core 0 is slow and noisy, as CPU 0 so often is.
fn scenario() -> (common::TempTree, Topology, BenchmarkResults) {
    let tree = FakeMachine::typical().write("pipeline");
    let topology = Sysfs::with_roots(tree.sys(), tree.proc())
        .read_topology(None)
        .expect("fixture topology");

    // (core, cpu, compute median ns, jitter median ns, jitter p99 ns)
    let profile = [
        (0u32, 0u32, 120_000.0, 4_000.0, 60_000.0),
        (1, 1, 100_000.0, 4_100.0, 12_000.0),
        (2, 2, 112_000.0, 4_050.0, 20_000.0),
        (3, 3, 118_000.0, 4_300.0, 5_000.0),
    ];

    let mut measurements = Vec::new();
    for (core, cpu, compute_ns, jitter_ns, jitter_p99) in profile {
        measurements.push(measurement(
            core,
            cpu,
            "int-alu",
            WorkloadKind::Compute,
            compute_ns,
            compute_ns * 1.02,
        ));
        measurements.push(measurement(
            core,
            cpu,
            "mem-l1",
            WorkloadKind::Memory,
            50_000.0 + core as f64 * 500.0,
            51_000.0,
        ));
        measurements.push(measurement(
            core,
            cpu,
            "jitter",
            WorkloadKind::Latency,
            jitter_ns,
            jitter_p99,
        ));
    }

    let results = BenchmarkResults {
        config: RunConfig::default(),
        clock_overhead_ns: 22.0,
        clock_resolution_ns: 40,
        started_unix: 1_700_000_000,
        duration_seconds: 31.4,
        measurements,
        warnings: vec!["SMT is enabled".to_string()],
    };
    (tree, topology, results)
}

#[test]
fn the_fastest_core_and_the_quietest_core_are_ranked_separately() {
    let (_tree, topology, results) = scenario();
    let analysis = Analysis::build(topology, results).expect("analysis");

    let compute = analysis.ranking("single_thread_compute").expect("ranking");
    assert_eq!(compute.entries[0].core, 1, "core 1 is the fastest");
    assert!((compute.entries[0].score - 100.0).abs() < 1e-9);

    let jitter = analysis.ranking("lowest_jitter").expect("ranking");
    assert_eq!(jitter.entries[0].core, 3, "core 3 has the smallest tail");

    // The two must not be the same core, or the fixture is not testing the
    // thing this project is about.
    assert_ne!(compute.entries[0].core, jitter.entries[0].core);
}

#[test]
fn recommendations_follow_the_rankings_they_are_built_from() {
    let (_tree, topology, results) = scenario();
    let analysis = Analysis::build(topology, results).expect("analysis");
    let recs = &analysis.recommendations;

    assert_eq!(recs.compute_heavy.as_ref().unwrap().core, 1);
    assert_eq!(recs.latency_critical.as_ref().unwrap().core, 3);
    // Every recommendation must be able to justify itself.
    for rec in [
        &recs.compute_heavy,
        &recs.latency_critical,
        &recs.memory_heavy,
    ] {
        assert!(!rec.as_ref().unwrap().reasons.is_empty());
    }
}

#[test]
fn corescout_reports_when_it_disagrees_with_the_firmware() {
    let (_tree, topology, results) = scenario();
    let analysis = Analysis::build(topology, results).expect("analysis");

    // The fixture's firmware favourite is core 2; measurement says core 1.
    assert_eq!(analysis.topology.firmware_favored_cpu().unwrap().id, 2);
    assert!(
        analysis
            .observations
            .iter()
            .any(|o| o.contains("preferred core") && o.contains("measured faster")),
        "the disagreement should be stated plainly: {:?}",
        analysis.observations
    );
}

#[test]
fn the_worker_pair_avoids_smt_siblings_and_crossing_numa() {
    let (_tree, topology, results) = scenario();
    let analysis = Analysis::build(topology, results).expect("analysis");
    let pair = analysis.recommendations.worker_pair.as_ref().expect("pair");

    assert_ne!(pair.cores[0], pair.cores[1]);
    // The two chosen CPUs must not be threads of one physical core.
    let a = analysis.topology.cpu(pair.cpus[0]).unwrap();
    assert!(
        !a.smt_siblings.contains(&pair.cpus[1]),
        "the pair picked two SMT siblings, which is one core wearing a disguise"
    );
    // Cores 2 and 3 win, not 0 and 1, and the reason is the rule the pair
    // scorer is built on: a pair runs at the speed of its *weaker* half. Core 1
    // is the fastest core in the machine, but its best same-L3 partner is the
    // slowest core (0), while cores 2 and 3 are both merely good. Pairing the
    // champion with a laggard is worse than pairing two solid cores.
    assert_eq!(pair.cores, [2, 3]);

    // Both halves must be on one NUMA node and one L3, or the locality rules
    // did not fire.
    let a = analysis.topology.cpu(pair.cpus[0]).unwrap();
    let b = analysis.topology.cpu(pair.cpus[1]).unwrap();
    assert_eq!(a.numa_node, b.numa_node);
    assert!(analysis
        .topology
        .shares_cache(pair.cpus[0], pair.cpus[1], 3));

    // And the weaker half of the chosen pair really is stronger than the
    // weaker half of the obvious "just take the two fastest" answer.
    let score = |core| analysis.core(core).unwrap().compute.unwrap();
    let chosen_weakest = score(pair.cores[0]).min(score(pair.cores[1]));
    let naive_weakest = score(1).min(score(0));
    assert!(chosen_weakest > naive_weakest);
}

#[test]
fn the_launcher_turns_a_profile_into_a_mask_and_an_explanation() {
    let (_tree, topology, results) = scenario();
    let analysis = Analysis::build(topology, results).expect("analysis");

    let selection = launch::select(&analysis, Profile::Latency).expect("selection");
    assert_eq!(selection.cpus.to_vec(), vec![3]);

    let text = launch::explain(&selection, Profile::Latency, "./trading_engine");
    assert!(text.contains("./trading_engine"));
    assert!(text.contains("CPU 3"));
    assert!(text.contains("profile: latency"));

    let pair = launch::select(&analysis, Profile::Pair).expect("pair selection");
    assert_eq!(pair.cpus.to_list(), "2-3");
    assert_eq!(pair.cpus.len(), 2);
}

#[test]
fn a_stale_profile_from_a_different_machine_is_not_used() {
    let (_tree, topology, results) = scenario();
    let analysis = Analysis::build(topology, results).expect("analysis");

    let this = corescout_analysis::profile_cache::fingerprint(&analysis.topology);

    // Same machine, different CPU brought offline: the profile must not match.
    let mut changed = analysis.topology.clone();
    changed.online_cpus.pop();
    assert_ne!(
        this,
        corescout_analysis::profile_cache::fingerprint(&changed)
    );

    // A different machine entirely.
    let tree = FakeMachine::hybrid().write("other-machine");
    let other = Sysfs::with_roots(tree.sys(), tree.proc())
        .read_topology(None)
        .unwrap();
    assert_ne!(this, corescout_analysis::profile_cache::fingerprint(&other));
}

#[test]
fn human_and_json_output_describe_the_same_analysis() {
    let (_tree, topology, results) = scenario();
    let analysis = Analysis::build(topology, results).expect("analysis");

    let text = corescout_report::human_debug::analysis(&analysis);
    assert!(text.contains("Single-thread compute"));
    assert!(text.contains("Latency-critical thread"));
    // Caveats from the benchmark must reach the reader, not be dropped.
    assert!(text.contains("SMT is enabled"));

    let json = corescout_report::json::analysis(&analysis).expect("json");
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert_eq!(
        value["recommendations"]["latency_critical"]["cpu"],
        analysis
            .recommendations
            .latency_critical
            .as_ref()
            .unwrap()
            .cpu
    );
    assert_eq!(
        value["benchmark"]["warnings"][0].as_str(),
        Some("SMT is enabled")
    );
}
