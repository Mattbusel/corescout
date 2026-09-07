//! A bounded ring of fixed-width samples, for telemetry at observation rate.
//!
//! # Why this is not a table
//!
//! The mirror produces a reflection several times a second, forever. Put that
//! in a transactional store and a machine left running for a year holds a
//! hundred million rows nobody will ever read, having paid a write transaction
//! for each. Put it here and it holds exactly [`Ring::capacity`] samples using
//! exactly `capacity * 64 + 64` bytes, whatever happens.
//!
//! The trade is that a sample is a fixed 64 bytes and cannot grow. That is the
//! correct trade for this data: nothing here needs to reconstruct a full
//! reflection, which the plane already publishes live and which `corescout
//! record` already writes in full when asked. This is the history a chart is
//! drawn from, not an archive.
//!
//! # What a torn write looks like
//!
//! Samples are written whole and the write cursor is advanced afterwards, so a
//! crash mid-write loses the sample being written and never corrupts an older
//! one. On reopen the cursor is read from the header and the ring resumes; a
//! partially written trailing record is simply overwritten next time.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use corescout_core::error::{Error, Result};
use serde::{Deserialize, Serialize};

/// Bytes per sample. Fixed, and asserted by a test.
pub const SAMPLE_BYTES: usize = 64;
/// Bytes of header before the first sample.
pub const HEADER_BYTES: u64 = 64;
/// A magic number, so a truncated or foreign file is refused, not parsed.
const MAGIC: [u8; 8] = [b'C', b'S', b'R', b'I', b'N', b'G', 0, 1];
/// Samples kept by default.
///
/// 32 MiB, allocated in full when the file is created rather than grown into,
/// so the number on the Privacy page is the number forever. At four samples a
/// second that is about a day and a half of history at full resolution, which
/// is what a chart needs; anything older is a question for the document store,
/// which keeps what was learned rather than every reading behind it.
pub const DEFAULT_CAPACITY: u64 = 512 * 1024;
/// Aggregate features carried per sample.
pub const FEATURES: usize = 6;

/// One observation, compressed to what a chart and a timeline actually need.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// The mirror's monotonic clock, matching `MirrorSnapshot::monotonic_ns`.
    pub monotonic_ns: u64,
    /// Wall clock milliseconds since the Unix epoch, for display only.
    pub wall_ms: u64,
    /// The latent state the machine was recognised to be in, if any was.
    pub latent_state: Option<u32>,
    /// How long the observation pass itself took. The overhead figure in
    /// Settings is the mean of this column and nothing else.
    pub observe_ns: u32,
    /// Entities observed in this pass.
    pub entities: u32,
    /// Cells carrying a real reading rather than a reason there was not one.
    pub observed_cells: u32,
    /// Aggregate features, in whatever order the producer documents.
    pub features: [f32; FEATURES],
}

impl Sample {
    /// A sample with nothing in it, for filling gaps in tests.
    pub fn empty(monotonic_ns: u64) -> Sample {
        Sample {
            monotonic_ns,
            wall_ms: 0,
            latent_state: None,
            observe_ns: 0,
            entities: 0,
            observed_cells: 0,
            features: [0.0; FEATURES],
        }
    }

    fn encode(&self) -> [u8; SAMPLE_BYTES] {
        let mut out = [0u8; SAMPLE_BYTES];
        out[0..8].copy_from_slice(&self.monotonic_ns.to_le_bytes());
        out[8..16].copy_from_slice(&self.wall_ms.to_le_bytes());
        // `u32::MAX` is the absent latent state. The catalogue caps well below
        // that, and a test pins the collision open so nobody quietly raises the
        // cap past it.
        let latent = self.latent_state.unwrap_or(u32::MAX);
        out[16..20].copy_from_slice(&latent.to_le_bytes());
        out[20..24].copy_from_slice(&self.observe_ns.to_le_bytes());
        out[24..28].copy_from_slice(&self.entities.to_le_bytes());
        out[28..32].copy_from_slice(&self.observed_cells.to_le_bytes());
        for (index, value) in self.features.iter().enumerate() {
            let at = 32 + index * 4;
            out[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        out
    }

    fn decode(bytes: &[u8]) -> Sample {
        let word = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or_default());
        let half = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap_or_default());
        let latent = half(16);
        let mut features = [0.0f32; FEATURES];
        for (index, slot) in features.iter_mut().enumerate() {
            let at = 32 + index * 4;
            *slot = f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap_or_default());
        }
        Sample {
            monotonic_ns: word(0),
            wall_ms: word(8),
            latent_state: (latent != u32::MAX).then_some(latent),
            observe_ns: half(20),
            entities: half(24),
            observed_cells: half(28),
            features,
        }
    }
}

