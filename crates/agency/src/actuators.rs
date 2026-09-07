//! Carrying actions out.
//!
//! # The order of checks
//!
//! ```text
//! scope      may I touch this target at all?
//! bounds     is this value inside the permitted range?
//! rate       have I acted on this target too recently?
//! read       what is the current state, so I can revert?
//! act        the syscall
//! confirm    read it back; did the machine actually change?
//! audit      record all of the above, including refusals
//! ```
//!
//! Every one of those steps happens before the next, and a failure at any step
//! produces an audit event rather than an early return with nothing recorded.
//! The expensive step, reading state back to confirm, is deliberately not
//! optional: an actuator that assumes its syscall worked is an actuator whose
//! effects cannot be learned from.
//!
//! # Confirmation is not always possible
//!
//! [`ActionOutcome::confirmed`] is an `Option`. Setting NUMA policy for a
//! *different* process, for instance, has no syscall at all on Linux, and
//! reading back another process's memory policy is not possible either. Where
//! the platform cannot confirm, the actuator says so rather than claiming
//! success.

use corescout_core::{clock, CpuSet};

use crate::audit::{AuditEvent, AuditLog, Disposition};
use crate::scope::Scope;
use crate::{Action, ActionKind, ActionOutcome, Bounds, PriorState, Target};

/// One kind of change to the machine.
pub trait Actuator {
    /// Stable family name, matching [`ActionKind::family`].
    fn family(&self) -> &'static str;

    /// Whether this actuator can work on this build and machine.
    fn available(&self) -> bool;

    /// Read the current state, so it can be restored.
    fn read(&self, target: Target) -> Option<PriorState>;

    /// Apply, having already passed scope, bounds and rate checks.
    fn apply(&self, action: &ActionKind) -> ActionOutcome;
}

/// The actuators available, plus the safety machinery around them.
///
/// This is the type an autonomous controller holds. It cannot reach the
/// syscalls directly.
pub struct ActuatorSet {
    scope: Scope,
    bounds: Bounds,
    log: AuditLog,
    actuators: Vec<Box<dyn Actuator>>,
    /// Sequence number of the reflection current when acting, recorded so an
    /// action can be paired with the before and after states.
    mirror_sequence: Option<u64>,
    dry_run: bool,
}

impl std::fmt::Debug for ActuatorSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActuatorSet")
            .field("actuators", &self.actuators.len())
            .field("scope", &self.scope.len())
            .field("dry_run", &self.dry_run)
            .field("audited", &self.log.len())
            .finish()
    }
}

impl ActuatorSet {
    /// The default set for this platform.
    pub fn new(scope: Scope, bounds: Bounds) -> ActuatorSet {
        ActuatorSet {
            scope,
            bounds,
            log: AuditLog::new(4096),
            actuators: default_actuators(),
            mirror_sequence: None,
            dry_run: false,
        }
    }

    /// A set that checks everything and changes nothing.
    ///
    /// Not a mock: the scope, bounds, rate limiting and audit trail are the
    /// real ones, and only the final syscall is skipped. It is how a policy is
    /// evaluated safely, and how the whole loop is exercised on a machine where
    /// actuation is unavailable.
    pub fn dry_run(scope: Scope, bounds: Bounds) -> ActuatorSet {
        ActuatorSet {
            dry_run: true,
            ..ActuatorSet::new(scope, bounds)
        }
    }

    pub fn is_dry_run(&self) -> bool {
        self.dry_run
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    pub fn scope_mut(&mut self) -> &mut Scope {
        &mut self.scope
    }

    pub fn bounds(&self) -> &Bounds {
        &self.bounds
    }

    pub fn log(&self) -> &AuditLog {
        &self.log
    }

    /// Tell the actuators which reflection is current, so audit events can be
    /// paired with the mirror states before and after them.
    pub fn observing(&mut self, mirror_sequence: u64) {
        self.mirror_sequence = Some(mirror_sequence);
    }

    /// Which action families can actually do anything here.
    pub fn available_families(&self) -> Vec<&'static str> {
        self.actuators
            .iter()
            .filter(|a| a.available())
            .map(|a| a.family())
            .collect()
    }

