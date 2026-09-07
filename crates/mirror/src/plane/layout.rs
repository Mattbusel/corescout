//! Binary layout of the self-state plane.
//!
//! # The target
//!
//! > Reading the machine's current self-state should feel closer to accessing
//! > memory than querying a monitoring service.
//!
//! So the plane is a fixed-layout region of shared memory, not a protocol. A
//! consumer maps it once, and thereafter reading the whole machine's state is a
//! `memcpy` of a contiguous `f64` matrix. There is no parsing, no allocation, no
//! syscall, and no negotiation on the read path.
//!
//! # Layout
//!
//! ```text
//! +------------------------------------------+
//! | header            256 B  seqlock, counts |  volatile
//! +------------------------------------------+
//! | entity records    32 B each               |  static within an epoch
//! | channel records   32 B each               |  static within an epoch
//! | relation records  48 B each               |  static within an epoch
//! | string table      UTF-8 blob              |  static within an epoch
//! +------------------------------------------+
//! | sensor records    48 B each               |  volatile
//! +------------------------------------------+
//! | state matrix A    rows x cols x 8 B       |  volatile, double buffered
//! | state matrix B    rows x cols x 8 B       |
//! +------------------------------------------+
//! ```
//!
//! The split between static and volatile is the layout's main idea. Entities,
//! relations and channel definitions change only when the machine's shape
//! changes, so they are written once per epoch and can be cached by a consumer
//! for as long as the epoch number holds. Only the matrix and a handful of
//! header words move on each tick.
//!
//! # Concurrency
//!
//! A **seqlock**, plus double buffering for the matrix.
//!
//! The writer bumps `seq` to an odd value, writes, then bumps it to the next
//! even value. A reader samples `seq`, copies, and samples `seq` again; if the
//! value changed or was odd, the data may be torn and the read is retried. This
//! gives wait-free readers that never block the writer and never need a lock,
//! which matters because a reader stalling the mirror would be an observer
//! perturbing the observed.
//!
//! Double buffering the matrix means the writer is never writing the same bytes
//! a reader is copying, so the large copy essentially never triggers a retry.
//! The seqlock still covers it, because a reader slow enough to span two whole
//! ticks would otherwise read a buffer that had been recycled underneath it.
//!
//! # Endianness and alignment
//!
//! Little-endian throughout, since the plane is shared between processes on one
//! machine and every target is little-endian. All sections are 64-byte aligned,
//! which keeps records off shared cache lines with the header and makes the
//! matrix cache-line aligned for a consumer that wants to vectorise over it.
//!
//! # Forward compatibility
//!
//! A consumer checks `magic` and `format_version` and refuses anything it does
//! not recognise. It must not attempt a best-effort parse: silently misreading a
//! machine's state is worse than declining to read it.

use corescout_core::error::{Error, Result};

/// `CSMIRROR` in little-endian bytes. Identifies the region as a plane.
pub const MAGIC: u64 = u64::from_le_bytes(*b"CSMIRROR");

/// Size of the fixed header.
pub const HEADER_BYTES: usize = 256;

/// Size of one entity record.
pub const ENTITY_RECORD: usize = 32;
/// Size of one channel record.
pub const CHANNEL_RECORD: usize = 32;
/// Size of one relation record.
pub const RELATION_RECORD: usize = 48;
/// Size of one sensor record.
pub const SENSOR_RECORD: usize = 64;

/// Alignment of every section.
pub const SECTION_ALIGN: usize = 64;

// ---------------------------------------------------------------------------
// Header field offsets. Explicit constants rather than a `#[repr(C)]` struct:
// the layout is a cross-process ABI, and a struct definition would let a
// compiler's padding decisions silently become part of it.
// ---------------------------------------------------------------------------

