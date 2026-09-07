//! The document store: small structured records, addressed by kind and id.
//!
//! Everything CoreScout keeps that is not observation-rate telemetry lives
//! here as a JSON document under a [`Kind`]. Documents are opaque to this
//! crate, which is deliberate: storage should not have to be recompiled
//! because a capability grew a field, and the crates that own those types
//! should not have to know how they are persisted.
//!
//! # Why the kinds are a closed enum
//!
//! An open string namespace means a typo silently creates a new collection
//! that nothing ever reads. The set of things this product stores is small
//! and known, so it is written down.

use serde::{Deserialize, Serialize};

/// A collection of documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// An AI agent that has connected, and what is known about it.
    Agent,
    /// One working session of one agent.
    Session,
    /// A unit of work an agent attempted within a session.
    Task,
    /// A single externally visible action: a tool call, a command, a build.
    Action,
    /// A repository or workspace an agent has worked in.
    Workspace,
    /// A recurring machine state the mirror discovered and named itself.
    LatentState,
    /// A concept that survived promotion, or the record of one that did not.
    Concept,
    /// A falsifiable claim, and its standing.
    Hypothesis,
    /// Accumulated evidence for one comparison, association and causal apart.
    Evidence,
    /// A recurring operational pattern found in agent activity.
    Pattern,
    /// A procedure reliable enough to be offered as a capability.
    Capability,
    /// A change CoreScout proposes, has made, or was refused.
    Proposal,
    /// A user decision: an approval, a rename, a disable.
    Decision,
    /// Settings, autonomy mode, permissions.
    Setting,
    /// Anything the product needs to remember about itself.
    Meta,
}

impl Kind {
    /// The stable on-disk name. Changing one of these is a migration.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Agent => "agent",
            Kind::Session => "session",
            Kind::Task => "task",
            Kind::Action => "action",
            Kind::Workspace => "workspace",
            Kind::LatentState => "latent_state",
            Kind::Concept => "concept",
            Kind::Hypothesis => "hypothesis",
            Kind::Evidence => "evidence",
            Kind::Pattern => "pattern",
            Kind::Capability => "capability",
            Kind::Proposal => "proposal",
            Kind::Decision => "decision",
            Kind::Setting => "setting",
            Kind::Meta => "meta",
        }
    }

    /// Parse an on-disk name back to a kind.
    pub fn parse(name: &str) -> Option<Kind> {
        Kind::all().into_iter().find(|kind| kind.as_str() == name)
    }

    /// Every kind, in declaration order.
    pub fn all() -> Vec<Kind> {
        vec![
            Kind::Agent,
            Kind::Session,
            Kind::Task,
            Kind::Action,
            Kind::Workspace,
            Kind::LatentState,
            Kind::Concept,
            Kind::Hypothesis,
            Kind::Evidence,
            Kind::Pattern,
            Kind::Capability,
            Kind::Proposal,
            Kind::Decision,
            Kind::Setting,
            Kind::Meta,
        ]
    }

    /// Whether documents of this kind describe what an agent did.
    ///
    /// The Privacy page groups by this, and so does "forget everything about
    /// my AI activity", which has to be a different button from "forget what
    /// you learned about my hardware".
    pub fn is_agent_activity(self) -> bool {
        matches!(
            self,
            Kind::Agent | Kind::Session | Kind::Task | Kind::Action | Kind::Workspace
        )
    }
}

/// A stored record: its address, when it was written, and its body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// Which collection it belongs to.
    pub kind: Kind,
    /// Unique within the kind.
    pub id: String,
    /// Wall clock milliseconds of the last write.
    pub updated_ms: u64,
    /// The body, as the owning crate serialised it.
    pub body: serde_json::Value,
}

impl Document {
    /// Build a document around a serialisable body.
    pub fn new<T: Serialize>(kind: Kind, id: impl Into<String>, body: &T) -> Document {
        Document {
            kind,
            id: id.into(),
            updated_ms: now_ms(),
            body: serde_json::to_value(body).unwrap_or(serde_json::Value::Null),
        }
    }

    /// Deserialise the body.
    ///
    /// Returns `None` rather than failing loudly: a document written by a
    /// newer version with a field this build cannot parse should be skipped,
    /// not turned into a crash on startup.
    pub fn parse<T: for<'de> Deserialize<'de>>(&self) -> Option<T> {
        serde_json::from_value(self.body.clone()).ok()
    }
}

/// Wall clock milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_through_its_name() {
        for kind in Kind::all() {
            assert_eq!(Kind::parse(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn no_two_kinds_share_a_name() {
        // Two kinds with one name is two collections in one keyspace, and the
        // symptom is documents of one type appearing in a list of another.
        let mut names: Vec<&str> = Kind::all().iter().map(|k| k.as_str()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn an_unknown_name_does_not_become_a_kind() {
        assert_eq!(Kind::parse("whatever"), None);
    }

    #[test]
    fn agent_activity_is_separable_from_what_was_learned() {
        // "Forget my AI history" must not also delete the machine model.
        assert!(Kind::Session.is_agent_activity());
        assert!(Kind::Action.is_agent_activity());
        assert!(!Kind::Concept.is_agent_activity());
        assert!(!Kind::LatentState.is_agent_activity());
        assert!(!Kind::Setting.is_agent_activity());
    }

    #[test]
    fn a_body_that_cannot_be_parsed_is_skipped_rather_than_fatal() {
        let document = Document::new(Kind::Meta, "x", &serde_json::json!({"a": 1}));
        assert_eq!(document.parse::<u64>(), None);
        assert!(document.parse::<serde_json::Value>().is_some());
    }
}