    /// Attempt an action. Always produces an audit event.
    pub fn perform(&mut self, action: Action) -> Disposition {
        let now = clock::now_ns();

        if let ActionKind::Hold = action.kind {
            return self.record(
                action,
                Disposition::Applied,
                ActionOutcome {
                    attempted: false,
                    confirmed: Some(true),
                    previous: None,
                    error: None,
                },
            );
        }

        // 1. Scope.
        let target = action
            .kind
            .target()
            .expect("non-hold actions have a target");
        if let Err(error) = self.scope.permits(target) {
            return self.record(
                action,
                Disposition::Refused,
                ActionOutcome::refused(error.to_string()),
            );
        }

        // 2. Bounds.
        if let Err(reason) = self.check_bounds(&action.kind) {
            return self.record(action, Disposition::Refused, ActionOutcome::refused(reason));
        }

        // 3. Rate.
        if let Err(reason) = self.check_rate(target, now) {
            return self.record(action, Disposition::Refused, ActionOutcome::refused(reason));
        }

        let Some(actuator) = self
            .actuators
            .iter()
            .find(|a| a.family() == action.kind.family())
        else {
            return self.record(
                action,
                Disposition::Refused,
                ActionOutcome::refused("no actuator for this action family"),
            );
        };
        // 4. Read the prior state, so this is reversible.
        let previous = actuator.read(target);

        // A dry run is checked before availability on purpose. Its value is
        // validating a *policy*, and a policy can be validated on a machine
        // that cannot actuate at all. Checking availability first would make
        // every dry run off Linux report "unavailable" and tell you nothing
        // about whether the decisions were sound.
        if self.dry_run {
            return self.record(
                action,
                Disposition::Refused,
                ActionOutcome {
                    attempted: false,
                    confirmed: None,
                    previous,
                    error: Some("dry run: every check passed, the syscall was skipped".into()),
                },
            );
        }

        if !actuator.available() {
            return self.record(
                action,
                Disposition::Refused,
                ActionOutcome::refused(format!(
                    "the `{}` actuator is unavailable on this platform",
                    actuator.family()
                )),
            );
        }

        // 5. Act, and 6. confirm.
        let mut outcome = actuator.apply(&action.kind);
        outcome.previous = previous;

        let disposition = if !outcome.attempted || outcome.error.is_some() {
            Disposition::Failed
        } else {
            match outcome.confirmed {
                Some(true) => Disposition::Applied,
                Some(false) => Disposition::Failed,
                None => Disposition::Unconfirmed,
            }
        };
        self.record(action, disposition, outcome)
    }

    /// Undo the most recent action that changed the machine.
    ///
    /// Returns `None` when there is nothing to undo, or when the prior state
    /// was not captured.
    pub fn revert_last(&mut self) -> Option<Disposition> {
        let event = self
            .log
            .events()
            .rev()
            .find(|e| e.disposition.changed_the_machine())?
            .clone();
        let previous = event.outcome.previous.clone()?;
        let target = event.action.kind.target()?;

        let kind = match previous {
            PriorState::Affinity(cpus) => ActionKind::SetAffinity { target, cpus },
            PriorState::Priority(nice) => ActionKind::SetPriority { target, nice },
            PriorState::NumaPreference(node) => ActionKind::SetNumaPreference { target, node },
            PriorState::Nothing => return None,
        };

        // A revert bypasses the rate limit deliberately: undoing a change must
        // never be the thing the safety layer blocks.
        let now = clock::now_ns();
        let actuator = self
            .actuators
            .iter()
            .find(|a| a.family() == kind.family())?;
        let outcome = if self.dry_run {
            ActionOutcome::refused("dry run: revert skipped")
        } else {
            actuator.apply(&kind)
        };
        let action = Action::new(kind, format!("revert of: {}", event.action.reason));
        let disposition = if outcome.succeeded() {
            Disposition::Reverted
        } else {
            Disposition::Failed
        };
        let _ = now;
        Some(self.record(action, disposition, outcome))
    }

    fn check_bounds(&self, kind: &ActionKind) -> Result<(), String> {
        match kind {
            ActionKind::SetAffinity { cpus, .. } => {
                if !self.bounds.permits_cpus(cpus) {
                    return Err(format!(
                        "CPU set {cpus} is outside the permitted set {} or narrower than {} CPUs",
                        self.bounds.allowed_cpus, self.bounds.min_cpus_per_target
                    ));
                }
                Ok(())
            }
            ActionKind::SetPriority { nice, .. } => {
                if !self.bounds.permits_nice(*nice) {
                    return Err(format!(
                        "niceness {nice} is outside the permitted range {:?}",
                        self.bounds.nice_range
                    ));
                }
                Ok(())
            }
            ActionKind::SetNumaPreference { .. } | ActionKind::Hold => Ok(()),
        }
    }

