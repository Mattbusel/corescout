//! End-to-end test of the mirror against a synthetic machine.
//!
//! The production sensors run unmodified here. They read a `/sys` and `/proc`
//! tree built by `tests/common`, which uses the kernel's own file formats, so
//! this exercises the real observation path on any host including one that is
//! not Linux and has none of these interfaces.
//!
//! What it cannot test is the syscall layer: `perf_event_open` and the shared
//! memory mapping need a real kernel, and are covered by `tests/mirror_plane.rs`
//! and `tests/linux_live.rs`.

mod common;

use common::FakeMachine;
use corescout_mirror::entity::EntityClass;
use corescout_mirror::relation::RelationKind;
use corescout_mirror::MirrorSnapshot;
use corescout_substrate::discovery::{Roots, Substrate};
use corescout_substrate::observation::{default_sensors, Perturbation};
use corescout_substrate::platform::linux::sysfs::Sysfs;
use corescout_substrate::Reflector;

/// Build a mirror over a synthetic machine and take one reflection.
fn observe(machine: &FakeMachine, tag: &str) -> (common::TempTree, MirrorSnapshot) {
    let tree = machine.write(tag);
    let topology = Sysfs::with_roots(tree.sys(), tree.proc())
        .read_topology(None)
        .expect("fixture topology");
    let substrate = Substrate::new(topology, Roots::new(tree.sys(), tree.proc()));
    let mut mirror = Reflector::build(substrate, default_sensors()).expect("build mirror");
    mirror.observe();
    (tree, mirror.snapshot())
}

#[test]
fn the_mirror_describes_the_whole_machine_as_one_graph() {
    let (_tree, snapshot) = observe(&FakeMachine::typical(), "graph");

    // Structure: machine, package, 2 NUMA nodes, 4 cores, 7 online CPUs,
    // caches, plus the thermal entities the sensors declared.
    assert_eq!(snapshot.rows_of_class(EntityClass::Machine).len(), 1);
    assert_eq!(snapshot.rows_of_class(EntityClass::PhysicalCore).len(), 4);
    assert_eq!(snapshot.rows_of_class(EntityClass::LogicalCpu).len(), 7);
    assert!(!snapshot.rows_of_class(EntityClass::ThermalZone).is_empty());

    // Every entity has a stable identity derived from its key.
    for entity in &snapshot.entities {
        assert_eq!(entity.id, corescout_mirror::EntityId::derive(&entity.key));
    }

    let view = snapshot.relations();
    assert!(view.of_kind(RelationKind::Contains).count() > 0);
    assert!(view.of_kind(RelationKind::SmtSibling).count() > 0);
    assert!(view.of_kind(RelationKind::CacheMember).count() > 0);
    assert!(view.of_kind(RelationKind::NumaLocal).count() > 0);
    assert!(view.of_kind(RelationKind::ThermalDomain).count() > 0);
}

#[test]
fn frequency_state_reaches_the_matrix() {
    let (_tree, snapshot) = observe(&FakeMachine::typical(), "frequency");
    // The fixture clocks core 1 (CPUs 1 and 5) higher than the rest.
    assert_eq!(
        snapshot.lookup("cpu/1", "cpu.frequency.current"),
        Some(4_800_000.0)
    );
    assert_eq!(
        snapshot.lookup("cpu/0", "cpu.frequency.current"),
        Some(3_600_000.0)
    );
    assert_eq!(
        snapshot.lookup("cpu/0", "cpu.frequency.max"),
        Some(5_200_000.0)
    );
}

#[test]
fn cumulative_counters_are_published_raw() {
    let (_tree, snapshot) = observe(&FakeMachine::typical(), "counters");

    // Idle residency, straight from the counter, not a computed percentage.
    let residency = snapshot
        .lookup("cpu/2", "cpu.idle.state0.residency")
        .expect("idle residency");
    assert_eq!(residency, 15_000_000.0);

    // CPU time, converted from USER_HZ ticks to nanoseconds once, here.
    let idle_ns = snapshot
        .lookup("cpu/1", "cpu.time.idle")
        .expect("idle time");
    assert!(idle_ns > 0.0);

    // Every one of these must be declared cumulative, or a consumer will read a
    // running total as if it were a reading.
    for key in [
        "cpu.idle.state0.residency",
        "cpu.time.idle",
        "cpu.sched.wait_time",
        "cpu.interrupts.total",
    ] {
        let channel = snapshot
            .channels
            .iter()
            .find(|c| c.key == key)
            .unwrap_or_else(|| panic!("missing channel {key}"));
        assert_eq!(
            channel.semantics,
            corescout_mirror::Semantics::Cumulative,
            "{key} must be declared cumulative"
        );
    }
}

