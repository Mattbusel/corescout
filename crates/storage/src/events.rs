//! The event log: the activity timeline and the audit trail, which are one log.
//!
//! # Why they are not two logs
//!
//! An audit trail that lives apart from the timeline is an audit trail nobody
//! reads, and a timeline that omits the actions CoreScout took is a timeline
//! that lies by omission. Every entry carries a [`Severity`] and an
//! [`EventKind`]; the Activity screen shows a filtered view and the audit
//! export shows another, over the same rows.
//!
//! # What an action entry must carry
//!
//! The standing rule for this project is that every action produces a record
//! naming the action, its target, the reason, the expected result, the actual
//! result, and the time. [`Event::action`] takes all of them, so an action
//! logged without a reason does not compile.

use serde::{Deserialize, Serialize};

/// How loudly an event should be reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Routine. Kept, never surfaced on its own.
    Debug,
    /// Worth showing on the timeline.
    Info,
    /// Something the user may want to know about.
    Notice,
    /// Something went wrong, and CoreScout handled it.
    Warning,
    /// Something went wrong that the user should see.
    Error,
}

/// What kind of thing happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// The service started, stopped, paused, or resumed.
    Lifecycle,
    /// An AI connected or disconnected.
    Connection,
    /// An agent did something externally visible.
    AgentActivity,
    /// The mirror observed something notable about the machine.
    Observation,
    /// A pattern, concept, or hypothesis was formed.
    Discovery,
    /// A claim was tested, supported, or refuted.
    Evidence,
    /// A belief was withdrawn.
    Retirement,
    /// CoreScout proposed a change.
    Proposal,
    /// CoreScout carried out an action. Always audited.
    Action,
    /// A user approved, refused, or reverted something.
    UserDecision,
    /// A permission was granted, refused, or exhausted.
    Permission,
    /// Something failed.
    Fault,
}

impl EventKind {
    /// Whether entries of this kind belong in the audit export.
    ///
    /// Actions, permissions and user decisions do. Everything else is
    /// narrative.
    pub fn is_audited(self) -> bool {
        matches!(
            self,
            EventKind::Action | EventKind::Permission | EventKind::UserDecision
        )
    }

    /// The stable on-disk name.
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Lifecycle => "lifecycle",
            EventKind::Connection => "connection",
            EventKind::AgentActivity => "agent_activity",
            EventKind::Observation => "observation",
            EventKind::Discovery => "discovery",
            EventKind::Evidence => "evidence",
            EventKind::Retirement => "retirement",
            EventKind::Proposal => "proposal",
            EventKind::Action => "action",
            EventKind::UserDecision => "user_decision",
            EventKind::Permission => "permission",
            EventKind::Fault => "fault",
        }
    }
}

/// One entry in the log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Assigned by the store on append; zero before then.
    #[serde(default)]
    pub sequence: u64,
    /// Wall clock milliseconds since the Unix epoch.
    pub at_ms: u64,
    /// What kind of thing happened.
    pub kind: EventKind,
    /// How loudly to report it.
    pub severity: Severity,
    /// A short line, written for a person rather than a log parser.
    pub summary: String,
    /// What it happened to: a capability id, a session id, a CPU set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Why CoreScout did it. Required for actions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// What CoreScout expected to happen. Required for actions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    /// What actually happened. Filled in when the outcome is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    /// Structured detail, for the technical view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl Event {
    /// A narrative entry.
    pub fn new(kind: EventKind, severity: Severity, summary: impl Into<String>) -> Event {
        Event {
            sequence: 0,
            at_ms: crate::docs::now_ms(),
            kind,
            severity,
            summary: summary.into(),
            subject: None,
            reason: None,
            expected: None,
            actual: None,
            detail: None,
        }
    }

    /// An audited action entry.
    ///
    /// Every parameter is required because a record missing any of them cannot
    /// answer the question the audit log exists to answer.
    pub fn action(
        summary: impl Into<String>,
        subject: impl Into<String>,
        reason: impl Into<String>,
        expected: impl Into<String>,
    ) -> Event {
        Event {
            subject: Some(subject.into()),
            reason: Some(reason.into()),
            expected: Some(expected.into()),
            ..Event::new(EventKind::Action, Severity::Notice, summary)
        }
    }

    /// Name what this happened to.
    pub fn about(mut self, subject: impl Into<String>) -> Event {
        self.subject = Some(subject.into());
        self
    }

    /// Record what actually happened.
    pub fn outcome(mut self, actual: impl Into<String>) -> Event {
        self.actual = Some(actual.into());
        self
    }

    /// Attach structured detail for the technical view.
    pub fn with<T: Serialize>(mut self, detail: &T) -> Event {
        self.detail = serde_json::to_value(detail).ok();
        self
    }

    /// Whether this entry belongs in an audit export.
    pub fn is_audited(&self) -> bool {
        self.kind.is_audited()
    }

    /// Whether this entry is complete enough to be audited.
    ///
    /// An action recorded without a reason, an expectation, or an outcome is
    /// an action nobody can review. The service asserts this before shipping
    /// an audit export, so a gap is a visible failure rather than a silent
    /// hole in the record.
    pub fn is_accountable(&self) -> bool {
        if !self.is_audited() {
            return true;
        }
        self.subject.is_some()
            && self.reason.is_some()
            && self.expected.is_some()
            && self.actual.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_action_carries_everything_an_audit_needs() {
        let event = Event::action(
            "pinned the build to the performance cores",
            "capability:build_on_p_cores",
            "6 of 9 builds on this repository ran on efficiency cores",
            "the build completes without changing its output",
        )
        .outcome("completed, output verified identical");
        assert!(event.is_audited());
        assert!(event.is_accountable());
    }

    #[test]
    fn an_action_without_an_outcome_is_not_yet_accountable() {
        // In-flight actions are legitimately incomplete. What is not
        // legitimate is exporting them as if they were reviewed.
        let event = Event::action("a", "b", "c", "d");
        assert!(!event.is_accountable());
    }

    #[test]
    fn narrative_entries_are_not_held_to_the_audit_standard() {
        let event = Event::new(EventKind::Discovery, Severity::Info, "found a pattern");
        assert!(!event.is_audited());
        assert!(event.is_accountable());
    }

    #[test]
    fn the_audited_kinds_are_the_ones_that_change_something() {
        for kind in [
            EventKind::Action,
            EventKind::Permission,
            EventKind::UserDecision,
        ] {
            assert!(kind.is_audited(), "{} should be audited", kind.as_str());
        }
        for kind in [
            EventKind::Observation,
            EventKind::Discovery,
            EventKind::AgentActivity,
        ] {
            assert!(!kind.is_audited(), "{} is narrative", kind.as_str());
        }
    }

    #[test]
    fn severity_orders_from_quiet_to_loud() {
        assert!(Severity::Debug < Severity::Info);
        assert!(Severity::Notice < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
    }
}
