//! Where reflections come from.
//!
//! # The point of this abstraction
//!
//! `mirror-observer`, `mirror-learn` and `mirror-agent` take a [`Source`] and
//! **cannot ask which kind it is**. Live plane and recorded file present the
//! same interface, so:
//!
//! - an analysis can be re-run on identical input, which is what makes the
//!   labelled/unlabelled comparison a controlled experiment rather than two
//!   runs against a machine that moved in between,
//! - a bug seen once on a live machine can be reproduced from a recording,
//! - and a model can be trained offline and scored against exactly the frames
//!   the online system saw.
//!
//! An analysis that behaved differently on recorded data could not be checked
//! at all, so the indistinguishability is a correctness property, not a
//! convenience.
//!
//! # Pacing
//!
//! A live source is paced by the machine: reflections arrive when the daemon
//! publishes them. A recording can be replayed as fast as it reads, or at the
//! rate it was captured. Both are useful, and which one is in force is the one
//! thing a consumer *can* see, through [`Source::is_realtime`], because a
//! consumer measuring wall-clock latency needs to know.

use std::path::Path;

use corescout_core::{Error, Result};
use corescout_mirror::plane::PlaneReader;
use corescout_mirror::MirrorSnapshot;

use crate::recording::Recording;

/// A stream of reflections.
pub trait ReflectionSource {
    /// The next reflection, or `None` when the stream has ended.
    ///
    /// A live source blocks until a genuinely new reflection is available; it
    /// never returns the same one twice, because a repeated snapshot would
    /// fabricate a zero-change step and make everything look more predictable
    /// than it is.
    fn next_reflection(&mut self) -> Result<Option<MirrorSnapshot>>;

    /// Whether this source is paced by a real machine.
    fn is_realtime(&self) -> bool;

    /// A short description, for logs.
    fn describe(&self) -> String;
}

/// The concrete sources.
pub enum Source {
    /// A live self-state plane.
    Live {
        reader: PlaneReader,
        poll_interval: std::time::Duration,
        last_sequence: Option<u64>,
        /// Give up waiting for a new reflection after this many polls, so a
        /// stopped daemon ends the stream instead of hanging a consumer.
        patience: u32,
    },
    /// A recorded session.
    Replay {
        recording: Recording,
        /// Replay at the rate it was captured rather than as fast as possible.
        paced: bool,
        last_monotonic_ns: Option<u64>,
    },
    /// Reflections held in memory. For tests, and for feeding a model a
    /// sequence assembled from somewhere else.
    Memory {
        snapshots: std::vec::IntoIter<MirrorSnapshot>,
    },
}

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe())
    }
}

impl Source {
    /// Attach to a live plane.
    pub fn live(path: &Path, poll_interval: std::time::Duration) -> Result<Source> {
        let reader = PlaneReader::open(path).map_err(|e| {
            Error::invalid(format!(
                "{e}\n\nIs the mirror running? Start it with `corescout mirror`."
            ))
        })?;
        Ok(Source::Live {
            reader,
            poll_interval,
            last_sequence: None,
            patience: 200,
        })
    }

    /// Replay a recording as fast as it reads.
    pub fn replay(path: &Path) -> Result<Source> {
        Ok(Source::Replay {
            recording: Recording::open(path)?,
            paced: false,
            last_monotonic_ns: None,
        })
    }

    /// Replay a recording at the rate it was captured.
    pub fn replay_paced(path: &Path) -> Result<Source> {
        Ok(Source::Replay {
            recording: Recording::open(path)?,
            paced: true,
            last_monotonic_ns: None,
        })
    }

    /// A source over reflections already in hand.
    pub fn from_memory(snapshots: Vec<MirrorSnapshot>) -> Source {
        Source::Memory {
            snapshots: snapshots.into_iter(),
        }
    }

    /// Take up to `count` reflections.
    pub fn take(&mut self, count: usize) -> Result<Vec<MirrorSnapshot>> {
        let mut out = Vec::with_capacity(count.min(4096));
        while out.len() < count {
            match self.next_reflection()? {
                Some(snapshot) => out.push(snapshot),
                None => break,
            }
        }
        Ok(out)
    }
}

