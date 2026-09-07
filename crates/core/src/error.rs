//! Error type shared by every module.
//!
//! CoreScout deliberately avoids a error-handling dependency: the failure modes
//! are few and mostly amount to "a file under /sys was missing or unparseable",
//! so a hand written enum keeps the dependency surface at zero and the messages
//! specific.

use std::fmt;
use std::path::{Path, PathBuf};

/// Result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// An I/O failure, annotated with the path that caused it. Bare
    /// `io::Error`s are almost useless when you are walking hundreds of sysfs
    /// files, so the path is always carried along.
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// A sysfs (or procfs) file existed but did not contain what the kernel ABI
    /// documents.
    Parse { path: PathBuf, detail: String },
    /// A syscall failed. `errno` is captured at the call site.
    Syscall { call: &'static str, errno: i32 },
    /// The running platform has no implementation for a requested capability.
    Unsupported(String),
    /// The user asked for something impossible (bad CPU list, empty core set).
    Invalid(String),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    pub fn parse(path: impl Into<PathBuf>, detail: impl Into<String>) -> Self {
        Error::Parse {
            path: path.into(),
            detail: detail.into(),
        }
    }

    pub fn unsupported(what: impl Into<String>) -> Self {
        Error::Unsupported(what.into())
    }

    pub fn invalid(what: impl Into<String>) -> Self {
        Error::Invalid(what.into())
    }

    /// True when the error is "this file does not exist", which for optional
    /// sysfs attributes is a normal condition rather than a failure.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Error::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => write!(f, "{}: {}", display(path), source),
            Error::Parse { path, detail } => write!(f, "{}: {}", display(path), detail),
            Error::Syscall { call, errno } => {
                write!(f, "{call} failed: errno {errno}")
            }
            Error::Unsupported(what) => write!(f, "unsupported on this platform: {what}"),
            Error::Invalid(what) => write!(f, "{what}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn display(p: &Path) -> String {
    p.display().to_string()
}
