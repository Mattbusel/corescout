//! Integration tests for Linux topology discovery.
//!
//! These drive the production sysfs parser against synthetic `/sys` trees, so
//! they cover the real code path that runs on a Linux machine while remaining
//! runnable on any host. Anything the parser gets wrong here, it gets wrong on
//! hardware.

mod common;

use common::FakeMachine;
use corescout_substrate::platform::linux::sysfs::Sysfs;
use corescout_substrate::topology::{CacheKind, CoreType, Topology};

fn parse(machine: &FakeMachine, tag: &str) -> (common::TempTree, Topology) {
    let tree = machine.write(tag);
    let sysfs = Sysfs::with_roots(tree.sys(), tree.proc());
    let topology = sysfs
        .read_topology(None)
        .expect("topology should parse from a well-formed sysfs tree");
    (tree, topology)
}

#[test]
fn identifies_the_cpu() {
    let (_tree, t) = parse(&FakeMachine::typical(), "identity");
    assert_eq!(t.model_name, "Synthetic Core Ultra 9 000X");
    assert_eq!(t.vendor, "GenuineTest");
}

#[test]
fn groups_logical_cpus_into_physical_cores() {
    let (_tree, t) = parse(&FakeMachine::typical(), "cores");
    // All 8 CPUs are *present*; CPU 7 is merely offline, and is listed as
    // such rather than vanishing, because a user asking `corescout info` wants
    // to know it exists and could be brought back.
    assert_eq!(t.logical_count(), 8);
    assert!(!t.cpu(7).unwrap().online);
    assert!(t.logical_cpus.iter().filter(|c| c.online).count() == 7);
    // Physical cores, by contrast, only count cores we can actually schedule
    // on, and core 3 still exists because CPU 3 is online.
    assert_eq!(t.physical_count(), 4);
    assert!(t.smt_enabled());
    assert_eq!(t.smt_width(), 2);
}

#[test]
fn detects_smt_siblings_across_the_cpu_numbering_gap() {
    // The trap this exists to catch: CPUs 0 and 4 look unrelated by number and
    // are the same physical core.
    let (_tree, t) = parse(&FakeMachine::typical(), "smt");
    let cpu0 = t.cpu(0).expect("cpu 0");
    assert_eq!(cpu0.smt_siblings, vec![4]);
    assert!(cpu0.is_smt());

    let core = t.core_of_cpu(4).expect("core of cpu 4");
    assert_eq!(core.logical_cpus, vec![0, 4]);
    assert_eq!(t.core_of_cpu(0).unwrap().id, core.id);
}

#[test]
fn an_offlined_cpu_leaves_its_core_with_one_thread() {
    let (_tree, t) = parse(&FakeMachine::typical(), "offline");
    assert_eq!(t.offline_cpus, vec![7]);
    assert!(!t.online_cpus.contains(&7));

    let core = t.core_of_cpu(3).expect("core of cpu 3");
    assert_eq!(
        core.logical_cpus,
        vec![3],
        "an offline sibling must not be presented as usable"
    );
    // And the offlined CPU must never be a benchmark target.
    assert!(t
        .benchmarkable_cores()
        .iter()
        .all(|c| !c.logical_cpus.contains(&7)));
}

#[test]
fn deduplicates_caches_and_records_sharing() {
    let (_tree, t) = parse(&FakeMachine::typical(), "caches");

    // sysfs lists each cache once per participating CPU; the parser must
    // collapse those into one instance per real cache.
    let l3s: Vec<_> = t.caches.iter().filter(|c| c.level == 3).collect();
    assert_eq!(l3s.len(), 2, "two L3 instances, one per NUMA node");
    assert_eq!(l3s[0].kind, CacheKind::L3Unified);
    assert_eq!(l3s[0].size_bytes, Some(16 * 1024 * 1024));

    let l1d: Vec<_> = t
        .caches
        .iter()
        .filter(|c| c.kind == CacheKind::L1Data)
        .collect();
    assert_eq!(l1d.len(), 4, "one L1d per physical core");

    // SMT siblings share every level; separate cores share only the L3.
    assert!(t.shares_cache(0, 4, 1));
    assert!(t.shares_cache(0, 4, 2));
    assert!(!t.shares_cache(0, 1, 2));
    assert!(t.shares_cache(0, 1, 3));
    // Across NUMA nodes, not even the L3 is shared.
    assert!(!t.shares_cache(0, 2, 3));
}

#[test]
fn maps_numa_nodes_and_their_memory() {
    let (_tree, t) = parse(&FakeMachine::typical(), "numa");
    assert_eq!(t.numa_nodes.len(), 2);
    assert_eq!(t.numa_nodes[0].cpus, vec![0, 1, 4, 5]);
    assert_eq!(t.numa_nodes[0].memory_kb, Some(16_000_000));
    assert_eq!(t.cpu(2).unwrap().numa_node, Some(1));
    assert_eq!(t.cpu(0).unwrap().numa_node, Some(0));
}

