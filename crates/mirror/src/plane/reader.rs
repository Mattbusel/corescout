//! Reading the self-state plane.
//!
//! # What a consumer does
//!
//! ```text
//! open + mmap PROT_READ        once
//! decode the static tables     once per epoch
//! copy the state matrix        every read
//! ```
//!
//! The static tables, entities, relations and channel definitions, are decoded
//! once and cached against the epoch number. A steady-state read is therefore a
//! sequence check, a `memcpy` of the matrix, and a second sequence check. No
//! parsing, no allocation, no syscall.
//!
//! # Read protocol
//!
//! ```text
//! loop {
//!     s1 = seq            if odd, the writer is mid-update: retry
//!     copy the volatile data
//!     s2 = seq            if s2 != s1, it changed under us: retry
//!     done
//! }
//! ```
//!
//! Retries are bounded. A reader that cannot get a consistent view after a
//! generous number of attempts reports that rather than spinning forever,
//! because an unbounded spin in a consumer is exactly the kind of load that
//! would make observation perturb the machine.

use crate::entity::{Entity, EntityClass, EntityId};
use crate::plane::layout::{
    self, channel_record, entity_record, read_string, relation_record, sensor_record, Layout,
};
use crate::plane::memory::PlaneMemory;
use crate::relation::{Relation, RelationKind, RELATION_ATTRS};
use crate::schema::{Availability, AvailabilityMatrix, Perturbation, SensorId, SensorReport};
use crate::snapshot::{MirrorSnapshot, FORMAT_VERSION};
use crate::state::{ChannelId, ChannelSpec, Semantics, StateMatrix, Unit};
use corescout_core::error::{Error, Result};

/// How many times a read will retry before giving up on a consistent view.
///
/// Each retry means the writer published during the copy. At any sane tick rate
/// this should essentially never happen twice, so a hundred failures means
/// something is pathologically wrong and the caller should be told.
const MAX_RETRIES: u32 = 100;

/// A read-only consumer of a plane.
///
/// This type is the entire hardware-introspection budget of a mirror consumer.
/// It reads mapped memory and nothing else: no `/proc`, no `/sys`, no `perf`,
/// no syscalls beyond the initial `open`/`mmap`.
#[derive(Debug)]
pub struct PlaneReader {
    memory: PlaneMemory,
    layout: Layout,
    epoch: u64,
    /// Static tables, decoded once per epoch.
    entities: Vec<Entity>,
    channels: Vec<ChannelSpec>,
    relations: Vec<Relation>,
    sensor_keys: Vec<(SensorId, String)>,
}

impl PlaneReader {
    /// Map an existing plane read-only and decode its static tables.
    pub fn open(path: &std::path::Path) -> Result<PlaneReader> {
        PlaneReader::from_memory(PlaneMemory::open_read_only(path)?)
    }