pub const OFF_MAGIC: usize = 0;
pub const OFF_FORMAT_VERSION: usize = 8;
pub const OFF_HEADER_BYTES: usize = 12;
pub const OFF_TOTAL_BYTES: usize = 16;
pub const OFF_EPOCH: usize = 24;
/// The seqlock counter. Odd means a write is in progress.
pub const OFF_SEQ: usize = 32;
pub const OFF_SEQUENCE: usize = 40;
pub const OFF_MONOTONIC_NS: usize = 48;
pub const OFF_REALTIME_NS: usize = 56;
pub const OFF_ENTITY_COUNT: usize = 64;
pub const OFF_CHANNEL_COUNT: usize = 68;
pub const OFF_RELATION_COUNT: usize = 72;
pub const OFF_SENSOR_COUNT: usize = 76;
pub const OFF_ENTITIES_OFF: usize = 80;
pub const OFF_CHANNELS_OFF: usize = 84;
pub const OFF_RELATIONS_OFF: usize = 88;
pub const OFF_SENSORS_OFF: usize = 92;
pub const OFF_STRINGS_OFF: usize = 96;
pub const OFF_STRINGS_LEN: usize = 100;
pub const OFF_STATE_OFF_A: usize = 104;
pub const OFF_STATE_OFF_B: usize = 108;
pub const OFF_STATE_STRIDE: usize = 112;
pub const OFF_ACTIVE_BUFFER: usize = 116;
pub const OFF_WRITER_PID: usize = 120;
pub const OFF_FLAGS: usize = 124;
/// Offsets of the two availability-code buffers, parallel to the state buffers.
pub const OFF_AVAIL_OFF_A: usize = 128;
pub const OFF_AVAIL_OFF_B: usize = 132;
pub const OFF_AVAIL_BYTES: usize = 136;

/// Computed section offsets for a given machine shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub entity_count: u32,
    pub channel_count: u32,
    pub relation_count: u32,
    pub sensor_count: u32,
    pub strings_len: u32,
    pub entities_off: u32,
    pub channels_off: u32,
    pub relations_off: u32,
    pub strings_off: u32,
    pub sensors_off: u32,
    pub state_off: [u32; 2],
    /// Bytes per matrix row.
    pub state_stride: u32,
    /// Bytes in one state buffer.
    pub state_bytes: u32,
    /// Offsets of the two availability buffers. One byte per cell, saying why
    /// an empty cell is empty.
    pub avail_off: [u32; 2],
    /// Bytes in one availability buffer.
    pub avail_bytes: u32,
    pub total_bytes: u32,
}

impl Layout {
    /// Compute the layout for a machine of the given shape.
    pub fn compute(
        entity_count: usize,
        channel_count: usize,
        relation_count: usize,
        sensor_count: usize,
        strings_len: usize,
    ) -> Layout {
        let mut cursor = HEADER_BYTES;
        let entities_off = cursor;
        cursor = align(cursor + entity_count * ENTITY_RECORD);
        let channels_off = cursor;
        cursor = align(cursor + channel_count * CHANNEL_RECORD);
        let relations_off = cursor;
        cursor = align(cursor + relation_count * RELATION_RECORD);
        let strings_off = cursor;
        cursor = align(cursor + strings_len);
        let sensors_off = cursor;
        cursor = align(cursor + sensor_count * SENSOR_RECORD);

        let state_stride = channel_count * std::mem::size_of::<f64>();
        let state_bytes = entity_count * state_stride;
        let state_a = cursor;
        cursor = align(cursor + state_bytes);
        let state_b = cursor;
        cursor = align(cursor + state_bytes);

        // The reasons travel with the numbers, and are double buffered for the
        // same reason: a reader must never pair this tick's values with last
        // tick's explanations.
        let avail_bytes = entity_count * channel_count;
        let avail_a = cursor;
        cursor = align(cursor + avail_bytes);
        let avail_b = cursor;
        cursor = align(cursor + avail_bytes);

        Layout {
            entity_count: entity_count as u32,
            channel_count: channel_count as u32,
            relation_count: relation_count as u32,
            sensor_count: sensor_count as u32,
            strings_len: strings_len as u32,
            entities_off: entities_off as u32,
            channels_off: channels_off as u32,
            relations_off: relations_off as u32,
            strings_off: strings_off as u32,
            sensors_off: sensors_off as u32,
            state_off: [state_a as u32, state_b as u32],
            state_stride: state_stride as u32,
            state_bytes: state_bytes as u32,
            avail_off: [avail_a as u32, avail_b as u32],
            avail_bytes: avail_bytes as u32,
            total_bytes: cursor as u32,
        }
    }