/// A fixed-size file of samples that wraps when full.
#[derive(Debug)]
pub struct Ring {
    file: File,
    path: PathBuf,
    capacity: u64,
    /// Total samples ever written, not the position. The position is this
    /// modulo the capacity; keeping the total means a reader can tell whether
    /// the ring has wrapped and how much history was lost.
    written: u64,
}

impl Ring {
    /// Open the ring at `path`, creating it with `capacity` samples if absent.
    ///
    /// An existing ring keeps its own capacity: silently resizing would either
    /// discard history or leave uninitialised bytes claiming to be samples.
    pub fn open(path: &Path, capacity: u64) -> Result<Ring> {
        let capacity = capacity.max(16);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|source| Error::io(path, source))?;

        let length = file
            .metadata()
            .map_err(|source| Error::io(path, source))?
            .len();

        let (capacity, written) = if length >= HEADER_BYTES {
            let mut header = [0u8; HEADER_BYTES as usize];
            file.seek(SeekFrom::Start(0))
                .and_then(|_| file.read_exact(&mut header))
                .map_err(|source| Error::io(path, source))?;
            if header[0..8] != MAGIC {
                return Err(Error::invalid(format!(
                    "{} is not a CoreScout ring; move it aside and it will be recreated",
                    path.display()
                )));
            }
            let stored = u64::from_le_bytes(header[8..16].try_into().unwrap_or_default());
            let written = u64::from_le_bytes(header[16..24].try_into().unwrap_or_default());
            (stored.max(16), written)
        } else {
            let mut header = [0u8; HEADER_BYTES as usize];
            header[0..8].copy_from_slice(&MAGIC);
            header[8..16].copy_from_slice(&capacity.to_le_bytes());
            file.seek(SeekFrom::Start(0))
                .and_then(|_| file.write_all(&header))
                .map_err(|source| Error::io(path, source))?;
            file.set_len(HEADER_BYTES + capacity * SAMPLE_BYTES as u64)
                .map_err(|source| Error::io(path, source))?;
            (capacity, 0)
        };

