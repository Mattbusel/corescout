//! Reflections with known contents, for tests in this crate and its dependents.
//!
//! Compiled unconditionally rather than under `cfg(test)`, because `cfg(test)`
//! modules are invisible to other crates and every crate downstream of the
//! mirror needs a snapshot to test against. The cost is a few hundred bytes in
//! a release binary; the alternative is every consumer hand-rolling its own
//! fixture and them drifting apart.
//!
//! Nothing here is a mock. These are real [`MirrorSnapshot`] values with real
//! entities, relations and availability codes; they simply describe a machine
//! that does not exist.

use crate::entity::{Entity, EntityClass};
use crate::relation::{Relation, RelationKind};
use crate::schema::{Availability, AvailabilityMatrix, Perturbation, SensorId, SensorReport};
use crate::state::{ChannelId, ChannelSpec, Semantics, StateMatrix, Unit};
use crate::{MirrorSnapshot, FORMAT_VERSION};

/// A small hand-built reflection: two CPUs on one core, one channel observed on
/// one of them, one deliberately absent for want of privilege.
///
/// Contains at least one of everything that has ever caused a bug: an
/// unobserved cell, a cumulative channel, a symmetric relation, and a cell that
/// is empty for a *reason* rather than by omission.
pub fn fixture() -> MirrorSnapshot {
    let entities = vec![
        Entity::new("machine", EntityClass::Machine, None),
        Entity::new("core/0/0", EntityClass::PhysicalCore, Some(0)),
        Entity::new("cpu/0", EntityClass::LogicalCpu, Some(0)),
        Entity::new("cpu/1", EntityClass::LogicalCpu, Some(1)),
    ];
    let channels = vec![
        ChannelSpec {
            id: ChannelId(0),
            key: "cpu.frequency.current".into(),
            unit: Unit::Kilohertz,
            semantics: Semantics::Instant,
            sensor: SensorId(1),
        },
        ChannelSpec {
            id: ChannelId(1),
            key: "cpu.time.idle".into(),
            unit: Unit::Nanosecond,
            semantics: Semantics::Cumulative,
            sensor: SensorId(2),
        },
    ];
    let relations = vec![
        Relation::new(0, 1, RelationKind::Contains),
        Relation::new(1, 2, RelationKind::Contains),
        Relation::new(1, 3, RelationKind::Contains),
        Relation::new(2, 3, RelationKind::SmtSibling),
        Relation::new(3, 2, RelationKind::SmtSibling),
    ];

    let mut state = StateMatrix::new(entities.len(), channels.len());
    state.set(2, 0, 3_600_000.0);
    state.set(2, 1, 12_345.0);
    state.set(3, 0, 800_000.0);

    let mut availability = AvailabilityMatrix::new(entities.len(), channels.len());
    availability.set(2, 0, Availability::Observed);
    availability.set(2, 1, Availability::Observed);
    availability.set(3, 0, Availability::Observed);
    // CPU 1's idle time is missing for a reason, which is the case that
    // distinguishes a mirror from a table of numbers.
    availability.set(3, 1, Availability::PermissionDenied);

    MirrorSnapshot {
        format_version: FORMAT_VERSION,
        epoch: 7,
        sequence: 42,
        monotonic_ns: 1_000_000_000,
        realtime_ns: 1_700_000_000_000_000_000,
        entities,
        channels,
        relations,
        state,
        availability,
        sensors: vec![
            SensorReport {
                id: SensorId(1),
                key: "frequency".into(),
                perturbation: Perturbation::Low,
                last_cost_ns: 45_000,
                sampling_latency_ns: 45_000,
                sample_age_ns: 0,
                samples: 2,
                errors: 0,
                confidence: 1.0,
                availability: Availability::Observed,
                inactive: false,
            },
            SensorReport {
                id: SensorId(2),
                key: "idle".into(),
                perturbation: Perturbation::Negligible,
                last_cost_ns: 12_000,
                sampling_latency_ns: 12_000,
                sample_age_ns: 0,
                samples: 1,
                errors: 1,
                confidence: 0.5,
                availability: Availability::Observed,
                inactive: false,
            },
        ],
    }
}

/// A series of reflections from the fixture machine, ticking at 10 Hz.
pub fn series(count: u64) -> Vec<MirrorSnapshot> {
    (0..count)
        .map(|i| {
            let mut snapshot = fixture();
            snapshot.sequence = i;
            snapshot.monotonic_ns = i * 100_000_000;
            // A counter that actually counts, so differencing produces
            // something other than zero.
            snapshot.state.set(2, 1, 12_345.0 + i as f64 * 1_000.0);
            snapshot
        })
        .collect()
}
