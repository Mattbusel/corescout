//! Watching an AI without asking it to cooperate.
//!
//! # The problem this exists to solve
//!
//! Until this file existed, CoreScout only learned about an AI's work if the
//! model *chose* to report it. That is a product whose central promise depends
//! on a model remembering to call a tool, which it will do sometimes. It made
//! the whole thing a demonstration rather than something that works while you
//! are not thinking about it.
//!
//! A hook runs on every tool call, whether or not the model thought about it.
//! That is the difference between a tool that learns from your work and one
//! that learns from the fraction of your work an agent felt like mentioning.
//!
//! # What a hook can and cannot see
//!
//! It sees the command, when it ran, how long it took, which files changed,
//! and what came before what. Those are exactly what the precursor analysis
//! needs, and they are reliable.
//!
//! It does **not** reliably see an exit code. Claude Code reports a tool
//! result, not a process status, and the shape of that result differs between
//! versions and between tools. So [`classify`] returns a definite outcome only
//! on a signal it can actually justify, and returns "nothing was said"
//! otherwise — which the rest of CoreScout already treats as an unknown rather
//! than a success.
//!
//! For an exact outcome there is `corescout run`, which owns the process and
//! therefore owns its exit code. The two are complementary: the hook gives
//! breadth without asking anything of anybody, and the wrapper gives certainty
//! where somebody wants it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use corescout_agent_observation::Raw;

/// One tool call, as a hook reports it.
///
/// Deliberately tolerant: every field is optional, because hook payloads
/// differ between clients and between versions of one client, and a shape
/// CoreScout does not recognise should cost one observation rather than break
/// the user's agent.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct HookEvent {
    /// The agent's own session identifier.
    #[serde(default)]
    pub session_id: String,
    /// Which hook fired.
    #[serde(default)]
    pub hook_event_name: String,
    /// The tool that was called.
    #[serde(default)]
    pub tool_name: String,
    /// What it was called with.
    #[serde(default)]
    pub tool_input: Value,
    /// What came back.
    #[serde(default)]
    pub tool_response: Value,
    /// Where the agent is working.
    #[serde(default)]
    pub cwd: String,
}

/// What the hook could establish about the outcome, and on what grounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// It worked, and there is a reason to believe that.
    Worked(&'static str),
    /// It failed, and there is a reason to believe that.
    Failed(&'static str),
    /// Nothing in the payload settles it.
    Unknown,
}

impl Outcome {
    /// The string the observation layer expects.
    pub fn reported(&self) -> Option<&'static str> {
        match self {
            Outcome::Worked(_) => Some("success"),
            Outcome::Failed(_) => Some("failure"),
            Outcome::Unknown => None,
        }
    }

    /// Why, for the record.
    pub fn because(&self) -> &'static str {
        match self {
            Outcome::Worked(why) | Outcome::Failed(why) => why,
            Outcome::Unknown => "nothing in the tool result settled it",
        }
    }
}

/// Decide what the tool result says about the outcome.
///
/// Conservative on purpose, and in the direction that produces silence. Every
/// arm below names the signal it acted on, and the ones that are guesses are
/// not here: an inferred success is the single most damaging thing that could
/// enter this system, because it teaches CoreScout that a broken procedure is
/// reliable.
pub fn classify(response: &Value) -> Outcome {
    // A process status, when the client provides one. Unambiguous.
    for key in ["exitCode", "exit_code", "returncode", "status"] {
        if let Some(code) = response.get(key).and_then(Value::as_i64) {
            return if code == 0 {
                Outcome::Worked("the process exited zero")
            } else {
                Outcome::Failed("the process exited non-zero")
            };
        }
    }
    // Explicit error flags, which several clients set.
    for key in ["is_error", "isError", "error"] {
        match response.get(key) {
            Some(Value::Bool(true)) => return Outcome::Failed("the tool reported an error"),
            Some(Value::String(text)) if !text.is_empty() => {
                return Outcome::Failed("the tool reported an error")
            }
            _ => {}
        }
    }
    if response.get("interrupted") == Some(&Value::Bool(true)) {
        return Outcome::Failed("the tool call was interrupted");
    }
    if response.get("success") == Some(&Value::Bool(true)) {
        return Outcome::Worked("the tool reported success");
    }
    if response.get("success") == Some(&Value::Bool(false)) {
        return Outcome::Failed("the tool reported failure");
    }

    // A bare string result. Claude Code returns the error text this way when a
    // command fails, and it is prefixed, so the prefix is the signal rather
    // than the presence of the word "error" anywhere in a build log.
    let text = response
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            response
                .get("stdout")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    let head = text.trim_start().to_lowercase();
    if head.starts_with("error:") || head.starts_with("command failed") {
        return Outcome::Failed("the tool result begins with an error");
    }

    // Deliberately nothing else. "stderr is non-empty" is not a failure --
    // cargo writes its whole progress to stderr -- and "the word error appears
    // somewhere" is not either.
    Outcome::Unknown
}