    /// Attach to an already-obtained region. Used by tests and by in-process
    /// consumers.
    pub fn from_memory(memory: PlaneMemory) -> Result<PlaneReader> {
        let layout = Layout::from_header(memory.as_slice())?;
        let epoch = layout::get_u64(memory.as_slice(), layout::OFF_EPOCH);
        let mut reader = PlaneReader {
            memory,
            layout,
            epoch,
            entities: Vec::new(),
            channels: Vec::new(),
            relations: Vec::new(),
            sensor_keys: Vec::new(),
        };
        reader.decode_static();
        Ok(reader)
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// The entities, in row order. Valid for the current epoch.
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// The channel definitions, in column order.
    pub fn channels(&self) -> &[ChannelSpec] {
        &self.channels
    }

    /// The edges.
    pub fn relations(&self) -> &[Relation] {
        &self.relations
    }

    /// The process publishing this plane, as recorded in the header.
    pub fn writer_pid(&self) -> u32 {
        layout::get_u32(self.memory.as_slice(), layout::OFF_WRITER_PID)
    }

    /// Whether the plane's shape changed since this reader decoded it.
    ///
    /// A consumer caching row indices must check this. It is cheap: one word.
    pub fn epoch_changed(&self) -> bool {
        layout::get_u64(self.memory.as_slice(), layout::OFF_EPOCH) != self.epoch
    }

    /// The backing region, for tests that need to manipulate the seqlock or
    /// the header directly to exercise failure paths.
    #[cfg(test)]
    pub(crate) fn memory_for_test(&self) -> &PlaneMemory {
        &self.memory
    }

    #[cfg(test)]
    pub(crate) fn memory_for_test_mut(&mut self) -> &mut PlaneMemory {
        &mut self.memory
    }

    /// Read the current state, consistently.
    ///
    /// Returns the sequence number, timestamps, the state matrix and the sensor
    /// reports. The static tables are already held by the reader.
    pub fn read_state(&self) -> Result<PlaneState> {
        for _ in 0..MAX_RETRIES {
            let first = self.memory.load_seq();
            if first % 2 != 0 {
                // A write is in progress. Retry rather than read torn data.
                std::hint::spin_loop();
                continue;
            }

            let candidate = self.copy_volatile();

            // Everything above must be ordered before this load, which the
            // acquire on `load_seq` provides.
            let second = self.memory.load_seq();
            if first == second {
                return Ok(candidate);
            }
            std::hint::spin_loop();
        }
        Err(Error::invalid(
            "could not obtain a consistent view of the self-state plane; \
             the writer is publishing faster than this reader can copy",
        ))
    }

    /// Read the current state as a full [`MirrorSnapshot`].
    ///
    /// Allocates. A consumer on a hot path should prefer [`PlaneReader::read_state`]
    /// and index the matrix directly, which is what the layout is designed for.
    pub fn read_snapshot(&self) -> Result<MirrorSnapshot> {
        let state = self.read_state()?;
        let mut matrix = StateMatrix::new(
            self.layout.entity_count as usize,
            self.layout.channel_count as usize,
        );
        matrix.copy_from_slice(&state.values);
        let mut availability = AvailabilityMatrix::new(
            self.layout.entity_count as usize,
            self.layout.channel_count as usize,
        );
        availability.copy_from_slice(&state.availability);
        Ok(MirrorSnapshot {
            format_version: FORMAT_VERSION,
            epoch: self.epoch,
            sequence: state.sequence,
            monotonic_ns: state.monotonic_ns,
            realtime_ns: state.realtime_ns,
            entities: self.entities.clone(),
            channels: self.channels.clone(),
            relations: self.relations.clone(),
            state: matrix,
            availability,
            sensors: state.sensors,
        })
    }

    /// Copy the volatile region once, without checking consistency. Only ever
    /// called between two sequence samples.
    fn copy_volatile(&self) -> PlaneState {
        let buf = self.memory.as_slice();
        let active = layout::get_u32(buf, layout::OFF_ACTIVE_BUFFER) as usize & 1;
        let range = self.layout.state_range(active);

        let cells = (self.layout.entity_count as usize) * (self.layout.channel_count as usize);
        let mut values = Vec::with_capacity(cells);
        for index in 0..cells {
            values.push(layout::get_f64(buf, range.start + index * 8));
        }
        let avail_range = self.layout.avail_range(active);
        let availability = buf[avail_range].to_vec();

        let mut sensors = Vec::with_capacity(self.sensor_keys.len());
        for (index, (id, key)) in self.sensor_keys.iter().enumerate() {
            let cursor = self.layout.sensors_off as usize + index * layout::SENSOR_RECORD;
            sensors.push(SensorReport {
                id: *id,
                key: key.clone(),
                perturbation: Perturbation::from_u8(buf[cursor + sensor_record::PERTURBATION]),
                last_cost_ns: layout::get_u64(buf, cursor + sensor_record::LAST_COST_NS),
                sampling_latency_ns: layout::get_u64(
                    buf,
                    cursor + sensor_record::SAMPLING_LATENCY_NS,
                ),
                sample_age_ns: layout::get_u64(buf, cursor + sensor_record::SAMPLE_AGE_NS),
                samples: layout::get_u32(buf, cursor + sensor_record::SAMPLES),
                errors: layout::get_u32(buf, cursor + sensor_record::ERRORS),
                confidence: layout::get_u32(buf, cursor + sensor_record::CONFIDENCE) as f64
                    / 1_000_000.0,
                availability: Availability::from_u8(buf[cursor + sensor_record::AVAILABILITY]),
                inactive: buf[cursor + sensor_record::FLAGS] & sensor_record::FLAG_INACTIVE != 0,
            });
        }

        PlaneState {
            sequence: layout::get_u64(buf, layout::OFF_SEQUENCE),
            monotonic_ns: layout::get_u64(buf, layout::OFF_MONOTONIC_NS),
            realtime_ns: layout::get_u64(buf, layout::OFF_REALTIME_NS),
            rows: self.layout.entity_count as usize,
            cols: self.layout.channel_count as usize,
            values,
            availability,
            sensors,
        }
    }

    fn decode_static(&mut self) {
        let buf = self.memory.as_slice();
        let strings = &buf[self.layout.strings_range()];

        self.entities.clear();
        for index in 0..self.layout.entity_count as usize {
            let cursor = self.layout.entities_off as usize + index * layout::ENTITY_RECORD;
            let flags = layout::get_u16(buf, cursor + entity_record::FLAGS);
            self.entities.push(Entity {
                id: EntityId(layout::get_u64(buf, cursor + entity_record::ID)),
                key: read_string(
                    strings,
                    layout::get_u32(buf, cursor + entity_record::KEY_OFF),
                    layout::get_u32(buf, cursor + entity_record::KEY_LEN),
                ),
                class_hint: EntityClass::from_u16(layout::get_u16(
                    buf,
                    cursor + entity_record::CLASS,
                )),
                natural_index: if flags & entity_record::FLAG_HAS_NATURAL_INDEX != 0 {
                    Some(layout::get_u32(buf, cursor + entity_record::NATURAL_INDEX))
                } else {
                    None
                },
            });
        }

        self.channels.clear();
        for index in 0..self.layout.channel_count as usize {
            let cursor = self.layout.channels_off as usize + index * layout::CHANNEL_RECORD;
            self.channels.push(ChannelSpec {
                id: ChannelId(index as u16),
                key: read_string(
                    strings,
                    layout::get_u32(buf, cursor + channel_record::KEY_OFF),
                    layout::get_u32(buf, cursor + channel_record::KEY_LEN),
                ),
                unit: Unit::from_u16(layout::get_u16(buf, cursor + channel_record::UNIT)),
                semantics: Semantics::from_u16(layout::get_u16(
                    buf,
                    cursor + channel_record::SEMANTICS,
                )),
                sensor: SensorId(layout::get_u16(buf, cursor + channel_record::SENSOR)),
            });
        }

        self.relations.clear();
        for index in 0..self.layout.relation_count as usize {
            let cursor = self.layout.relations_off as usize + index * layout::RELATION_RECORD;
            let mut attributes = [f64::NAN; RELATION_ATTRS];
            for (slot, attribute) in attributes.iter_mut().enumerate() {
                *attribute = layout::get_f64(buf, cursor + relation_record::ATTRS + slot * 8);
            }
            self.relations.push(Relation {
                source: layout::get_u32(buf, cursor + relation_record::SOURCE),
                target: layout::get_u32(buf, cursor + relation_record::TARGET),
                kind: RelationKind::from_u16(layout::get_u16(buf, cursor + relation_record::KIND)),
                attributes,
            });
        }

        self.sensor_keys.clear();
        for index in 0..self.layout.sensor_count as usize {
            let cursor = self.layout.sensors_off as usize + index * layout::SENSOR_RECORD;
            self.sensor_keys.push((
                SensorId(layout::get_u16(buf, cursor + sensor_record::ID)),
                read_string(
                    strings,
                    layout::get_u32(buf, cursor + sensor_record::KEY_OFF),
                    layout::get_u32(buf, cursor + sensor_record::KEY_LEN),
                ),
            ));
        }
    }
}

/// One consistent read of the volatile part of the plane.
///
/// The matrix is a flat, row-major `f64` slice: exactly the form a numeric
/// consumer wants, with no per-cell decoding.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaneState {
    pub sequence: u64,
    pub monotonic_ns: u64,
    pub realtime_ns: u64,
    pub rows: usize,
    pub cols: usize,
    pub values: Vec<f64>,
    /// One code per cell saying why an empty cell is empty. Same shape and
    /// ordering as `values`.
    pub availability: Vec<u8>,
    pub sensors: Vec<SensorReport>,
}

