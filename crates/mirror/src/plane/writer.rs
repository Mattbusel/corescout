//! Publishing snapshots into the self-state plane.
//!
//! Exactly one writer exists per plane: the mirror itself. Everything else in
//! the world is a reader.
//!
//! # Write protocol
//!
//! ```text
//! seq += 1                     (now odd: "a write is in progress")
//! release fence
//! write the inactive state buffer
//! write volatile header fields and the sensor table
//! flip active_buffer
//! release fence
//! seq += 1                     (now even: "consistent")
//! ```
//!
//! The writer never blocks and is never blocked. A reader that catches the
//! region mid-write detects it and retries; it cannot stall the mirror. That
//! asymmetry is deliberate, because a mirror that can be slowed down by being
//! looked at is a mirror whose observation cost depends on how many observers
//! there are.

use std::sync::atomic::{fence, Ordering};

use crate::plane::layout::{
    self, channel_record, entity_record, relation_record, sensor_record, Layout, StringTable,
};
use crate::plane::memory::PlaneMemory;
use crate::schema::SensorReport;
use crate::snapshot::MirrorSnapshot;
use corescout_core::error::{Error, Result};

/// Writes snapshots into a plane region.
#[derive(Debug)]
pub struct PlaneWriter {
    memory: PlaneMemory,
    layout: Layout,
    epoch: u64,
}

impl PlaneWriter {
    /// Create a writer over a region sized for the given snapshot's shape.
    ///
    /// The static tables are written immediately; only volatile data moves on
    /// subsequent publishes.
    pub fn new(
        mut memory_for: impl FnMut(usize) -> Result<PlaneMemory>,
        snapshot: &MirrorSnapshot,
    ) -> Result<PlaneWriter> {
        let (layout, strings) = plan(snapshot);
        let memory = memory_for(layout.total_bytes as usize)?;
        if !memory.is_writable() {
            return Err(Error::invalid(
                "cannot publish into a read-only self-state plane",
            ));
        }
        if memory.len() < layout.total_bytes as usize {
            return Err(Error::invalid(
                "self-state plane region is smaller than the machine it must describe",
            ));
        }
        let mut writer = PlaneWriter {
            memory,
            layout,
            epoch: snapshot.epoch,
        };
        writer.write_static(snapshot, &strings);
        Ok(writer)
    }

    /// Convenience constructor over a private region, for tests and for a mirror
    /// running without publishing.
    pub fn anonymous(snapshot: &MirrorSnapshot) -> Result<PlaneWriter> {
        PlaneWriter::new(|len| Ok(PlaneMemory::anonymous(len)), snapshot)
    }

