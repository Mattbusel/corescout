//! `M(t)`: one instantaneous reflection of the machine.
//!
//! # What a snapshot is, and what it is forbidden to be
//!
//! ```text
//! M(t) != recommendation
//! M(t) != prediction
//! M(t) != historical summary
//! M(t) != benchmark result
//! ```
//!
//! A snapshot contains the entities that exist, the relations between them, and
//! the values observed at one moment. It contains no averages, no deltas, no
//! trends and no advice. Everything derived lives in a layer above.
//!
//! The one apparent exception proves the rule: cumulative counters. A counter's
//! current value is a present fact about the machine, readable in a single
//! observation, and is published as such. The *rate* it implies requires two
//! observations and belongs to `corescout_memory`.
//!
//! # Epochs
//!
//! `sequence` counts snapshots. `epoch` counts changes to the *shape* of the
//! machine: a CPU going offline, a sensor binding for the first time, anything
//! that alters the entity, channel or relation tables.
//!
//! The distinction matters to any consumer holding cached row indices. Within an
//! epoch, row 12 is the same entity in every snapshot and a consumer may cache
//! freely. Across an epoch boundary the tables were rebuilt and every cached
//! index must be discarded. Making that a visible, checkable number is what
//! stops a consumer from silently reading the wrong row after a hotplug event.

use serde::{Deserialize, Serialize};

use crate::entity::{Entity, EntityId};
use crate::relation::{Relation, RelationView};
use crate::schema::{Availability, AvailabilityMatrix, Perturbation, SensorReport};
use crate::state::{ChannelId, ChannelSpec, StateMatrix};

/// Bumped when the binary layout of the self-state plane changes in a way that
/// an older consumer could misread. A consumer must refuse a plane whose
/// version it does not recognise rather than guess.
pub const FORMAT_VERSION: u32 = 2;

/// One complete reflection of the machine at a moment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MirrorSnapshot {
    pub format_version: u32,
    /// Generation of the entity/channel/relation tables.
    pub epoch: u64,
    /// Monotonically increasing snapshot counter within this mirror's lifetime.
    pub sequence: u64,
    /// `CLOCK_MONOTONIC_RAW` at the start of the observation pass. The clock to
    /// use for measuring intervals between snapshots.
    pub monotonic_ns: u64,
    /// Wall-clock time, for correlating with logs and with other machines. Not
    /// monotonic; do not compute intervals from it.
    pub realtime_ns: u64,
    /// `V`: the entities, in row order.
    pub entities: Vec<Entity>,
    /// The columns of the state matrix.
    pub channels: Vec<ChannelSpec>,
    /// `E`: the edges, referencing entity rows.
    pub relations: Vec<Relation>,
    /// `X(t)`: the dense state matrix.
    pub state: StateMatrix,
    /// Why each cell of `state` is empty, where it is. Same shape as `state`.
    /// A consumer that only wants numbers can ignore this; a consumer that
    /// needs to know the difference between "zero" and "this machine cannot
    /// tell you" cannot.
    pub availability: AvailabilityMatrix,
    /// What this reflection cost to produce. The mirror describing its own act
    /// of looking.
    pub sensors: Vec<SensorReport>,
}

impl MirrorSnapshot {
    /// Row index of an entity by its stable id.
    ///
    /// Linear: entity counts are in the hundreds, and a consumer that cares
    /// about lookup cost should resolve ids to rows once per epoch, which is
    /// exactly the usage the epoch counter exists to make safe.
    pub fn row_of(&self, id: EntityId) -> Option<u32> {
        self.entities
            .iter()
            .position(|e| e.id == id)
            .map(|i| i as u32)
    }

    /// Row index of an entity by natural key.
    pub fn row_of_key(&self, key: &str) -> Option<u32> {
        self.entities
            .iter()
            .position(|e| e.key == key)
            .map(|i| i as u32)
    }

    /// Column index of a channel by key.
    pub fn channel(&self, key: &str) -> Option<ChannelId> {
        self.channels.iter().find(|c| c.key == key).map(|c| c.id)
    }

    /// One observed value, or `None` when the cell was not observed.
    ///
    /// Returning `Option` rather than `NaN` at the API boundary makes the
    /// absent case impossible to ignore by accident. The raw matrix still uses
    /// `NaN`, because that is what a numeric consumer wants.
    pub fn value(&self, row: u32, channel: ChannelId) -> Option<f64> {
        let (row, col) = (row as usize, channel.index());
        if row >= self.state.rows() || col >= self.state.cols() {
            return None;
        }
        let v = self.state.get(row, col);
        if v.is_nan() {
            None
        } else {
            Some(v)
        }
    }

    /// Look a value up by entity key and channel key. Convenient, and slow
    /// enough that nothing on a hot path should use it.
    pub fn lookup(&self, entity_key: &str, channel_key: &str) -> Option<f64> {
        let row = self.row_of_key(entity_key)?;
        let channel = self.channel(channel_key)?;
        self.value(row, channel)
    }

