//! The audit log: what was attempted, why, and what actually happened.
//!
//! # Why every attempt is recorded, including refusals
//!
//! A refused action is the more informative record. "The controller wanted to
//! pin this thread to CPU 3 and was stopped by a rate limit" tells you both
//! what the policy is doing and that the safety layer is working. Logging only
//! successes would hide exactly the behaviour worth watching.
//!
//! # This is also the training signal
//!
//! Each event pairs an *expectation* with an *outcome*. That pairing is what
//! makes an autonomous loop able to learn: without the expectation recorded at
//! the time, a later reflection cannot be attributed to anything, and the
//! system is acting without being able to find out whether acting helped.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::{Action, ActionOutcome};

/// What became of a proposed action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// Carried out, and reading the state back confirmed it.
    Applied,
    /// The syscall succeeded but the result could not be confirmed.
    Unconfirmed,
    /// Refused before anything was attempted, by scope or bounds.
    Refused,
    /// Attempted and failed.
    Failed,
    /// Undone.
    Reverted,
}

impl Disposition {
    pub fn label(self) -> &'static str {
        match self {
            Disposition::Applied => "applied",
            Disposition::Unconfirmed => "unconfirmed",
            Disposition::Refused => "refused",
            Disposition::Failed => "failed",
            Disposition::Reverted => "reverted",
        }
    }

    /// Whether the machine changed as a result.
    pub fn changed_the_machine(self) -> bool {
        matches!(
            self,
            Disposition::Applied | Disposition::Unconfirmed | Disposition::Reverted
        )
    }
}

/// One entry in the audit log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Monotonic nanoseconds, for lining up with mirror snapshots.
    pub monotonic_ns: u64,
    /// Wall clock, for lining up with everything else.
    pub realtime_ns: u64,
    /// The mirror sequence current when the action was taken, so the reflection
    /// before and after can both be found.
    pub mirror_sequence: Option<u64>,
    pub action: Action,
    pub disposition: Disposition,
    pub outcome: ActionOutcome,
}

impl AuditEvent {
    /// A one-line rendering, for a log or a terminal.
    pub fn summary(&self) -> String {
        let mut line = format!(
            "[{:>11}] {} :: {}",
            self.disposition.label(),
            self.action.kind,
            self.action.reason
        );
        if let Some(expected) = &self.action.expectation {
            line.push_str(&format!(" (expected {expected})"));
        }
        if let Some(error) = &self.outcome.error {
            line.push_str(&format!(" [{error}]"));
        }
        line
    }
}

/// A bounded log of recent actions.
///
/// Bounded because an autonomous loop runs indefinitely and an unbounded audit
/// log is a memory leak with good intentions. Durable recording is a separate
/// concern: [`AuditLog::drain_to`] hands events to whatever wants to persist
/// them.
#[derive(Debug)]
pub struct AuditLog {
    capacity: usize,
    events: VecDeque<AuditEvent>,
    applied: u64,
    refused: u64,
    failed: u64,
}

impl AuditLog {
    pub fn new(capacity: usize) -> AuditLog {
        AuditLog {
            capacity: capacity.max(1),
            events: VecDeque::new(),
            applied: 0,
            refused: 0,
            failed: 0,
        }
    }