#[test]
fn no_rate_average_or_percentage_is_published() {
    // The rule that separates mirror from memory, asserted against every
    // channel the machine actually declares.
    let (_tree, snapshot) = observe(&FakeMachine::typical(), "no-derived");
    for channel in &snapshot.channels {
        let key = channel.key.as_str();
        for forbidden in [
            "_avg",
            "average",
            "load",
            "_rate",
            "per_sec",
            "percent",
            "utilization",
        ] {
            assert!(
                !key.contains(forbidden),
                "channel `{key}` looks like a derived quantity; those belong to the memory layer"
            );
        }
    }
    // Specifically: /proc/loadavg was parsed, and only its instantaneous half
    // was kept.
    assert_eq!(
        snapshot.lookup("machine", "machine.tasks.runnable"),
        Some(3.0)
    );
    assert_eq!(
        snapshot.lookup("machine", "machine.tasks.total"),
        Some(1234.0)
    );
}

#[test]
fn interrupt_asymmetry_is_visible() {
    // The thing that used to be inferred indirectly from disturbed benchmark
    // samples is now observed directly.
    let (_tree, snapshot) = observe(&FakeMachine::typical(), "interrupts");
    let cpu0 = snapshot
        .lookup("cpu/0", "cpu.interrupts.total")
        .expect("cpu0 interrupts");
    let cpu1 = snapshot
        .lookup("cpu/1", "cpu.interrupts.total")
        .expect("cpu1 interrupts");
    let cpu2 = snapshot
        .lookup("cpu/2", "cpu.interrupts.total")
        .expect("cpu2 interrupts");
    assert_eq!(cpu0, 31.0 + 1_000_000.0);
    assert_eq!(cpu1, 88_192.0 + 2_000_000.0, "the NIC queue lands on CPU 1");
    assert_eq!(cpu2, 3_000_000.0);
}

#[test]
fn per_core_thermal_sensors_are_linked_to_the_cores_they_measure() {
    let (_tree, snapshot) = observe(&FakeMachine::typical(), "thermal");

    let zone = snapshot
        .entities
        .iter()
        .position(|e| e.key.contains("Core 0"))
        .expect("a per-core thermal entity") as u32;
    assert_eq!(
        snapshot.value(zone, snapshot.channel("thermal.temperature").unwrap()),
        Some(52.0)
    );

    // The sensor is its own entity, joined to the core by an edge rather than
    // being a field on it.
    let linked: Vec<&corescout_mirror::Relation> = snapshot
        .relations()
        .from(zone)
        .filter(|r| r.kind == RelationKind::ThermalDomain)
        .collect();
    assert_eq!(linked.len(), 1);
    let target = &snapshot.entities[linked[0].target as usize];
    assert_eq!(target.class_hint, EntityClass::PhysicalCore);
    assert_eq!(target.key, "core/0/0");
}

#[test]
fn the_package_zone_is_linked_to_the_machine_not_to_a_core() {
    let (_tree, snapshot) = observe(&FakeMachine::typical(), "pkg-thermal");
    let zone = snapshot
        .row_of_key("thermal/x86_pkg_temp/0")
        .expect("package thermal zone");
    let edge = snapshot
        .relations()
        .from(zone)
        .find(|r| r.kind == RelationKind::ThermalDomain)
        .expect("a thermal domain edge");
    assert_eq!(snapshot.entities[edge.target as usize].key, "machine");
    assert_eq!(
        snapshot.value(zone, snapshot.channel("thermal.temperature").unwrap()),
        Some(47.0)
    );
}