    fn check_rate(&self, target: Target, now: u64) -> Result<(), String> {
        if let Some(last) = self.log.last_action_ns(target) {
            let since = now.saturating_sub(last);
            let minimum = self.bounds.min_interval_ms * 1_000_000;
            if since < minimum {
                return Err(format!(
                    "only {:.0} ms since the last action on {target}; the minimum is {} ms",
                    since as f64 / 1e6,
                    self.bounds.min_interval_ms
                ));
            }
        }
        let in_last_minute = self.log.recent_for(None, now, 60_000_000_000);
        if in_last_minute >= self.bounds.max_actions_per_minute as usize {
            return Err(format!(
                "{in_last_minute} actions in the last minute reaches the limit of {}",
                self.bounds.max_actions_per_minute
            ));
        }
        Ok(())
    }

    fn record(
        &mut self,
        action: Action,
        disposition: Disposition,
        outcome: ActionOutcome,
    ) -> Disposition {
        self.log.record(AuditEvent {
            monotonic_ns: clock::now_ns(),
            realtime_ns: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0),
            mirror_sequence: self.mirror_sequence,
            action,
            disposition,
            outcome,
        });
        disposition
    }
}

fn default_actuators() -> Vec<Box<dyn Actuator>> {
    vec![
        Box::new(AffinityActuator),
        Box::new(PriorityActuator),
        Box::new(NumaActuator),
    ]
}

// ---------------------------------------------------------------------------
// Affinity
// ---------------------------------------------------------------------------

/// Confine a thread or process to a set of CPUs.
///
/// The one actuator that is fully implemented, fully confirmable, and directly
/// useful: it is the mechanism behind every placement decision the system can
/// make.
pub struct AffinityActuator;

impl Actuator for AffinityActuator {
    fn family(&self) -> &'static str {
        "affinity"
    }

    fn available(&self) -> bool {
        cfg!(target_os = "linux")
    }

    fn read(&self, target: Target) -> Option<PriorState> {
        #[cfg(target_os = "linux")]
        {
            crate::linux::get_affinity(target.raw())
                .ok()
                .map(PriorState::Affinity)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = target;
            None
        }
    }

    fn apply(&self, action: &ActionKind) -> ActionOutcome {
        let ActionKind::SetAffinity { target, cpus } = action else {
            return ActionOutcome::refused("wrong action family");
        };
        #[cfg(target_os = "linux")]
        {
            match crate::linux::set_affinity(target.raw(), cpus) {
                Ok(()) => {
                    // Read it back. The kernel silently intersects the request
                    // with the cpuset the process is confined to, so a
                    // successful call does not mean the mask is what was asked
                    // for.
                    let confirmed = crate::linux::get_affinity(target.raw())
                        .ok()
                        .map(|actual| actual == *cpus);
                    ActionOutcome {
                        attempted: true,
                        confirmed,
                        previous: None,
                        error: confirmed.and_then(|ok| {
                            (!ok).then(|| {
                                "the kernel applied a different mask, probably because a \
                                 cpuset or cgroup restricts this process further"
                                    .to_string()
                            })
                        }),
                    }
                }
                Err(error) => ActionOutcome {
                    attempted: true,
                    confirmed: Some(false),
                    previous: None,
                    error: Some(error.to_string()),
                },
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (target, cpus);
            ActionOutcome::refused("thread affinity is implemented for Linux only")
        }
    }
}

// ---------------------------------------------------------------------------
// Priority
// ---------------------------------------------------------------------------

/// Change scheduling niceness.
pub struct PriorityActuator;

impl Actuator for PriorityActuator {
    fn family(&self) -> &'static str {
        "priority"
    }

    fn available(&self) -> bool {
        cfg!(target_os = "linux")
    }

    fn read(&self, target: Target) -> Option<PriorState> {
        #[cfg(target_os = "linux")]
        {
            crate::linux::get_priority(target.raw())
                .ok()
                .map(PriorState::Priority)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = target;
            None
        }
    }

    fn apply(&self, action: &ActionKind) -> ActionOutcome {
        let ActionKind::SetPriority { target, nice } = action else {
            return ActionOutcome::refused("wrong action family");
        };
        #[cfg(target_os = "linux")]
        {
            match crate::linux::set_priority(target.raw(), *nice) {
                Ok(()) => {
                    let confirmed = crate::linux::get_priority(target.raw())
                        .ok()
                        .map(|actual| actual == *nice);
                    ActionOutcome {
                        attempted: true,
                        confirmed,
                        previous: None,
                        error: None,
                    }
                }
                Err(error) => ActionOutcome {
                    attempted: true,
                    confirmed: Some(false),
                    previous: None,
                    // Lowering niceness needs CAP_SYS_NICE, and this is the
                    // most likely failure by far.
                    error: Some(format!("{error}; lowering niceness requires CAP_SYS_NICE")),
                },
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (target, nice);
            ActionOutcome::refused("priority control is implemented for Linux only")
        }
    }
}

