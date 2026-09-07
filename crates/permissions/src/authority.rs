//! What may be touched, regardless of how much autonomy is switched on.
//!
//! # Authority is not autonomy
//!
//! Autonomy decides whether a person is asked. Authority decides what is on
//! the table at all. Raising autonomy to Autopilot must not widen reach, so
//! the authority check runs first and no mode can override it.
//!
//! # The forbidden list is not a policy, it is a floor
//!
//! Some paths are refused whatever the user grants: the Windows directory,
//! the system32 tree, the registry hives, and the places credentials live.
//! A person can grant CoreScout their project folder. They cannot grant it
//! their SSH keys, because there is no operational improvement that requires
//! those and an accident that reaches them is unrecoverable.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Something an action would touch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum Target {
    /// CoreScout's own process, its threads and its scheduling.
    OwnProcess,
    /// A process registered by the user or by an opted-in agent session.
    Process(i32),
    /// A file or directory.
    Path(PathBuf),
    /// A program to run, named by its executable.
    Command(String),
    /// A Windows service or a daemon.
    Service(String),
    /// An environment variable in a workload CoreScout launches.
    Environment(String),
    /// Anything that leaves this machine.
    Network(String),
}

impl Target {
    /// A short label for the interface and the audit log.
    pub fn label(&self) -> String {
        match self {
            Target::OwnProcess => "CoreScout itself".into(),
            Target::Process(pid) => format!("process {pid}"),
            Target::Path(path) => path.display().to_string(),
            Target::Command(name) => format!("command {name}"),
            Target::Service(name) => format!("service {name}"),
            Target::Environment(name) => format!("environment {name}"),
            Target::Network(host) => format!("network {host}"),
        }
    }
}

/// Why a target was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Denial {
    /// The path is on the floor list and cannot be granted by anyone.
    Forbidden(PathBuf),
    /// Nothing has granted this area.
    OutsideGrantedArea(String),
    /// The command is not on the allowed list.
    UnknownCommand(String),
    /// Nothing leaves this machine unless it was explicitly allowed to.
    NetworkNotPermitted(String),
    /// The process was never registered.
    UnregisteredProcess(i32),
}

impl std::fmt::Display for Denial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Denial::Forbidden(path) => write!(
                f,
                "{} is off limits to CoreScout and cannot be granted",
                path.display()
            ),
            Denial::OutsideGrantedArea(what) => {
                write!(f, "{what} is outside the folders you have shared")
            }
            Denial::UnknownCommand(name) => {
                write!(f, "{name} is not one of the commands CoreScout may run")
            }
            Denial::NetworkNotPermitted(host) => {
                write!(f, "CoreScout is not allowed to reach {host}")
            }
            Denial::UnregisteredProcess(pid) => {
                write!(f, "process {pid} was never registered with CoreScout")
            }
        }
    }
}

/// Paths CoreScout refuses whatever it has been granted.
///
/// Matched case-insensitively against any component of the path, so this
/// catches `C:\Windows\System32` and `/etc/shadow` alike without needing a
/// platform switch.
const FORBIDDEN: &[&str] = &[
    "\\windows\\system32",
    "\\windows\\syswow64",
    "\\windows\\winsxs",
    "\\$recycle.bin",
    "\\config\\systemprofile",
    "\\.ssh",
    "\\.aws",
    "\\.gnupg",
    "\\microsoft\\credentials",
    "\\microsoft\\protect",
    "/etc/shadow",
    "/etc/sudoers",
    "/.ssh",
    "/.aws",
    "/.gnupg",
];

/// Filenames that are refused wherever they appear.
const FORBIDDEN_NAMES: &[&str] = &[
    "id_rsa",
    "id_ed25519",
    ".env",
    ".npmrc",
    "credentials",
    "sam",
    "security",
    "ntuser.dat",
];

/// The reach CoreScout has been given.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Authority {
    /// Directories CoreScout may read and write within.
    roots: Vec<PathBuf>,
    /// Executables it may run.
    commands: BTreeSet<String>,
    /// Services it may start, stop or reorder.
    services: BTreeSet<String>,
    /// Hosts it may reach. Empty by default, and normally stays empty.
    hosts: BTreeSet<String>,
    /// Processes registered for control.
    processes: BTreeSet<i32>,
    /// Whether CoreScout may act on its own process.
    own_process: bool,
}

