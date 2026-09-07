//! Agency: the only path from software back to the physical machine.
//!
//! ```text
//! Mirror(t) -> decision -> ACTION -> physical machine changes -> Mirror(t+1)
//! ```
//!
//! Everything else in this workspace observes, remembers, learns or predicts.
//! This crate is where something actually happens. It is deliberately the
//! smallest crate that can do that, and the one with the most constraints on
//! it.
//!
//! # The contract every actuator satisfies
//!
//! 1. **Explicit.** No side effect happens as a consequence of observing,
//!    analysing or predicting. If the machine changed, an actuator was called
//!    by name.
//! 2. **Scoped.** An actuator refuses to touch anything outside
//!    [`scope::Scope`]. By default that is this process and processes
//!    explicitly registered with it. There is no system-wide mode.
//! 3. **Bounded.** Rate limits and value ranges are checked before the syscall,
//!    not after.
//! 4. **Reversible.** Every [`Action`] captures the prior state, so
//!    `Actuator::revert` can put it back.
//! 5. **Audited.** Every attempt produces an [`audit::AuditEvent`] recording
//!    what was requested, why, what was expected, and what actually happened.
//! 6. **Honest about outcome.** "The syscall returned 0" and "the machine is
//!    now in the requested state" are different claims. Where they can be
//!    distinguished, [`ActionOutcome`] distinguishes them by reading the state
//!    back.
//!
//! # What is deliberately absent
//!
//! No policy. Nothing here decides *which* action to take; that is
//! `corescout-autonomy`. This crate knows how to carry an action out and how to
//! say what happened. Keeping the two apart is what makes it possible to audit
//! the mechanism separately from the judgement, and to run the mechanism with
//! no judgement attached at all.

pub mod actuators;
pub mod audit;
pub mod invention;
pub mod launch;
pub mod scope;

#[cfg(target_os = "linux")]
mod linux;

pub use actuators::{Actuator, ActuatorSet};
pub use audit::{AuditEvent, AuditLog, Disposition};
pub use scope::{Scope, ScopeError};

use std::fmt;

use corescout_core::{CpuSet, LogicalId};
use serde::{Deserialize, Serialize};

/// What a process or thread is identified by.
///
/// A `Thread` on Linux is a task id from `gettid`, which `sched_setaffinity`
/// accepts in place of a pid. Keeping them distinct in the type stops a caller
/// from accidentally moving a whole process when it meant one thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Target {
    /// The calling thread.
    CurrentThread,
    /// The calling process.
    CurrentProcess,
    /// Another process, by pid. Must be in scope.
    Process(i32),
    /// A specific task, by tid. Must belong to a process in scope.
    Thread(i32),
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::CurrentThread => write!(f, "self:thread"),
            Target::CurrentProcess => write!(f, "self:process"),
            Target::Process(pid) => write!(f, "pid:{pid}"),
            Target::Thread(tid) => write!(f, "tid:{tid}"),
        }
    }
}

impl Target {
    /// The pid or tid to pass to a syscall. `0` means "the caller", which is
    /// what Linux uses for both.
    pub fn raw(self) -> i32 {
        match self {
            Target::CurrentThread | Target::CurrentProcess => 0,
            Target::Process(pid) => pid,
            Target::Thread(tid) => tid,
        }
    }

    /// The pid whose scope membership governs this target.
    pub fn owning_pid(self) -> Option<i32> {
        match self {
            Target::CurrentThread | Target::CurrentProcess => None,
            Target::Process(pid) => Some(pid),
            Target::Thread(tid) => Some(tid),
        }
    }
}

/// Something that can be done to the machine.
///
/// Deliberately a closed enumeration. An open trait would let a caller invent
/// an action the safety layer has never heard of and therefore cannot bound.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ActionKind {
    /// Confine a thread or process to a set of logical CPUs.
    SetAffinity { target: Target, cpus: CpuSet },
    /// Change scheduling niceness. Lower is more favourable; raising it needs
    /// no privilege, lowering it usually does.
    SetPriority { target: Target, nice: i32 },
    /// Prefer a NUMA node for future memory allocations.
    SetNumaPreference { target: Target, node: Option<u32> },
    /// Do nothing. A real choice, and the one a policy should make when its
    /// predicted improvement does not exceed its uncertainty.
    Hold,
}

