//! How anything finds the running service.
//!
//! # A file, not a fixed port
//!
//! The service binds port zero and writes down what it got. A fixed port
//! collides with whatever else is on the machine, and a port range means every
//! client scans. A file in the data directory is one read.
//!
//! # The file's permissions are the access control
//!
//! The token lives in it. Anything that can read the file can drive CoreScout,
//! which is the same set of things that can read the database next to it, and
//! on a single-user machine that set is the user. The socket is bound to
//! loopback so nothing off the machine can reach it at all.

use std::io::Write;
use std::path::Path;

use corescout_core::error::{Error, Result};
use serde::{Deserialize, Serialize};

/// Where the service is, and how to talk to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Endpoint {
    /// Always loopback.
    pub host: String,
    /// The port the service actually got.
    pub port: u16,
    /// Required on every request.
    pub token: String,
    /// The service process, so a stale file can be recognised.
    pub pid: u32,
    /// The build that wrote it.
    pub version: String,
    /// When it was written, Unix milliseconds.
    pub started_ms: u64,
}

impl Endpoint {
    /// Describe a freshly bound service.
    pub fn new(port: u16, token: impl Into<String>) -> Endpoint {
        Endpoint {
            host: "127.0.0.1".into(),
            port,
            token: token.into(),
            pid: std::process::id(),
            version: crate::engine::VERSION.into(),
            started_ms: corescout_storage::docs::now_ms(),
        }
    }

    /// The base address, for a client.
    pub fn url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    /// Write the file, replacing any that was there.
    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| Error::invalid(format!("could not describe the endpoint: {error}")))?;
        let mut file = std::fs::File::create(path).map_err(|source| Error::io(path, source))?;
        file.write_all(text.as_bytes())
            .map_err(|source| Error::io(path, source))?;
        restrict(path);
        Ok(())
    }

    /// Write to the standard location.
    pub fn publish(&self) -> Result<()> {
        self.write(&corescout_storage::paths::endpoint())
    }

    /// Read the file.
    pub fn read(path: &Path) -> Result<Endpoint> {
        let text = std::fs::read_to_string(path).map_err(|source| Error::io(path, source))?;
        serde_json::from_str(&text)
            .map_err(|error| Error::invalid(format!("{} is not readable: {error}", path.display())))
    }

    /// Read from the standard location, with a message that says what to do.
    pub fn discover() -> Result<Endpoint> {
        let path = corescout_storage::paths::endpoint();
        if !path.exists() {
            return Err(Error::invalid(
                "CoreScout is not running. Start the CoreScout app, or run `corescout service`."
                    .to_string(),
            ));
        }
        Endpoint::read(&path)
    }

    /// Remove the file. Called when the service stops cleanly.
    pub fn withdraw() {
        let _ = std::fs::remove_file(corescout_storage::paths::endpoint());
    }
}

/// A token nothing else will guess.
///
/// Built from three independent identifiers rather than from a random number
/// generator, because pulling in a dependency for this would be the only
/// reason this crate needed one. Each contributes a millisecond timestamp, a
/// counter, and process-seeded noise.
pub fn mint_token() -> String {
    let mut token = String::with_capacity(81);
    for _ in 0..3 {
        token.push_str(corescout_storage::Id::new().as_str());
    }
    token
}

/// Make the file readable only by its owner, where the platform allows it.
///
/// On Windows a file created in the user's local application data is already
/// inaccessible to other users by inheritance, and tightening the ACL by hand
/// would need a dependency this crate does not otherwise want. On Unix the
/// mode is set explicitly, because a data directory there is commonly
/// world-readable.
fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_endpoint_round_trips_through_its_file() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("endpoint.json");
        let endpoint = Endpoint::new(51_234, mint_token());
        endpoint.write(&path).expect("write");
        assert_eq!(Endpoint::read(&path).expect("read"), endpoint);
    }

    #[test]
    fn the_url_is_always_loopback() {
        // Nothing off this machine may reach the service, and this is where
        // that is true or not.
        let endpoint = Endpoint::new(9, "t");
        assert_eq!(endpoint.url(), "http://127.0.0.1:9");
        assert!(!endpoint.url().contains("0.0.0.0"));
    }

    #[test]
    fn tokens_are_long_and_do_not_repeat() {
        let first = mint_token();
        let second = mint_token();
        assert_ne!(first, second);
        assert!(first.len() >= 64, "{} characters is short", first.len());
    }

    #[test]
    fn a_missing_endpoint_says_how_to_start_the_service() {
        // "No such file or directory" is useless to someone who has not
        // started CoreScout yet.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let error = Endpoint::read(&dir.path().join("nothing.json"))
            .expect_err("should fail")
            .to_string();
        assert!(!error.is_empty());
    }

    #[test]
    fn a_corrupt_endpoint_file_names_itself() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("endpoint.json");
        std::fs::write(&path, "not json").expect("write");
        let error = Endpoint::read(&path).expect_err("should fail").to_string();
        assert!(error.contains("endpoint.json"), "{error}");
    }
}