impl Authority {
    /// The starting authority: CoreScout's own process, and nothing else.
    pub fn own_process_only() -> Authority {
        Authority {
            own_process: true,
            ..Authority::default()
        }
    }

    /// An authority that permits nothing at all, for a dry run.
    pub fn none() -> Authority {
        Authority::default()
    }

    /// Grant a directory.
    ///
    /// Returns false if the directory is on the floor list, in which case
    /// nothing is granted and the caller should say so rather than pretending
    /// it worked.
    pub fn grant_root(&mut self, path: impl Into<PathBuf>) -> bool {
        let path = path.into();
        if is_forbidden(&path) {
            return false;
        }
        if !self.roots.contains(&path) {
            self.roots.push(path);
        }
        true
    }

    /// Withdraw a directory.
    pub fn revoke_root(&mut self, path: &Path) {
        self.roots.retain(|root| root != path);
    }

    /// Directories currently granted.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Allow an executable to be run.
    pub fn grant_command(&mut self, name: impl Into<String>) {
        self.commands.insert(normalise(&name.into()));
    }

    /// Commands currently allowed.
    pub fn commands(&self) -> Vec<String> {
        self.commands.iter().cloned().collect()
    }

    /// Allow a service to be managed.
    pub fn grant_service(&mut self, name: impl Into<String>) {
        self.services.insert(normalise(&name.into()));
    }

    /// Allow a host to be reached. Rare, and deliberately explicit.
    pub fn grant_host(&mut self, host: impl Into<String>) {
        self.hosts.insert(normalise(&host.into()));
    }

    /// Register a process for control.
    pub fn register_process(&mut self, pid: i32) {
        self.processes.insert(pid);
    }

    /// Unregister a process.
    pub fn unregister_process(&mut self, pid: i32) {
        self.processes.remove(&pid);
    }

    /// Processes currently registered.
    pub fn processes(&self) -> Vec<i32> {
        self.processes.iter().copied().collect()
    }

    /// Whether every target is within reach.
    pub fn permits(&self, targets: &[Target]) -> Result<(), Denial> {
        for target in targets {
            self.permits_one(target)?;
        }
        Ok(())
    }

    /// Whether one target is within reach.
    pub fn permits_one(&self, target: &Target) -> Result<(), Denial> {
        match target {
            Target::OwnProcess => {
                if self.own_process {
                    Ok(())
                } else {
                    Err(Denial::OutsideGrantedArea("CoreScout itself".into()))
                }
            }
            Target::Process(pid) => {
                if self.processes.contains(pid) {
                    Ok(())
                } else {
                    Err(Denial::UnregisteredProcess(*pid))
                }
            }
            Target::Path(path) => {
                if is_forbidden(path) {
                    return Err(Denial::Forbidden(path.clone()));
                }
                if self.roots.iter().any(|root| under(path, root)) {
                    Ok(())
                } else {
                    Err(Denial::OutsideGrantedArea(path.display().to_string()))
                }
            }
            Target::Command(name) => {
                if self.commands.contains(&normalise(name)) {
                    Ok(())
                } else {
                    Err(Denial::UnknownCommand(name.clone()))
                }
            }
            Target::Service(name) => {
                if self.services.contains(&normalise(name)) {
                    Ok(())
                } else {
                    Err(Denial::OutsideGrantedArea(format!("service {name}")))
                }
            }
            // An environment variable only ever applies to a workload
            // CoreScout itself launches, so it is bounded by the same
            // authority that let it launch one.
            Target::Environment(_) => Ok(()),
            Target::Network(host) => {
                if self.hosts.contains(&normalise(host)) {
                    Ok(())
                } else {
                    Err(Denial::NetworkNotPermitted(host.clone()))
                }
            }
        }
    }

