//! What one observer has seen.
//!
//! # This is not the memory layer
//!
//! `corescout_memory` is a planned service *on the mirror's side*: a bounded
//! history of `M(t)` that any consumer could query. This is something else and
//! smaller: one observer's private record of the reflections it personally
//! witnessed.
//!
//! The distinction is worth keeping even once the memory layer exists, because
//! the two answer different questions. "What was the machine doing at t-500ms"
//! is a fact about the machine. "What have I seen" is a fact about the observer,
//! and two observers that started at different times have different answers
//! while the machine has only one history.
//!
//! # Shape
//!
//! A ring of `(time, state matrix)` pairs plus the static tables, which are
//! stored once because they do not change within an epoch. An epoch change
//! invalidates every row index, so the ring is cleared: an observer must not
//! silently continue a series across a machine whose shape changed underneath
//! it.

use std::collections::VecDeque;

use corescout_mirror::{ChannelSpec, Entity, MirrorSnapshot, Relation};

/// One remembered reflection: when, and the numbers.
#[derive(Debug, Clone)]
pub struct Observation {
    pub monotonic_ns: u64,
    pub sequence: u64,
    /// Row-major `entities x channels`, `NaN` for unobserved.
    pub values: Vec<f64>,
}

/// A bounded record of what an observer has seen.
#[derive(Debug)]
pub struct Experience {
    capacity: usize,
    epoch: Option<u64>,
    rows: usize,
    cols: usize,
    entities: Vec<Entity>,
    channels: Vec<ChannelSpec>,
    relations: Vec<Relation>,
    frames: VecDeque<Observation>,
    /// Reflections dropped because the ring was full.
    evicted: u64,
    /// Times the machine's shape changed under this observer.
    epoch_changes: u64,
}

impl Experience {
    pub fn new(capacity: usize) -> Experience {
        Experience {
            capacity: capacity.max(2),
            epoch: None,
            rows: 0,
            cols: 0,
            entities: Vec::new(),
            channels: Vec::new(),
            relations: Vec::new(),
            frames: VecDeque::new(),
            evicted: 0,
            epoch_changes: 0,
        }
    }

