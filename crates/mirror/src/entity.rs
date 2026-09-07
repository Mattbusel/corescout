//! Entity identity.
//!
//! # Why identity is the hard part
//!
//! Everything else in the mirror is a number that changes. Identity is the thing
//! that must not. `entity_17` at `t0` and `entity_17` at `t2` have to refer to
//! the same piece of substrate even when every observable property of it has
//! changed, or the memory layer cannot form a series and the model layer has
//! nothing to learn from.
//!
//! # How identity is derived
//!
//! An entity's id is a 64-bit hash of a **natural key**: a short, canonical
//! string describing the entity's position in the machine, never its state.
//!
//! ```text
//! machine                       -> the machine as a whole
//! package/0                     -> socket 0
//! core/0/5                      -> package 0, kernel core_id 5
//! cpu/11                        -> logical CPU 11
//! cache/0/L3/unified/0          -> package 0's L3, anchored at core 0
//! numa/1                        -> NUMA node 1
//! thermal/x86_pkg_temp/0        -> a thermal zone
//! ```
//!
//! Hashing rather than allocating sequential ids means identity is *derivable*:
//! two CoreScout processes, or the same process before and after a restart,
//! independently compute the same id for the same component without
//! coordinating. There is no id registry to persist, corrupt or disagree about.
//!
//! # What this deliberately does not solve
//!
//! Keys are stable against state change and against restart. They are **not**
//! stable against every kind of physical change, and pretending otherwise would
//! be worse than admitting it:
//!
//! - Offlining a CPU that is a cache's anchor changes that cache's key. The
//!   epoch mechanism (see [`crate::snapshot`]) makes the discontinuity
//!   explicit rather than silently renaming an entity mid-series.
//! - Under a hypervisor, `cpu/11` may be a different physical core minute to
//!   minute. The mirror observes the machine the kernel presents; where that
//!   abstraction lies, the mirror inherits the lie.
//! - Migrating a VM preserves keys and replaces the hardware entirely.
//!
//! These are recorded as open questions in `RESEARCH.md` rather than papered
//! over here.

use serde::{Deserialize, Serialize};

/// A stable, machine-wide entity identifier.
///
/// Derived from a natural key, so it survives restarts and is computed
/// identically by independent observers of the same machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(pub u64);

impl EntityId {
    /// Derive an id from a natural key.
    ///
    /// FNV-1a: not cryptographic and does not need to be. The requirement is
    /// determinism across processes and architectures, which a fixed-constant
    /// integer hash satisfies and a `DefaultHasher` explicitly does not.
    pub fn derive(key: &str) -> EntityId {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in key.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
        // Zero is reserved to mean "no entity" in the binary layout.
        EntityId(if hash == 0 { 1 } else { hash })
    }

    pub fn raw(self) -> u64 {
        self.0
    }

    /// A short form for debugging output. Not an identifier; do not parse it.
    pub fn short(self) -> String {
        format!("e{:012x}", self.0 & 0xffff_ffff_ffff)
    }
}

/// A human-facing hint about what kind of thing an entity is.
///
/// **This is an annotation, not the structure.** Nothing in the mirror requires
/// a consumer to route its reasoning through these categories, and a learner is
/// free to ignore the column entirely and find its own groupings from observed
/// behaviour. The names are here because they are usually true and always
/// useful for debugging, not because they are the machine's real ontology.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u16)]
pub enum EntityClass {
    /// The machine as a whole: the root of the containment tree.
    Machine = 1,
    /// A socket or package.
    Package = 2,
    /// A physical core.
    PhysicalCore = 3,
    /// A logical CPU as the OS numbers it.
    LogicalCpu = 4,
    /// One cache instance at some level.
    Cache = 5,
    /// A NUMA node.
    NumaNode = 6,
    /// A thermal sensor's zone of responsibility.
    ThermalZone = 7,
    /// A domain over which a power limit or energy counter applies.
    PowerDomain = 8,
    /// An interrupt source.
    InterruptSource = 9,
    /// Something the substrate reported that we have no name for. This variant
    /// is not a failure mode; it is the escape hatch that keeps the
    /// representation open.
    Unclassified = 0,
}

impl EntityClass {
    pub fn as_u16(self) -> u16 {
        self as u16
    }

    pub fn from_u16(value: u16) -> EntityClass {
        match value {
            1 => EntityClass::Machine,
            2 => EntityClass::Package,
            3 => EntityClass::PhysicalCore,
            4 => EntityClass::LogicalCpu,
            5 => EntityClass::Cache,
            6 => EntityClass::NumaNode,
            7 => EntityClass::ThermalZone,
            8 => EntityClass::PowerDomain,
            9 => EntityClass::InterruptSource,
            _ => EntityClass::Unclassified,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EntityClass::Machine => "machine",
            EntityClass::Package => "package",
            EntityClass::PhysicalCore => "core",
            EntityClass::LogicalCpu => "cpu",
            EntityClass::Cache => "cache",
            EntityClass::NumaNode => "numa",
            EntityClass::ThermalZone => "thermal",
            EntityClass::PowerDomain => "power",
            EntityClass::InterruptSource => "irq",
            EntityClass::Unclassified => "entity",
        }
    }
}

