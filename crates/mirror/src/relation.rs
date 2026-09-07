//! Relations: the edges of `G(t) = (V, E, X(t))`.
//!
//! # Why relations are first-class rather than implied by fields
//!
//! In the original CoreScout, "CPU 0 and CPU 4 are SMT siblings" was a `Vec<u32>`
//! field on a CPU struct, and "these eight CPUs share an L3" was a field on a
//! cache struct. Both facts were real and neither was reachable generically: a
//! consumer had to know that `smt_siblings` existed and what it meant.
//!
//! As edges, both are the same kind of thing, discoverable by iterating. A model
//! can ask "which pairs of entities are connected, and does connectivity predict
//! correlated behaviour" without being told in advance that SMT siblings share
//! execution units. That question is the point of the representation.
//!
//! # Edges reference rows, not ids
//!
//! Relations store entity **row indices**, not [`EntityId`](crate::EntityId)s.
//! Row indices are compact, make an edge a fixed-size record, and let a consumer
//! index straight into the state matrix without a hash lookup. They are stable
//! within an epoch and meaningless across epochs, which is exactly the lifetime
//! of the relation set itself: if the entity set changed, the edges were rebuilt
//! anyway.

use serde::{Deserialize, Serialize};

/// Number of attribute slots carried by every edge.
///
/// Fixed width keeps relation records a constant size, which keeps the plane's
/// layout computable without a second pass. Four is enough for the relations
/// that exist today (cache level, cache size, distance, sharing width) with a
/// slot to spare.
pub const RELATION_ATTRS: usize = 4;

/// What kind of connection an edge represents.
///
/// Like [`EntityClass`](crate::EntityClass), these names are annotations
/// over a general structure. A consumer may treat every edge as an untyped
/// connection and still see the machine's shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u16)]
pub enum RelationKind {
    /// Structural containment: machine contains package contains core contains
    /// logical CPU. Directed from container to contained.
    Contains = 1,
    /// Two logical CPUs on one physical core. Emitted in both directions,
    /// because the relation is symmetric and a consumer should not have to know
    /// which way it was written.
    SmtSibling = 2,
    /// A cache and a logical CPU that uses it. Attribute 0 is the level,
    /// attribute 1 the size in bytes, attribute 2 the number of sharers.
    CacheMember = 3,
    /// A NUMA node and a logical CPU local to it.
    NumaLocal = 4,
    /// Entities that change frequency together.
    FrequencyDomain = 5,
    /// A thermal zone and the entities whose temperature it reports.
    ThermalDomain = 6,
    /// A power domain and the entities whose energy it accounts for.
    PowerDomain = 7,
    /// An interrupt source and the CPU that services it.
    InterruptAffinity = 8,
    /// A relation the substrate reported that has no name yet.
    Unclassified = 0,
}

impl RelationKind {
    pub fn as_u16(self) -> u16 {
        self as u16
    }

    pub fn from_u16(value: u16) -> RelationKind {
        match value {
            1 => RelationKind::Contains,
            2 => RelationKind::SmtSibling,
            3 => RelationKind::CacheMember,
            4 => RelationKind::NumaLocal,
            5 => RelationKind::FrequencyDomain,
            6 => RelationKind::ThermalDomain,
            7 => RelationKind::PowerDomain,
            8 => RelationKind::InterruptAffinity,
            _ => RelationKind::Unclassified,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RelationKind::Contains => "contains",
            RelationKind::SmtSibling => "smt_sibling",
            RelationKind::CacheMember => "cache_member",
            RelationKind::NumaLocal => "numa_local",
            RelationKind::FrequencyDomain => "frequency_domain",
            RelationKind::ThermalDomain => "thermal_domain",
            RelationKind::PowerDomain => "power_domain",
            RelationKind::InterruptAffinity => "interrupt_affinity",
            RelationKind::Unclassified => "unclassified",
        }
    }
}

/// One edge of the machine graph.
///
/// `PartialEq` is implemented by hand rather than derived, because the
/// attributes use `NaN` for "unobserved" and `NaN != NaN`. A derived comparison
/// would report two identical edges as different whenever either has an unused
/// attribute slot, which is most of them. Here, unobserved compares equal to
/// unobserved.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Relation {
    /// Row index of the source entity within this epoch.
    pub source: u32,
    /// Row index of the target entity within this epoch.
    pub target: u32,
    pub kind: RelationKind,
    /// Numeric attributes whose meaning is defined per [`RelationKind`].
    /// Unused slots are `NaN`, following the same convention as the state
    /// matrix.
    #[serde(with = "crate::nanjson::attrs")]
    pub attributes: [f64; RELATION_ATTRS],
}

impl PartialEq for Relation {
    fn eq(&self, other: &Relation) -> bool {
        self.source == other.source
            && self.target == other.target
            && self.kind == other.kind
            && self
                .attributes
                .iter()
                .zip(&other.attributes)
                .all(|(a, b)| same_value(*a, *b))
    }
}