    /// Read the layout back out of an existing plane's header.
    pub fn from_header(buf: &[u8]) -> Result<Layout> {
        if buf.len() < HEADER_BYTES {
            return Err(Error::invalid(
                "self-state plane is smaller than its header",
            ));
        }
        let magic = get_u64(buf, OFF_MAGIC);
        if magic != MAGIC {
            return Err(Error::invalid(
                "not a CoreScout self-state plane (bad magic)",
            ));
        }
        let version = get_u32(buf, OFF_FORMAT_VERSION);
        if version != crate::snapshot::FORMAT_VERSION {
            return Err(Error::invalid(format!(
                "self-state plane format version {version} is not version {}; \
                 refusing to guess at its layout",
                crate::snapshot::FORMAT_VERSION
            )));
        }

        let layout = Layout {
            entity_count: get_u32(buf, OFF_ENTITY_COUNT),
            channel_count: get_u32(buf, OFF_CHANNEL_COUNT),
            relation_count: get_u32(buf, OFF_RELATION_COUNT),
            sensor_count: get_u32(buf, OFF_SENSOR_COUNT),
            strings_len: get_u32(buf, OFF_STRINGS_LEN),
            entities_off: get_u32(buf, OFF_ENTITIES_OFF),
            channels_off: get_u32(buf, OFF_CHANNELS_OFF),
            relations_off: get_u32(buf, OFF_RELATIONS_OFF),
            strings_off: get_u32(buf, OFF_STRINGS_OFF),
            sensors_off: get_u32(buf, OFF_SENSORS_OFF),
            state_off: [get_u32(buf, OFF_STATE_OFF_A), get_u32(buf, OFF_STATE_OFF_B)],
            state_stride: get_u32(buf, OFF_STATE_STRIDE),
            state_bytes: get_u32(buf, OFF_ENTITY_COUNT)
                .saturating_mul(get_u32(buf, OFF_STATE_STRIDE)),
            avail_off: [get_u32(buf, OFF_AVAIL_OFF_A), get_u32(buf, OFF_AVAIL_OFF_B)],
            avail_bytes: get_u32(buf, OFF_AVAIL_BYTES),
            total_bytes: get_u32(buf, OFF_TOTAL_BYTES),
        };
        layout.validate(buf.len())?;
        Ok(layout)
    }

    /// Check that every section named by the header actually fits.
    ///
    /// A consumer maps memory another process wrote. Validating the header
    /// before trusting its offsets is what stops a corrupt or truncated plane
    /// from turning into an out-of-bounds read.
    pub fn validate(&self, available: usize) -> Result<()> {
        let sections: [(u32, u64); 5] = [
            (
                self.entities_off,
                self.entity_count as u64 * ENTITY_RECORD as u64,
            ),
            (
                self.channels_off,
                self.channel_count as u64 * CHANNEL_RECORD as u64,
            ),
            (
                self.relations_off,
                self.relation_count as u64 * RELATION_RECORD as u64,
            ),
            (self.strings_off, self.strings_len as u64),
            (
                self.sensors_off,
                self.sensor_count as u64 * SENSOR_RECORD as u64,
            ),
        ];
        for (offset, len) in sections {
            let end = offset as u64 + len;
            if end > available as u64 {
                return Err(Error::invalid(
                    "self-state plane header describes a section past the end of the mapping",
                ));
            }
        }
        let expected_stride = self.channel_count as u64 * 8;
        if self.state_stride as u64 != expected_stride {
            return Err(Error::invalid(
                "self-state plane row stride does not match its channel count",
            ));
        }
        for offset in self.state_off {
            let end = offset as u64 + self.state_bytes as u64;
            if end > available as u64 {
                return Err(Error::invalid(
                    "self-state plane state buffer extends past the end of the mapping",
                ));
            }
        }
        if self.avail_bytes as u64 != self.entity_count as u64 * self.channel_count as u64 {
            return Err(Error::invalid(
                "self-state plane availability region does not match its matrix shape",
            ));
        }
        for offset in self.avail_off {
            let end = offset as u64 + self.avail_bytes as u64;
            if end > available as u64 {
                return Err(Error::invalid(
                    "self-state plane availability buffer extends past the end of the mapping",
                ));
            }
        }
        Ok(())
    }

    /// Byte range of one state buffer.
    pub fn state_range(&self, buffer: usize) -> std::ops::Range<usize> {
        let start = self.state_off[buffer & 1] as usize;
        start..start + self.state_bytes as usize
    }

    /// Byte range of one availability buffer.
    pub fn avail_range(&self, buffer: usize) -> std::ops::Range<usize> {
        let start = self.avail_off[buffer & 1] as usize;
        start..start + self.avail_bytes as usize
    }