impl ReflectionSource for Source {
    fn next_reflection(&mut self) -> Result<Option<MirrorSnapshot>> {
        match self {
            Source::Live {
                reader,
                poll_interval,
                last_sequence,
                patience,
            } => {
                for _ in 0..*patience {
                    let snapshot = reader.read_snapshot()?;
                    if Some(snapshot.sequence) != *last_sequence {
                        *last_sequence = Some(snapshot.sequence);
                        return Ok(Some(snapshot));
                    }
                    std::thread::sleep(*poll_interval);
                }
                // The daemon has stopped publishing. Ending the stream is more
                // useful than blocking forever: a consumer can report what it
                // learned from what it did see.
                Ok(None)
            }
            Source::Replay {
                recording,
                paced,
                last_monotonic_ns,
            } => {
                let snapshot = recording.next_snapshot()?;
                if let (true, Some(snapshot)) = (*paced, snapshot.as_ref()) {
                    if let Some(previous) = *last_monotonic_ns {
                        let gap = snapshot.monotonic_ns.saturating_sub(previous);
                        // Cap the wait: a recording with a long gap in it
                        // should not stall a replay for minutes.
                        let capped = gap.min(2_000_000_000);
                        std::thread::sleep(std::time::Duration::from_nanos(capped));
                    }
                    *last_monotonic_ns = Some(snapshot.monotonic_ns);
                }
                Ok(snapshot)
            }
            Source::Memory { snapshots } => Ok(snapshots.next()),
        }
    }

    fn is_realtime(&self) -> bool {
        matches!(self, Source::Live { .. })
    }

    fn describe(&self) -> String {
        match self {
            Source::Live { reader, .. } => {
                format!(
                    "live plane, epoch {}, written by pid {}",
                    reader.epoch(),
                    reader.writer_pid()
                )
            }
            Source::Replay {
                recording, paced, ..
            } => format!(
                "replay of {}{}",
                recording.path().display(),
                if *paced { " (paced)" } else { "" }
            ),
            Source::Memory { .. } => "in-memory reflections".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::test_support::fixture;

    fn snapshots(count: u64) -> Vec<MirrorSnapshot> {
        (0..count)
            .map(|i| {
                let mut snapshot = fixture();
                snapshot.sequence = i;
                snapshot.monotonic_ns = i * 100_000_000;
                snapshot
            })
            .collect()
    }

    #[test]
    fn a_memory_source_yields_everything_then_ends() {
        let mut source = Source::from_memory(snapshots(3));
        assert_eq!(source.next_reflection().unwrap().unwrap().sequence, 0);
        assert_eq!(source.next_reflection().unwrap().unwrap().sequence, 1);
        assert_eq!(source.next_reflection().unwrap().unwrap().sequence, 2);
        assert!(source.next_reflection().unwrap().is_none());
    }

    #[test]
    fn take_stops_at_the_end_of_a_short_source() {
        let mut source = Source::from_memory(snapshots(3));
        assert_eq!(source.take(10).unwrap().len(), 3);
    }

    #[test]
    fn a_replay_and_an_in_memory_source_produce_the_same_sequence() {
        // The property the whole abstraction exists for: a consumer cannot tell
        // them apart, and gets identical input either way.
        let path =
            std::env::temp_dir().join(format!("corescout-source-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut recorder = crate::recording::Recorder::create(&path).unwrap();
        for snapshot in snapshots(6) {
            recorder.record(&snapshot).unwrap();
        }
        recorder.flush().unwrap();
        drop(recorder);

        let from_file = Source::replay(&path).unwrap().take(10).unwrap();
        let from_memory = Source::from_memory(snapshots(6)).take(10).unwrap();

        assert_eq!(from_file.len(), from_memory.len());
        for (a, b) in from_file.iter().zip(&from_memory) {
            assert_eq!(a.sequence, b.sequence);
            assert_eq!(a.entities, b.entities);
            assert_eq!(a.state, b.state);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn only_a_live_source_claims_to_be_realtime() {
        assert!(!Source::from_memory(snapshots(1)).is_realtime());
    }

    #[test]
    fn a_source_describes_itself_for_a_log() {
        let source = Source::from_memory(snapshots(1));
        assert!(source.describe().contains("in-memory"));
    }
}
