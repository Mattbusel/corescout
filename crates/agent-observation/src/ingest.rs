//! Turning what an integration reports into entities that persist.
//!
//! # One door
//!
//! Every route into CoreScout's picture of agent activity comes through
//! [`Ingest::observe`]: the MCP server, the CLI, and any future adapter. That
//! means redaction, fingerprinting and retry detection happen once, in one
//! place, and an integration written later cannot accidentally skip them.
//!
//! # Retries are inferred, not reported
//!
//! Agents do not announce that they are retrying. They simply run the same
//! thing again. So a retry is inferred: the same fingerprint, in the same
//! session, following one that visibly failed, inside a window. That is a
//! judgement, and it is a conservative one, because counting unrelated repeats
//! as retries would inflate every failure statistic the product shows.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::action::{Action, ActionKind, Report, Verification};
use crate::fingerprint;
use crate::redact;

/// How long after a failure a repeat still counts as a retry.
pub const RETRY_WINDOW_MS: u64 = 10 * 60 * 1000;

/// What an integration reports.
///
/// Deliberately loose: an adapter should be able to send what it has without
/// filling in fields it cannot know. Everything optional here has a meaning
/// for its absence, and none of them are defaulted into a claim.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Raw {
    /// The session this belongs to.
    pub session: String,
    /// The task, if the agent tracks them.
    #[serde(default)]
    pub task: Option<String>,
    /// The workspace root, if there is one.
    #[serde(default)]
    pub workspace: Option<String>,
    /// What sort of action, by name. Unknown names become `other`.
    #[serde(default)]
    pub kind: String,
    /// The command line or tool name.
    pub name: String,
    /// When it started, Unix milliseconds. Zero means "now".
    #[serde(default)]
    pub started_ms: u64,
    /// How long it took.
    #[serde(default)]
    pub duration_ms: u64,
    /// The process exit code, if there was a process.
    #[serde(default)]
    pub exit_code: Option<i32>,
    /// What the tool said, if it said anything.
    #[serde(default)]
    pub reported: Option<String>,
    /// The failure text.
    #[serde(default)]
    pub detail: Option<String>,
    /// What was checked afterwards, if anything was.
    #[serde(default)]
    pub verified: Option<String>,
    /// How reality disagreed, or why the check could not be made.
    #[serde(default)]
    pub verification_detail: Option<String>,
    /// Files this action changed.
    #[serde(default)]
    pub files: Vec<String>,
}

/// The state needed to normalise a stream of reports.
#[derive(Debug, Default)]
pub struct Ingest {
    /// The last action per (session, fingerprint), for retry detection.
    last: HashMap<(String, String), Recent>,
    /// Actions observed, for id minting.
    counter: u64,
}

#[derive(Debug, Clone)]
struct Recent {
    id: String,
    at_ms: u64,
    failed: bool,
}

impl Ingest {
    /// A fresh accumulator.
    pub fn new() -> Ingest {
        Ingest::default()
    }

    /// Normalise one reported action.
    ///
    /// `now_ms` fills in a missing start time, so an adapter that has no clock
    /// of its own still produces an ordered stream.
    pub fn observe(&mut self, raw: &Raw, now_ms: u64) -> Action {
        let started_ms = if raw.started_ms == 0 {
            now_ms
        } else {
            raw.started_ms
        };
        // Redaction happens here, before anything reaches storage, so a value
        // that was never written cannot leak from anywhere later.
        let name = redact::scrub(&raw.name);
        let fingerprint = fingerprint::normalise(&name);
        let kind = classify(&raw.kind, &fingerprint);

        let reported = match raw.reported.as_deref() {
            Some("success") => Report::Success,
            Some("failure") => Report::Failure(detail(raw.detail.as_deref(), &raw.exit_code)),
            Some("error") => Report::Error(detail(raw.detail.as_deref(), &raw.exit_code)),
            Some("silent") | Some("") => Report::Silent,
            // Nothing was said. An exit code is a statement; its absence is
            // not, and inventing a success from silence is exactly the mistake
            // this crate exists to avoid.
            _ => match raw.exit_code {
                Some(0) => Report::Success,
                Some(code) => Report::Failure(format!("exit code {code}")),
                None => Report::Silent,
            },
        };

        let verified = match raw.verified.as_deref() {
            Some("confirmed") => Some(Verification::Confirmed),
            Some("contradicted") => Some(Verification::Contradicted(redact::scrub(
                raw.verification_detail
                    .as_deref()
                    .unwrap_or("it did not hold"),
            ))),
            Some("unavailable") => Some(Verification::Unavailable(redact::scrub(
                raw.verification_detail
                    .as_deref()
                    .unwrap_or("the check could not be made"),
            ))),
            _ => None,
        };

        let key = (raw.session.clone(), fingerprint.clone());
        let retry_of = self.last.get(&key).and_then(|recent| {
            let within = started_ms.saturating_sub(recent.at_ms) <= RETRY_WINDOW_MS;
            (recent.failed && within).then(|| recent.id.clone())
        });

        self.counter += 1;
        let id = format!("act-{started_ms:013}-{:06}", self.counter);

        let action = Action {
            id: id.clone(),
            session: raw.session.clone(),
            task: raw.task.clone(),
            workspace: raw.workspace.clone(),
            started_ms,
            duration_ms: raw.duration_ms,
            kind,
            name,
            fingerprint: fingerprint.clone(),
            reported,
            verified,
            exit_code: raw.exit_code,
            files: raw.files.iter().map(|f| redact::scrub(f)).collect(),
            retry_of,
            machine_state: None,
        };

        self.last.insert(
            key,
            Recent {
                id,
                at_ms: started_ms,
                failed: action.visibly_failed(),
            },
        );
        // Bound the memory: an accumulator that grows with every distinct
        // command a long-running session issues is a leak with a slow fuse.
        if self.last.len() > 4096 {
            let floor = now_ms.saturating_sub(RETRY_WINDOW_MS);
            self.last.retain(|_, recent| recent.at_ms >= floor);
        }
        action
    }