    /// Byte range of the entity table.
    pub fn entities_range(&self) -> std::ops::Range<usize> {
        let start = self.entities_off as usize;
        start..start + self.entity_count as usize * ENTITY_RECORD
    }

    pub fn channels_range(&self) -> std::ops::Range<usize> {
        let start = self.channels_off as usize;
        start..start + self.channel_count as usize * CHANNEL_RECORD
    }

    pub fn relations_range(&self) -> std::ops::Range<usize> {
        let start = self.relations_off as usize;
        start..start + self.relation_count as usize * RELATION_RECORD
    }

    pub fn sensors_range(&self) -> std::ops::Range<usize> {
        let start = self.sensors_off as usize;
        start..start + self.sensor_count as usize * SENSOR_RECORD
    }

    pub fn strings_range(&self) -> std::ops::Range<usize> {
        let start = self.strings_off as usize;
        start..start + self.strings_len as usize
    }

    /// Write the layout and the constant header fields.
    pub fn write_header(&self, buf: &mut [u8], epoch: u64, writer_pid: u32) {
        put_u64(buf, OFF_MAGIC, MAGIC);
        put_u32(buf, OFF_FORMAT_VERSION, crate::snapshot::FORMAT_VERSION);
        put_u32(buf, OFF_HEADER_BYTES, HEADER_BYTES as u32);
        put_u64(buf, OFF_TOTAL_BYTES, self.total_bytes as u64);
        put_u64(buf, OFF_EPOCH, epoch);
        put_u32(buf, OFF_ENTITY_COUNT, self.entity_count);
        put_u32(buf, OFF_CHANNEL_COUNT, self.channel_count);
        put_u32(buf, OFF_RELATION_COUNT, self.relation_count);
        put_u32(buf, OFF_SENSOR_COUNT, self.sensor_count);
        put_u32(buf, OFF_ENTITIES_OFF, self.entities_off);
        put_u32(buf, OFF_CHANNELS_OFF, self.channels_off);
        put_u32(buf, OFF_RELATIONS_OFF, self.relations_off);
        put_u32(buf, OFF_SENSORS_OFF, self.sensors_off);
        put_u32(buf, OFF_STRINGS_OFF, self.strings_off);
        put_u32(buf, OFF_STRINGS_LEN, self.strings_len);
        put_u32(buf, OFF_STATE_OFF_A, self.state_off[0]);
        put_u32(buf, OFF_STATE_OFF_B, self.state_off[1]);
        put_u32(buf, OFF_STATE_STRIDE, self.state_stride);
        put_u32(buf, OFF_AVAIL_OFF_A, self.avail_off[0]);
        put_u32(buf, OFF_AVAIL_OFF_B, self.avail_off[1]);
        put_u32(buf, OFF_AVAIL_BYTES, self.avail_bytes);
        put_u32(buf, OFF_WRITER_PID, writer_pid);
    }
}

fn align(value: usize) -> usize {
    value.div_ceil(SECTION_ALIGN) * SECTION_ALIGN
}

// ---------------------------------------------------------------------------
// Record encoding. Field offsets within each record are local constants,
// documented here rather than spread through the writer and reader.
// ---------------------------------------------------------------------------

/// Entity record: `id | class | flags | natural_index | key_off | key_len | pad`.
pub mod entity_record {
    pub const ID: usize = 0;
    pub const CLASS: usize = 8;
    pub const FLAGS: usize = 10;
    pub const NATURAL_INDEX: usize = 12;
    pub const KEY_OFF: usize = 16;
    pub const KEY_LEN: usize = 20;
    /// Set when `natural_index` carries a real value.
    pub const FLAG_HAS_NATURAL_INDEX: u16 = 1;
}

/// Channel record: `key_off | key_len | unit | semantics | sensor | flags | pad`.
pub mod channel_record {
    pub const KEY_OFF: usize = 0;
    pub const KEY_LEN: usize = 4;
    pub const UNIT: usize = 8;
    pub const SEMANTICS: usize = 10;
    pub const SENSOR: usize = 12;
    pub const FLAGS: usize = 14;
}

/// Relation record: `source | target | kind | flags | attributes[4]`.
pub mod relation_record {
    pub const SOURCE: usize = 0;
    pub const TARGET: usize = 4;
    pub const KIND: usize = 8;
    pub const FLAGS: usize = 10;
    pub const ATTRS: usize = 16;
}

