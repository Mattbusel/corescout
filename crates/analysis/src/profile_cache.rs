//! Cached machine profiles.
//!
//! A full analysis takes tens of seconds, which is fine to run once and
//! unacceptable to run before every process launch. `corescout run` therefore
//! reads a cached profile when one exists for *this* machine.
//!
//! "This machine" is the hard part. A profile is only valid for the hardware
//! and the kernel configuration it was measured on, so each cache entry is
//! keyed by a fingerprint covering the CPU model, the core and CPU counts, and
//! the online CPU list. Change any of those, by offlining a CPU or moving the
//! disk to another box, and the fingerprint changes and the stale profile is
//! simply not found. That is deliberately conservative: a wrong pin derived
//! from a stale profile is worse than no pin at all.

use std::path::{Path, PathBuf};

use crate::{Analysis, SCHEMA_VERSION};
use corescout_core::error::{Error, Result};
use corescout_substrate::topology::Topology;

/// Where profiles live.
///
/// Follows the XDG base directory spec on Linux: `$XDG_CACHE_HOME/corescout`,
/// falling back to `$HOME/.cache/corescout`.
pub fn cache_dir() -> Result<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("corescout"));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        if !home.is_empty() {
            return Ok(PathBuf::from(home).join(".cache").join("corescout"));
        }
    }
    Err(Error::unsupported(
        "no cache directory: neither XDG_CACHE_HOME nor HOME is set",
    ))
}

/// A short, stable fingerprint of the machine's CPU configuration.
///
/// Not a hash of the whole topology: frequencies drift between reads and would
/// invalidate the cache on every run for no reason.
pub fn fingerprint(topology: &Topology) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64; // FNV-1a
    let mut mix = |bytes: &[u8]| {
        for b in bytes {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
    };
    mix(topology.model_name.as_bytes());
    mix(topology.vendor.as_bytes());
    mix(&(topology.physical_count() as u64).to_le_bytes());
    mix(&(topology.logical_count() as u64).to_le_bytes());
    for cpu in &topology.online_cpus {
        mix(&cpu.to_le_bytes());
    }
    format!("{hash:016x}")
}

/// Path of the profile for a given topology.
pub fn profile_path(topology: &Topology) -> Result<PathBuf> {
    Ok(cache_dir()?.join(format!("profile-{}.json", fingerprint(topology))))
}

/// Write an analysis to the cache, creating the directory if needed.
pub fn save(analysis: &Analysis) -> Result<PathBuf> {
    let path = profile_path(&analysis.topology)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    let json = serde_json::to_string_pretty(analysis)
        .map_err(|e| Error::invalid(format!("could not serialise the analysis: {e}")))?;
    // Write to a temporary file and rename, so a profile is never observed
    // half-written by a concurrent `corescout run`.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| Error::io(&tmp, e))?;
    std::fs::rename(&tmp, &path).map_err(|e| Error::io(&path, e))?;
    Ok(path)
}

/// Load the cached profile for this topology, if there is a usable one.
///
/// Returns `Ok(None)` when no profile exists or the stored one is from an
/// incompatible version, since neither is an error the user needs to act on:
/// the caller just measures again.
pub fn load(topology: &Topology) -> Result<Option<Analysis>> {
    let path = profile_path(topology)?;
    load_from(&path)
}

/// Load a profile from an explicit path.
pub fn load_from(path: &Path) -> Result<Option<Analysis>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io(path, e)),
    };
    let analysis: Analysis = match serde_json::from_str(&raw) {
        Ok(a) => a,
        // A profile written by a different CoreScout version is not a failure;
        // it is a cache miss.
        Err(_) => return Ok(None),
    };
    if analysis.schema_version != SCHEMA_VERSION {
        return Ok(None);
    }
    Ok(Some(analysis))
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_substrate::test_support::fake_topology;

    #[test]
    fn fingerprint_is_stable_for_the_same_machine() {
        let a = fake_topology();
        let mut b = fake_topology();
        // Frequency readings change constantly and must not invalidate.
        b.logical_cpus[0].frequency.current_khz = Some(4_000_000);
        assert_eq!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn fingerprint_changes_when_the_machine_does() {
        let a = fake_topology();

        let mut renamed = fake_topology();
        renamed.model_name = "Another CPU".into();
        assert_ne!(fingerprint(&a), fingerprint(&renamed));

        let mut offlined = fake_topology();
        offlined.online_cpus = vec![0, 1, 2];
        assert_ne!(
            fingerprint(&a),
            fingerprint(&offlined),
            "offlining a CPU must invalidate the profile"
        );
    }

    #[test]
    fn missing_profile_is_a_miss_not_an_error() {
        let path = std::env::temp_dir().join("corescout-does-not-exist-4f3a.json");
        assert!(load_from(&path).unwrap().is_none());
    }

    #[test]
    fn corrupt_or_foreign_profile_is_a_miss() {
        let dir = std::env::temp_dir().join("corescout-test-cache");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corrupt.json");
        std::fs::write(&path, "{ not json ").unwrap();
        assert!(load_from(&path).unwrap().is_none());

        let path = dir.join("future.json");
        std::fs::write(&path, r#"{"schema_version": 9999}"#).unwrap();
        assert!(load_from(&path).unwrap().is_none());
    }

    #[test]
    fn cache_dir_honours_xdg() {
        // Set both so the test does not depend on the developer's environment.
        std::env::set_var("XDG_CACHE_HOME", "/tmp/xdg-test");
        let dir = cache_dir().unwrap();
        std::env::remove_var("XDG_CACHE_HOME");
        assert!(dir.ends_with("corescout"));
        assert!(dir.to_string_lossy().contains("xdg-test"));
    }
}