/// Whether a tool call is worth recording at all.
///
/// Reading a file is not an operation with an outcome, and recording thousands
/// of them would bury the handful that matter under noise CoreScout has no use
/// for. This is also a privacy decision: fewer things observed is fewer things
/// stored.
pub fn is_worth_recording(tool: &str) -> bool {
    !matches!(
        tool,
        "Read" | "Glob" | "Grep" | "TodoWrite" | "NotebookRead" | "WebFetch" | "WebSearch"
    )
}

/// What kind of action a tool call is.
pub fn kind_of(tool: &str, command: &str) -> &'static str {
    match tool {
        "Bash" | "BashOutput" => {
            let mut words = command.split_whitespace();
            let head = words.next().unwrap_or("");
            // The subcommand decides for the tools that do several jobs:
            // `cargo test` is a test run and `cargo build` is not.
            let next = words.find(|word| !word.starts_with('-')).unwrap_or("");
            match (head, next) {
                ("git", _) => "git",
                (_, "test" | "tests") => "test",
                ("pytest" | "jest" | "vitest" | "mocha", _) => "test",
                ("npm" | "pnpm" | "yarn", "run") => "shell",
                (
                    "cargo" | "make" | "msbuild" | "dotnet" | "go" | "tsc" | "webpack" | "vite",
                    _,
                ) => "build",
                ("docker" | "kubectl" | "terraform" | "vercel" | "fly", _) => "deploy",
                ("curl" | "wget" | "http", _) => "api",
                _ => "shell",
            }
        }
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => "file_edit",
        "Task" | "Agent" => "tool_call",
        _ => "tool_call",
    }
}

/// Turn a hook payload into something CoreScout can record.
///
/// Returns `None` for a call not worth recording, so the caller can exit
/// immediately without touching the service.
pub fn to_raw(event: &HookEvent, duration_ms: u64) -> Option<Raw> {
    if !is_worth_recording(&event.tool_name) {
        return None;
    }
    let command = event
        .tool_input
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_default();

    // For a shell call the command is the operation. For anything else the
    // tool name is, because "Edit" repeated forty times is one operation and
    // forty different file paths are not forty operations.
    let name = if command.is_empty() {
        event.tool_name.clone()
    } else {
        command
    };
    if name.trim().is_empty() {
        return None;
    }

    let outcome = classify(&event.tool_response);
    let files = ["file_path", "path", "notebook_path"]
        .iter()
        .filter_map(|key| event.tool_input.get(*key).and_then(Value::as_str))
        .map(str::to_string)
        .collect();

    Some(Raw {
        session: if event.session_id.is_empty() {
            "hook".into()
        } else {
            event.session_id.clone()
        },
        task: None,
        workspace: workspace_of(&event.cwd),
        kind: kind_of(&event.tool_name, &name).to_string(),
        name,
        started_ms: 0,
        duration_ms,
        exit_code: None,
        reported: outcome.reported().map(str::to_string),
        detail: match &outcome {
            Outcome::Failed(_) => Some(first_line(&event.tool_response)),
            _ => None,
        },
        // A hook never verifies anything. It watched; it did not check. Saying
        // otherwise here would undo the distinction the whole product rests on.
        verified: None,
        verification_detail: None,
        files,
    })
}

/// The folder the agent is working in.
///
/// The raw path, deliberately. Identifying it here is what made a hook and an
/// MCP client disagree about what a workspace is called: one sent
/// `app-1dc1d813` and the other sent the folder, so nothing an agent asked
/// about ever matched what the hook had recorded. The engine identifies, once,
/// for every route in.
fn workspace_of(cwd: &str) -> Option<String> {
    let cwd = cwd.trim();
    (!cwd.is_empty()).then(|| cwd.to_string())
}

