//! A bounded ring of recent reflections.
//!
//! # Sizing
//!
//! At the default 10 Hz, a 4096-entry ring is roughly seven minutes of machine
//! history. On a 32-CPU desktop each snapshot's volatile part is about 20 KB,
//! so the ring is tens of megabytes: affordable for a daemon, and worth knowing
//! about rather than discovering.
//!
//! The static tables are stored once per epoch rather than per frame, because
//! they do not change within one. That is most of the saving.
//!
//! # Epoch boundaries end a series
//!
//! When the machine's shape changes, row 12 stops meaning what it meant. The
//! ring does not stitch across that: it starts a new segment. Silently
//! continuing would produce a time series that compares one piece of hardware
//! against another, which is the kind of error that never announces itself.

use std::collections::VecDeque;

use corescout_mirror::schema::SensorReport;
use corescout_mirror::{ChannelSpec, Entity, MirrorSnapshot, Relation};

/// One remembered reflection.
///
/// Holds only what changes per tick. The entities, channels and relations live
/// once in the [`History`] that owns this frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub sequence: u64,
    pub monotonic_ns: u64,
    pub realtime_ns: u64,
    /// Row-major `entities x channels`, `NaN` where unobserved.
    pub values: Vec<f64>,
    /// Why each unobserved cell is unobserved.
    pub availability: Vec<u8>,
    pub sensors: Vec<SensorReport>,
}

impl Frame {
    /// Bytes this frame occupies, approximately.
    pub fn footprint(&self) -> usize {
        self.values.len() * std::mem::size_of::<f64>()
            + self.availability.len()
            + self.sensors.len() * std::mem::size_of::<SensorReport>()
    }
}

/// A bounded history of reflections at one epoch.
#[derive(Debug)]
pub struct History {
    capacity: usize,
    epoch: Option<u64>,
    rows: usize,
    cols: usize,
    entities: Vec<Entity>,
    channels: Vec<ChannelSpec>,
    relations: Vec<Relation>,
    frames: VecDeque<Frame>,
    /// Frames dropped because the ring was full.
    evicted: u64,
    /// Times the machine's shape changed under this history.
    epoch_changes: u64,
}