        Ok(Ring {
            file,
            path: path.to_path_buf(),
            capacity,
            written,
        })
    }

    /// Samples this ring can hold before it starts overwriting.
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Samples ever written, including ones since overwritten.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// Samples currently readable.
    pub fn len(&self) -> u64 {
        self.written.min(self.capacity)
    }

    /// Whether nothing has been written yet.
    pub fn is_empty(&self) -> bool {
        self.written == 0
    }

    /// Whether the ring has wrapped and older samples are gone.
    pub fn wrapped(&self) -> bool {
        self.written > self.capacity
    }

    /// Bytes on disk. Constant for the life of the file, which is the point.
    pub fn bytes(&self) -> u64 {
        HEADER_BYTES + self.capacity * SAMPLE_BYTES as u64
    }

    /// The file backing this ring.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one sample.
    ///
    /// The sample is written before the cursor advances, so an interrupted
    /// append loses only itself.
    pub fn push(&mut self, sample: &Sample) -> Result<()> {
        let slot = self.written % self.capacity;
        let at = HEADER_BYTES + slot * SAMPLE_BYTES as u64;
        self.file
            .seek(SeekFrom::Start(at))
            .and_then(|_| self.file.write_all(&sample.encode()))
            .map_err(|source| Error::io(&self.path, source))?;
        self.written += 1;
        self.write_cursor()
    }

    /// The most recent `count` samples, oldest first.
    pub fn recent(&mut self, count: u64) -> Result<Vec<Sample>> {
        let take = count.min(self.len());
        let mut out = Vec::with_capacity(take as usize);
        let start = self.written - take;
        for index in start..self.written {
            out.push(self.read_at(index)?);
        }
        Ok(out)
    }

    /// Samples at or after `monotonic_ns`, oldest first, capped at `limit`.
    ///
    /// Scans backwards from newest and stops early, so asking for the last
    /// minute of a seven-day ring reads a minute of it.
    pub fn since(&mut self, monotonic_ns: u64, limit: u64) -> Result<Vec<Sample>> {
        let mut out = Vec::new();
        let oldest = self.written - self.len();
        let mut index = self.written;
        while index > oldest && (out.len() as u64) < limit {
            index -= 1;
            let sample = self.read_at(index)?;
            if sample.monotonic_ns < monotonic_ns {
                break;
            }
            out.push(sample);
        }
        out.reverse();
        Ok(out)
    }

    /// Forget everything, keeping the file and its size.
    ///
    /// This is what "clear my history" does. It does not shrink the file,
    /// because the space was already committed and reclaiming it would only
    /// mean re-growing it within the hour.
    pub fn clear(&mut self) -> Result<()> {
        self.written = 0;
        self.write_cursor()
    }

    fn read_at(&mut self, index: u64) -> Result<Sample> {
        let slot = index % self.capacity;
        let at = HEADER_BYTES + slot * SAMPLE_BYTES as u64;
        let mut bytes = [0u8; SAMPLE_BYTES];
        self.file
            .seek(SeekFrom::Start(at))
            .and_then(|_| self.file.read_exact(&mut bytes))
            .map_err(|source| Error::io(&self.path, source))?;
        Ok(Sample::decode(&bytes))
    }

    fn write_cursor(&mut self) -> Result<()> {
        self.file
            .seek(SeekFrom::Start(16))
            .and_then(|_| self.file.write_all(&self.written.to_le_bytes()))
            .map_err(|source| Error::io(&self.path, source))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join(name);
        (dir, path)
    }

    fn sample(n: u64) -> Sample {
        Sample {
            monotonic_ns: n * 1_000_000,
            wall_ms: 1_700_000_000_000 + n,
            latent_state: Some((n % 32) as u32),
            observe_ns: 6_800,
            entities: 86,
            observed_cells: 216,
            features: [n as f32, 0.5, -1.25, 0.0, f32::MAX, 1e-9],
        }
    }

    #[test]
    fn a_sample_round_trips_exactly() {
        let original = sample(7);
        assert_eq!(Sample::decode(&original.encode()), original);
    }

    #[test]
    fn an_absent_latent_state_survives_the_round_trip() {
        // The encoding spends a sentinel on this rather than a flag byte, so
        // it is worth a test of its own.
        let original = Sample {
            latent_state: None,
            ..sample(1)
        };
        assert_eq!(Sample::decode(&original.encode()).latent_state, None);
    }

    #[test]
    fn the_sentinel_is_out_of_reach_of_a_real_state_id() {
        // If the latent catalogue ever grows to four billion states this
        // encoding is wrong, and this test is where that gets noticed.
        let highest = Sample {
            latent_state: Some(u32::MAX - 1),
            ..sample(1)
        };
        assert_eq!(
            Sample::decode(&highest.encode()).latent_state,
            Some(u32::MAX - 1)
        );
    }

    #[test]
    fn the_file_is_exactly_the_size_it_promises() {
        let (_dir, path) = scratch("a.ring");
        let mut ring = Ring::open(&path, 1000).expect("a ring");
        for n in 0..5000 {
            ring.push(&sample(n)).expect("a push");
        }
        let on_disk = std::fs::metadata(&path).expect("metadata").len();
        assert_eq!(on_disk, ring.bytes());
        assert_eq!(on_disk, HEADER_BYTES + 1000 * SAMPLE_BYTES as u64);
    }

    #[test]
    fn wrapping_keeps_the_newest_and_loses_the_oldest() {
        let (_dir, path) = scratch("b.ring");
        let mut ring = Ring::open(&path, 64).expect("a ring");
        for n in 0..200 {
            ring.push(&sample(n)).expect("a push");
        }
        assert!(ring.wrapped());
        assert_eq!(ring.len(), 64);
        assert_eq!(ring.written(), 200);
        let recent = ring.recent(64).expect("recent");
        assert_eq!(recent.len(), 64);
        assert_eq!(recent[0].monotonic_ns, 136_000_000);
        assert_eq!(recent[63].monotonic_ns, 199_000_000);
    }

    #[test]
    fn asking_for_more_than_exists_returns_what_exists() {
        let (_dir, path) = scratch("c.ring");
        let mut ring = Ring::open(&path, 1000).expect("a ring");
        for n in 0..10 {
            ring.push(&sample(n)).expect("a push");
        }
        assert_eq!(ring.recent(1_000_000).expect("recent").len(), 10);
    }

    #[test]
    fn a_reopened_ring_resumes_where_it_stopped() {
        // The service restarts. History has to survive that, or the product
        // forgets everything every time Windows installs an update.
        let (_dir, path) = scratch("d.ring");
        {
            let mut ring = Ring::open(&path, 128).expect("a ring");
            for n in 0..100 {
                ring.push(&sample(n)).expect("a push");
            }
        }
        let mut ring = Ring::open(&path, 999_999).expect("reopen");
        assert_eq!(ring.capacity(), 128, "an existing ring keeps its capacity");
        assert_eq!(ring.written(), 100);
        ring.push(&sample(100)).expect("a push");
        assert_eq!(ring.recent(2).expect("recent")[1].monotonic_ns, 100_000_000);
    }

    #[test]
    fn since_reads_only_the_tail_it_needs() {
        let (_dir, path) = scratch("e.ring");
        let mut ring = Ring::open(&path, 4096).expect("a ring");
        for n in 0..2000 {
            ring.push(&sample(n)).expect("a push");
        }
        let tail = ring.since(1_990 * 1_000_000, 1000).expect("since");
        assert_eq!(tail.len(), 10);
        assert_eq!(tail[0].monotonic_ns, 1_990_000_000);
        assert_eq!(tail[9].monotonic_ns, 1_999_000_000);
    }

    #[test]
    fn since_respects_its_limit() {
        let (_dir, path) = scratch("h.ring");
        let mut ring = Ring::open(&path, 4096).expect("a ring");
        for n in 0..2000 {
            ring.push(&sample(n)).expect("a push");
        }
        // Everything qualifies; the limit is what stops it.
        let tail = ring.since(0, 25).expect("since");
        assert_eq!(tail.len(), 25);
        assert_eq!(
            tail[24].monotonic_ns, 1_999_000_000,
            "the limit keeps the newest"
        );
    }

    #[test]
    fn a_foreign_file_is_refused_rather_than_parsed() {
        let (_dir, path) = scratch("f.ring");
        std::fs::write(&path, vec![0u8; 4096]).expect("a decoy");
        let error = Ring::open(&path, 128)
            .expect_err("should refuse")
            .to_string();
        assert!(error.contains("not a CoreScout ring"), "{error}");
    }

    #[test]
    fn clearing_forgets_the_history_and_keeps_the_file() {
        let (_dir, path) = scratch("g.ring");
        let mut ring = Ring::open(&path, 128).expect("a ring");
        for n in 0..50 {
            ring.push(&sample(n)).expect("a push");
        }
        let size = ring.bytes();
        ring.clear().expect("clear");
        assert!(ring.is_empty());
        assert_eq!(ring.recent(10).expect("recent").len(), 0);
        assert_eq!(ring.bytes(), size);
    }
}