/// The first usable line of a tool result, for the failure detail.
fn first_line(response: &Value) -> String {
    let text = response
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            response
                .get("stderr")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
        })
        .or_else(|| {
            response
                .get("stdout")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    line.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(tool: &str, input: Value, response: Value) -> HookEvent {
        HookEvent {
            session_id: "s1".into(),
            hook_event_name: "PostToolUse".into(),
            tool_name: tool.into(),
            tool_input: input,
            tool_response: response,
            cwd: "C:\\Projects\\app".into(),
        }
    }

    #[test]
    fn an_exit_code_settles_it_either_way() {
        assert_eq!(
            classify(&json!({ "exitCode": 0 })),
            Outcome::Worked("the process exited zero")
        );
        assert_eq!(
            classify(&json!({ "exit_code": 101 })),
            Outcome::Failed("the process exited non-zero")
        );
    }

    #[test]
    fn an_explicit_error_flag_settles_it() {
        assert!(matches!(
            classify(&json!({ "is_error": true })),
            Outcome::Failed(_)
        ));
        assert!(matches!(
            classify(&json!({ "interrupted": true })),
            Outcome::Failed(_)
        ));
    }

    #[test]
    fn an_error_prefixed_result_is_a_failure() {
        assert!(matches!(
            classify(&json!("Error: command failed with exit code 1")),
            Outcome::Failed(_)
        ));
        assert!(matches!(
            classify(&json!("Command failed: cargo build")),
            Outcome::Failed(_)
        ));
    }

    #[test]
    fn a_build_log_that_merely_mentions_errors_is_not_a_failure() {
        // The single most damaging mistake available here. `cargo build`
        // succeeding while printing "0 errors", or a test run that prints the
        // word, must not be recorded as a failure -- and, far worse, the
        // reverse must never happen either.
        let noisy = json!({
            "stdout": "warning: unused variable\n    Finished dev profile\n0 errors",
            "stderr": "   Compiling corescout v0.3.0\n    Finished in 4.2s"
        });
        assert_eq!(classify(&noisy), Outcome::Unknown);
    }

    #[test]
    fn stderr_alone_is_not_evidence_of_anything() {
        // cargo writes its entire progress to stderr. A tool that read that as
        // failure would decide every build in this repository had failed.
        let cargo = json!({ "stdout": "", "stderr": "   Compiling ...\n    Finished" });
        assert_eq!(classify(&cargo), Outcome::Unknown);
    }

    #[test]
    fn silence_stays_silence() {
        assert_eq!(classify(&json!({})), Outcome::Unknown);
        assert_eq!(classify(&Value::Null), Outcome::Unknown);
        assert_eq!(classify(&json!("done")), Outcome::Unknown);
        assert_eq!(Outcome::Unknown.reported(), None);
    }

    #[test]
    fn a_hook_never_claims_to_have_verified_anything() {
        // It watched. It did not check. Recording otherwise would collapse the
        // one distinction the whole product rests on.
        let raw = to_raw(
            &event(
                "Bash",
                json!({ "command": "cargo build" }),
                json!({ "exitCode": 0 }),
            ),
            1200,
        )
        .expect("worth recording");
        assert_eq!(raw.reported.as_deref(), Some("success"));
        assert_eq!(raw.verified, None, "a hook verifies nothing");
    }

    #[test]
    fn reading_files_is_not_recorded() {
        // Thousands of reads would bury the handful of operations that matter,
        // and every one not recorded is one not stored.
        for tool in ["Read", "Glob", "Grep", "TodoWrite", "WebSearch"] {
            assert!(!is_worth_recording(tool), "{tool}");
            assert!(to_raw(&event(tool, json!({}), json!({})), 0).is_none());
        }
        for tool in ["Bash", "Edit", "Write", "Task"] {
            assert!(is_worth_recording(tool), "{tool}");
        }
    }

    #[test]
    fn a_shell_call_is_named_by_its_command_and_an_edit_by_its_tool() {
        // Forty edits to forty files are one operation, not forty. Naming an
        // edit by its path would make every file its own failure mode.
        let shell = to_raw(
            &event("Bash", json!({ "command": "cargo test" }), json!({})),
            0,
        )
        .expect("recorded");
        assert_eq!(shell.name, "cargo test");
        assert_eq!(
            shell.kind, "test",
            "the subcommand decides, not the program"
        );
        assert_eq!(
            to_raw(
                &event(
                    "Bash",
                    json!({ "command": "cargo build --release" }),
                    json!({})
                ),
                0
            )
            .expect("recorded")
            .kind,
            "build"
        );

        let edit = to_raw(
            &event("Edit", json!({ "file_path": "src/main.rs" }), json!({})),
            0,
        )
        .expect("recorded");
        assert_eq!(edit.name, "Edit");
        assert_eq!(edit.kind, "file_edit");
        assert_eq!(edit.files, vec!["src/main.rs"]);
    }

    #[test]
    fn the_working_directory_is_passed_through_as_it_is() {
        // The folder as it is, not an identifier for it. Identifying here is
        // what made a hook and an MCP client disagree about what a workspace
        // is called, so an agent asking about a repository CoreScout knew a
        // great deal about was told it knew nothing. The engine identifies,
        // once, for every route in.
        let raw = to_raw(
            &event("Bash", json!({ "command": "cargo build" }), json!({})),
            0,
        )
        .expect("recorded");
        assert_eq!(raw.workspace.as_deref(), Some("C:\\Projects\\app"));
    }

    #[test]
    fn a_payload_shaped_differently_costs_one_observation_and_nothing_else() {
        // Hook payloads differ between clients and between versions. A shape
        // CoreScout does not recognise must not break somebody's agent.
        let odd: HookEvent = serde_json::from_str(r#"{"something":"else"}"#).expect("tolerant");
        assert_eq!(odd.tool_name, "");
        assert!(to_raw(&odd, 0).is_none());
    }

    #[test]
    fn a_command_of_nothing_is_not_recorded() {
        assert!(to_raw(&event("Bash", json!({ "command": "   " }), json!({})), 0).is_none());
    }

    #[test]
    fn a_failure_carries_the_first_useful_line_and_not_a_whole_log() {
        let raw = to_raw(
            &event(
                "Bash",
                json!({ "command": "cargo build" }),
                json!({
                    "exitCode": 1,
                    "stderr": "\n\nerror[E0433]: failed to resolve\n  --> src/main.rs:4:5\n",
                }),
            ),
            0,
        )
        .expect("recorded");
        let detail = raw.detail.expect("a detail");
        assert!(detail.starts_with("error[E0433]"), "{detail}");
        assert!(detail.len() <= 200);
    }
}
