//! Where CoreScout keeps its data, and why it is that place.
//!
//! Everything lives under one directory so that "delete my data" is a
//! directory removal a user can perform themselves without trusting the
//! uninstaller, and so the Privacy page can name a single path.

use std::path::PathBuf;

/// The directory holding the database, the ring, and the logs.
///
/// * Windows: `%LOCALAPPDATA%\CoreScout`. Local rather than roaming, because a
///   ring file of machine telemetry has no business following a user onto
///   another machine.
/// * Otherwise: `$XDG_DATA_HOME/corescout`, else `~/.local/share/corescout`.
///
/// `CORESCOUT_DATA_DIR` overrides both, which is what the tests use.
pub fn data_dir() -> PathBuf {
    if let Some(explicit) = std::env::var_os("CORESCOUT_DATA_DIR") {
        return PathBuf::from(explicit);
    }
    // A packaged build's writes under `%LOCALAPPDATA%` are redirected by
    // Windows into the package's own store, so writing there would succeed and
    // put the files somewhere this function does not name. The Privacy page
    // shows whatever this returns, and it has to be true.
    if let Some(packaged) = crate::packaged::package_data_dir() {
        return packaged;
    }
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local).join("CoreScout");
        }
    }
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("corescout");
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        return PathBuf::from(home).join(".local/share/corescout");
    }
    PathBuf::from(".corescout")
}

/// The transactional store: documents and the event log.
pub fn database() -> PathBuf {
    data_dir().join("corescout.redb")
}

/// The bounded ring of observation-rate telemetry.
pub fn ring() -> PathBuf {
    data_dir().join("mirror.ring")
}

/// Where the service writes the file describing how to reach it.
///
/// The desktop app, the CLI and the MCP bridge all read this rather than
/// guessing a port, and it carries the loopback token, so its permissions are
/// the access control.
pub fn endpoint() -> PathBuf {
    data_dir().join("endpoint.json")
}

/// Create the data directory if it is not there.
pub fn ensure_data_dir() -> std::io::Result<PathBuf> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These tests mutate a process-global environment variable, so they take
    /// turns. Without this one test reads `data_dir()` while the other is
    /// halfway through pointing it somewhere else.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn an_explicit_data_dir_wins() {
        let _guard = exclusive();
        // Every test in this workspace that touches storage relies on this, so
        // if it stops working the failure should be here and not in fifty
        // other tests writing to a user's real profile.
        let previous = std::env::var_os("CORESCOUT_DATA_DIR");
        std::env::set_var("CORESCOUT_DATA_DIR", "/somewhere/explicit");
        assert_eq!(data_dir(), PathBuf::from("/somewhere/explicit"));
        assert!(database().ends_with("corescout.redb"));
        assert!(ring().ends_with("mirror.ring"));
        match previous {
            Some(value) => std::env::set_var("CORESCOUT_DATA_DIR", value),
            None => std::env::remove_var("CORESCOUT_DATA_DIR"),
        }
    }

    #[test]
    fn a_packaged_build_keeps_its_data_where_it_says_it_does() {
        // Not exercised here -- a test binary is never packaged -- so this
        // asserts the shape of the answer rather than the answer. The failure
        // it guards against is the Privacy page naming a folder the data is
        // not in, which would make "delete everything" delete nothing.
        let _guard = exclusive();
        match crate::packaged::package_data_dir() {
            Some(path) => {
                assert!(path.to_string_lossy().contains("LocalCache"));
                assert!(path.ends_with("CoreScout"));
            }
            None => assert!(!crate::packaged::is_packaged()),
        }
    }

    #[test]
    fn every_file_is_under_one_directory() {
        // The Privacy page names one path. If a file escapes it, that page is
        // lying.
        let _guard = exclusive();
        let dir = data_dir();
        for path in [database(), ring(), endpoint()] {
            assert!(
                path.starts_with(&dir),
                "{} escaped {}",
                path.display(),
                dir.display()
            );
        }
    }
}