// ---------------------------------------------------------------------------
// NUMA placement
// ---------------------------------------------------------------------------

/// Prefer a NUMA node for future allocations.
///
/// # A real limitation, stated rather than worked around
///
/// Linux has no syscall to set another process's memory policy. `set_mempolicy`
/// applies to the calling thread only, which is why `numactl` works by
/// launching the process itself. So this actuator supports
/// [`Target::CurrentThread`] and refuses everything else, instead of silently
/// doing nothing for a target it cannot reach.
///
/// It also only affects *future* allocations. Pages already faulted in stay
/// where they are unless migrated, which needs `move_pages` or `migrate_pages`
/// and is a heavier operation than this layer should perform implicitly.
pub struct NumaActuator;

impl Actuator for NumaActuator {
    fn family(&self) -> &'static str {
        "numa"
    }

    fn available(&self) -> bool {
        cfg!(target_os = "linux")
    }

    fn read(&self, target: Target) -> Option<PriorState> {
        #[cfg(target_os = "linux")]
        {
            if !matches!(target, Target::CurrentThread) {
                return None;
            }
            crate::linux::get_numa_preference()
                .ok()
                .map(PriorState::NumaPreference)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = target;
            None
        }
    }

    fn apply(&self, action: &ActionKind) -> ActionOutcome {
        let ActionKind::SetNumaPreference { target, node } = action else {
            return ActionOutcome::refused("wrong action family");
        };
        if !matches!(target, Target::CurrentThread) {
            return ActionOutcome::refused(
                "Linux can only set the calling thread's NUMA policy; there is no syscall \
                 to set another process's, so this is declined rather than faked",
            );
        }
        #[cfg(target_os = "linux")]
        {
            match crate::linux::set_numa_preference(*node) {
                Ok(()) => {
                    let confirmed = crate::linux::get_numa_preference()
                        .ok()
                        .map(|actual| actual == *node);
                    ActionOutcome {
                        attempted: true,
                        confirmed,
                        previous: None,
                        error: None,
                    }
                }
                Err(error) => ActionOutcome {
                    attempted: true,
                    confirmed: Some(false),
                    previous: None,
                    error: Some(error.to_string()),
                },
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = node;
            ActionOutcome::refused("NUMA placement is implemented for Linux only")
        }
    }
}

