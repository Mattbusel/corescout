//! What an actuator is allowed to touch.
//!
//! # Opt-in, always
//!
//! An experimental controller must not be able to reorganise a machine it
//! shares with other people's work. The default scope is this process and
//! nothing else. A pid enters scope only by being registered explicitly, and
//! registration checks that the caller could signal it anyway, so scope can
//! never grant more reach than the operating system already would.
//!
//! There is deliberately no "everything" variant. Adding one later would be a
//! two-line change and should be argued for on its own; leaving it out means
//! the question has to be asked.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Target;

/// Why a target was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeError {
    /// The pid was never registered.
    NotRegistered(i32),
    /// The pid was registered but has gone away, or is no longer reachable.
    Unreachable(i32),
    /// The scope is frozen: an emergency stop is in force.
    Frozen,
}

impl fmt::Display for ScopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScopeError::NotRegistered(pid) => write!(
                f,
                "process {pid} is not in scope; register it explicitly before acting on it"
            ),
            ScopeError::Unreachable(pid) => {
                write!(f, "process {pid} is registered but no longer reachable")
            }
            ScopeError::Frozen => write!(f, "actuation is frozen by an emergency stop"),
        }
    }
}

impl std::error::Error for ScopeError {}

/// The set of processes an actuator may touch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    /// Explicitly registered pids.
    registered: BTreeSet<i32>,
    /// Whether the controlling process may act on itself. True by default: a
    /// controller experimenting on its own threads is the least dangerous case
    /// and the one the self-experimentation loop needs.
    include_self: bool,
    /// When set, nothing is permitted at all.
    frozen: bool,
}

impl Scope {
    /// The default: this process only.
    pub fn own_process() -> Scope {
        Scope {
            registered: BTreeSet::new(),
            include_self: true,
            frozen: false,
        }
    }

    /// A scope that permits nothing, for a dry run.
    pub fn none() -> Scope {
        Scope {
            registered: BTreeSet::new(),
            include_self: false,
            frozen: false,
        }
    }

    /// Register a process, after checking it is reachable.
    ///
    /// Returns false when the pid cannot be signalled by this user, which means
    /// registering it would have been a lie: the actuator would fail later with
    /// `EPERM` and the audit log would blame the wrong thing.
    pub fn register(&mut self, pid: i32) -> bool {
        if !reachable(pid) {
            return false;
        }
        self.registered.insert(pid);
        true
    }

    pub fn unregister(&mut self, pid: i32) {
        self.registered.remove(&pid);
    }

    pub fn registered(&self) -> impl Iterator<Item = i32> + '_ {
        self.registered.iter().copied()
    }

    pub fn len(&self) -> usize {
        self.registered.len() + usize::from(self.include_self)
    }

    pub fn is_empty(&self) -> bool {
        self.registered.is_empty() && !self.include_self
    }

    /// Stop everything. Survives until explicitly lifted.
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    pub fn unfreeze(&mut self) {
        self.frozen = false;
    }

    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Check a target, with a reason when it is refused.
    pub fn permits(&self, target: Target) -> Result<(), ScopeError> {
        if self.frozen {
            return Err(ScopeError::Frozen);
        }
        match target.owning_pid() {
            None => {
                if self.include_self {
                    Ok(())
                } else {
                    Err(ScopeError::NotRegistered(0))
                }
            }
            Some(pid) => {
                if !self.registered.contains(&pid) {
                    return Err(ScopeError::NotRegistered(pid));
                }
                if !reachable(pid) {
                    return Err(ScopeError::Unreachable(pid));
                }
                Ok(())
            }
        }
    }

    /// Drop registrations for processes that have exited.
    ///
    /// Pids are reused. A registration left behind after a process exits could
    /// silently come to mean a different process, which is a way for a bounded
    /// controller to act far outside its bounds without any single line of code
    /// being wrong.
    pub fn prune(&mut self) -> usize {
        let before = self.registered.len();
        self.registered.retain(|pid| reachable(*pid));
        before - self.registered.len()
    }
}

fn reachable(pid: i32) -> bool {
    #[cfg(target_os = "linux")]
    {
        crate::linux::process_is_reachable(pid)
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Without a way to check, refuse. Optimism here would let a scope claim
        // authority it cannot demonstrate.
        let _ = pid;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_scope_is_this_process_and_nothing_else() {
        let scope = Scope::own_process();
        assert!(scope.permits(Target::CurrentThread).is_ok());
        assert!(scope.permits(Target::CurrentProcess).is_ok());
        assert_eq!(
            scope.permits(Target::Process(4242)),
            Err(ScopeError::NotRegistered(4242))
        );
    }

    #[test]
    fn an_empty_scope_permits_nothing() {
        let scope = Scope::none();
        assert!(scope.permits(Target::CurrentThread).is_err());
        assert!(scope.is_empty());
    }

    #[test]
    fn freezing_overrides_every_permission() {
        let mut scope = Scope::own_process();
        scope.freeze();
        assert_eq!(
            scope.permits(Target::CurrentThread),
            Err(ScopeError::Frozen)
        );
        scope.unfreeze();
        assert!(scope.permits(Target::CurrentThread).is_ok());
    }

    #[test]
    fn registering_an_unreachable_pid_fails_rather_than_pretending() {
        let mut scope = Scope::own_process();
        // Pid 1 is init; an ordinary user cannot signal it. On platforms where
        // reachability cannot be checked at all, nothing registers.
        assert!(!scope.register(-1));
        assert!(scope.permits(Target::Process(-1)).is_err());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn our_own_pid_can_be_registered_and_pruned() {
        let mut scope = Scope::none();
        let pid = std::process::id() as i32;
        assert!(scope.register(pid));
        assert!(scope.permits(Target::Process(pid)).is_ok());
        assert_eq!(scope.prune(), 0, "a live process must survive pruning");
        scope.unregister(pid);
        assert!(scope.permits(Target::Process(pid)).is_err());
    }
}