    /// Total size of the region.
    pub fn size(&self) -> usize {
        self.memory.len()
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Whether this writer's region can still describe the given snapshot.
    ///
    /// A shape change means the static tables are wrong and the region must be
    /// rebuilt. Returning `false` rather than silently reallocating keeps epoch
    /// transitions explicit at the call site.
    pub fn accepts(&self, snapshot: &MirrorSnapshot) -> bool {
        snapshot.epoch == self.epoch
            && snapshot.entities.len() == self.layout.entity_count as usize
            && snapshot.channels.len() == self.layout.channel_count as usize
            && snapshot.relations.len() == self.layout.relation_count as usize
            && snapshot.sensors.len() == self.layout.sensor_count as usize
    }

    /// Publish a snapshot. This is the only mutating operation on the plane.
    pub fn publish(&mut self, snapshot: &MirrorSnapshot) -> Result<()> {
        if !self.accepts(snapshot) {
            return Err(Error::invalid(
                "snapshot shape does not match the plane; the epoch changed and the \
                 region must be rebuilt",
            ));
        }

        let inactive = {
            let buf = self.memory.as_slice();
            (layout::get_u32(buf, layout::OFF_ACTIVE_BUFFER) as usize + 1) & 1
        };

        // Enter the write. From here until seq goes even again, any reader that
        // samples the region will discard what it read.
        let seq = self.memory.load_seq();
        self.memory.seq().store(seq | 1, Ordering::Release);
        fence(Ordering::Release);

        let range = self.layout.state_range(inactive);
        let avail_range = self.layout.avail_range(inactive);
        let values = snapshot.state.as_slice();
        let codes = snapshot.availability.as_slice();
        {
            let buf = self.memory.as_mut_slice();
            for (index, value) in values.iter().enumerate() {
                layout::put_f64(buf, range.start + index * 8, *value);
            }
            debug_assert_eq!(values.len() * 8, self.layout.state_bytes as usize);
            // Reasons go into the same buffer index as the values they explain,
            // so a reader that got a consistent view of one has the other.
            debug_assert_eq!(codes.len(), self.layout.avail_bytes as usize);
            buf[avail_range].copy_from_slice(codes);

            layout::put_u64(buf, layout::OFF_SEQUENCE, snapshot.sequence);
            layout::put_u64(buf, layout::OFF_MONOTONIC_NS, snapshot.monotonic_ns);
            layout::put_u64(buf, layout::OFF_REALTIME_NS, snapshot.realtime_ns);
        }

        self.write_sensor_state(&snapshot.sensors);

        {
            let buf = self.memory.as_mut_slice();
            layout::put_u32(buf, layout::OFF_ACTIVE_BUFFER, inactive as u32);
        }

        // Leave the write. Everything above is visible to any reader that sees
        // this store.
        fence(Ordering::Release);
        self.memory
            .seq()
            .store((seq | 1).wrapping_add(1), Ordering::Release);
        Ok(())
    }

    /// The backing region, for tests that need to inspect or copy raw bytes.
    #[cfg(test)]
    pub(crate) fn memory_for_test(&self) -> &PlaneMemory {
        &self.memory
    }

    /// Write the tables that are constant for the lifetime of an epoch.
    fn write_static(&mut self, snapshot: &MirrorSnapshot, strings: &StringTable) {
        let layout = self.layout;
        let pid = current_pid();
        let buf = self.memory.as_mut_slice();
        buf.fill(0);
        layout.write_header(buf, snapshot.epoch, pid);

        // Entities.
        let mut cursor = layout.entities_off as usize;
        let mut table = StringTable::new();
        for entity in &snapshot.entities {
            let (key_off, key_len) = table.add(&entity.key);
            layout::put_u64(buf, cursor + entity_record::ID, entity.id.raw());
            layout::put_u16(
                buf,
                cursor + entity_record::CLASS,
                entity.class_hint.as_u16(),
            );
            let flags = if entity.natural_index.is_some() {
                entity_record::FLAG_HAS_NATURAL_INDEX
            } else {
                0
            };
            layout::put_u16(buf, cursor + entity_record::FLAGS, flags);
            layout::put_u32(
                buf,
                cursor + entity_record::NATURAL_INDEX,
                entity.natural_index.unwrap_or(0),
            );
            layout::put_u32(buf, cursor + entity_record::KEY_OFF, key_off);
            layout::put_u32(buf, cursor + entity_record::KEY_LEN, key_len);
            cursor += layout::ENTITY_RECORD;
        }

        // Channels.
        cursor = layout.channels_off as usize;
        for channel in &snapshot.channels {
            let (key_off, key_len) = table.add(&channel.key);
            layout::put_u32(buf, cursor + channel_record::KEY_OFF, key_off);
            layout::put_u32(buf, cursor + channel_record::KEY_LEN, key_len);
            layout::put_u16(buf, cursor + channel_record::UNIT, channel.unit.as_u16());
            layout::put_u16(
                buf,
                cursor + channel_record::SEMANTICS,
                channel.semantics.as_u16(),
            );
            layout::put_u16(buf, cursor + channel_record::SENSOR, channel.sensor.0);
            cursor += layout::CHANNEL_RECORD;
        }

        // Relations.
        cursor = layout.relations_off as usize;
        for relation in &snapshot.relations {
            layout::put_u32(buf, cursor + relation_record::SOURCE, relation.source);
            layout::put_u32(buf, cursor + relation_record::TARGET, relation.target);
            layout::put_u16(buf, cursor + relation_record::KIND, relation.kind.as_u16());
            for (slot, value) in relation.attributes.iter().enumerate() {
                layout::put_f64(buf, cursor + relation_record::ATTRS + slot * 8, *value);
            }
            cursor += layout::RELATION_RECORD;
        }

        // Sensor identity. Their per-tick numbers are written by `publish`.
        cursor = layout.sensors_off as usize;
        for sensor in &snapshot.sensors {
            let (key_off, key_len) = table.add(&sensor.key);
            layout::put_u16(buf, cursor + sensor_record::ID, sensor.id.0);
            layout::put_u32(buf, cursor + sensor_record::KEY_OFF, key_off);
            layout::put_u32(buf, cursor + sensor_record::KEY_LEN, key_len);
            cursor += layout::SENSOR_RECORD;
        }

        // The string table itself.
        debug_assert_eq!(
            table.bytes(),
            strings.bytes(),
            "string table planning drifted"
        );
        let strings_range = layout.strings_range();
        buf[strings_range].copy_from_slice(table.bytes());

        // Start life at an even sequence: consistent, and empty.
        self.memory.seq().store(0, Ordering::Release);
    }

    fn write_sensor_state(&mut self, sensors: &[SensorReport]) {
        let base = self.layout.sensors_off as usize;
        let buf = self.memory.as_mut_slice();
        for (index, sensor) in sensors.iter().enumerate() {
            let cursor = base + index * layout::SENSOR_RECORD;
            buf[cursor + sensor_record::PERTURBATION] = sensor.perturbation.as_u8();
            buf[cursor + sensor_record::FLAGS] = if sensor.inactive {
                sensor_record::FLAG_INACTIVE
            } else {
                0
            };
            buf[cursor + sensor_record::AVAILABILITY] = sensor.availability.as_u8();
            layout::put_u64(
                buf,
                cursor + sensor_record::LAST_COST_NS,
                sensor.last_cost_ns,
            );
            layout::put_u32(buf, cursor + sensor_record::SAMPLES, sensor.samples);
            layout::put_u32(buf, cursor + sensor_record::ERRORS, sensor.errors);
            // Confidence in millionths, so the record stays integer-only and
            // its bytes do not depend on float formatting.
            layout::put_u32(
                buf,
                cursor + sensor_record::CONFIDENCE,
                (sensor.confidence.clamp(0.0, 1.0) * 1_000_000.0) as u32,
            );
            layout::put_u64(
                buf,
                cursor + sensor_record::SAMPLE_AGE_NS,
                sensor.sample_age_ns,
            );
            layout::put_u64(
                buf,
                cursor + sensor_record::SAMPLING_LATENCY_NS,
                sensor.sampling_latency_ns,
            );
        }
    }
}

/// Compute the layout and string table for a snapshot's shape.
fn plan(snapshot: &MirrorSnapshot) -> (Layout, StringTable) {
    let mut strings = StringTable::new();
    for entity in &snapshot.entities {
        strings.add(&entity.key);
    }
    for channel in &snapshot.channels {
        strings.add(&channel.key);
    }
    for sensor in &snapshot.sensors {
        strings.add(&sensor.key);
    }
    let layout = Layout::compute(
        snapshot.entities.len(),
        snapshot.channels.len(),
        snapshot.relations.len(),
        snapshot.sensors.len(),
        strings.len(),
    );
    (layout, strings)
}

fn current_pid() -> u32 {
    std::process::id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixture;

    #[test]
    fn a_fresh_plane_is_consistent_and_carries_the_header() {
        let snapshot = fixture();
        let writer = PlaneWriter::anonymous(&snapshot).unwrap();
        let buf = writer.memory.as_slice();
        assert_eq!(layout::get_u64(buf, layout::OFF_MAGIC), layout::MAGIC);
        assert_eq!(layout::get_u64(buf, layout::OFF_EPOCH), 7);
        assert_eq!(
            writer.memory.load_seq() % 2,
            0,
            "a plane must start in a consistent state"
        );
        assert_eq!(layout::get_u32(buf, layout::OFF_ENTITY_COUNT), 4);
        assert_eq!(layout::get_u32(buf, layout::OFF_CHANNEL_COUNT), 2);
    }

    #[test]
    fn publishing_advances_the_sequence_by_two_and_leaves_it_even() {
        let snapshot = fixture();
        let mut writer = PlaneWriter::anonymous(&snapshot).unwrap();
        let before = writer.memory.load_seq();
        writer.publish(&snapshot).unwrap();
        let after = writer.memory.load_seq();
        assert_eq!(after, before + 2);
        assert_eq!(
            after % 2,
            0,
            "a completed write must leave the plane consistent"
        );
    }

    #[test]
    fn publishing_alternates_buffers() {
        let snapshot = fixture();
        let mut writer = PlaneWriter::anonymous(&snapshot).unwrap();
        let active =
            |w: &PlaneWriter| layout::get_u32(w.memory.as_slice(), layout::OFF_ACTIVE_BUFFER);
        writer.publish(&snapshot).unwrap();
        let first = active(&writer);
        writer.publish(&snapshot).unwrap();
        let second = active(&writer);
        assert_ne!(
            first, second,
            "the writer must not write the buffer readers hold"
        );
        writer.publish(&snapshot).unwrap();
        assert_eq!(active(&writer), first);
    }

    #[test]
    fn state_values_reach_the_active_buffer() {
        let snapshot = fixture();
        let mut writer = PlaneWriter::anonymous(&snapshot).unwrap();
        writer.publish(&snapshot).unwrap();
        let active = layout::get_u32(writer.memory.as_slice(), layout::OFF_ACTIVE_BUFFER) as usize;
        let range = writer.layout.state_range(active);
        let buf = writer.memory.as_slice();
        // Row 2, column 0 of the fixture is CPU 0's frequency.
        // Row 2, column 0: entity index 2 times the 2-channel stride.
        let offset = range.start + (2 * 2) * 8;
        assert_eq!(layout::get_f64(buf, offset), 3_600_000.0);
        // Row 0 column 0 was never observed.
        assert!(layout::get_f64(buf, range.start).is_nan());
    }

    #[test]
    fn a_shape_change_is_refused_rather_than_silently_reshaped() {
        let snapshot = fixture();
        let mut writer = PlaneWriter::anonymous(&snapshot).unwrap();
        let mut changed = fixture();
        changed.entities.pop();
        assert!(!writer.accepts(&changed));
        let err = writer.publish(&changed).unwrap_err();
        assert!(err.to_string().contains("epoch changed"));
    }

    #[test]
    fn sensor_costs_are_republished_each_tick() {
        let mut snapshot = fixture();
        let mut writer = PlaneWriter::anonymous(&snapshot).unwrap();
        writer.publish(&snapshot).unwrap();
        snapshot.sensors[0].last_cost_ns = 99_000;
        snapshot.sensors[0].inactive = true;
        writer.publish(&snapshot).unwrap();

        let cursor = writer.layout.sensors_off as usize;
        let buf = writer.memory.as_slice();
        assert_eq!(
            layout::get_u64(buf, cursor + sensor_record::LAST_COST_NS),
            99_000
        );
        assert_eq!(
            buf[cursor + sensor_record::FLAGS] & sensor_record::FLAG_INACTIVE,
            sensor_record::FLAG_INACTIVE
        );
    }

    #[test]
    fn the_region_is_only_as_large_as_the_machine_needs() {
        let snapshot = fixture();
        let writer = PlaneWriter::anonymous(&snapshot).unwrap();
        // Four entities, two channels: this must be kilobytes, not megabytes.
        assert!(writer.size() < 4096, "plane was {} bytes", writer.size());
    }
}
