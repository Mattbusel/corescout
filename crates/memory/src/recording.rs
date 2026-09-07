//! Durable recordings, and replay from them.
//!
//! # Format
//!
//! Newline-delimited JSON, one reflection per line. Deliberately not the
//! binary plane layout:
//!
//! - A recording outlives the version of CoreScout that wrote it, and a
//!   self-describing text format survives a schema change that a fixed binary
//!   layout would not.
//! - Recordings get shared, diffed, truncated with `head`, and inspected with
//!   `jq`. That is worth more here than the bytes it costs.
//! - The plane's whole design rationale is per-tick read cost, and a recording
//!   is read once.
//!
//! The canonical *live* representation is still binary. This is the archival
//! one, and the difference is deliberate.
//!
//! # Why replay has to be indistinguishable from live
//!
//! An analysis that behaves differently on recorded data cannot be checked, and
//! an experiment that cannot be re-run on the same input is not an experiment.
//! [`crate::source::Source`] is what enforces that: a consumer takes a
//! `Source` and cannot ask which kind it is.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use corescout_core::{Error, Result};
use corescout_mirror::MirrorSnapshot;

/// Appends reflections to a file.
#[derive(Debug)]
pub struct Recorder {
    path: PathBuf,
    writer: BufWriter<File>,
    written: u64,
    bytes: u64,
    /// Stop after this many bytes. A mirror left recording overnight should not
    /// fill a disk.
    limit_bytes: Option<u64>,
}

impl Recorder {
    /// Create or append to a recording.
    pub fn create(path: impl AsRef<Path>) -> Result<Recorder> {
        Recorder::with_limit(path, None)
    }

    pub fn with_limit(path: impl AsRef<Path>, limit_bytes: Option<u64>) -> Result<Recorder> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
            }
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| Error::io(&path, e))?;
        let bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Recorder {
            path,
            writer: BufWriter::new(file),
            written: 0,
            bytes,
            limit_bytes,
        })
    }

    /// Append one reflection.
    ///
    /// Returns `false` once the size limit is reached, so a caller can stop
    /// recording without treating a full recording as an error.
    pub fn record(&mut self, snapshot: &MirrorSnapshot) -> Result<bool> {
        if let Some(limit) = self.limit_bytes {
            if self.bytes >= limit {
                return Ok(false);
            }
        }
        let line = serde_json::to_string(snapshot)
            .map_err(|e| Error::invalid(format!("could not serialise a reflection: {e}")))?;
        writeln!(self.writer, "{line}").map_err(|e| Error::io(&self.path, e))?;
        self.bytes += line.len() as u64 + 1;
        self.written += 1;
        Ok(true)
    }

    pub fn flush(&mut self) -> Result<()> {
        self.writer.flush().map_err(|e| Error::io(&self.path, e))
    }

    pub fn written(&self) -> u64 {
        self.written
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // A recording truncated by a forgotten flush is worse than useless: it
        // looks complete.
        let _ = self.writer.flush();
    }
}

/// A recording on disk, read lazily.
#[derive(Debug)]
pub struct Recording {
    path: PathBuf,
    reader: BufReader<File>,
    line: u64,
    /// Reflections skipped because they could not be parsed.
    corrupt: u64,
}

impl Recording {
    pub fn open(path: impl AsRef<Path>) -> Result<Recording> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path).map_err(|e| Error::io(&path, e))?;
        Ok(Recording {
            path,
            reader: BufReader::new(file),
            line: 0,
            corrupt: 0,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn corrupt_lines(&self) -> u64 {
        self.corrupt
    }

    /// The next reflection, or `None` at the end.
    ///
    /// A line that will not parse is counted and skipped rather than ending the
    /// replay. A recording truncated mid-write by a killed daemon is a normal
    /// artefact, and losing the whole session because its last line is half
    /// there would be a poor trade.
    pub fn next_snapshot(&mut self) -> Result<Option<MirrorSnapshot>> {
        loop {
            let mut buffer = String::new();
            let read = self
                .reader
                .read_line(&mut buffer)
                .map_err(|e| Error::io(&self.path, e))?;
            if read == 0 {
                return Ok(None);
            }
            self.line += 1;
            if buffer.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<MirrorSnapshot>(&buffer) {
                Ok(snapshot) => return Ok(Some(snapshot)),
                Err(_) => {
                    self.corrupt += 1;
                    continue;
                }
            }
        }
    }

    /// Read the whole recording.
    pub fn read_all(mut self) -> Result<Vec<MirrorSnapshot>> {
        let mut out = Vec::new();
        while let Some(snapshot) = self.next_snapshot()? {
            out.push(snapshot);
        }
        Ok(out)
    }

    /// Read at most `limit` reflections.
    pub fn read_at_most(&mut self, limit: usize) -> Result<Vec<MirrorSnapshot>> {
        let mut out = Vec::with_capacity(limit.min(4096));
        while out.len() < limit {
            match self.next_snapshot()? {
                Some(snapshot) => out.push(snapshot),
                None => break,
            }
        }
        Ok(out)
    }
}

/// A quick description of a recording without loading all of it.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordingSummary {
    pub reflections: usize,
    pub corrupt_lines: u64,
    pub epochs: Vec<u64>,
    pub span_ns: u64,
    pub entities: usize,
    pub channels: usize,
    pub first_sequence: u64,
    pub last_sequence: u64,
}