/// Convenience: pin the calling thread, checking everything on the way.
pub fn pin_current_thread(
    set: &mut ActuatorSet,
    cpus: CpuSet,
    reason: impl Into<String>,
) -> Disposition {
    set.perform(Action::new(
        ActionKind::SetAffinity {
            target: Target::CurrentThread,
            cpus,
        },
        reason,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn permissive_bounds() -> Bounds {
        Bounds {
            allowed_cpus: (0u32..64).collect(),
            min_interval_ms: 0,
            ..Bounds::default()
        }
    }

    #[test]
    fn an_out_of_scope_target_is_refused_and_audited() {
        let mut set = ActuatorSet::new(Scope::own_process(), permissive_bounds());
        let disposition = set.perform(Action::new(
            ActionKind::SetAffinity {
                target: Target::Process(4242),
                cpus: [1u32].into_iter().collect(),
            },
            "test",
        ));
        assert_eq!(disposition, Disposition::Refused);
        assert_eq!(set.log().len(), 1, "a refusal is still a record");
        assert!(set.log().last().unwrap().outcome.error.is_some());
    }

    #[test]
    fn an_out_of_bounds_cpu_set_is_refused_before_any_syscall() {
        let mut set = ActuatorSet::new(
            Scope::own_process(),
            Bounds::within([0u32, 1].into_iter().collect()),
        );
        let disposition = set.perform(Action::new(
            ActionKind::SetAffinity {
                target: Target::CurrentThread,
                cpus: [9u32].into_iter().collect(),
            },
            "test",
        ));
        assert_eq!(disposition, Disposition::Refused);
        assert!(set
            .log()
            .last()
            .unwrap()
            .outcome
            .error
            .as_ref()
            .unwrap()
            .contains("outside"));
    }

    #[test]
    fn priority_outside_the_permitted_range_is_refused() {
        let mut set = ActuatorSet::new(Scope::own_process(), permissive_bounds());
        let disposition = set.perform(Action::new(
            ActionKind::SetPriority {
                target: Target::CurrentThread,
                nice: -20,
            },
            "go faster",
        ));
        assert_eq!(disposition, Disposition::Refused);
    }

    #[test]
    fn the_rate_limiter_stops_a_runaway_policy() {
        let mut set = ActuatorSet::dry_run(
            Scope::own_process(),
            Bounds {
                max_actions_per_minute: 2,
                min_interval_ms: 0,
                allowed_cpus: (0u32..64).collect(),
                ..Bounds::default()
            },
        );
        // A dry run records refusals, which do not count against the rate
        // limit, so drive the limiter through `hold`, which always applies.
        for _ in 0..2 {
            set.perform(Action::hold("nothing to do"));
        }
        let disposition = set.perform(Action::new(
            ActionKind::SetAffinity {
                target: Target::CurrentThread,
                cpus: [1u32].into_iter().collect(),
            },
            "third action this minute",
        ));
        assert_eq!(disposition, Disposition::Refused);
        assert!(set
            .log()
            .last()
            .unwrap()
            .outcome
            .error
            .as_ref()
            .unwrap()
            .contains("in the last minute"));
    }

    #[test]
    fn a_dry_run_checks_everything_and_changes_nothing() {
        let mut set = ActuatorSet::dry_run(Scope::own_process(), permissive_bounds());
        let disposition = set.perform(Action::new(
            ActionKind::SetAffinity {
                target: Target::CurrentThread,
                cpus: [0u32].into_iter().collect(),
            },
            "test",
        ));
        assert_eq!(disposition, Disposition::Refused);
        let event = set.log().last().unwrap();
        assert!(!event.outcome.attempted);
        assert!(event.outcome.error.as_ref().unwrap().contains("dry run"));
    }

    #[test]
    fn hold_is_recorded_as_an_action() {
        // A policy that decides to do nothing has made a decision, and the
        // audit trail should show it rather than a gap.
        let mut set = ActuatorSet::new(Scope::own_process(), permissive_bounds());
        assert_eq!(
            set.perform(Action::hold("predicted gain within uncertainty")),
            Disposition::Applied
        );
        assert_eq!(set.log().last().unwrap().action.kind.family(), "hold");
    }

    #[test]
    fn a_frozen_scope_refuses_everything() {
        let mut scope = Scope::own_process();
        scope.freeze();
        let mut set = ActuatorSet::new(scope, permissive_bounds());
        assert_eq!(
            set.perform(Action::new(
                ActionKind::SetAffinity {
                    target: Target::CurrentThread,
                    cpus: [0u32].into_iter().collect(),
                },
                "test",
            )),
            Disposition::Refused
        );
    }

    #[test]
    fn numa_declines_targets_linux_cannot_reach() {
        let mut scope = Scope::own_process();
        scope.register(std::process::id() as i32);
        let mut set = ActuatorSet::new(scope, permissive_bounds());
        let disposition = set.perform(Action::new(
            ActionKind::SetNumaPreference {
                target: Target::Process(std::process::id() as i32),
                node: Some(0),
            },
            "test",
        ));
        // Either refused by scope (non-Linux, where registration fails) or by
        // the actuator itself. Never silently "applied".
        assert_ne!(disposition, Disposition::Applied);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn pinning_the_current_thread_applies_and_confirms() {
        let permitted = crate::permitted_cpus();
        let first = permitted.iter().next().expect("at least one CPU");
        let mut set = ActuatorSet::new(Scope::own_process(), Bounds::within(permitted.clone()));

        let disposition = pin_current_thread(
            &mut set,
            [first].into_iter().collect(),
            "test: confine to one CPU",
        );
        assert_eq!(disposition, Disposition::Applied);

        let event = set.log().last().unwrap();
        assert_eq!(event.outcome.confirmed, Some(true));
        assert!(matches!(
            event.outcome.previous,
            Some(PriorState::Affinity(_))
        ));

        // And it is reversible.
        assert_eq!(set.revert_last(), Some(Disposition::Reverted));
        assert_eq!(
            crate::linux::get_affinity(0).unwrap(),
            permitted,
            "revert must restore the original mask"
        );
    }
}