impl PlaneState {
    /// One entity's state vector.
    pub fn row(&self, row: usize) -> &[f64] {
        let start = row * self.cols;
        &self.values[start..start + self.cols]
    }

    /// Why one cell is empty.
    pub fn availability(&self, row: usize, col: usize) -> Availability {
        if row >= self.rows || col >= self.cols {
            return Availability::NotApplicable;
        }
        Availability::from_u8(self.availability[row * self.cols + col])
    }

    /// One observed value, or `None` if the cell was not observed.
    pub fn value(&self, row: usize, col: usize) -> Option<f64> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        let v = self.values[row * self.cols + col];
        if v.is_nan() {
            None
        } else {
            Some(v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::writer::PlaneWriter;
    use crate::test_support::fixture;
    use std::sync::atomic::Ordering;

    /// Build a writer, then a reader over a copy of its region.
    ///
    /// A copy rather than a shared mapping: these tests exercise encoding,
    /// decoding and the seqlock's decision logic, all of which are
    /// single-threaded. Genuine cross-process sharing is exercised on Linux by
    /// `tests/mirror_plane.rs`.
    fn pair(snapshot: &MirrorSnapshot) -> (PlaneWriter, PlaneReader) {
        let writer = PlaneWriter::anonymous(snapshot).unwrap();
        let bytes = writer.memory_for_test().as_slice().to_vec();
        let mut memory = PlaneMemory::anonymous(bytes.len());
        memory.as_mut_slice().copy_from_slice(&bytes);
        let reader = PlaneReader::from_memory(memory).unwrap();
        (writer, reader)
    }

    #[test]
    fn static_tables_survive_the_binary_round_trip() {
        let snapshot = fixture();
        let (_writer, reader) = pair(&snapshot);

        assert_eq!(reader.entities().len(), snapshot.entities.len());
        for (before, after) in snapshot.entities.iter().zip(reader.entities()) {
            assert_eq!(before.id, after.id, "identity must survive the plane");
            assert_eq!(before.key, after.key);
            assert_eq!(before.class_hint, after.class_hint);
            assert_eq!(before.natural_index, after.natural_index);
        }
        assert_eq!(reader.channels().len(), snapshot.channels.len());
        for (before, after) in snapshot.channels.iter().zip(reader.channels()) {
            assert_eq!(before.key, after.key);
            assert_eq!(before.unit, after.unit);
            assert_eq!(before.semantics, after.semantics);
            assert_eq!(before.sensor, after.sensor);
        }
        assert_eq!(reader.relations(), snapshot.relations.as_slice());
    }

    #[test]
    fn a_published_snapshot_reads_back_identically() {
        let snapshot = fixture();
        let mut writer = PlaneWriter::anonymous(&snapshot).unwrap();
        writer.publish(&snapshot).unwrap();

        let bytes = writer.memory_for_test().as_slice().to_vec();
        let mut memory = PlaneMemory::anonymous(bytes.len());
        memory.as_mut_slice().copy_from_slice(&bytes);
        let reader = PlaneReader::from_memory(memory).unwrap();

        let back = reader.read_snapshot().unwrap();
        assert_eq!(back.sequence, snapshot.sequence);
        assert_eq!(back.monotonic_ns, snapshot.monotonic_ns);
        assert_eq!(back.realtime_ns, snapshot.realtime_ns);
        assert_eq!(back.epoch, snapshot.epoch);
        assert_eq!(back.entities, snapshot.entities);
        assert_eq!(back.relations, snapshot.relations);
        assert_eq!(back.sensors, snapshot.sensors);

        // The numbers, including the holes.
        assert_eq!(
            back.lookup("cpu/0", "cpu.frequency.current"),
            Some(3_600_000.0)
        );
        assert_eq!(back.lookup("cpu/1", "cpu.time.idle"), None);
        assert_eq!(back.state.observed_cells(), snapshot.state.observed_cells());
    }

    #[test]
    fn a_read_during_a_write_is_refused_rather_than_torn() {
        let snapshot = fixture();
        let (_writer, reader) = pair(&snapshot);
        // Simulate a writer that entered a write and never left.
        reader.memory_for_test().seq().store(1, Ordering::Release);
        let err = reader.read_state().unwrap_err();
        assert!(
            err.to_string().contains("consistent view"),
            "a reader must never return data it could not verify: {err}"
        );
    }

    #[test]
    fn a_sequence_change_during_the_copy_is_detected() {
        let snapshot = fixture();
        let (_writer, reader) = pair(&snapshot);
        // An even but perpetually changing counter is the other failure mode:
        // the reader sees a consistent start and a different end every time.
        reader.memory_for_test().seq().store(2, Ordering::Release);
        let state = reader.read_state();
        assert!(state.is_ok(), "a stable even counter must read cleanly");
    }

    #[test]
    fn the_reader_notices_an_epoch_change() {
        let snapshot = fixture();
        let (_writer, mut reader) = pair(&snapshot);
        assert!(!reader.epoch_changed());
        // A new epoch means every cached row index is stale.
        layout::put_u64(
            reader.memory_for_test_mut().as_mut_slice(),
            layout::OFF_EPOCH,
            99,
        );
        assert!(reader.epoch_changed());
    }

    #[test]
    fn plane_state_exposes_rows_and_absent_cells() {
        let snapshot = fixture();
        let mut writer = PlaneWriter::anonymous(&snapshot).unwrap();
        writer.publish(&snapshot).unwrap();
        let bytes = writer.memory_for_test().as_slice().to_vec();
        let mut memory = PlaneMemory::anonymous(bytes.len());
        memory.as_mut_slice().copy_from_slice(&bytes);
        let reader = PlaneReader::from_memory(memory).unwrap();

        let state = reader.read_state().unwrap();
        assert_eq!(state.rows, 4);
        assert_eq!(state.cols, 2);
        assert_eq!(state.row(2).len(), 2);
        assert_eq!(state.value(2, 0), Some(3_600_000.0));
        assert_eq!(state.value(0, 0), None, "unobserved must not become zero");
        assert_eq!(state.value(99, 0), None);
    }

    #[test]
    fn a_reader_cannot_be_opened_over_a_foreign_region() {
        let memory = PlaneMemory::anonymous(4096);
        assert!(PlaneReader::from_memory(memory).is_err());
    }
}