/// Summarise a recording, reading it once.
pub fn summarise(path: impl AsRef<Path>) -> Result<RecordingSummary> {
    let mut recording = Recording::open(path)?;
    let mut summary = RecordingSummary {
        reflections: 0,
        corrupt_lines: 0,
        epochs: Vec::new(),
        span_ns: 0,
        entities: 0,
        channels: 0,
        first_sequence: 0,
        last_sequence: 0,
    };
    let mut first_ns = None;
    let mut last_ns = 0u64;

    while let Some(snapshot) = recording.next_snapshot()? {
        if summary.reflections == 0 {
            summary.first_sequence = snapshot.sequence;
            summary.entities = snapshot.entities.len();
            summary.channels = snapshot.channels.len();
        }
        summary.reflections += 1;
        summary.last_sequence = snapshot.sequence;
        if !summary.epochs.contains(&snapshot.epoch) {
            summary.epochs.push(snapshot.epoch);
        }
        first_ns.get_or_insert(snapshot.monotonic_ns);
        last_ns = snapshot.monotonic_ns;
    }
    summary.corrupt_lines = recording.corrupt_lines();
    summary.span_ns = last_ns.saturating_sub(first_ns.unwrap_or(0));
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::test_support::fixture;

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "corescout-recording-{tag}-{}.jsonl",
            std::process::id()
        ))
    }

    #[test]
    fn a_recording_round_trips() {
        let path = temp_path("round-trip");
        let _ = std::fs::remove_file(&path);

        let mut recorder = Recorder::create(&path).unwrap();
        for i in 0..5u64 {
            let mut snapshot = fixture();
            snapshot.sequence = i;
            snapshot.monotonic_ns = i * 100_000_000;
            assert!(recorder.record(&snapshot).unwrap());
        }
        recorder.flush().unwrap();
        assert_eq!(recorder.written(), 5);
        drop(recorder);

        let back = Recording::open(&path).unwrap().read_all().unwrap();
        assert_eq!(back.len(), 5);
        assert_eq!(back[3].sequence, 3);
        // The parts that carry meaning survive the text format.
        assert_eq!(back[0].entities, fixture().entities);
        assert_eq!(back[0].relations, fixture().relations);
        assert_eq!(back[0].availability, fixture().availability);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unobserved_cells_survive_the_text_format() {
        // The trap: JSON has no NaN. If this regresses, every recorded hole
        // silently becomes a zero.
        let path = temp_path("nan");
        let _ = std::fs::remove_file(&path);
        let mut recorder = Recorder::create(&path).unwrap();
        recorder.record(&fixture()).unwrap();
        recorder.flush().unwrap();
        drop(recorder);

        let back = Recording::open(&path).unwrap().read_all().unwrap();
        assert_eq!(back[0].lookup("cpu/1", "cpu.time.idle"), None);
        assert_eq!(
            back[0].state.observed_cells(),
            fixture().state.observed_cells()
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_truncated_final_line_does_not_destroy_the_replay() {
        // What a killed daemon leaves behind.
        let path = temp_path("truncated");
        let _ = std::fs::remove_file(&path);
        let mut recorder = Recorder::create(&path).unwrap();
        recorder.record(&fixture()).unwrap();
        recorder.record(&fixture()).unwrap();
        recorder.flush().unwrap();
        drop(recorder);

        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("{\"format_version\":2,\"epoch\":1,\"sequ");
        std::fs::write(&path, text).unwrap();

        let mut recording = Recording::open(&path).unwrap();
        let mut count = 0;
        while recording.next_snapshot().unwrap().is_some() {
            count += 1;
        }
        assert_eq!(count, 2, "the intact reflections must still be readable");
        assert_eq!(recording.corrupt_lines(), 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_size_limit_stops_recording_without_erroring() {
        let path = temp_path("limited");
        let _ = std::fs::remove_file(&path);
        let mut recorder = Recorder::with_limit(&path, Some(64)).unwrap();
        let mut accepted = 0;
        for _ in 0..20 {
            if recorder.record(&fixture()).unwrap() {
                accepted += 1;
            }
        }
        assert!(accepted >= 1, "at least one reflection fits");
        assert!(accepted < 20, "the limit must actually stop it");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_recording_can_be_summarised_without_loading_it_all() {
        let path = temp_path("summary");
        let _ = std::fs::remove_file(&path);
        let mut recorder = Recorder::create(&path).unwrap();
        for i in 0..10u64 {
            let mut snapshot = fixture();
            snapshot.sequence = i;
            snapshot.monotonic_ns = i * 100_000_000;
            if i >= 5 {
                snapshot.epoch = 8;
            }
            recorder.record(&snapshot).unwrap();
        }
        recorder.flush().unwrap();
        drop(recorder);

        let summary = summarise(&path).unwrap();
        assert_eq!(summary.reflections, 10);
        assert_eq!(summary.epochs.len(), 2, "the shape changed mid-recording");
        assert_eq!(summary.span_ns, 900_000_000);
        assert_eq!(summary.entities, 4);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn opening_a_missing_recording_reports_the_path() {
        let error = Recording::open("/definitely/not/here.jsonl").unwrap_err();
        assert!(error.to_string().contains("here.jsonl"));
    }
}
