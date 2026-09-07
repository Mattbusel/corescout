//! A human-readable projection of the mirror.
//!
//! # This is not the mirror
//!
//! Everything in this file is a debugging convenience. The canonical form of
//! `M(t)` is the binary matrix in the self-state plane; what follows is one
//! lossy rendering of it for a person who wants to check that the numbers look
//! right.
//!
//! That ordering matters and is easy to lose. In the original CoreScout the
//! human table and the JSON *were* the output, so every design decision was
//! quietly shaped by what would read well in a terminal. Here the rendering is
//! downstream of a representation that was designed for a consumer that cannot
//! read.
//!
//! A test asserts that nothing in this module is reachable from the mirror
//! itself: the dependency points one way only.

use crate::{pad, rpad};
use corescout_mirror::entity::EntityClass;
use corescout_mirror::relation::RelationKind;
use corescout_mirror::state::Semantics;
use corescout_mirror::MirrorSnapshot;

/// Header, sensors, and the shape of the graph.
pub fn summary(snapshot: &MirrorSnapshot) -> String {
    let mut out = String::new();
    out.push_str("Computational Mirror\n\n");
    out.push_str(&format!(
        "{} {}\n",
        pad("epoch", 22),
        format_args!(
            "{}  sequence {}  t+{:.3}s",
            snapshot.epoch,
            snapshot.sequence,
            snapshot.monotonic_ns as f64 / 1e9
        )
    ));
    out.push_str(&format!(
        "{} {} entities x {} channels, {:.0}% observed\n",
        pad("state", 22),
        snapshot.entities.len(),
        snapshot.channels.len(),
        snapshot.coverage() * 100.0
    ));
    out.push_str(&format!(
        "{} {:.1} us this pass, worst perturbation {}\n",
        pad("observation cost", 22),
        snapshot.observation_cost_ns() as f64 / 1000.0,
        snapshot.worst_perturbation().label()
    ));

    out.push_str("\nSensors\n");
    out.push_str(&format!(
        "  {} {} {} {} {}\n",
        pad("sensor", 14),
        pad("perturbs", 12),
        rpad("cost", 10),
        rpad("samples", 8),
        rpad("errors", 7)
    ));
    for sensor in &snapshot.sensors {
        if sensor.inactive {
            out.push_str(&format!(
                "  {} {}\n",
                pad(&sensor.key, 14),
                "inactive on this machine"
            ));
            continue;
        }
        out.push_str(&format!(
            "  {} {} {} {} {}\n",
            pad(&sensor.key, 14),
            pad(sensor.perturbation.label(), 12),
            rpad(
                &format!("{:.1} us", sensor.last_cost_ns as f64 / 1000.0),
                10
            ),
            rpad(&sensor.samples.to_string(), 8),
            rpad(&sensor.errors.to_string(), 7),
        ));
    }

    out.push_str("\nEntities\n");
    for (class, count) in class_counts(snapshot) {
        out.push_str(&format!("  {} {}\n", pad(class, 14), count));
    }

    out.push_str("\nRelations\n");
    for (kind, count) in relation_counts(snapshot) {
        out.push_str(&format!("  {} {}\n", pad(kind, 20), count));
    }

    out
}

/// Every observed value, entity by entity.
///
/// `filter` matches against the entity key as a substring, so `cpu/` shows the
/// logical CPUs and `cpu/3` shows one of them.
pub fn detail(snapshot: &MirrorSnapshot, filter: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str("Observed state\n");

    for (row, entity) in snapshot.entities.iter().enumerate() {
        if let Some(filter) = filter {
            if !entity.key.contains(filter) {
                continue;
            }
        }
        let values: Vec<(String, f64, Semantics)> = snapshot
            .channels
            .iter()
            .filter_map(|channel| {
                snapshot
                    .value(row as u32, channel.id)
                    .map(|v| (channel.key.clone(), v, channel.semantics))
            })
            .collect();
        if values.is_empty() {
            continue;
        }

        out.push_str(&format!(
            "\n  {} [{}] {}\n",
            entity.key,
            entity.class_hint.label(),
            entity.id.short()
        ));
        for (key, value, semantics) in values {
            let unit = snapshot
                .channels
                .iter()
                .find(|c| c.key == key)
                .map(|c| c.unit.symbol())
                .unwrap_or("");
            out.push_str(&format!(
                "    {} {} {}{}\n",
                pad(&key, 32),
                rpad(&format_value(value), 18),
                unit,
                if semantics == Semantics::Cumulative {
                    "  (cumulative)"
                } else {
                    ""
                }
            ));
        }
    }
    out
}