    pub fn record(&mut self, event: AuditEvent) {
        match event.disposition {
            Disposition::Applied | Disposition::Unconfirmed | Disposition::Reverted => {
                self.applied += 1
            }
            Disposition::Refused => self.refused += 1,
            Disposition::Failed => self.failed += 1,
        }
        if self.events.len() == self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    pub fn events(&self) -> impl DoubleEndedIterator<Item = &AuditEvent> {
        self.events.iter()
    }

    pub fn last(&self) -> Option<&AuditEvent> {
        self.events.back()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Lifetime counts, which survive eviction from the ring.
    pub fn totals(&self) -> AuditTotals {
        AuditTotals {
            applied: self.applied,
            refused: self.refused,
            failed: self.failed,
        }
    }

    /// Hand every held event to a sink and forget them.
    pub fn drain_to(&mut self, sink: &mut impl FnMut(&AuditEvent)) {
        for event in &self.events {
            sink(event);
        }
        self.events.clear();
    }

    /// Actions taken on a given target within the last `window_ns`.
    ///
    /// The rate limiter's input. Counted from the log rather than a separate
    /// counter so that what limits the controller is exactly what is auditable.
    pub fn recent_for(&self, target: Option<crate::Target>, now_ns: u64, window_ns: u64) -> usize {
        self.events
            .iter()
            .filter(|event| event.disposition.changed_the_machine())
            .filter(|event| now_ns.saturating_sub(event.monotonic_ns) <= window_ns)
            .filter(|event| match target {
                Some(target) => event.action.kind.target() == Some(target),
                None => true,
            })
            .count()
    }

    /// When the target was last acted on.
    pub fn last_action_ns(&self, target: crate::Target) -> Option<u64> {
        self.events
            .iter()
            .filter(|event| event.disposition.changed_the_machine())
            .filter(|event| event.action.kind.target() == Some(target))
            .map(|event| event.monotonic_ns)
            .max()
    }
}

/// Lifetime action counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditTotals {
    pub applied: u64,
    pub refused: u64,
    pub failed: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionKind, Target};

    fn event(monotonic_ns: u64, disposition: Disposition, target: Target) -> AuditEvent {
        AuditEvent {
            monotonic_ns,
            realtime_ns: 0,
            mirror_sequence: Some(7),
            action: Action::new(
                ActionKind::SetAffinity {
                    target,
                    cpus: [1u32].into_iter().collect(),
                },
                "test",
            ),
            disposition,
            outcome: ActionOutcome {
                attempted: true,
                confirmed: Some(true),
                previous: None,
                error: None,
            },
        }
    }

    #[test]
    fn refusals_are_recorded_not_dropped() {
        let mut log = AuditLog::new(10);
        log.record(event(1, Disposition::Refused, Target::CurrentThread));
        assert_eq!(log.len(), 1);
        assert_eq!(log.totals().refused, 1);
        assert_eq!(log.totals().applied, 0);
    }

    #[test]
    fn the_ring_is_bounded_but_the_totals_are_not() {
        let mut log = AuditLog::new(2);
        for i in 0..5 {
            log.record(event(i, Disposition::Applied, Target::CurrentThread));
        }
        assert_eq!(log.len(), 2);
        assert_eq!(log.totals().applied, 5, "totals survive eviction");
    }

    #[test]
    fn rate_accounting_only_counts_actions_that_changed_the_machine() {
        let mut log = AuditLog::new(10);
        log.record(event(1_000, Disposition::Applied, Target::CurrentThread));
        log.record(event(2_000, Disposition::Refused, Target::CurrentThread));
        log.record(event(3_000, Disposition::Failed, Target::CurrentThread));
        assert_eq!(log.recent_for(None, 3_000, 10_000), 1);
    }

    #[test]
    fn rate_accounting_respects_the_window_and_the_target() {
        let mut log = AuditLog::new(10);
        log.record(event(1_000, Disposition::Applied, Target::CurrentThread));
        log.record(event(9_000, Disposition::Applied, Target::Process(42)));
        // A window that excludes the older event.
        assert_eq!(log.recent_for(None, 9_000, 5_000), 1);
        assert_eq!(
            log.recent_for(Some(Target::CurrentThread), 9_000, 100_000),
            1
        );
        assert_eq!(log.last_action_ns(Target::Process(42)), Some(9_000));
    }

    #[test]
    fn a_summary_names_the_action_the_reason_and_the_expectation() {
        let mut e = event(1, Disposition::Applied, Target::CurrentThread);
        e.action.expectation = Some("p99 below 25 us".into());
        let summary = e.summary();
        assert!(summary.contains("applied"));
        assert!(summary.contains("set affinity"));
        assert!(summary.contains("expected p99 below 25 us"));
    }
}
