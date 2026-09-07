//! Reading kernel text interfaces.
//!
//! Every sensor funnels its file access through here, for two reasons.
//!
//! **Absence is normal.** A missing `cpufreq` directory, an unreadable
//! `energy_uj`, a `thermal_zone` that vanished on hotplug: none of these are
//! errors, they are facts about the machine, and the correct response is an
//! unobserved cell rather than a failed observation pass. So these helpers
//! return `Option` and never propagate an error upward.
//!
//! **Normalisation happens once.** The whole point of the self-state plane is
//! that consumers do not each reparse `/proc`. The parsing lives here, runs once
//! per tick in one process, and everyone else reads numbers.

use std::path::Path;

use corescout_mirror::schema::Availability;
use corescout_mirror::state::ChannelId;

use crate::observation::{SensorOutcome, StateWriter};

/// Read a file, trimming trailing whitespace. `None` if it cannot be read.
pub fn string(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim_end().to_string())
}

/// Read a file containing a single integer.
pub fn u64(path: impl AsRef<Path>) -> Option<u64> {
    string(path)?.trim().parse().ok()
}

/// Read a file containing a single number, as `f64`.
pub fn f64(path: impl AsRef<Path>) -> Option<f64> {
    u64(path).map(|v| v as f64)
}

/// List the subdirectories of `dir` whose names start with `prefix`, paired with
/// the integer suffix, sorted by that suffix.
///
/// Sorting matters: it makes entity row assignment deterministic, and
/// `read_dir` order is not.
pub fn numbered_children(dir: impl AsRef<Path>, prefix: &str) -> Vec<(u32, std::path::PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir.as_ref()) else {
        return Vec::new();
    };
    let mut found: Vec<(u32, std::path::PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let index = name.strip_prefix(prefix)?.parse::<u32>().ok()?;
            Some((index, entry.path()))
        })
        .collect();
    found.sort_by_key(|(index, _)| *index);
    found
}

/// Children of `dir` whose names start with `prefix`, sorted by name.
///
/// For interfaces like powercap where the suffix is not a bare integer
/// (`intel-rapl:0:1`).
pub fn named_children(dir: impl AsRef<Path>, prefix: &str) -> Vec<(String, std::path::PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir.as_ref()) else {
        return Vec::new();
    };
    let mut found: Vec<(String, std::path::PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(prefix) {
                Some((name, entry.path()))
            } else {
                None
            }
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// Ticks per second, as reported by `sysconf(_SC_CLK_TCK)`.
///
/// `/proc/stat` reports CPU time in these units, and the value is not
/// discoverable from the file itself. It is almost always 100, but reading it
/// rather than assuming is the difference between publishing nanoseconds and
/// publishing a plausible wrong number.
pub fn clock_ticks_per_second() -> u64 {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: sysconf with a valid name; returns -1 on failure.
        let value = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if value > 0 {
            return value as u64;
        }
    }
    100
}

/// Read one file into one cell, counting the outcome.
///
/// `required` distinguishes "this file should be here and was not", which is
/// worth reporting as an error, from "this interface is optional and absent on
/// this machine", which is simply an unobserved cell. Conflating the two would
/// make the error count meaningless on any machine missing an optional
/// interface, which is every machine.
pub fn sample(
    out: &mut StateWriter<'_>,
    outcome: &mut SensorOutcome,
    row: u32,
    channel: Option<ChannelId>,
    path: impl AsRef<Path>,
    required: bool,
) {
    let Some(channel) = channel else { return };
    let path = path.as_ref();
    match self::f64(path) {
        Some(value) => {
            out.set(row, channel, value);
            outcome.sample();
        }
        None => {
            // Why, not just "no". A file that exists and will not open is a
            // different fact from one that was never there.
            let why = match std::fs::metadata(path) {
                Ok(_) => Availability::PermissionDenied,
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    Availability::PermissionDenied
                }
                Err(_) if required => Availability::Unavailable,
                Err(_) => Availability::Unsupported,
            };
            out.missing(row, channel, why);
            if required {
                outcome.error();
            }
        }
    }
}

/// Write an already-parsed value into one cell.
pub fn emit(
    out: &mut StateWriter<'_>,
    outcome: &mut SensorOutcome,
    row: u32,
    channel: Option<ChannelId>,
    value: f64,
) {
    let Some(channel) = channel else { return };
    out.set(row, channel, value);
    outcome.sample();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("corescout-source-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_files_are_absent_not_errors() {
        assert_eq!(string("/definitely/not/here"), None);
        assert_eq!(u64("/definitely/not/here"), None);
        assert_eq!(f64("/definitely/not/here"), None);
    }

    #[test]
    fn values_are_trimmed_of_the_kernel_trailing_newline() {
        let dir = temp_dir("trim");
        std::fs::write(dir.join("value"), "3600000\n").unwrap();
        assert_eq!(string(dir.join("value")).as_deref(), Some("3600000"));
        assert_eq!(u64(dir.join("value")), Some(3_600_000));
        assert_eq!(f64(dir.join("value")), Some(3_600_000.0));
    }

    #[test]
    fn non_numeric_content_is_absent_rather_than_zero() {
        let dir = temp_dir("garbage");
        std::fs::write(dir.join("value"), "not a number\n").unwrap();
        assert_eq!(u64(dir.join("value")), None);
    }

    #[test]
    fn numbered_children_are_sorted_numerically() {
        let dir = temp_dir("numbered");
        for index in [10u32, 2, 1, 20] {
            std::fs::create_dir_all(dir.join(format!("state{index}"))).unwrap();
        }
        std::fs::create_dir_all(dir.join("other")).unwrap();
        let found = numbered_children(&dir, "state");
        let indices: Vec<u32> = found.iter().map(|(i, _)| *i).collect();
        assert_eq!(
            indices,
            vec![1, 2, 10, 20],
            "lexicographic ordering would put state10 before state2"
        );
    }

    #[test]
    fn named_children_are_sorted_and_filtered() {
        // The real names are `intel-rapl:0`, `intel-rapl:0:0` and so on. A
        // colon cannot appear in a filename on Windows, where this test also
        // runs, so the fixture uses the same shape with a legal separator. The
        // function under test only cares about the prefix and the ordering.
        let dir = temp_dir("named");
        for name in [
            "intel-rapl-1",
            "intel-rapl-0",
            "intel-rapl-0-0",
            "unrelated",
        ] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
        }
        let found = named_children(&dir, "intel-rapl-");
        let names: Vec<&str> = found.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec!["intel-rapl-0", "intel-rapl-0-0", "intel-rapl-1"]
        );
    }

    #[test]
    fn missing_directories_yield_nothing() {
        assert!(numbered_children("/definitely/not/here", "x").is_empty());
        assert!(named_children("/definitely/not/here", "x").is_empty());
    }

    #[test]
    fn clock_ticks_are_plausible() {
        let ticks = clock_ticks_per_second();
        assert!((1..=10_000).contains(&ticks), "implausible CLK_TCK {ticks}");
    }
}