/// The graph around one entity: what contains it, what it contains, what it is
/// connected to.
pub fn neighbourhood(snapshot: &MirrorSnapshot, key: &str) -> String {
    let Some(row) = snapshot.row_of_key(key) else {
        return format!("no entity with key `{key}`\n");
    };
    let view = snapshot.relations();
    let mut out = format!("Neighbourhood of {key}\n");

    let name = |r: u32| -> String {
        snapshot
            .entities
            .get(r as usize)
            .map(|e| e.key.clone())
            .unwrap_or_else(|| format!("row {r}"))
    };

    out.push_str("\n  incoming\n");
    for relation in view.to(row) {
        out.push_str(&format!(
            "    {} {}\n",
            pad(relation.kind.label(), 20),
            name(relation.source)
        ));
    }
    out.push_str("\n  outgoing\n");
    for relation in view.from(row) {
        out.push_str(&format!(
            "    {} {}\n",
            pad(relation.kind.label(), 20),
            name(relation.target)
        ));
    }
    out
}

fn class_counts(snapshot: &MirrorSnapshot) -> Vec<(&'static str, usize)> {
    let classes = [
        EntityClass::Machine,
        EntityClass::Package,
        EntityClass::NumaNode,
        EntityClass::PhysicalCore,
        EntityClass::LogicalCpu,
        EntityClass::Cache,
        EntityClass::ThermalZone,
        EntityClass::PowerDomain,
        EntityClass::InterruptSource,
        EntityClass::Unclassified,
    ];
    classes
        .iter()
        .filter_map(|class| {
            let count = snapshot
                .entities
                .iter()
                .filter(|e| e.class_hint == *class)
                .count();
            (count > 0).then_some((class.label(), count))
        })
        .collect()
}

fn relation_counts(snapshot: &MirrorSnapshot) -> Vec<(&'static str, usize)> {
    let kinds = [
        RelationKind::Contains,
        RelationKind::SmtSibling,
        RelationKind::CacheMember,
        RelationKind::NumaLocal,
        RelationKind::FrequencyDomain,
        RelationKind::ThermalDomain,
        RelationKind::PowerDomain,
        RelationKind::InterruptAffinity,
        RelationKind::Unclassified,
    ];
    let view = snapshot.relations();
    kinds
        .iter()
        .filter_map(|kind| {
            let count = view.of_kind(*kind).count();
            (count > 0).then_some((kind.label(), count))
        })
        .collect()
}

/// Render a value at a readable magnitude without losing its identity.
fn format_value(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude >= 1e12 {
        format!("{:.3}e12", value / 1e12)
    } else if magnitude >= 1e9 {
        format!("{:.3}e9", value / 1e9)
    } else if magnitude == value.trunc() && magnitude < 1e9 {
        format!("{value:.0}")
    } else {
        format!("{value:.3}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::test_support::fixture;

    #[test]
    fn the_summary_reports_shape_cost_and_perturbation() {
        let text = summary(&fixture());
        assert!(text.contains("Computational Mirror"));
        assert!(text.contains("epoch"));
        assert!(text.contains("entities x"));
        // The observer effect must be visible in any projection of the mirror.
        assert!(text.contains("observation cost"));
        assert!(text.contains("worst perturbation low"));
    }

    #[test]
    fn inactive_sensors_are_named_rather_than_shown_as_zero() {
        let mut snapshot = fixture();
        snapshot.sensors[1].inactive = true;
        let text = summary(&snapshot);
        assert!(text.contains("inactive on this machine"));
    }

    #[test]
    fn detail_shows_observed_cells_and_omits_holes() {
        let text = detail(&fixture(), None);
        assert!(text.contains("cpu/0"));
        assert!(text.contains("cpu.frequency.current"));
        // cpu/1 has a frequency but no idle time; the hole is simply not shown.
        let cpu1 = text.split("cpu/1").nth(1).unwrap_or("");
        assert!(!cpu1.contains("cpu.time.idle"));
    }

    #[test]
    fn cumulative_channels_are_marked_as_such() {
        // A reader must not mistake a running total for a reading.
        let text = detail(&fixture(), None);
        assert!(text.contains("(cumulative)"));
    }

    #[test]
    fn detail_can_be_filtered_by_entity_key() {
        let text = detail(&fixture(), Some("cpu/1"));
        assert!(text.contains("cpu/1"));
        assert!(!text.contains("cpu/0 ["));
    }

    #[test]
    fn the_neighbourhood_shows_both_directions_of_the_graph() {
        let text = neighbourhood(&fixture(), "cpu/0");
        assert!(text.contains("incoming"));
        assert!(text.contains("outgoing"));
        assert!(text.contains("contains"));
        assert!(text.contains("smt_sibling"));
    }

    #[test]
    fn an_unknown_entity_says_so() {
        let text = neighbourhood(&fixture(), "cpu/99");
        assert!(text.contains("no entity"));
    }

    #[test]
    fn values_render_at_a_readable_magnitude() {
        assert_eq!(format_value(3_600_000.0), "3600000");
        assert_eq!(format_value(1.5), "1.500");
        assert_eq!(format_value(9_812_345_678_901.0), "9.812e12");
    }
}