    /// A short description for the Settings page.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if self.own_process {
            parts.push("its own process".to_string());
        }
        match self.roots.len() {
            0 => {}
            1 => parts.push(format!("1 folder ({})", self.roots[0].display())),
            n => parts.push(format!("{n} folders")),
        }
        if !self.processes.is_empty() {
            parts.push(format!("{} registered processes", self.processes.len()));
        }
        if !self.commands.is_empty() {
            parts.push(format!("{} commands", self.commands.len()));
        }
        if !self.services.is_empty() {
            parts.push(format!("{} services", self.services.len()));
        }
        if !self.hosts.is_empty() {
            parts.push(format!("{} network hosts", self.hosts.len()));
        }
        if parts.is_empty() {
            return "nothing".into();
        }
        parts.join(", ")
    }
}

/// Whether a path is on the floor list.
pub fn is_forbidden(path: &Path) -> bool {
    let text = path.to_string_lossy().to_lowercase().replace('/', "\\");
    if FORBIDDEN.iter().any(|bad| {
        let bad = bad.replace('/', "\\");
        text.contains(&bad)
    }) {
        return true;
    }
    path.file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .is_some_and(|name| FORBIDDEN_NAMES.contains(&name.as_str()))
}

/// Whether `path` is inside `root`, resisting the obvious escape.
///
/// Compared component by component rather than by string prefix, because
/// `C:\Projects\app` is not inside `C:\Projects\a`, and a prefix test says it
/// is. `..` anywhere in the candidate is refused outright rather than
/// resolved, since resolving it would need the filesystem and this crate does
/// not touch one.
fn under(path: &Path, root: &Path) -> bool {
    use std::path::Component;
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    let mut root_parts = root.components();
    let mut path_parts = path.components();
    loop {
        match (root_parts.next(), path_parts.next()) {
            (None, _) => return true,
            (Some(_), None) => return false,
            (Some(a), Some(b)) => {
                if a.as_os_str().to_string_lossy().to_lowercase()
                    != b.as_os_str().to_string_lossy().to_lowercase()
                {
                    return false;
                }
            }
        }
    }
}