/// Sensor record: identity plus what the last observation pass cost.
pub mod sensor_record {
    pub const ID: usize = 0;
    pub const PERTURBATION: usize = 2;
    pub const FLAGS: usize = 3;
    pub const KEY_OFF: usize = 4;
    pub const KEY_LEN: usize = 8;
    pub const LAST_COST_NS: usize = 16;
    pub const SAMPLES: usize = 24;
    pub const ERRORS: usize = 28;
    /// Confidence in millionths, so the record stays integer-only.
    pub const CONFIDENCE: usize = 32;
    pub const AVAILABILITY: usize = 36;
    pub const SAMPLE_AGE_NS: usize = 40;
    pub const SAMPLING_LATENCY_NS: usize = 48;
    /// Set when the sensor failed to bind and its columns are permanently
    /// unobserved for this epoch.
    pub const FLAG_INACTIVE: u8 = 1;
}

// ---------------------------------------------------------------------------
// Little-endian primitives.
// ---------------------------------------------------------------------------

#[inline]
pub fn get_u16(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

#[inline]
pub fn get_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().expect("4 bytes"))
}

#[inline]
pub fn get_u64(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(buf[off..off + 8].try_into().expect("8 bytes"))
}

#[inline]
pub fn get_f64(buf: &[u8], off: usize) -> f64 {
    f64::from_le_bytes(buf[off..off + 8].try_into().expect("8 bytes"))
}

#[inline]
pub fn put_u16(buf: &mut [u8], off: usize, value: u16) {
    buf[off..off + 2].copy_from_slice(&value.to_le_bytes());
}

#[inline]
pub fn put_u32(buf: &mut [u8], off: usize, value: u32) {
    buf[off..off + 4].copy_from_slice(&value.to_le_bytes());
}

#[inline]
pub fn put_u64(buf: &mut [u8], off: usize, value: u64) {
    buf[off..off + 8].copy_from_slice(&value.to_le_bytes());
}

#[inline]
pub fn put_f64(buf: &mut [u8], off: usize, value: f64) {
    buf[off..off + 8].copy_from_slice(&value.to_le_bytes());
}

/// Builds the string table, returning `(offset, length)` for each string added.
#[derive(Debug, Default)]
pub struct StringTable {
    bytes: Vec<u8>,
}

impl StringTable {
    pub fn new() -> StringTable {
        StringTable::default()
    }