impl ActionKind {
    pub fn target(&self) -> Option<Target> {
        match self {
            ActionKind::SetAffinity { target, .. }
            | ActionKind::SetPriority { target, .. }
            | ActionKind::SetNumaPreference { target, .. } => Some(*target),
            ActionKind::Hold => None,
        }
    }

    /// Short stable name, for audit records and for grouping actions into
    /// families a model can learn about.
    pub fn family(&self) -> &'static str {
        match self {
            ActionKind::SetAffinity { .. } => "affinity",
            ActionKind::SetPriority { .. } => "priority",
            ActionKind::SetNumaPreference { .. } => "numa",
            ActionKind::Hold => "hold",
        }
    }

    /// CPUs this action would confine work to, where it names any. Used by the
    /// counterfactual model, which reasons about placement.
    pub fn cpus(&self) -> Option<&CpuSet> {
        match self {
            ActionKind::SetAffinity { cpus, .. } => Some(cpus),
            _ => None,
        }
    }
}

impl fmt::Display for ActionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionKind::SetAffinity { target, cpus } => {
                write!(f, "set affinity of {target} to CPU {cpus}")
            }
            ActionKind::SetPriority { target, nice } => {
                write!(f, "set priority of {target} to nice {nice}")
            }
            ActionKind::SetNumaPreference { target, node } => match node {
                Some(node) => write!(f, "prefer NUMA node {node} for {target}"),
                None => write!(f, "clear NUMA preference for {target}"),
            },
            ActionKind::Hold => write!(f, "hold"),
        }
    }
}

/// A proposed action, with the reasoning that produced it.
///
/// The `reason` and `expectation` are not decoration. An action taken by an
/// autonomous system with no recorded expectation cannot be learned from: when
/// the next reflection arrives there is nothing to compare it against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub kind: ActionKind,
    /// Why, in one line, in whatever vocabulary the caller uses.
    pub reason: String,
    /// What the caller expects to observe afterwards. Free-form, because the
    /// model that formed it owns its own units.
    pub expectation: Option<String>,
}

impl Action {
    pub fn new(kind: ActionKind, reason: impl Into<String>) -> Action {
        Action {
            kind,
            reason: reason.into(),
            expectation: None,
        }
    }

    pub fn expecting(mut self, expectation: impl Into<String>) -> Action {
        self.expectation = Some(expectation.into());
        self
    }

    /// The do-nothing action.
    pub fn hold(reason: impl Into<String>) -> Action {
        Action::new(ActionKind::Hold, reason)
    }
}

/// What happened when an action was attempted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionOutcome {
    /// Whether the syscall reported success.
    pub attempted: bool,
    /// Whether reading the state back confirmed the change. `None` when the
    /// platform offers no way to check, which is itself worth publishing.
    pub confirmed: Option<bool>,
    /// The state before, so the action can be reverted.
    pub previous: Option<PriorState>,
    /// A message when something went wrong.
    pub error: Option<String>,
}

impl ActionOutcome {
    pub fn succeeded(&self) -> bool {
        self.attempted && self.confirmed != Some(false) && self.error.is_none()
    }

    pub fn refused(error: impl Into<String>) -> ActionOutcome {
        ActionOutcome {
            attempted: false,
            confirmed: None,
            previous: None,
            error: Some(error.into()),
        }
    }
}

/// Enough of the prior state to undo an action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PriorState {
    Affinity(CpuSet),
    Priority(i32),
    NumaPreference(Option<u32>),
    Nothing,
}

/// Limits every actuator checks before touching anything.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    /// CPUs any affinity action must stay within. Empty means "whatever this
    /// process is already allowed", read at construction.
    pub allowed_cpus: CpuSet,
    /// Inclusive niceness range.
    pub nice_range: (i32, i32),
    /// Minimum interval between two actions on the same target.
    pub min_interval_ms: u64,
    /// Maximum actions in any rolling minute, across all targets.
    pub max_actions_per_minute: u32,
    /// Refuse to leave a target confined to fewer than this many CPUs.
    /// Pinning a multithreaded process to one CPU is a plausible policy output
    /// and a catastrophic one.
    pub min_cpus_per_target: usize,
}