#[test]
fn the_mirror_reports_what_observing_cost_it() {
    let (_tree, snapshot) = observe(&FakeMachine::typical(), "cost");

    // Every sensor that bound reports a cost and a sample count.
    let active: Vec<_> = snapshot.sensors.iter().filter(|s| !s.inactive).collect();
    assert!(!active.is_empty());
    assert!(active.iter().any(|s| s.samples > 0));
    assert!(snapshot.observation_cost_ns() > 0, "looking is never free");

    // And the mirror knows how much it disturbed the machine to produce this.
    // Frequency and thermal are both Low, so the pass as a whole is Low.
    assert_eq!(snapshot.worst_perturbation(), Perturbation::Low);
}

#[test]
fn unavailable_sensors_are_inactive_rather_than_wrong() {
    // A minimal container: no cpufreq, no cpuidle, no thermal, no interrupts.
    let (_tree, snapshot) = observe(&FakeMachine::minimal(), "minimal");

    let inactive: Vec<&str> = snapshot
        .sensors
        .iter()
        .filter(|s| s.inactive)
        .map(|s| s.key.as_str())
        .collect();
    for expected in ["frequency", "idle", "thermal", "scheduler", "interrupts"] {
        assert!(
            inactive.contains(&expected),
            "`{expected}` should be inactive on a machine that exposes nothing: {inactive:?}"
        );
    }

    // The machine is still described, just with fewer columns.
    assert!(!snapshot.entities.is_empty());
    assert_eq!(
        snapshot.lookup("cpu/0", "cpu.frequency.current"),
        None,
        "an unavailable sensor must leave a hole, not a zero"
    );
    // An inactive sensor perturbs nothing, so it must not raise the reported
    // perturbation of the pass.
    assert!(snapshot.worst_perturbation() < Perturbation::Material);
}

#[test]
fn a_hybrid_machine_is_reflected_without_the_labels_being_load_bearing() {
    let (_tree, snapshot) = observe(&FakeMachine::hybrid(), "hybrid");

    // The class hints are there and correct.
    assert_eq!(snapshot.rows_of_class(EntityClass::PhysicalCore).len(), 6);

    // ...and the same distinction is reachable without them. The E-cores share
    // an L2 cluster; the P-cores do not. A consumer that ignores every label
    // can still see two kinds of core here.
    let cluster_sizes: Vec<usize> = snapshot
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| e.class_hint == EntityClass::Cache)
        .map(|(row, _)| {
            snapshot
                .relations()
                .from(row as u32)
                .filter(|r| r.kind == RelationKind::CacheMember)
                .count()
        })
        .collect();
    assert!(
        cluster_sizes.contains(&4),
        "the four-CPU E-core L2 cluster should be visible as a shape: {cluster_sizes:?}"
    );
}

#[test]
fn observing_twice_advances_the_sequence_and_refreshes_every_cell() {
    let tree = FakeMachine::typical().write("twice");
    let topology = Sysfs::with_roots(tree.sys(), tree.proc())
        .read_topology(None)
        .unwrap();
    let substrate = Substrate::new(topology, Roots::new(tree.sys(), tree.proc()));
    let mut mirror = Reflector::build(substrate, default_sensors()).unwrap();

    mirror.observe();
    let first = mirror.snapshot();
    mirror.observe();
    let second = mirror.snapshot();

    assert_eq!(second.sequence, first.sequence + 1);
    assert!(second.monotonic_ns >= first.monotonic_ns);
    assert_eq!(second.epoch, first.epoch, "the shape did not change");
    // The fixture is static, so the values are identical; what matters is that
    // they were re-read rather than carried over.
    assert_eq!(second.state.observed_cells(), first.state.observed_cells());
}

#[test]
fn coverage_is_reported_and_partial() {
    let (_tree, snapshot) = observe(&FakeMachine::typical(), "coverage");
    let coverage = snapshot.coverage();
    assert!(coverage > 0.1, "too little was observed: {coverage}");
    assert!(
        coverage < 1.0,
        "full coverage is implausible: thermal zones have no frequency, CPUs have no \
         temperature, and the matrix is deliberately sparse"
    );
}
