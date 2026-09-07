//! Repositories and folders, as persistent entities.
//!
//! What CoreScout learns is usually learned *about somewhere*: this repository
//! needs its schema regenerated before it builds, that one has a test that
//! fails when the machine is busy. A workspace is the thing those facts attach
//! to, and it has to survive the sessions that discovered them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A folder an agent has worked in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    /// Stable across sessions, derived from the path.
    pub id: String,
    /// The root folder.
    pub root: PathBuf,
    /// What to call it: the folder name, unless something better is known.
    pub name: String,
    /// Which version control system, if any was detected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcs: Option<String>,
    /// When it was first seen, Unix milliseconds.
    pub first_seen_ms: u64,
    /// When it was last worked in.
    pub last_seen_ms: u64,
    /// Sessions that have touched it.
    pub sessions: u64,
    /// Actions observed in it.
    pub actions: u64,
    /// Actions that visibly failed in it.
    pub failures: u64,
}

impl Workspace {
    /// Register a folder.
    pub fn new(root: impl Into<PathBuf>, now_ms: u64) -> Workspace {
        let root = root.into();
        let name = root
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| root.display().to_string());
        Workspace {
            id: identify(&root),
            root,
            name,
            vcs: None,
            first_seen_ms: now_ms,
            last_seen_ms: now_ms,
            sessions: 0,
            actions: 0,
            failures: 0,
        }
    }

    /// The share of actions here that visibly failed.
    pub fn failure_rate(&self) -> Option<f64> {
        (self.actions > 0).then(|| self.failures as f64 / self.actions as f64)
    }

    /// Note activity.
    pub fn touch(&mut self, now_ms: u64) {
        self.last_seen_ms = now_ms;
    }

    /// Express a path relative to this workspace where possible.
    ///
    /// Stored paths are relative so that what was learned about a repository
    /// still applies after it is moved or cloned somewhere else, and so a
    /// support export does not carry a home directory layout.
    pub fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .map(|rest| rest.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"))
    }
}

/// A stable identifier for a folder.
///
/// Derived from the path rather than from anything inside it, so a workspace
/// is recognised before CoreScout has read a single file in it. Case-folded,
/// because Windows paths differ in case between one tool and the next and they
/// are the same folder.
pub fn identify(root: &Path) -> String {
    let text = root
        .to_string_lossy()
        .to_lowercase()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let slug: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(24)
        .collect();
    if slug.is_empty() {
        format!("ws-{hash:016x}")
    } else {
        format!("{slug}-{:08x}", hash as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_folder_is_the_same_workspace_across_sessions() {
        // This is what lets knowledge about a repository outlive the session
        // that discovered it.
        let a = Workspace::new("C:\\Projects\\app", 1);
        let b = Workspace::new("C:\\Projects\\app", 9999);
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn windows_path_casing_does_not_split_one_folder_into_two() {
        let a = Workspace::new("C:\\Projects\\App", 1);
        let b = Workspace::new("c:\\projects\\app", 1);
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn a_trailing_separator_does_not_split_one_folder_into_two() {
        assert_eq!(
            identify(Path::new("C:\\Projects\\app\\")),
            identify(Path::new("C:\\Projects\\app"))
        );
    }

    #[test]
    fn different_folders_get_different_identifiers() {
        let a = Workspace::new("C:\\Projects\\app", 1);
        let b = Workspace::new("C:\\Projects\\other", 1);
        let c = Workspace::new("D:\\Projects\\app", 1);
        assert_ne!(a.id, b.id);
        assert_ne!(a.id, c.id, "same name, different drive");
    }

    #[test]
    fn the_identifier_is_readable_and_usable_as_a_storage_key() {
        let workspace = Workspace::new("C:\\Projects\\my-app", 1);
        assert!(workspace.id.starts_with("my-app-"), "{}", workspace.id);
        assert!(workspace
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    }

    #[test]
    fn paths_are_stored_relative_to_the_workspace() {
        // So what was learned still applies after the repository is cloned
        // somewhere else, and an export carries no home directory layout.
        let workspace = Workspace::new("C:\\Projects\\app", 1);
        assert_eq!(
            workspace.relative(Path::new("C:\\Projects\\app\\src\\main.rs")),
            "src/main.rs"
        );
    }

    #[test]
    fn a_path_outside_the_workspace_is_kept_as_it_is() {
        let workspace = Workspace::new("C:\\Projects\\app", 1);
        let outside = Path::new("C:\\Other\\thing.txt");
        assert_eq!(workspace.relative(outside), "C:/Other/thing.txt");
    }

    #[test]
    fn a_workspace_with_no_actions_has_no_failure_rate() {
        assert_eq!(Workspace::new("C:\\Projects\\app", 1).failure_rate(), None);
    }

    #[test]
    fn the_name_is_the_folder_name() {
        assert_eq!(Workspace::new("C:\\Projects\\my-app", 1).name, "my-app");
    }

    #[test]
    fn a_workspace_survives_storage() {
        let mut workspace = Workspace::new("C:\\Projects\\app", 1);
        workspace.vcs = Some("git".into());
        let json = serde_json::to_string(&workspace).expect("serialise");
        assert_eq!(
            serde_json::from_str::<Workspace>(&json).expect("deserialise"),
            workspace
        );
    }
}