    /// Add a string, returning its `(offset, length)` within the table.
    pub fn add(&mut self, value: &str) -> (u32, u32) {
        let offset = self.bytes.len() as u32;
        self.bytes.extend_from_slice(value.as_bytes());
        (offset, value.len() as u32)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Read a string out of a plane's string table.
///
/// Returns an empty string for a range that does not fit or is not UTF-8, since
/// a label is an annotation: a corrupt one must not stop a consumer reading the
/// numbers, which are what actually matter.
pub fn read_string(strings: &[u8], offset: u32, len: u32) -> String {
    let (start, end) = (offset as usize, offset as usize + len as usize);
    if end > strings.len() {
        return String::new();
    }
    String::from_utf8_lossy(&strings[start..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_is_the_expected_ascii() {
        assert_eq!(&MAGIC.to_le_bytes(), b"CSMIRROR");
    }

    #[test]
    fn sections_are_aligned_and_ordered() {
        let layout = Layout::compute(70, 40, 300, 7, 900);
        for offset in [
            layout.entities_off,
            layout.channels_off,
            layout.relations_off,
            layout.strings_off,
            layout.sensors_off,
            layout.state_off[0],
            layout.state_off[1],
        ] {
            assert_eq!(offset as usize % SECTION_ALIGN, 0, "unaligned section");
        }
        assert!(layout.entities_off >= HEADER_BYTES as u32);
        assert!(layout.channels_off > layout.entities_off);
        assert!(layout.state_off[1] > layout.state_off[0]);
        assert!(layout.total_bytes >= layout.state_off[1] + layout.state_bytes);
    }

    #[test]
    fn state_buffers_do_not_overlap() {
        let layout = Layout::compute(64, 32, 100, 5, 500);
        let a = layout.state_range(0);
        let b = layout.state_range(1);
        assert!(a.end <= b.start, "double buffers must be disjoint");
        assert_eq!(a.len(), b.len());
        assert_eq!(a.len(), 64 * 32 * 8);
    }

    #[test]
    fn header_round_trips() {
        let layout = Layout::compute(10, 5, 20, 3, 64);
        let mut buf = vec![0u8; layout.total_bytes as usize];
        layout.write_header(&mut buf, 9, 1234);
        let back = Layout::from_header(&buf).unwrap();
        assert_eq!(back.entity_count, 10);
        assert_eq!(back.channel_count, 5);
        assert_eq!(back.relation_count, 20);
        assert_eq!(back.sensor_count, 3);
        assert_eq!(back.state_off, layout.state_off);
        assert_eq!(get_u64(&buf, OFF_EPOCH), 9);
        assert_eq!(get_u32(&buf, OFF_WRITER_PID), 1234);
    }

    #[test]
    fn a_foreign_region_is_refused_rather_than_parsed() {
        let buf = vec![0u8; 4096];
        let err = Layout::from_header(&buf).unwrap_err();
        assert!(err.to_string().contains("bad magic"));
    }

    #[test]
    fn a_future_format_version_is_refused() {
        let layout = Layout::compute(2, 2, 0, 1, 0);
        let mut buf = vec![0u8; layout.total_bytes as usize];
        layout.write_header(&mut buf, 1, 0);
        put_u32(&mut buf, OFF_FORMAT_VERSION, 999);
        let err = Layout::from_header(&buf).unwrap_err();
        assert!(err.to_string().contains("refusing to guess"), "got: {err}");
    }

    #[test]
    fn a_truncated_plane_is_refused() {
        let layout = Layout::compute(64, 32, 10, 2, 100);
        let mut buf = vec![0u8; layout.total_bytes as usize];
        layout.write_header(&mut buf, 1, 0);
        // Present the same header over a mapping that is far too small.
        let truncated = &buf[..HEADER_BYTES + 8];
        let err = Layout::from_header(truncated).unwrap_err();
        assert!(err.to_string().contains("past the end"), "got: {err}");
    }

    #[test]
    fn an_inconsistent_stride_is_refused() {
        // A header claiming more channels than its rows can hold would make a
        // consumer read past the end of every row.
        let layout = Layout::compute(4, 4, 0, 1, 0);
        let mut buf = vec![0u8; layout.total_bytes as usize];
        layout.write_header(&mut buf, 1, 0);
        put_u32(&mut buf, OFF_CHANNEL_COUNT, 8);
        let err = Layout::from_header(&buf).unwrap_err();
        assert!(err.to_string().contains("row stride"), "got: {err}");
    }

    #[test]
    fn primitives_round_trip() {
        let mut buf = vec![0u8; 64];
        put_u16(&mut buf, 0, 0xBEEF);
        put_u32(&mut buf, 8, 0xDEAD_BEEF);
        put_u64(&mut buf, 16, 0x0123_4567_89AB_CDEF);
        put_f64(&mut buf, 32, -1.5);
        put_f64(&mut buf, 40, f64::NAN);
        assert_eq!(get_u16(&buf, 0), 0xBEEF);
        assert_eq!(get_u32(&buf, 8), 0xDEAD_BEEF);
        assert_eq!(get_u64(&buf, 16), 0x0123_4567_89AB_CDEF);
        assert_eq!(get_f64(&buf, 32), -1.5);
        assert!(
            get_f64(&buf, 40).is_nan(),
            "unobserved cells must survive the binary form exactly"
        );
    }

    #[test]
    fn string_table_offsets_are_correct() {
        let mut table = StringTable::new();
        let (a_off, a_len) = table.add("cpu/0");
        let (b_off, b_len) = table.add("cpu/11");
        assert_eq!((a_off, a_len), (0, 5));
        assert_eq!((b_off, b_len), (5, 6));
        assert_eq!(read_string(table.bytes(), a_off, a_len), "cpu/0");
        assert_eq!(read_string(table.bytes(), b_off, b_len), "cpu/11");
    }

    #[test]
    fn a_corrupt_string_range_yields_an_empty_label_not_a_panic() {
        let table = StringTable::new();
        assert_eq!(read_string(table.bytes(), 100, 10), "");
    }

    #[test]
    fn an_empty_machine_still_produces_a_valid_layout() {
        let layout = Layout::compute(0, 0, 0, 0, 0);
        let mut buf = vec![0u8; layout.total_bytes.max(HEADER_BYTES as u32) as usize];
        layout.write_header(&mut buf, 0, 0);
        assert!(Layout::from_header(&buf).is_ok());
    }
}
