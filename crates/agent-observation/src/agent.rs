//! The AI systems that have connected, as persistent entities.
//!
//! An agent is an entity in the mirror's sense: it persists across sessions,
//! it has observable properties, and what is known about it accumulates. That
//! is what makes "Codex inherited what the computer learned while Claude was
//! here" a question the data model can even express.

use serde::{Deserialize, Serialize};

/// Which AI system this is.
///
/// Named variants exist for the ones with a setup flow of their own. Anything
/// speaking the protocol without being recognised is [`AgentKind::Mcp`], which
/// is a first-class case rather than a fallback: the generic path is the one
/// that has to work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "name")]
pub enum AgentKind {
    /// Anthropic's Claude Code.
    ClaudeCode,
    /// An OpenAI coding agent.
    Codex,
    /// OpenCode.
    OpenCode,
    /// Cursor.
    Cursor,
    /// Any other client speaking the protocol.
    Mcp,
    /// A command line tool using the local API directly.
    Cli,
    /// Something that named itself and is not otherwise recognised.
    Named(String),
}

impl AgentKind {
    /// Recognise a client from the name it gives when it connects.
    ///
    /// Matching is on a lowercased substring, because clients report
    /// themselves with version suffixes and inconsistent spacing, and getting
    /// this wrong costs nothing worse than a generic label.
    pub fn recognise(reported: &str) -> AgentKind {
        let name = reported.trim().to_lowercase();
        if name.is_empty() {
            return AgentKind::Mcp;
        }
        if name.contains("claude") {
            return AgentKind::ClaudeCode;
        }
        if name.contains("codex") || name.contains("openai") {
            return AgentKind::Codex;
        }
        if name.contains("opencode") {
            return AgentKind::OpenCode;
        }
        if name.contains("cursor") {
            return AgentKind::Cursor;
        }
        if name.contains("corescout-cli") || name.contains("cli") {
            return AgentKind::Cli;
        }
        AgentKind::Named(reported.trim().to_string())
    }

    /// The name shown in the interface.
    pub fn title(&self) -> String {
        match self {
            AgentKind::ClaudeCode => "Claude Code".into(),
            AgentKind::Codex => "Codex".into(),
            AgentKind::OpenCode => "OpenCode".into(),
            AgentKind::Cursor => "Cursor".into(),
            AgentKind::Mcp => "MCP client".into(),
            AgentKind::Cli => "Command line".into(),
            AgentKind::Named(name) => name.clone(),
        }
    }

    /// A stable slug, for storage keys and configuration file names.
    pub fn slug(&self) -> String {
        match self {
            AgentKind::ClaudeCode => "claude-code".into(),
            AgentKind::Codex => "codex".into(),
            AgentKind::OpenCode => "opencode".into(),
            AgentKind::Cursor => "cursor".into(),
            AgentKind::Mcp => "mcp".into(),
            AgentKind::Cli => "cli".into(),
            AgentKind::Named(name) => name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() {
                        c.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .collect(),
        }
    }

    /// The ones with a setup flow written for them.
    pub fn with_setup_flow() -> [AgentKind; 4] {
        [
            AgentKind::ClaudeCode,
            AgentKind::Codex,
            AgentKind::OpenCode,
            AgentKind::Cursor,
        ]
    }
}

/// An AI system that has connected to CoreScout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    /// Stable across sessions. Derived from the kind and the reported name.
    pub id: String,
    /// Which system it is.
    pub kind: AgentKind,
    /// What it called itself.
    pub reported_name: String,
    /// The version it reported, if it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// When it first connected, Unix milliseconds.
    pub first_seen_ms: u64,
    /// When it was last heard from.
    pub last_seen_ms: u64,
    /// Sessions it has had.
    pub sessions: u64,
    /// Actions observed from it, across every session.
    pub actions: u64,
    /// Whether it is connected right now.
    #[serde(default)]
    pub connected: bool,
}

impl Agent {
    /// Register an agent that has just identified itself.
    pub fn new(reported_name: &str, version: Option<String>, now_ms: u64) -> Agent {
        let kind = AgentKind::recognise(reported_name);
        Agent {
            id: kind.slug(),
            kind,
            reported_name: reported_name.trim().to_string(),
            version,
            first_seen_ms: now_ms,
            last_seen_ms: now_ms,
            sessions: 0,
            actions: 0,
            connected: true,
        }
    }

    /// Note that this agent is active.
    pub fn touch(&mut self, now_ms: u64) {
        self.last_seen_ms = now_ms;
        self.connected = true;
    }

    /// Note that this agent has gone.
    pub fn disconnect(&mut self, now_ms: u64) {
        self.last_seen_ms = now_ms;
        self.connected = false;
    }