impl Default for Bounds {
    fn default() -> Self {
        Bounds {
            allowed_cpus: CpuSet::new(),
            // Only raising niceness by default: lowering it needs privilege and
            // takes CPU away from everything else on the machine.
            nice_range: (0, 19),
            min_interval_ms: 250,
            max_actions_per_minute: 60,
            min_cpus_per_target: 1,
        }
    }
}

impl Bounds {
    /// Bounds confined to a set of CPUs.
    pub fn within(cpus: CpuSet) -> Bounds {
        Bounds {
            allowed_cpus: cpus,
            ..Bounds::default()
        }
    }

    /// Whether an affinity request stays inside the allowed set.
    pub fn permits_cpus(&self, cpus: &CpuSet) -> bool {
        if cpus.len() < self.min_cpus_per_target {
            return false;
        }
        if self.allowed_cpus.is_empty() {
            return true;
        }
        cpus.iter().all(|cpu| self.allowed_cpus.contains(cpu))
    }

    pub fn permits_nice(&self, nice: i32) -> bool {
        nice >= self.nice_range.0 && nice <= self.nice_range.1
    }
}

/// A CPU set describing every CPU the current process may use.
///
/// The natural default for [`Bounds::allowed_cpus`]: an autonomous controller
/// should not be able to place work somewhere its own process could not run.
pub fn permitted_cpus() -> CpuSet {
    #[cfg(target_os = "linux")]
    {
        linux::process_affinity().unwrap_or_default()
    }
    #[cfg(not(target_os = "linux"))]
    {
        CpuSet::new()
    }
}

/// Whether this build can actuate at all.
///
/// False off Linux, where every actuator refuses cleanly rather than pretending
/// to have worked. A caller can use this to decide whether to run a control
/// loop or only observe.
pub fn actuation_available() -> bool {
    cfg!(target_os = "linux")
}

/// Every logical CPU mentioned by an action, for a caller checking scope.
pub fn action_cpus(action: &ActionKind) -> Vec<LogicalId> {
    action.cpus().map(|c| c.to_vec()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_refuse_cpus_outside_the_allowed_set() {
        let bounds = Bounds::within([0u32, 1, 2, 3].into_iter().collect());
        assert!(bounds.permits_cpus(&[1u32, 2].into_iter().collect()));
        assert!(!bounds.permits_cpus(&[3u32, 9].into_iter().collect()));
    }

    #[test]
    fn bounds_refuse_an_empty_mask() {
        // An empty affinity mask is EINVAL at the syscall and nonsense as a
        // policy output.
        let bounds = Bounds::default();
        assert!(!bounds.permits_cpus(&CpuSet::new()));
    }

    #[test]
    fn bounds_can_require_a_minimum_width() {
        let bounds = Bounds {
            min_cpus_per_target: 2,
            ..Bounds::default()
        };
        assert!(!bounds.permits_cpus(&[3u32].into_iter().collect()));
        assert!(bounds.permits_cpus(&[3u32, 4].into_iter().collect()));
    }

    #[test]
    fn the_default_priority_range_does_not_let_a_policy_take_over_the_machine() {
        // Lowering niceness below zero needs privilege and starves everything
        // else. A controller that wants it must ask for it explicitly.
        let bounds = Bounds::default();
        assert!(bounds.permits_nice(5));
        assert!(!bounds.permits_nice(-5));
        assert!(!bounds.permits_nice(25));
    }

    #[test]
    fn actions_carry_their_reasoning() {
        let action = Action::new(
            ActionKind::SetAffinity {
                target: Target::CurrentThread,
                cpus: [2u32, 3].into_iter().collect(),
            },
            "lowest predicted jitter",
        )
        .expecting("p99 below 25 us");
        assert_eq!(action.kind.family(), "affinity");
        assert_eq!(action.expectation.as_deref(), Some("p99 below 25 us"));
        assert!(action.kind.to_string().contains("2-3"));
    }

    #[test]
    fn hold_is_a_real_action_with_no_target() {
        let action = Action::hold("predicted gain is inside the uncertainty");
        assert_eq!(action.kind.target(), None);
        assert_eq!(action.kind.family(), "hold");
    }

    #[test]
    fn an_outcome_that_was_not_confirmed_is_not_a_success() {
        let outcome = ActionOutcome {
            attempted: true,
            confirmed: Some(false),
            previous: None,
            error: None,
        };
        assert!(!outcome.succeeded());
        assert!(!ActionOutcome::refused("out of scope").succeeded());
    }
}
