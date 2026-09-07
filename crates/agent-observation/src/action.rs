//! One externally visible thing an agent did.

use serde::{Deserialize, Serialize};

/// What sort of thing it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// A tool the agent called through its own tool interface.
    ToolCall,
    /// A command run in a shell.
    Shell,
    /// A compile.
    Build,
    /// A test run.
    Test,
    /// A version control operation.
    Git,
    /// A process the agent started.
    ProcessLaunch,
    /// A call to a remote service.
    Api,
    /// A file the agent wrote.
    FileEdit,
    /// A check the agent made of the external world.
    Verification,
    /// A deployment or a service restart.
    Deploy,
    /// Something that did not fit. Kept rather than dropped.
    Other,
}

impl ActionKind {
    /// The stable name used in storage and on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            ActionKind::ToolCall => "tool_call",
            ActionKind::Shell => "shell",
            ActionKind::Build => "build",
            ActionKind::Test => "test",
            ActionKind::Git => "git",
            ActionKind::ProcessLaunch => "process_launch",
            ActionKind::Api => "api",
            ActionKind::FileEdit => "file_edit",
            ActionKind::Verification => "verification",
            ActionKind::Deploy => "deploy",
            ActionKind::Other => "other",
        }
    }

    /// Parse a stored name, falling back to [`ActionKind::Other`].
    ///
    /// Falling back rather than failing is deliberate: an agent integration
    /// sending a kind this build does not know should still have its action
    /// recorded, because the timing and the outcome are the useful parts.
    pub fn parse(name: &str) -> ActionKind {
        match name {
            "tool_call" => ActionKind::ToolCall,
            "shell" => ActionKind::Shell,
            "build" => ActionKind::Build,
            "test" => ActionKind::Test,
            "git" => ActionKind::Git,
            "process_launch" => ActionKind::ProcessLaunch,
            "api" => ActionKind::Api,
            "file_edit" => ActionKind::FileEdit,
            "verification" => ActionKind::Verification,
            "deploy" => ActionKind::Deploy,
            _ => ActionKind::Other,
        }
    }

    /// Whether this kind of action changes the world outside the agent.
    ///
    /// Reading a file is not the same class of event as restarting a service,
    /// and the pattern layer weighs them differently.
    pub fn is_effectful(self) -> bool {
        matches!(
            self,
            ActionKind::Shell
                | ActionKind::ProcessLaunch
                | ActionKind::FileEdit
                | ActionKind::Deploy
                | ActionKind::Git
                | ActionKind::Build
        )
    }
}

/// What the tool said happened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "said", content = "detail")]
pub enum Report {
    /// The tool reported success.
    Success,
    /// The tool reported a failure, and said why.
    Failure(String),
    /// The tool itself errored: it did not run.
    Error(String),
    /// The tool said nothing usable.
    Silent,
}

impl Report {
    /// Whether the tool claimed this worked.
    pub fn claims_success(&self) -> bool {
        matches!(self, Report::Success)
    }

    /// The failure text, if there was one.
    pub fn detail(&self) -> Option<&str> {
        match self {
            Report::Failure(text) | Report::Error(text) => Some(text),
            _ => None,
        }
    }
}

/// What was actually checked afterwards.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "found", content = "detail")]
pub enum Verification {
    /// Reality agreed with the report.
    Confirmed,
    /// Reality disagreed, and this is how.
    Contradicted(String),
    /// A check was attempted and could not be made.
    ///
    /// Not the same as no check: "the health endpoint timed out" is a fact,
    /// and "nobody looked" is an absence. The mirror represents the difference
    /// rather than folding one into the other.
    Unavailable(String),
}

/// One externally visible action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Action {
    /// Unique across this machine.
    pub id: String,
    /// The session it belongs to.
    pub session: String,
    /// The task it was part of, if the agent said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// The workspace it happened in, if there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// When it started, Unix milliseconds.
    pub started_ms: u64,
    /// How long it took.
    pub duration_ms: u64,
    /// What sort of thing it was.
    pub kind: ActionKind,
    /// What it was: a tool name, a command line, a target.
    pub name: String,
    /// The normalised form, for counting recurrence.
    pub fingerprint: String,
    /// What the tool said.
    pub reported: Report,
    /// What was checked, if anything was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified: Option<Verification>,
    /// A process exit code, where there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Files this action changed, relative to the workspace where possible.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// The action this one was a retry of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_of: Option<String>,
    /// The latent state the machine was in when it started, if one was known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_state: Option<u32>,
}

impl Action {
    /// Whether the tool said it worked and something proved otherwise.
    ///
    /// This is the event the product exists to find. It is deliberately narrow:
    /// an unverified success is not a silent failure, it is an unknown.
    pub fn is_silent_failure(&self) -> bool {
        self.reported.claims_success()
            && matches!(self.verified, Some(Verification::Contradicted(_)))
    }

    /// Whether this action worked, as far as anyone actually knows.
    ///
    /// `None` means nobody checked and the tool's word is all there is, which
    /// is a different thing from a success and is represented as one.
    pub fn succeeded(&self) -> Option<bool> {
        match (&self.reported, &self.verified) {
            (_, Some(Verification::Contradicted(_))) => Some(false),
            (Report::Success, Some(Verification::Confirmed)) => Some(true),
            (Report::Failure(_) | Report::Error(_), _) => Some(false),
            _ => None,
        }
    }

    /// Whether this action failed in a way anyone can see.
    pub fn visibly_failed(&self) -> bool {
        matches!(self.succeeded(), Some(false))
    }

    /// Whether this action's outcome rests only on the tool's own word.
    pub fn unverified(&self) -> bool {
        self.verified.is_none()
    }

