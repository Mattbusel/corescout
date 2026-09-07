//! Tests that require real Linux hardware and real syscalls.
//!
//! Everything here is compiled out on other platforms. These are the tests that
//! cannot be faked: whether a pin actually takes, whether the benchmark engine
//! produces sane numbers on silicon, and whether a launched child really
//! inherits the mask we set.

#![cfg(target_os = "linux")]

use corescout_analysis::Analysis;
use corescout_core::CpuSet;
use corescout_experiment::benchmarks::workloads::{workloads_of_kind, WorkloadKind};
use corescout_experiment::benchmarks::{Progress, RunConfig, Runner};
use corescout_substrate::platform;

/// A configuration small enough to run in a test suite.
fn fast_config() -> RunConfig {
    RunConfig {
        rounds: 2,
        samples_per_visit: 2,
        warmup_iterations: 1,
        ..RunConfig::default()
    }
}

#[test]
fn topology_discovery_matches_the_kernel() {
    let platform = platform::detect();
    let t = platform.discover_topology().expect("discover topology");

    assert!(t.logical_count() >= 1);
    assert!(t.physical_count() >= 1);
    assert!(t.physical_count() <= t.logical_count());
    assert!(!t.online_cpus.is_empty());
    assert!(!t.model_name.is_empty());

    // Every online CPU must belong to exactly one physical core.
    for cpu in t.logical_cpus.iter().filter(|c| c.online) {
        let core = t.core_of_cpu(cpu.id).expect("every online CPU has a core");
        assert!(core.logical_cpus.contains(&cpu.id));
    }

    // SMT sibling relationships must be symmetric, or the pinning advice is
    // wrong for one of the two threads.
    for cpu in &t.logical_cpus {
        for sibling in &cpu.smt_siblings {
            let other = t.cpu(*sibling).expect("sibling exists");
            assert!(
                other.smt_siblings.contains(&cpu.id),
                "CPU {} lists {} as a sibling but not the reverse",
                cpu.id,
                sibling
            );
        }
    }
}

#[test]
fn pinning_actually_restricts_the_thread() {
    let platform = platform::detect();
    let original = platform.current_thread_affinity().expect("read affinity");
    let target = original.iter().next().expect("at least one usable CPU");

    platform.pin_current_thread(target).expect("pin");
    let after = platform.current_thread_affinity().expect("read affinity");
    assert_eq!(
        after.to_vec(),
        vec![target],
        "the kernel did not honour the pin"
    );

    platform
        .set_current_thread_affinity(&original)
        .expect("restore affinity");
    assert_eq!(
        platform.current_thread_affinity().unwrap(),
        original,
        "affinity was not restored"
    );
}

#[test]
fn benchmarking_produces_plausible_numbers_and_restores_affinity() {
    let platform = platform::detect();
    let topology = platform.discover_topology().expect("topology");
    let before = platform.current_thread_affinity().expect("affinity");

    // One compute workload only, to keep the test quick.
    let workloads = workloads_of_kind(WorkloadKind::Compute);
    let runner = Runner::new(platform.as_ref(), fast_config());
    let results = runner
        .run(&topology, &workloads[..1], &mut |_: Progress<'_>| {})
        .expect("benchmark");

    assert!(!results.measurements.is_empty());
    for m in &results.measurements {
        assert!(m.clean.median > 0.0, "a zero median means nothing ran");
        assert!(m.ops_per_second > 0.0);
        assert!(
            m.clean.min <= m.clean.median && m.clean.median <= m.clean.max,
            "summary statistics are out of order"
        );
        assert!(m.raw.p99 >= m.clean.median, "p99 below the median");
        assert!(m.samples_collected > 0);
        // The measured CPU must be one we were allowed to use.
        assert!(topology.process_affinity.contains(&m.cpu));
    }

    // Sanity: cores of one machine should not differ by more than about 5x on
    // a compute workload. A larger spread means we measured something else,
    // such as a core that was throttled to a stop or an E-core cluster on a
    // machine we failed to detect as hybrid.
    if results.measurements.len() > 1 && !topology.hybrid {
        let rates: Vec<f64> = results
            .measurements
            .iter()
            .map(|m| m.ops_per_second)
            .collect();
        let max = rates.iter().copied().fold(f64::MIN, f64::max);
        let min = rates.iter().copied().fold(f64::MAX, f64::min);
        assert!(max / min < 5.0, "implausible spread: {min} to {max} ops/s");
    }

    assert_eq!(
        platform.current_thread_affinity().unwrap(),
        before,
        "the runner must restore the caller's affinity"
    );
}

#[test]
fn a_full_analysis_ranks_every_measured_core() {
    let platform = platform::detect();
    let topology = platform.discover_topology().expect("topology");
    let expected_cores = topology.benchmarkable_cores().len();

    let workloads = corescout_experiment::benchmarks::workloads::default_workloads();
    // Compute plus latency only: the DRAM chase allocates 64 MiB per visit and
    // is more than a smoke test needs.
    let subset: Vec<_> = workloads
        .into_iter()
        .filter(|w| w.spec().id == "int-alu" || w.spec().id == "jitter")
        .collect();

    let runner = Runner::new(platform.as_ref(), fast_config());
    let results = runner
        .run(&topology, &subset, &mut |_: Progress<'_>| {})
        .expect("benchmark");
    let analysis = Analysis::build(topology, results).expect("analysis");

    assert_eq!(analysis.cores.len(), expected_cores);
    for core in &analysis.cores {
        assert!(core.compute.is_some());
        assert!(core.jitter.is_some());
        assert!((0.0..=100.0).contains(&core.compute.unwrap()));
        assert!((0.0..=100.0).contains(&core.stability));
    }

    // Exactly one core scores 100 in each ranking, by construction.
    for ranking in &analysis.rankings {
        assert!(!ranking.entries.is_empty());
        assert!((ranking.entries[0].score - 100.0).abs() < 1e-6);
        // Entries must be in descending score order.
        for pair in ranking.entries.windows(2) {
            assert!(pair[0].score >= pair[1].score, "{} is unsorted", ranking.id);
        }
    }

    assert!(analysis.recommendations.latency_critical.is_some());
    assert!(analysis.recommendations.compute_heavy.is_some());
}

#[test]
fn a_child_process_inherits_the_mask_it_was_given() {
    let platform = platform::detect();
    let topology = platform.discover_topology().expect("topology");
    let cpu = *topology
        .process_affinity
        .first()
        .expect("at least one usable CPU");

    let mut cpus = CpuSet::new();
    cpus.insert(cpu);

    // `taskset -p` would need the package installed; reading the child's own
    // status file needs nothing. Cpus_allowed_list is the kernel's own report
    // of the mask actually in force.
    let mut command = std::process::Command::new("/bin/sh");
    command
        .arg("-c")
        .arg("grep Cpus_allowed_list /proc/self/status")
        .stdout(std::process::Stdio::piped());

    let child = platform
        .spawn_with_affinity(&mut command, &cpus)
        .expect("spawn");
    let output = child.wait_with_output().expect("wait");
    let text = String::from_utf8_lossy(&output.stdout);
    let reported = text
        .split(':')
        .nth(1)
        .expect("Cpus_allowed_list line")
        .trim();

    assert_eq!(
        reported,
        cpu.to_string(),
        "the child was not confined to CPU {cpu}"
    );
}
