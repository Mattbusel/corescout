//! Shared plumbing for the commands.
//!
//! # The one interesting thing in here
//!
//! [`open_source`] returns a [`Source`] without the caller learning whether it
//! is a live plane, a recorded trace, or a list held in memory. Every consuming
//! command is written against that, so `corescout learn --source trace.jsonl`
//! and `corescout learn` run the identical code path over the identical types.
//! That is not a convenience: it is the property the memory layer exists to
//! provide, and routing every command through one function is what stops it
//! from quietly decaying into two code paths that drift apart.

use std::path::{Path, PathBuf};
use std::time::Duration;

use corescout_core::error::{Error, Result};
use corescout_memory::source::Source;
use corescout_mirror::plane::default_path;
use corescout_mirror::MirrorSnapshot;
use corescout_substrate::observation::default_sensors;
use corescout_substrate::platform;
use corescout_substrate::{Reflector, Substrate};

use crate::cli::SourceOptions;

/// Build a reflector over this machine.
///
/// This is the only function in the application that reaches hardware, and
/// every command that calls it is one that observes or perturbs. The consuming
/// commands never call it.
pub fn reflector() -> Result<Reflector> {
    let platform = platform::detect();
    let substrate = Substrate::discover(platform.as_ref())?;
    Reflector::build(substrate, default_sensors())
}

/// The CPUs this process is already permitted to use.
///
/// Every candidate action is bounded by this. Falling back to the empty set on
/// failure is deliberate: a controller that cannot establish what it is allowed
/// to touch should consider nothing, not everything.
pub fn permitted_cpus() -> corescout_core::CpuSet {
    platform::detect()
        .process_affinity()
        .unwrap_or_else(|_| corescout_core::CpuSet::new())
}

/// Open a reflection source, live or recorded, without the caller caring which.
///
/// The path decides: a file that exists and is not a plane is read as a trace;
/// otherwise the plane is opened. When neither is available the error says what
/// to do about it, because "no such file" is a useless thing to print at
/// someone who has not started the mirror yet.
pub fn open_source(options: &SourceOptions) -> Result<Source> {
    let interval = Duration::from_millis(options.interval_ms.max(1));
    match &options.path {
        Some(path) if looks_like_a_trace(path) => Source::replay(path),
        Some(path) => Source::live(path, interval),
        None => {
            let path = default_path();
            if !path.exists() {
                return Err(Error::invalid(format!(
                    "no self-state plane at {}; start one with `corescout-lab mirror`, \
                     or read a recording with --source <trace>",
                    path.display()
                )));
            }
            Source::live(&path, interval)
        }
    }
}

/// Read reflections from a source, up to a limit.
///
/// A `None` limit means "everything the source has", which terminates for a
/// recording and does not for a live plane; the commands that pass `None` are
/// the ones meant to run until interrupted.
pub fn take_frames(source: &mut Source, frames: Option<usize>) -> Result<Vec<MirrorSnapshot>> {
    match frames {
        Some(count) => source.take(count),
        None => source.take(usize::MAX),
    }
}

/// Whether a path is a recorded trace rather than a plane.
///
/// A trace is a text file of JSON lines; a plane is a fixed-layout binary
/// mapping. Deciding on the extension is crude, but the alternative is opening
/// the file and guessing from its first bytes, which is crude *and* silently
/// wrong when a plane happens to start with a brace.
fn looks_like_a_trace(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("jsonl" | "json" | "trace" | "log")
    )
}

/// Where to keep the plane for a mirror run.
pub fn plane_path(explicit: Option<&PathBuf>) -> PathBuf {
    explicit.cloned().unwrap_or_else(default_path)
}

/// Render a value as pretty JSON, turning a serialisation failure into our own
/// error type.
///
/// Worth a function because `?` on a `serde_json::Error` does not convert, and
/// the alternative is `.map_err(...)` at every print site, which is where a
/// silent `unwrap` eventually gets written instead.
pub fn to_json<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string_pretty(value)
        .map_err(|error| Error::invalid(format!("could not serialise the result: {error}")))
}

/// Print an error the way a command line tool should: to stderr, prefixed, and
/// without a stack trace.
pub fn report_error(error: &Error) {
    eprintln!("corescout-lab: {error}");
    if let Error::Io { path, source } = error {
        if source.kind() == std::io::ErrorKind::PermissionDenied {
            eprintln!(
                "  {} needs privilege this process does not have. The mirror will \
                 record that gap rather than guessing at the value.",
                path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traces_are_recognised_by_extension_and_planes_are_not() {
        assert!(looks_like_a_trace(Path::new("run.jsonl")));
        assert!(looks_like_a_trace(Path::new("/tmp/a.json")));
        assert!(!looks_like_a_trace(Path::new("/run/corescout/mirror")));
        assert!(!looks_like_a_trace(Path::new("mirror.plane")));
    }

    #[test]
    fn a_missing_plane_explains_how_to_start_one() {
        // "No such file or directory" is useless to someone who has not run the
        // mirror yet.
        let options = SourceOptions {
            path: None,
            ..SourceOptions::default()
        };
        if !default_path().exists() {
            let error = open_source(&options).unwrap_err().to_string();
            assert!(error.contains("corescout-lab mirror"), "{error}");
        }
    }

    #[test]
    fn an_explicit_plane_path_is_used_over_the_default() {
        let explicit = PathBuf::from("/tmp/somewhere/else");
        assert_eq!(plane_path(Some(&explicit)), explicit);
        assert_eq!(plane_path(None), default_path());
    }
}