    /// Whether this agent has been quiet long enough to call it disconnected.
    ///
    /// Agents do not reliably say goodbye: a terminal closes and the process
    /// is gone. So being connected is inferred from recent activity, and the
    /// window is generous because an agent thinking for four minutes has not
    /// disconnected.
    pub fn is_stale(&self, now_ms: u64, window_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_seen_ms) > window_ms
    }

    /// The line shown on the AI screen.
    pub fn summary(&self, now_ms: u64) -> String {
        if self.connected {
            format!(
                "{} connected, {} actions observed",
                self.kind.title(),
                self.actions
            )
        } else {
            let ago = now_ms.saturating_sub(self.last_seen_ms) / 60_000;
            match ago {
                0 => format!("{} last seen just now", self.kind.title()),
                1 => format!("{} last seen a minute ago", self.kind.title()),
                n if n < 60 => format!("{} last seen {n} minutes ago", self.kind.title()),
                n => format!("{} last seen {} hours ago", self.kind.title(), n / 60),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_named_clients_are_recognised_however_they_spell_themselves() {
        assert_eq!(AgentKind::recognise("claude-code"), AgentKind::ClaudeCode);
        assert_eq!(
            AgentKind::recognise("Claude Code 2.1"),
            AgentKind::ClaudeCode
        );
        assert_eq!(AgentKind::recognise("codex-cli"), AgentKind::Codex);
        assert_eq!(AgentKind::recognise("OpenCode"), AgentKind::OpenCode);
        assert_eq!(AgentKind::recognise("Cursor"), AgentKind::Cursor);
    }

    #[test]
    fn an_unrecognised_client_is_still_a_first_class_agent() {
        // The generic path is the one that has to work. A client CoreScout has
        // never heard of gets a name and a record, not a rejection.
        let unknown = AgentKind::recognise("some-new-agent");
        assert_eq!(unknown, AgentKind::Named("some-new-agent".into()));
        assert_eq!(unknown.title(), "some-new-agent");
        assert_eq!(unknown.slug(), "some-new-agent");
    }

    #[test]
    fn a_client_that_gives_no_name_is_a_generic_mcp_client() {
        assert_eq!(AgentKind::recognise(""), AgentKind::Mcp);
        assert_eq!(AgentKind::recognise("   "), AgentKind::Mcp);
    }

    #[test]
    fn a_slug_is_always_usable_as_a_storage_key() {
        // Storage refuses ids containing its separator, and configuration
        // files are named from these.
        let awkward = AgentKind::recognise("Weird/Agent v2.0 \u{1}");
        let slug = awkward.slug();
        assert!(slug.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
        assert!(!slug.is_empty());
    }

    #[test]
    fn an_agent_is_the_same_entity_across_sessions() {
        // This is what lets one model's experience outlive its session, which
        // is the claim the whole product is built to test.
        let first = Agent::new("claude-code", None, 1000);
        let second = Agent::new("Claude Code", Some("2.1".into()), 9000);
        assert_eq!(first.id, second.id);
    }

    #[test]
    fn an_agent_that_stops_talking_goes_stale_rather_than_lying() {
        let mut agent = Agent::new("claude-code", None, 0);
        agent.touch(60_000);
        assert!(!agent.is_stale(70_000, 300_000));
        assert!(agent.is_stale(400_000, 300_000));
    }

    #[test]
    fn a_disconnected_agent_says_when_it_was_last_here() {
        let mut agent = Agent::new("claude-code", None, 0);
        agent.disconnect(0);
        let now = 45 * 60_000;
        assert_eq!(agent.summary(now), "Claude Code last seen 45 minutes ago");
        assert_eq!(
            agent.summary(3 * 60 * 60_000),
            "Claude Code last seen 3 hours ago"
        );
    }

    #[test]
    fn a_connected_agent_says_what_has_been_seen() {
        let mut agent = Agent::new("claude-code", None, 0);
        agent.actions = 23;
        agent.touch(1000);
        assert_eq!(
            agent.summary(1000),
            "Claude Code connected, 23 actions observed"
        );
    }

    #[test]
    fn every_client_with_a_setup_flow_has_a_distinct_slug() {
        let mut slugs: Vec<String> = AgentKind::with_setup_flow()
            .iter()
            .map(|kind| kind.slug())
            .collect();
        slugs.sort();
        let count = slugs.len();
        slugs.dedup();
        assert_eq!(slugs.len(), count);
    }

    #[test]
    fn an_agent_survives_storage() {
        let agent = Agent::new("claude-code", Some("2.1".into()), 42);
        let json = serde_json::to_string(&agent).expect("serialise");
        let back: Agent = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back, agent);
    }
}