/// Equality that treats unobserved as equal to unobserved.
#[inline]
pub(crate) fn same_value(a: f64, b: f64) -> bool {
    a == b || (a.is_nan() && b.is_nan())
}

impl Relation {
    pub fn new(source: u32, target: u32, kind: RelationKind) -> Relation {
        Relation {
            source,
            target,
            kind,
            attributes: [f64::NAN; RELATION_ATTRS],
        }
    }

    /// Builder-style attribute setter.
    pub fn with(mut self, slot: usize, value: f64) -> Relation {
        self.attributes[slot] = value;
        self
    }
}

/// A read-only view over a snapshot's edges.
///
/// Deliberately minimal: enough to walk the graph, not an attempt to be a graph
/// library. Anything that wants adjacency lists, transitive closure or shortest
/// paths can build them from the edge list, and doing so is the consumer's
/// business rather than the mirror's.
#[derive(Debug, Clone, Copy)]
pub struct RelationView<'a> {
    relations: &'a [Relation],
}

impl<'a> RelationView<'a> {
    pub fn new(relations: &'a [Relation]) -> RelationView<'a> {
        RelationView { relations }
    }

    pub fn all(&self) -> &'a [Relation] {
        self.relations
    }

    /// Edges leaving a given entity row.
    pub fn from(&self, source: u32) -> impl Iterator<Item = &'a Relation> + '_ {
        self.relations.iter().filter(move |r| r.source == source)
    }

    /// Edges arriving at a given entity row.
    pub fn to(&self, target: u32) -> impl Iterator<Item = &'a Relation> + '_ {
        self.relations.iter().filter(move |r| r.target == target)
    }

    /// Edges of one kind.
    pub fn of_kind(&self, kind: RelationKind) -> impl Iterator<Item = &'a Relation> + '_ {
        self.relations.iter().filter(move |r| r.kind == kind)
    }

    /// Whether two entity rows are directly connected by an edge of some kind.
    pub fn connected(&self, a: u32, b: u32, kind: RelationKind) -> bool {
        self.relations
            .iter()
            .any(|r| r.kind == kind && r.source == a && r.target == b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<Relation> {
        vec![
            Relation::new(0, 1, RelationKind::Contains),
            Relation::new(1, 2, RelationKind::Contains),
            Relation::new(2, 3, RelationKind::SmtSibling),
            Relation::new(3, 2, RelationKind::SmtSibling),
            Relation::new(4, 2, RelationKind::CacheMember)
                .with(0, 3.0)
                .with(1, 32.0),
        ]
    }

    #[test]
    fn unobserved_attributes_do_not_break_equality() {
        // Without a hand-written PartialEq this fails, and with it every
        // round-trip test in the crate becomes untrustworthy.
        let a = Relation::new(0, 1, RelationKind::Contains);
        let b = Relation::new(0, 1, RelationKind::Contains);
        assert_eq!(a, b);
        assert_ne!(a, Relation::new(0, 2, RelationKind::Contains));
        assert_ne!(a, a.with(0, 1.0));
    }

    #[test]
    fn attributes_default_to_unobserved() {
        let r = Relation::new(0, 1, RelationKind::Contains);
        assert!(r.attributes.iter().all(|a| a.is_nan()));
    }

    #[test]
    fn attributes_can_be_set_positionally() {
        let r = Relation::new(4, 2, RelationKind::CacheMember).with(0, 3.0);
        assert_eq!(r.attributes[0], 3.0);
        assert!(r.attributes[1].is_nan());
    }

    #[test]
    fn edges_can_be_walked_in_both_directions() {
        let relations = fixture();
        let view = RelationView::new(&relations);
        assert_eq!(view.from(1).count(), 1);
        assert_eq!(view.to(2).count(), 3, "contains, smt sibling, cache member");
    }

    #[test]
    fn symmetric_relations_are_emitted_both_ways() {
        // A consumer must never have to guess which direction SMT was written.
        let relations = fixture();
        let view = RelationView::new(&relations);
        assert!(view.connected(2, 3, RelationKind::SmtSibling));
        assert!(view.connected(3, 2, RelationKind::SmtSibling));
    }

    #[test]
    fn filtering_by_kind_works() {
        let relations = fixture();
        let view = RelationView::new(&relations);
        assert_eq!(view.of_kind(RelationKind::Contains).count(), 2);
        assert_eq!(view.of_kind(RelationKind::PowerDomain).count(), 0);
    }

    #[test]
    fn unknown_kinds_degrade_rather_than_fail() {
        assert_eq!(RelationKind::from_u16(4242), RelationKind::Unclassified);
        for kind in [
            RelationKind::Contains,
            RelationKind::SmtSibling,
            RelationKind::CacheMember,
            RelationKind::NumaLocal,
            RelationKind::ThermalDomain,
            RelationKind::PowerDomain,
        ] {
            assert_eq!(RelationKind::from_u16(kind.as_u16()), kind);
        }
    }
}