    /// Record a reflection.
    ///
    /// A snapshot from a different epoch resets the record: the entity rows it
    /// describes are not the entity rows already stored, and stitching them
    /// into one series would silently compare one piece of hardware against
    /// another.
    pub fn record(&mut self, snapshot: &MirrorSnapshot) {
        if self.epoch != Some(snapshot.epoch) {
            if self.epoch.is_some() {
                self.epoch_changes += 1;
            }
            self.frames.clear();
            self.epoch = Some(snapshot.epoch);
            self.rows = snapshot.state.rows();
            self.cols = snapshot.state.cols();
            self.entities = snapshot.entities.clone();
            self.channels = snapshot.channels.clone();
            self.relations = snapshot.relations.clone();
        }

        if self.frames.len() == self.capacity {
            self.frames.pop_front();
            self.evicted += 1;
        }
        self.frames.push_back(Observation {
            monotonic_ns: snapshot.monotonic_ns,
            sequence: snapshot.sequence,
            values: snapshot.state.as_slice().to_vec(),
        });
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    pub fn channels(&self) -> &[ChannelSpec] {
        &self.channels
    }

    pub fn relations(&self) -> &[Relation] {
        &self.relations
    }

    pub fn epoch(&self) -> Option<u64> {
        self.epoch
    }

    pub fn epoch_changes(&self) -> u64 {
        self.epoch_changes
    }

    pub fn frames(&self) -> impl Iterator<Item = &Observation> {
        self.frames.iter()
    }

    pub fn frame(&self, index: usize) -> Option<&Observation> {
        self.frames.get(index)
    }

    /// The series of one cell over everything witnessed, in order.
    ///
    /// Unobserved samples come through as `NaN`; callers decide whether to drop
    /// them or treat the gap as meaningful, because those are different
    /// questions and the record should not pick one.
    pub fn series(&self, row: usize, col: usize) -> Vec<f64> {
        let index = row * self.cols + col;
        self.frames
            .iter()
            .map(|frame| frame.values.get(index).copied().unwrap_or(f64::NAN))
            .collect()
    }

    /// Elapsed time between the first and last remembered reflection.
    pub fn span_ns(&self) -> u64 {
        match (self.frames.front(), self.frames.back()) {
            (Some(first), Some(last)) => last.monotonic_ns.saturating_sub(first.monotonic_ns),
            _ => 0,
        }
    }

    /// Mean interval between reflections, in nanoseconds.
    pub fn cadence_ns(&self) -> f64 {
        if self.frames.len() < 2 {
            return 0.0;
        }
        self.span_ns() as f64 / (self.frames.len() - 1) as f64
    }

    /// Cells that were observed in every remembered reflection.
    ///
    /// The usable working set: a variable present in only some frames cannot be
    /// correlated against one present in all of them without deciding what the
    /// gaps mean.
    pub fn complete_cells(&self) -> Vec<(usize, usize)> {
        let mut cells = Vec::new();
        for row in 0..self.rows {
            for col in 0..self.cols {
                let index = row * self.cols + col;
                if self
                    .frames
                    .iter()
                    .all(|f| f.values.get(index).is_some_and(|v| !v.is_nan()))
                {
                    cells.push((row, col));
                }
            }
        }
        cells
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::test_support::fixture;

    #[test]
    fn a_full_ring_evicts_the_oldest() {
        let mut experience = Experience::new(3);
        for sequence in 0..5 {
            let mut snapshot = fixture();
            snapshot.sequence = sequence;
            snapshot.monotonic_ns = sequence * 1_000_000;
            experience.record(&snapshot);
        }
        assert_eq!(experience.len(), 3);
        assert_eq!(experience.evicted, 2);
        assert_eq!(experience.frame(0).unwrap().sequence, 2);
    }

    #[test]
    fn an_epoch_change_discards_the_series() {
        // Row 3 before a hotplug and row 3 after are different hardware.
        // Continuing the series would compare one core against another.
        let mut experience = Experience::new(10);
        let mut snapshot = fixture();
        experience.record(&snapshot);
        experience.record(&snapshot);
        assert_eq!(experience.len(), 2);

        snapshot.epoch += 1;
        experience.record(&snapshot);
        assert_eq!(experience.len(), 1, "the old series must not be continued");
        assert_eq!(experience.epoch_changes(), 1);
    }

    #[test]
    fn series_are_extracted_in_time_order() {
        let mut experience = Experience::new(10);
        for step in 0..4u64 {
            let mut snapshot = fixture();
            snapshot.sequence = step;
            snapshot.monotonic_ns = step * 100;
            snapshot.state.set(2, 0, 1000.0 + step as f64);
            experience.record(&snapshot);
        }
        assert_eq!(
            experience.series(2, 0),
            vec![1000.0, 1001.0, 1002.0, 1003.0]
        );
    }

    #[test]
    fn cadence_is_measured_from_the_reflections_themselves() {
        let mut experience = Experience::new(10);
        for step in 0..5u64 {
            let mut snapshot = fixture();
            snapshot.monotonic_ns = step * 10_000_000;
            experience.record(&snapshot);
        }
        assert_eq!(experience.span_ns(), 40_000_000);
        assert!((experience.cadence_ns() - 10_000_000.0).abs() < 1.0);
    }

    #[test]
    fn only_consistently_observed_cells_count_as_complete() {
        let mut experience = Experience::new(10);
        let snapshot = fixture();
        experience.record(&snapshot);
        let mut partial = fixture();
        partial.state.set(2, 0, f64::NAN);
        experience.record(&partial);

        let complete = experience.complete_cells();
        assert!(
            !complete.contains(&(2, 0)),
            "a cell with a gap is not complete"
        );
        assert!(complete.contains(&(2, 1)));
    }
}
