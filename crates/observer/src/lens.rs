//! What an observer is allowed to know about what it is looking at.
//!
//! # The experiment this type exists for
//!
//! Two observers, identical code, same data, different view:
//!
//! ```text
//! Observer A   sees "cpu/3", "cpu.frequency.current", "smt_sibling", Celsius
//! Observer B   sees entity 7, variable x2, edge type 2, unit 0
//! ```
//!
//! The question is whether our ontology *helps* a system understand itself or
//! *constrains* it. That question is only answerable if the same learner can be
//! run both ways, so the difference is confined to this one type rather than
//! spread through the discovery code.
//!
//! # What is withheld, and what cannot be
//!
//! Withheld from `Unlabelled`: entity keys, channel keys, class hints, units,
//! and the `Semantics` tag saying whether a channel is a running total.
//!
//! Not withheld, because they are not labels: the number of entities, the
//! number of variables, which entity is which row, the existence of edges,
//! which pairs they join, and edge *types as opaque integers*. A learner has to
//! be able to tell one edge apart from another to say anything about edges at
//! all; what it must not be given is the word "smt_sibling".
//!
//! The `Semantics` case is the interesting one. `Cumulative` is genuinely
//! useful: knowing a variable is an accumulator changes how you predict it. It
//! is also *discoverable*, since an accumulator never decreases. So Observer B
//! has to work it out, and whether it manages to is a direct measurement of
//! what that particular human label was worth.

use corescout_mirror::state::Semantics;
use corescout_mirror::{ChannelSpec, Entity, Relation};

/// How much of the mirror's own vocabulary an observer may see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lens {
    /// Everything the plane carries, including every human-chosen name.
    Labelled,
    /// Identity, numbers, structure and time. No names, no units, no
    /// declared semantics.
    Unlabelled,
}

impl Lens {
    pub fn label(self) -> &'static str {
        match self {
            Lens::Labelled => "labelled",
            Lens::Unlabelled => "unlabelled",
        }
    }

    /// Name for an entity, as this observer is permitted to see it.
    pub fn entity_name(self, row: usize, entity: &Entity) -> String {
        match self {
            Lens::Labelled => entity.key.clone(),
            // The id is still shown: identity is not a label, it is the thing
            // that makes a series a series. It is rendered as an opaque number
            // so nothing can be inferred from its text.
            Lens::Unlabelled => format!("entity_{row}"),
        }
    }

    /// Name for a variable.
    pub fn channel_name(self, col: usize, channel: &ChannelSpec) -> String {
        match self {
            Lens::Labelled => channel.key.clone(),
            Lens::Unlabelled => format!("x{col}"),
        }
    }

    /// Name for an edge type.
    pub fn relation_name(self, relation: &Relation) -> String {
        match self {
            Lens::Labelled => relation.kind.label().to_string(),
            Lens::Unlabelled => format!("edge_type_{}", relation.kind.as_u16()),
        }
    }

    /// The declared semantics of a channel, if this observer may see them.
    ///
    /// `None` under the unlabelled lens: the observer must infer whether a
    /// variable accumulates by watching it.
    pub fn declared_semantics(self, channel: &ChannelSpec) -> Option<Semantics> {
        match self {
            Lens::Labelled => Some(channel.semantics),
            Lens::Unlabelled => None,
        }
    }

    /// The declared unit, if visible.
    pub fn declared_unit(self, channel: &ChannelSpec) -> Option<&'static str> {
        match self {
            Lens::Labelled => Some(channel.unit.symbol()),
            Lens::Unlabelled => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::entity::EntityClass;
    use corescout_mirror::relation::RelationKind;
    use corescout_mirror::schema::SensorId;
    use corescout_mirror::state::{ChannelId, Unit};

    fn channel() -> ChannelSpec {
        ChannelSpec {
            id: ChannelId(2),
            key: "cpu.frequency.current".into(),
            unit: Unit::Kilohertz,
            semantics: Semantics::Instant,
            sensor: SensorId(1),
        }
    }

    #[test]
    fn the_unlabelled_lens_withholds_every_human_name() {
        let entity = Entity::new("cpu/3", EntityClass::LogicalCpu, Some(3));
        let relation = Relation::new(0, 1, RelationKind::SmtSibling);
        let lens = Lens::Unlabelled;

        assert_eq!(lens.entity_name(7, &entity), "entity_7");
        assert_eq!(lens.channel_name(2, &channel()), "x2");
        assert_eq!(lens.relation_name(&relation), "edge_type_2");
        assert_eq!(lens.declared_semantics(&channel()), None);
        assert_eq!(lens.declared_unit(&channel()), None);

        // Nothing it can see contains the words a human would have chosen.
        for name in [
            lens.entity_name(7, &entity),
            lens.channel_name(2, &channel()),
            lens.relation_name(&relation),
        ] {
            for leak in ["cpu", "frequency", "smt", "sibling", "core"] {
                assert!(!name.contains(leak), "`{name}` leaks `{leak}`");
            }
        }
    }

    #[test]
    fn the_labelled_lens_passes_everything_through() {
        let entity = Entity::new("cpu/3", EntityClass::LogicalCpu, Some(3));
        let lens = Lens::Labelled;
        assert_eq!(lens.entity_name(7, &entity), "cpu/3");
        assert_eq!(lens.channel_name(2, &channel()), "cpu.frequency.current");
        assert_eq!(
            lens.relation_name(&Relation::new(0, 1, RelationKind::SmtSibling)),
            "smt_sibling"
        );
        assert_eq!(
            lens.declared_semantics(&channel()),
            Some(Semantics::Instant)
        );
    }

    #[test]
    fn edge_types_stay_distinguishable_without_being_named() {
        // A learner must be able to tell one kind of connection from another,
        // or it can say nothing about connection at all.
        let lens = Lens::Unlabelled;
        let smt = lens.relation_name(&Relation::new(0, 1, RelationKind::SmtSibling));
        let cache = lens.relation_name(&Relation::new(0, 1, RelationKind::CacheMember));
        assert_ne!(smt, cache);
    }
}