impl History {
    pub fn new(capacity: usize) -> History {
        History {
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
        self.frames.push_back(Frame {
            sequence: snapshot.sequence,
            monotonic_ns: snapshot.monotonic_ns,
            realtime_ns: snapshot.realtime_ns,
            values: snapshot.state.as_slice().to_vec(),
            availability: snapshot.availability.as_slice().to_vec(),
            sensors: snapshot.sensors.clone(),
        });
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn epoch(&self) -> Option<u64> {
        self.epoch
    }

    pub fn epoch_changes(&self) -> u64 {
        self.epoch_changes
    }

    pub fn evicted(&self) -> u64 {
        self.evicted
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

    pub fn frames(&self) -> impl DoubleEndedIterator<Item = &Frame> {
        self.frames.iter()
    }

    pub fn frame(&self, index: usize) -> Option<&Frame> {
        self.frames.get(index)
    }

    pub fn latest(&self) -> Option<&Frame> {
        self.frames.back()
    }

    /// Reconstruct a full snapshot from a remembered frame.
    ///
    /// The static tables come from the history, so this is exactly the
    /// reflection that was recorded, not an approximation of it.
    pub fn snapshot_at(&self, index: usize) -> Option<MirrorSnapshot> {
        let frame = self.frames.get(index)?;
        let mut state = corescout_mirror::StateMatrix::new(self.rows, self.cols);
        state.copy_from_slice(&frame.values);
        let mut availability =
            corescout_mirror::schema::AvailabilityMatrix::new(self.rows, self.cols);
        availability.copy_from_slice(&frame.availability);
        Some(MirrorSnapshot {
            format_version: corescout_mirror::FORMAT_VERSION,
            epoch: self.epoch.unwrap_or(0),
            sequence: frame.sequence,
            monotonic_ns: frame.monotonic_ns,
            realtime_ns: frame.realtime_ns,
            entities: self.entities.clone(),
            channels: self.channels.clone(),
            relations: self.relations.clone(),
            state,
            availability,
            sensors: frame.sensors.clone(),
        })
    }

    /// The reflection nearest to `monotonic_ns`, and how far off it was.
    ///
    /// The query "what did I look like 500 ms ago" resolves to a real recorded
    /// reflection plus the error in that answer, rather than to an interpolated
    /// state that never existed.
    pub fn nearest(&self, monotonic_ns: u64) -> Option<(&Frame, u64)> {
        self.frames
            .iter()
            .map(|frame| {
                let delta = frame.monotonic_ns.abs_diff(monotonic_ns);
                (frame, delta)
            })
            .min_by_key(|(_, delta)| *delta)
    }

    /// The reflection from approximately `ago_ns` before the latest one.
    pub fn ago(&self, ago_ns: u64) -> Option<(&Frame, u64)> {
        let latest = self.frames.back()?;
        self.nearest(latest.monotonic_ns.saturating_sub(ago_ns))
    }

    /// The series of one cell over the whole ring, oldest first.
    pub fn series(&self, row: usize, col: usize) -> Vec<f64> {
        if self.cols == 0 {
            return Vec::new();
        }
        let index = row * self.cols + col;
        self.frames
            .iter()
            .map(|frame| frame.values.get(index).copied().unwrap_or(f64::NAN))
            .collect()
    }

    /// Timestamps of the frames, aligned with [`History::series`].
    pub fn timeline(&self) -> Vec<u64> {
        self.frames.iter().map(|f| f.monotonic_ns).collect()
    }

    /// Elapsed time between the first and last remembered reflection.
    pub fn span_ns(&self) -> u64 {
        match (self.frames.front(), self.frames.back()) {
            (Some(first), Some(last)) => last.monotonic_ns.saturating_sub(first.monotonic_ns),
            _ => 0,
        }
    }

    /// Mean interval between reflections.
    pub fn cadence_ns(&self) -> f64 {
        if self.frames.len() < 2 {
            return 0.0;
        }
        self.span_ns() as f64 / (self.frames.len() - 1) as f64
    }

    /// Approximate bytes held.
    pub fn footprint(&self) -> usize {
        self.frames.iter().map(|f| f.footprint()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::test_support::fixture;

    fn at(sequence: u64, monotonic_ns: u64) -> MirrorSnapshot {
        let mut snapshot = fixture();
        snapshot.sequence = sequence;
        snapshot.monotonic_ns = monotonic_ns;
        snapshot
    }

    #[test]
    fn the_ring_is_bounded_and_evicts_the_oldest() {
        let mut history = History::new(3);
        for i in 0..6 {
            history.record(&at(i, i * 1_000_000));
        }
        assert_eq!(history.len(), 3);
        assert_eq!(history.evicted(), 3);
        assert_eq!(history.frame(0).unwrap().sequence, 3);
    }

    #[test]
    fn an_epoch_change_starts_a_new_series() {
        let mut history = History::new(10);
        history.record(&at(1, 1_000));
        history.record(&at(2, 2_000));
        let mut changed = at(3, 3_000);
        changed.epoch += 1;
        history.record(&changed);

        assert_eq!(history.len(), 1, "the previous series must not continue");
        assert_eq!(history.epoch_changes(), 1);
    }

    #[test]
    fn a_remembered_frame_reconstructs_the_reflection_exactly() {
        let mut history = History::new(10);
        let original = at(7, 7_000);
        history.record(&original);
        let back = history.snapshot_at(0).expect("frame 0");
        assert_eq!(back.sequence, original.sequence);
        assert_eq!(back.entities, original.entities);
        assert_eq!(back.relations, original.relations);
        assert_eq!(back.state, original.state);
        assert_eq!(back.availability, original.availability);
    }

    #[test]
    fn what_did_i_look_like_500ms_ago() {
        let mut history = History::new(100);
        for i in 0..20u64 {
            history.record(&at(i, i * 100_000_000)); // 10 Hz
        }
        let (frame, error) = history.ago(500_000_000).expect("a frame 500 ms back");
        assert_eq!(frame.sequence, 14, "five ticks before the latest");
        assert_eq!(error, 0, "the sample lands exactly on a tick here");
    }

    #[test]
    fn the_error_in_a_temporal_query_is_reported_rather_than_hidden() {
        // Ask for a moment between two samples: the answer is a real frame plus
        // how far off it is, never an interpolated state that never existed.
        let mut history = History::new(10);
        history.record(&at(0, 0));
        history.record(&at(1, 100_000_000));
        let (frame, error) = history.nearest(60_000_000).unwrap();
        assert_eq!(frame.sequence, 1);
        assert_eq!(error, 40_000_000);
    }

    #[test]
    fn series_and_timeline_line_up() {
        let mut history = History::new(10);
        for i in 0..5u64 {
            let mut snapshot = at(i, i * 1_000_000);
            snapshot.state.set(2, 0, 1000.0 + i as f64);
            history.record(&snapshot);
        }
        assert_eq!(
            history.series(2, 0),
            vec![1000.0, 1001.0, 1002.0, 1003.0, 1004.0]
        );
        assert_eq!(history.timeline().len(), 5);
        assert_eq!(history.cadence_ns(), 1_000_000.0);
    }

    #[test]
    fn an_empty_history_answers_without_panicking() {
        let history = History::new(10);
        assert!(history.is_empty());
        assert!(history.latest().is_none());
        assert!(history.ago(1_000).is_none());
        assert!(history.series(0, 0).is_empty());
        assert_eq!(history.span_ns(), 0);
        assert_eq!(history.cadence_ns(), 0.0);
    }
}