#[test]
fn reads_frequencies_without_ranking_by_them() {
    let (_tree, t) = parse(&FakeMachine::typical(), "freq");
    let cpu1 = t.cpu(1).unwrap();
    assert_eq!(cpu1.frequency.current_khz, Some(4_800_000));
    assert_eq!(cpu1.frequency.min_khz, Some(550_000));
    assert_eq!(cpu1.frequency.max_khz, Some(5_200_000));
}

#[test]
fn ranks_the_firmware_preferred_cores() {
    let (_tree, t) = parse(&FakeMachine::typical(), "cppc");
    let favourite = t.firmware_favored_cpu().expect("a preferred core");
    // Core 2 has the highest highest_perf, and CPU 2 is its lower-numbered
    // thread.
    assert_eq!(favourite.id, 2);
    let hint = favourite.favored.as_ref().unwrap();
    assert_eq!(hint.highest_perf, 255);
    assert_eq!(hint.rank, 1);
    assert_eq!(hint.source, "acpi_cppc/highest_perf");

    // Ranks are dense over distinct perf values, and siblings share a rank.
    assert_eq!(t.cpu(6).unwrap().favored.as_ref().unwrap().rank, 1);
    assert_eq!(t.cpu(1).unwrap().favored.as_ref().unwrap().rank, 2);
    assert_eq!(t.cpu(0).unwrap().favored.as_ref().unwrap().rank, 3);
}

#[test]
fn process_affinity_defaults_to_the_online_set_and_can_be_overridden() {
    let machine = FakeMachine::typical();
    let tree = machine.write("affinity");
    let sysfs = Sysfs::with_roots(tree.sys(), tree.proc());

    let default = sysfs.read_topology(None).unwrap();
    assert_eq!(default.process_affinity, default.online_cpus);

    // Simulate running inside a cpuset that only grants two CPUs.
    let restricted: corescout_core::CpuSet = [0u32, 4].into_iter().collect();
    let confined = sysfs.read_topology(Some(restricted)).unwrap();
    assert_eq!(confined.process_affinity, vec![0, 4]);
    let usable = confined.benchmarkable_cores();
    assert_eq!(
        usable.len(),
        1,
        "only one core is reachable inside the cpuset"
    );
    assert_eq!(usable[0].id, 0);
}

#[test]
fn a_minimal_container_view_still_parses() {
    // No caches, no NUMA, no cpufreq, no CPPC: the parser must degrade rather
    // than fail, because this is what a small VM or a locked-down container
    // actually looks like.
    let (_tree, t) = parse(&FakeMachine::minimal(), "minimal");
    assert_eq!(t.logical_count(), 1);
    assert_eq!(t.physical_count(), 1);
    assert!(!t.smt_enabled());
    assert!(t.caches.is_empty());
    assert!(t.numa_nodes.is_empty());
    assert_eq!(t.cpu(0).unwrap().numa_node, None);
    assert_eq!(t.cpu(0).unwrap().frequency.current_khz, None);
    assert!(t.cpu(0).unwrap().favored.is_none());
    assert!(t.offline_cpus.is_empty());
}

#[test]
fn hybrid_parts_are_classified_from_the_kernel_pmu_devices() {
    let (_tree, t) = parse(&FakeMachine::hybrid(), "hybrid");
    assert!(t.hybrid, "a mixed P/E machine must be flagged hybrid");
    assert_eq!(t.physical_count(), 6, "2 P-cores plus 4 E-cores");

    assert_eq!(t.cpu(0).unwrap().core_type, CoreType::Performance);
    assert_eq!(t.cpu(5).unwrap().core_type, CoreType::Efficiency);

    let counts = t.cores_by_type();
    assert_eq!(counts.get("P-core"), Some(&2));
    assert_eq!(counts.get("E-core"), Some(&4));

    // E-cores share an L2 as a cluster: four CPUs on one cache instance.
    let cluster = t
        .caches
        .iter()
        .find(|c| c.level == 2 && c.shared_cpus.len() == 4)
        .expect("E-core L2 cluster");
    assert_eq!(cluster.shared_cpus, vec![4, 5, 6, 7]);
    // P-cores have SMT, E-cores do not.
    assert!(t.cpu(0).unwrap().is_smt());
    assert!(!t.cpu(4).unwrap().is_smt());
}

#[test]
fn a_broken_tree_reports_the_path_that_failed() {
    let tree = common::TempTree::new("broken");
    let sysfs = Sysfs::with_roots(tree.sys(), tree.proc());
    let err = sysfs
        .read_topology(None)
        .expect_err("an empty tree cannot describe a machine");
    let message = err.to_string();
    assert!(
        message.contains("present"),
        "the error should name the missing file, got: {message}"
    );
}

#[test]
fn parsed_topology_survives_json() {
    let (_tree, t) = parse(&FakeMachine::typical(), "json");
    let encoded = corescout_report::json::topology(&t).unwrap();
    let decoded: Topology = serde_json::from_str(&encoded).unwrap();
    assert_eq!(t, decoded);
}
