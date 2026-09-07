//! Fixtures shared with the other crates' tests.
//!
//! See [`corescout_substrate::test_support`] for why these are exported rather
//! than hidden behind `cfg(test)`. Nothing here is ever used as a substitute
//! for a real measurement.

use corescout_experiment::benchmarks::stats::Summary;
use corescout_experiment::benchmarks::workloads::{Direction, WorkloadKind};
use corescout_experiment::benchmarks::{BenchmarkResults, Measurement, RunConfig};
use corescout_substrate::topology::{LogicalId, PhysicalId};

use crate::Analysis;

pub fn summary(median: f64, p99: f64) -> Summary {
    Summary {
        count: 30,
        min: median * 0.99,
        max: p99,
        mean: median,
        median,
        p95: p99,
        p99,
        mad: median * 0.01,
        stddev: median * 0.02,
        tail_excess: p99 - median,
    }
}

pub fn measurement(
    core: PhysicalId,
    cpu: LogicalId,
    workload: &str,
    kind: WorkloadKind,
    median_ns: f64,
    p99_ns: f64,
    ops: f64,
) -> Measurement {
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
        ops_per_second: ops / (median_ns / 1e9),
        ns_per_op: median_ns / ops,
        samples_collected: 30,
        samples_used: 30,
        outliers: 0,
        preempted: 0,
    }
}

/// Two cores: core 0 computes faster, core 1 has a much better tail.
/// This is the situation the whole tool exists to surface.
pub fn analysis_fixture() -> Analysis {
    let topology = corescout_substrate::test_support::fake_topology();
    let measurements = vec![
        measurement(
            0,
            0,
            "int-alu",
            WorkloadKind::Compute,
            100_000.0,
            101_000.0,
            1000.0,
        ),
        measurement(
            1,
            1,
            "int-alu",
            WorkloadKind::Compute,
            110_000.0,
            111_000.0,
            1000.0,
        ),
        measurement(
            0,
            0,
            "mem-l1",
            WorkloadKind::Memory,
            50_000.0,
            51_000.0,
            1000.0,
        ),
        measurement(
            1,
            1,
            "mem-l1",
            WorkloadKind::Memory,
            40_000.0,
            41_000.0,
            1000.0,
        ),
        // Core 0 has a 50 us tail; core 1 has a 1 us tail.
        measurement(
            0,
            0,
            "jitter",
            WorkloadKind::Latency,
            4_000.0,
            54_000.0,
            4000.0,
        ),
        measurement(
            1,
            1,
            "jitter",
            WorkloadKind::Latency,
            4_200.0,
            5_200.0,
            4000.0,
        ),
    ];
    let benchmark = BenchmarkResults {
        config: RunConfig::default(),
        clock_overhead_ns: 20.0,
        clock_resolution_ns: 40,
        started_unix: 0,
        duration_seconds: 1.0,
        measurements,
        warnings: vec![],
    };
    Analysis::build(topology, benchmark).unwrap()
}