    /// Distinct commands currently remembered for retry detection.
    pub fn tracked(&self) -> usize {
        self.last.len()
    }

    /// Forget everything. What "clear my AI history" calls.
    pub fn forget(&mut self) {
        self.last.clear();
    }
}

fn detail(text: Option<&str>, exit_code: &Option<i32>) -> String {
    match (text, exit_code) {
        (Some(text), _) if !text.is_empty() => redact::scrub(text),
        (_, Some(code)) => format!("exit code {code}"),
        _ => "no detail reported".into(),
    }
}

/// Decide what sort of action this is, preferring what the adapter said.
///
/// The fallback reads the command, which is a guess, so it only fires when the
/// adapter said nothing or said something this build does not know.
fn classify(reported: &str, fingerprint: &str) -> ActionKind {
    let named = ActionKind::parse(reported);
    if named != ActionKind::Other {
        return named;
    }
    let first = fingerprint.split_whitespace().next().unwrap_or("");
    let second = fingerprint.split_whitespace().nth(1).unwrap_or("");
    match (first, second) {
        ("git", _) => ActionKind::Git,
        ("cargo" | "npm" | "pnpm" | "yarn" | "go" | "dotnet" | "msbuild", "test") => {
            ActionKind::Test
        }
        ("pytest" | "jest" | "vitest" | "mocha", _) => ActionKind::Test,
        ("cargo" | "go" | "msbuild" | "dotnet" | "make" | "tsc" | "webpack" | "vite", _) => {
            ActionKind::Build
        }
        ("npm" | "pnpm" | "yarn", "build") => ActionKind::Build,
        ("docker" | "kubectl" | "terraform" | "vercel" | "fly" | "heroku", _) => ActionKind::Deploy,
        ("curl" | "wget" | "http", _) => ActionKind::Api,
        ("", _) => ActionKind::Other,
        _ => ActionKind::Shell,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(name: &str) -> Raw {
        Raw {
            session: "s1".into(),
            name: name.into(),
            ..Raw::default()
        }
    }

    #[test]
    fn a_secret_never_reaches_the_stored_action() {
        // Redaction at ingestion rather than at display: a value that was
        // never written cannot leak from a backup or an export.
        let mut ingest = Ingest::new();
        let action = ingest.observe(&raw("deploy --token abc123def456"), 1000);
        assert!(!action.name.contains("abc123def456"), "{}", action.name);
        assert!(!action.fingerprint.contains("abc123def456"));
    }

    #[test]
    fn silence_does_not_become_a_success() {
        // An action nobody reported an outcome for is unknown, and every
        // downstream count depends on that staying true.
        let mut ingest = Ingest::new();
        let action = ingest.observe(&raw("cargo build"), 1000);
        assert_eq!(action.reported, Report::Silent);
        assert_eq!(action.succeeded(), None);
    }

    #[test]
    fn an_exit_code_is_a_statement_and_is_read_as_one() {
        let mut ingest = Ingest::new();
        let ok = ingest.observe(
            &Raw {
                exit_code: Some(0),
                ..raw("cargo build")
            },
            1000,
        );
        assert_eq!(ok.reported, Report::Success);
        let bad = ingest.observe(
            &Raw {
                exit_code: Some(101),
                ..raw("cargo test")
            },
            1000,
        );
        assert_eq!(bad.reported, Report::Failure("exit code 101".into()));
    }

    #[test]
    fn a_repeat_after_a_failure_is_recorded_as_a_retry() {
        let mut ingest = Ingest::new();
        let first = ingest.observe(
            &Raw {
                exit_code: Some(1),
                started_ms: 1000,
                ..raw("cargo build --release")
            },
            1000,
        );
        let second = ingest.observe(
            &Raw {
                started_ms: 60_000,
                exit_code: Some(0),
                ..raw("cargo build --release")
            },
            60_000,
        );
        assert_eq!(second.retry_of, Some(first.id));
    }

    #[test]
    fn a_repeat_after_a_success_is_not_a_retry() {
        // Running the tests twice because they passed is not a retry, and
        // counting it as one inflates every failure statistic shown.
        let mut ingest = Ingest::new();
        ingest.observe(
            &Raw {
                exit_code: Some(0),
                started_ms: 1000,
                ..raw("cargo test")
            },
            1000,
        );
        let second = ingest.observe(
            &Raw {
                exit_code: Some(0),
                started_ms: 2000,
                ..raw("cargo test")
            },
            2000,
        );
        assert_eq!(second.retry_of, None);
    }

    #[test]
    fn a_repeat_much_later_is_not_a_retry() {
        let mut ingest = Ingest::new();
        ingest.observe(
            &Raw {
                exit_code: Some(1),
                started_ms: 1000,
                ..raw("cargo build")
            },
            1000,
        );
        let later = ingest.observe(
            &Raw {
                started_ms: 1000 + RETRY_WINDOW_MS + 1,
                ..raw("cargo build")
            },
            0,
        );
        assert_eq!(later.retry_of, None);
    }

    #[test]
    fn a_repeat_in_another_session_is_not_a_retry() {
        let mut ingest = Ingest::new();
        ingest.observe(
            &Raw {
                exit_code: Some(1),
                started_ms: 1000,
                ..raw("cargo build")
            },
            1000,
        );
        let other = ingest.observe(
            &Raw {
                session: "s2".into(),
                started_ms: 2000,
                ..raw("cargo build")
            },
            2000,
        );
        assert_eq!(other.retry_of, None);
    }

    #[test]
    fn a_contradicted_verification_survives_ingestion() {
        let mut ingest = Ingest::new();
        let action = ingest.observe(
            &Raw {
                reported: Some("success".into()),
                verified: Some("contradicted".into()),
                verification_detail: Some("the health endpoint returned 502".into()),
                ..raw("deploy")
            },
            1000,
        );
        assert!(action.is_silent_failure());
        assert!(action.summary().contains("502"));
    }

    #[test]
    fn a_check_that_could_not_be_made_is_kept_as_an_absence_of_evidence() {
        let mut ingest = Ingest::new();
        let action = ingest.observe(
            &Raw {
                reported: Some("success".into()),
                verified: Some("unavailable".into()),
                verification_detail: Some("no route to host".into()),
                ..raw("deploy")
            },
            1000,
        );
        assert!(!action.is_silent_failure());
        assert_eq!(action.succeeded(), None);
    }

    #[test]
    fn the_adapter_is_believed_about_what_kind_of_action_it_was() {
        let mut ingest = Ingest::new();
        let action = ingest.observe(
            &Raw {
                kind: "deploy".into(),
                ..raw("./scripts/ship.sh")
            },
            1000,
        );
        assert_eq!(action.kind, ActionKind::Deploy);
    }

    #[test]
    fn the_command_is_read_only_when_the_adapter_said_nothing() {
        let mut ingest = Ingest::new();
        assert_eq!(ingest.observe(&raw("git push"), 1).kind, ActionKind::Git);
        assert_eq!(ingest.observe(&raw("cargo test"), 1).kind, ActionKind::Test);
        assert_eq!(
            ingest.observe(&raw("cargo build"), 1).kind,
            ActionKind::Build
        );
        assert_eq!(
            ingest.observe(&raw("kubectl apply -f x"), 1).kind,
            ActionKind::Deploy
        );
    }

    #[test]
    fn actions_get_distinct_ids_in_time_order() {
        let mut ingest = Ingest::new();
        let a = ingest.observe(&raw("one"), 1000);
        let b = ingest.observe(&raw("two"), 1000);
        let c = ingest.observe(&raw("three"), 2000);
        assert_ne!(a.id, b.id);
        assert!(a.id < b.id, "{} then {}", a.id, b.id);
        assert!(b.id < c.id);
    }

    #[test]
    fn retry_memory_does_not_grow_without_bound() {
        // A long-lived session issuing thousands of distinct commands must not
        // slowly consume the service.
        let mut ingest = Ingest::new();
        for n in 0..20_000u64 {
            ingest.observe(
                &Raw {
                    started_ms: n,
                    ..raw(&format!("command-{n}"))
                },
                n + RETRY_WINDOW_MS * 2,
            );
        }
        assert!(ingest.tracked() <= 4096, "{}", ingest.tracked());
    }

    #[test]
    fn a_report_with_nothing_filled_in_is_still_recorded() {
        // An adapter that knows only that something happened should not be
        // rejected; the timing alone is worth having.
        let mut ingest = Ingest::new();
        let action = ingest.observe(
            &Raw {
                session: "s1".into(),
                name: "something".into(),
                ..Raw::default()
            },
            5000,
        );
        assert_eq!(action.started_ms, 5000);
        assert_eq!(action.reported, Report::Silent);
    }
}