/// A persistent observable entity.
///
/// The row index of an entity within a snapshot is its position in the state
/// matrix. That index is stable within an epoch and meaningless across epochs;
/// [`Entity::id`] is what is stable across time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    pub id: EntityId,
    /// The natural key the id was derived from. Kept so a consumer can verify
    /// the derivation and so debugging output can be read by a person.
    pub key: String,
    pub class_hint: EntityClass,
    /// The number the OS uses for this thing, where it has one: the logical CPU
    /// number, the NUMA node id, the package id. `None` for entities the OS does
    /// not number.
    pub natural_index: Option<u32>,
}

impl Entity {
    pub fn new(
        key: impl Into<String>,
        class_hint: EntityClass,
        natural_index: Option<u32>,
    ) -> Entity {
        let key = key.into();
        Entity {
            id: EntityId::derive(&key),
            key,
            class_hint,
            natural_index,
        }
    }
}

/// Canonical natural keys.
///
/// Centralised so that identity derivation is defined in exactly one place. A
/// second implementation that formats these strings slightly differently would
/// produce a machine whose entities silently fail to match across processes.
pub mod keys {
    pub fn machine() -> String {
        "machine".to_string()
    }

    pub fn package(package_id: u32) -> String {
        format!("package/{package_id}")
    }

    pub fn physical_core(package_id: u32, core_id: u32) -> String {
        format!("core/{package_id}/{core_id}")
    }

    pub fn logical_cpu(cpu: u32) -> String {
        format!("cpu/{cpu}")
    }

    /// A cache is anchored to the lowest core id among its sharers, which is
    /// what makes the key independent of how many CPUs happen to be online in
    /// the rest of the sharing set.
    pub fn cache(package_id: u32, level: u8, kind: &str, anchor_core: u32) -> String {
        format!("cache/{package_id}/L{level}/{kind}/{anchor_core}")
    }

    pub fn numa_node(node: u32) -> String {
        format!("numa/{node}")
    }

    pub fn thermal_zone(zone_type: &str, index: u32) -> String {
        format!("thermal/{zone_type}/{index}")
    }

    pub fn power_domain(name: &str) -> String {
        format!("power/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_deterministic() {
        assert_eq!(EntityId::derive("cpu/11"), EntityId::derive("cpu/11"));
        assert_ne!(EntityId::derive("cpu/11"), EntityId::derive("cpu/12"));
    }

    #[test]
    fn identity_never_collides_with_the_reserved_zero() {
        // Whatever key produces a zero hash, the id must not be zero, because
        // zero means "absent" in the binary layout.
        for i in 0..10_000u32 {
            assert_ne!(EntityId::derive(&format!("cpu/{i}")).raw(), 0);
        }
    }

    #[test]
    fn keys_are_distinct_across_classes() {
        // A core and a CPU with the same number must not collide.
        let ids = [
            EntityId::derive(&keys::logical_cpu(0)),
            EntityId::derive(&keys::physical_core(0, 0)),
            EntityId::derive(&keys::package(0)),
            EntityId::derive(&keys::numa_node(0)),
            EntityId::derive(&keys::cache(0, 3, "unified", 0)),
            EntityId::derive(&keys::machine()),
        ];
        let mut unique: Vec<u64> = ids.iter().map(|i| i.raw()).collect();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "natural keys collided");
    }

    #[test]
    fn identity_survives_state_change() {
        // The whole point: an entity's id depends on where it is, never on what
        // it is currently doing.
        let before = Entity::new(keys::logical_cpu(3), EntityClass::LogicalCpu, Some(3));
        let after = Entity::new(keys::logical_cpu(3), EntityClass::LogicalCpu, Some(3));
        assert_eq!(before.id, after.id);
    }

    #[test]
    fn class_hint_round_trips_through_its_wire_form() {
        for class in [
            EntityClass::Machine,
            EntityClass::Package,
            EntityClass::PhysicalCore,
            EntityClass::LogicalCpu,
            EntityClass::Cache,
            EntityClass::NumaNode,
            EntityClass::ThermalZone,
            EntityClass::PowerDomain,
            EntityClass::InterruptSource,
            EntityClass::Unclassified,
        ] {
            assert_eq!(EntityClass::from_u16(class.as_u16()), class);
        }
    }

    #[test]
    fn unknown_class_codes_degrade_rather_than_fail() {
        // A consumer reading a plane written by a newer CoreScout must not
        // choke on a class it has never heard of.
        assert_eq!(EntityClass::from_u16(4242), EntityClass::Unclassified);
    }
}