    /// Why one cell holds no value.
    pub fn availability(&self, row: u32, channel: ChannelId) -> Availability {
        self.availability.get(row as usize, channel.index())
    }

    /// Cells the machine refused to show for want of privilege.
    ///
    /// Actionable in a way no other gap is: the same mirror run with
    /// `CAP_PERFMON`, or as root, would fill these.
    pub fn privilege_gaps(&self) -> usize {
        self.availability
            .tally()
            .iter()
            .filter(|(availability, _)| availability.is_privilege_problem())
            .map(|(_, count)| *count)
            .sum()
    }

    /// A view over the edges.
    pub fn relations(&self) -> RelationView<'_> {
        RelationView::new(&self.relations)
    }

    /// Total time spent observing on the pass that produced this snapshot.
    pub fn observation_cost_ns(&self) -> u64 {
        self.sensors.iter().map(|s| s.last_cost_ns).sum()
    }

    /// The worst perturbation any active sensor caused producing this snapshot.
    ///
    /// A consumer holding a snapshot can therefore tell how much the machine was
    /// disturbed in the act of describing it, without knowing anything about
    /// which sensors exist.
    pub fn worst_perturbation(&self) -> Perturbation {
        self.sensors
            .iter()
            .filter(|s| !s.inactive)
            .map(|s| s.perturbation)
            .max()
            .unwrap_or(Perturbation::None)
    }

    /// Fraction of the state matrix that was actually observed.
    ///
    /// A structural property of this reflection, not of the machine: a low
    /// figure means sensors are missing or unprivileged, and a consumer should
    /// weigh the snapshot accordingly.
    pub fn coverage(&self) -> f64 {
        let cells = self.state.rows() * self.state.cols();
        if cells == 0 {
            return 0.0;
        }
        self.state.observed_cells() as f64 / cells as f64
    }

    /// Entities whose class hint matches, as row indices.
    ///
    /// Provided for debugging projections and for consumers that do want the
    /// human ontology. Nothing in the mirror requires its use.
    pub fn rows_of_class(&self, class: crate::entity::EntityClass) -> Vec<u32> {
        self.entities
            .iter()
            .enumerate()
            .filter(|(_, e)| e.class_hint == class)
            .map(|(i, _)| i as u32)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityClass;
    use crate::relation::RelationKind;
    use crate::test_support::fixture;

    #[test]
    fn entities_are_addressable_by_stable_id_and_by_key() {
        let s = fixture();
        let id = EntityId::derive("cpu/1");
        assert_eq!(s.row_of(id), Some(3));
        assert_eq!(s.row_of_key("cpu/1"), Some(3));
        assert_eq!(s.row_of_key("cpu/99"), None);
    }

    #[test]
    fn unobserved_cells_are_none_not_zero() {
        let s = fixture();
        assert_eq!(
            s.lookup("cpu/0", "cpu.frequency.current"),
            Some(3_600_000.0)
        );
        // CPU 1's idle time was never observed.
        assert_eq!(s.lookup("cpu/1", "cpu.time.idle"), None);
    }

    #[test]
    fn out_of_range_access_is_none_rather_than_a_panic() {
        let s = fixture();
        assert_eq!(s.value(99, ChannelId(0)), None);
        assert_eq!(s.value(0, ChannelId(99)), None);
    }

    #[test]
    fn the_snapshot_reports_what_it_cost_to_produce() {
        let s = fixture();
        assert_eq!(s.observation_cost_ns(), 57_000);
        assert_eq!(
            s.worst_perturbation(),
            Perturbation::Low,
            "the worst sensor sets the snapshot's perturbation"
        );
    }

    #[test]
    fn inactive_sensors_do_not_contribute_perturbation() {
        let mut s = fixture();
        s.sensors[0].inactive = true;
        assert_eq!(s.worst_perturbation(), Perturbation::Negligible);
    }

    #[test]
    fn coverage_describes_how_complete_this_reflection_is() {
        let s = fixture();
        // 3 observed cells out of 4 entities x 2 channels.
        assert!((s.coverage() - 3.0 / 8.0).abs() < 1e-9);
    }

    #[test]
    fn class_hints_are_available_but_not_required() {
        let s = fixture();
        assert_eq!(s.rows_of_class(EntityClass::LogicalCpu), vec![2, 3]);
        // ...and the same information is reachable without them, by walking
        // edges from the machine, which is the point of the representation.
        let view = s.relations();
        assert_eq!(view.of_kind(RelationKind::Contains).count(), 3);
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let s = fixture();
        let json = serde_json::to_string(&s).unwrap();
        let back: MirrorSnapshot = serde_json::from_str(&json).unwrap();
        // NaN does not survive JSON (it becomes null), so compare the parts
        // that are meant to be portable rather than the raw matrix.
        assert_eq!(s.entities, back.entities);
        assert_eq!(s.relations, back.relations);
        assert_eq!(s.epoch, back.epoch);
        assert_eq!(s.sequence, back.sequence);
    }
}