    /// One line for the timeline.
    pub fn summary(&self) -> String {
        let outcome = match (&self.reported, &self.verified) {
            (Report::Success, Some(Verification::Contradicted(how))) => {
                format!("reported success, but {how}")
            }
            (Report::Success, Some(Verification::Confirmed)) => "succeeded, verified".into(),
            (Report::Success, Some(Verification::Unavailable(why))) => {
                format!("reported success, could not check: {why}")
            }
            (Report::Success, None) => "reported success".into(),
            (Report::Failure(why), _) => format!("failed: {}", first_line(why)),
            (Report::Error(why), _) => format!("could not run: {}", first_line(why)),
            (Report::Silent, _) => "no result reported".into(),
        };
        format!("{} {}", self.name, outcome)
    }
}

fn first_line(text: &str) -> &str {
    let line = text.lines().next().unwrap_or("").trim();
    if line.len() > 120 {
        &line[..120]
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action() -> Action {
        Action {
            id: "a1".into(),
            session: "s1".into(),
            task: None,
            workspace: Some("w1".into()),
            started_ms: 1_700_000_000_000,
            duration_ms: 4200,
            kind: ActionKind::Deploy,
            name: "deploy".into(),
            fingerprint: "deploy".into(),
            reported: Report::Success,
            verified: None,
            exit_code: Some(0),
            files: Vec::new(),
            retry_of: None,
            machine_state: Some(21),
        }
    }

    #[test]
    fn a_reported_success_nobody_checked_is_not_a_success() {
        // The whole product turns on this line. An unverified claim is an
        // unknown, and folding it into "worked" is how a system learns that a
        // broken procedure is reliable.
        let action = action();
        assert_eq!(action.succeeded(), None);
        assert!(action.unverified());
        assert!(!action.is_silent_failure());
    }

    #[test]
    fn a_reported_success_that_reality_contradicts_is_a_silent_failure() {
        let contradicted = Action {
            verified: Some(Verification::Contradicted(
                "the service was not reachable for another 40 seconds".into(),
            )),
            ..action()
        };
        assert!(contradicted.is_silent_failure());
        assert_eq!(contradicted.succeeded(), Some(false));
        assert!(contradicted.visibly_failed());
    }

    #[test]
    fn a_check_that_could_not_be_made_is_not_a_contradiction() {
        // "The health endpoint timed out" and "the deploy did not work" are
        // different claims, and only one of them is evidence.
        let unavailable = Action {
            verified: Some(Verification::Unavailable("no network route".into())),
            ..action()
        };
        assert!(!unavailable.is_silent_failure());
        assert_eq!(unavailable.succeeded(), None);
    }

    #[test]
    fn a_verified_success_is_the_only_thing_that_counts_as_one() {
        let confirmed = Action {
            verified: Some(Verification::Confirmed),
            ..action()
        };
        assert_eq!(confirmed.succeeded(), Some(true));
    }

    #[test]
    fn a_reported_failure_is_a_failure_whether_or_not_anyone_checked() {
        let failed = Action {
            reported: Report::Failure("exit code 101".into()),
            ..action()
        };
        assert_eq!(failed.succeeded(), Some(false));
        assert!(!failed.is_silent_failure(), "it did not claim success");
    }

    #[test]
    fn a_contradiction_beats_a_reported_failure_too() {
        // A tool that says it failed while the change actually landed is the
        // mirror image of the silent failure and matters just as much.
        let odd = Action {
            reported: Report::Failure("timeout".into()),
            verified: Some(Verification::Contradicted("the version did change".into())),
            ..action()
        };
        assert_eq!(odd.succeeded(), Some(false));
    }

    #[test]
    fn summaries_are_readable_and_say_which_kind_of_outcome_it_was() {
        let unverified = action();
        assert_eq!(unverified.summary(), "deploy reported success");
        let silent = Action {
            verified: Some(Verification::Contradicted("the service was down".into())),
            ..action()
        };
        assert!(silent.summary().contains("reported success, but"));
    }

    #[test]
    fn a_long_error_is_trimmed_to_one_readable_line() {
        let noisy = Action {
            reported: Report::Failure(format!("{}\nand more\nand more", "x".repeat(400))),
            ..action()
        };
        let summary = noisy.summary();
        assert!(summary.len() < 200, "{}", summary.len());
        assert!(!summary.contains('\n'));
    }

    #[test]
    fn an_unknown_action_kind_is_kept_rather_than_dropped() {
        // An integration sending a kind this build predates should still have
        // its timing and outcome recorded.
        assert_eq!(ActionKind::parse("something_new"), ActionKind::Other);
        assert_eq!(ActionKind::parse("deploy"), ActionKind::Deploy);
    }

    #[test]
    fn every_kind_round_trips_through_its_name() {
        let kinds = [
            ActionKind::ToolCall,
            ActionKind::Shell,
            ActionKind::Build,
            ActionKind::Test,
            ActionKind::Git,
            ActionKind::ProcessLaunch,
            ActionKind::Api,
            ActionKind::FileEdit,
            ActionKind::Verification,
            ActionKind::Deploy,
            ActionKind::Other,
        ];
        for kind in kinds {
            assert_eq!(ActionKind::parse(kind.as_str()), kind);
        }
    }

    #[test]
    fn an_action_survives_storage() {
        let original = Action {
            verified: Some(Verification::Contradicted("still 404".into())),
            files: vec!["src/main.rs".into()],
            ..action()
        };
        let json = serde_json::to_string(&original).expect("serialise");
        let back: Action = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back, original);
        assert!(back.is_silent_failure());
    }
}