fn normalise(value: &str) -> String {
    value.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_starting_authority_reaches_only_corescout_itself() {
        let authority = Authority::own_process_only();
        assert!(authority.permits_one(&Target::OwnProcess).is_ok());
        assert!(authority.permits_one(&Target::Process(4242)).is_err());
        assert!(authority
            .permits_one(&Target::Path(PathBuf::from("C:\\Projects\\app")))
            .is_err());
    }

    #[test]
    fn a_granted_folder_permits_what_is_inside_it() {
        let mut authority = Authority::own_process_only();
        assert!(authority.grant_root("C:\\Projects\\app"));
        assert!(authority
            .permits_one(&Target::Path(PathBuf::from(
                "C:\\Projects\\app\\src\\main.rs"
            )))
            .is_ok());
        assert!(authority
            .permits_one(&Target::Path(PathBuf::from("C:\\Projects\\other\\x")))
            .is_err());
    }

    #[test]
    fn a_sibling_that_shares_a_prefix_is_not_inside() {
        // `C:\Projects\a` and `C:\Projects\app` share a string prefix and
        // share no directory. A prefix test gets this wrong and grants the
        // whole neighbouring project.
        let mut authority = Authority::own_process_only();
        authority.grant_root("C:\\Projects\\a");
        assert!(authority
            .permits_one(&Target::Path(PathBuf::from("C:\\Projects\\app\\secret")))
            .is_err());
    }

    #[test]
    fn dot_dot_does_not_climb_out_of_a_granted_folder() {
        let mut authority = Authority::own_process_only();
        authority.grant_root("C:\\Projects\\app");
        let escape = PathBuf::from("C:\\Projects\\app\\..\\..\\Windows\\System32\\config");
        assert!(authority.permits_one(&Target::Path(escape)).is_err());
    }

    #[test]
    fn the_forbidden_floor_cannot_be_granted() {
        // Not "is refused when unlisted" but "cannot be added at all". If a
        // future settings screen loops over user input calling grant_root,
        // this is what stops it.
        let mut authority = Authority::own_process_only();
        assert!(!authority.grant_root("C:\\Windows\\System32"));
        assert!(!authority.grant_root("/home/someone/.ssh"));
        assert!(authority.roots().is_empty());
    }

    #[test]
    fn a_forbidden_path_inside_a_granted_folder_is_still_forbidden() {
        // Someone grants their home directory. Their SSH key is inside it.
        let mut authority = Authority::own_process_only();
        assert!(authority.grant_root("C:\\Users\\someone"));
        let key = PathBuf::from("C:\\Users\\someone\\.ssh\\id_ed25519");
        assert_eq!(
            authority.permits_one(&Target::Path(key.clone())),
            Err(Denial::Forbidden(key))
        );
    }

    #[test]
    fn credential_shaped_filenames_are_refused_wherever_they_live() {
        let mut authority = Authority::own_process_only();
        authority.grant_root("C:\\Projects\\app");
        for name in ["id_rsa", ".env", ".npmrc"] {
            let path = PathBuf::from(format!("C:\\Projects\\app\\{name}"));
            assert!(
                authority.permits_one(&Target::Path(path)).is_err(),
                "{name} should be refused"
            );
        }
    }

    #[test]
    fn case_does_not_defeat_the_floor_list() {
        assert!(is_forbidden(Path::new("C:\\WINDOWS\\System32\\drivers")));
        assert!(is_forbidden(Path::new("c:/windows/system32/drivers")));
    }

    #[test]
    fn nothing_reaches_the_network_by_default() {
        let authority = Authority::own_process_only();
        assert_eq!(
            authority.permits_one(&Target::Network("example.com".into())),
            Err(Denial::NetworkNotPermitted("example.com".into()))
        );
    }

    #[test]
    fn a_command_must_be_named_before_it_can_be_run() {
        let mut authority = Authority::own_process_only();
        assert!(authority
            .permits_one(&Target::Command("cargo".into()))
            .is_err());
        authority.grant_command("cargo");
        assert!(authority
            .permits_one(&Target::Command("Cargo".into()))
            .is_ok());
        assert!(authority
            .permits_one(&Target::Command("curl".into()))
            .is_err());
    }

    #[test]
    fn a_set_of_targets_is_permitted_only_if_all_of_them_are() {
        let mut authority = Authority::own_process_only();
        authority.grant_root("C:\\Projects\\app");
        authority.grant_command("cargo");
        let good = vec![
            Target::Command("cargo".into()),
            Target::Path(PathBuf::from("C:\\Projects\\app\\Cargo.toml")),
        ];
        assert!(authority.permits(&good).is_ok());
        let mixed = vec![
            Target::Command("cargo".into()),
            Target::Network("crates.io".into()),
        ];
        assert!(authority.permits(&mixed).is_err());
    }

    #[test]
    fn revoking_takes_the_reach_away_again() {
        let mut authority = Authority::own_process_only();
        authority.grant_root("C:\\Projects\\app");
        let file = Target::Path(PathBuf::from("C:\\Projects\\app\\x"));
        assert!(authority.permits_one(&file).is_ok());
        authority.revoke_root(Path::new("C:\\Projects\\app"));
        assert!(authority.permits_one(&file).is_err());
    }

    #[test]
    fn the_description_is_readable_and_never_empty() {
        assert_eq!(Authority::none().describe(), "nothing");
        let mut authority = Authority::own_process_only();
        assert_eq!(authority.describe(), "its own process");
        authority.grant_root("C:\\Projects\\app");
        assert!(authority.describe().contains("1 folder"));
        authority.grant_root("C:\\Projects\\web");
        assert!(authority.describe().contains("2 folders"));
    }

    #[test]
    fn every_denial_says_something_a_person_can_act_on() {
        let denials = [
            Denial::Forbidden(PathBuf::from("C:\\Windows\\System32")),
            Denial::OutsideGrantedArea("C:\\Other".into()),
            Denial::UnknownCommand("curl".into()),
            Denial::NetworkNotPermitted("example.com".into()),
            Denial::UnregisteredProcess(9),
        ];
        for denial in denials {
            let text = denial.to_string();
            assert!(text.len() > 15, "{text}");
            assert!(!text.contains("Err("), "{text}");
        }
    }

    #[test]
    fn authority_survives_being_stored_and_read_back() {
        let mut authority = Authority::own_process_only();
        authority.grant_root("C:\\Projects\\app");
        authority.grant_command("cargo");
        authority.register_process(1234);
        let json = serde_json::to_string(&authority).expect("serialise");
        let back: Authority = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back, authority);
        assert!(back.permits_one(&Target::Process(1234)).is_ok());
    }
}
