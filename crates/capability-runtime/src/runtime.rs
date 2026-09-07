//! Running a capability, inside its boundaries.
//!
//! # Nothing runs that was not permitted twice
//!
//! Before a single process starts, the capability is validated and the
//! permission layer rules on it. Then each step is checked again against the
//! authority as it is about to run, because a capability that was permitted
//! when it started must not become able to touch something new halfway
//! through by having its own definition edited underneath it.
//!
//! # There is no shell
//!
//! Steps name a program and pass arguments as a vector. Nothing is ever handed
//! to `cmd.exe` or `sh` for re-parsing, so there is no place for a quoting
//! mistake to become an execution.
//!
//! # A verification failure is a rollback, not a footnote
//!
//! If the steps all exit zero and the verification step says otherwise, the
//! run failed. That is the whole point: an exit code is a claim and the
//! verification is the check. When it fails and a rollback exists, the
//! rollback runs, and the record says both things happened.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use corescout_permissions::{Permissions, Request, Ruling};
use serde::{Deserialize, Serialize};

use crate::capability::{Capability, Operation};

/// Output kept per step. Beyond this the middle is dropped, keeping the start
/// and the end, which is where the useful part of a build log lives.
pub const OUTPUT_LIMIT: usize = 8 * 1024;

/// How often a running step is checked for having finished.
const POLL: Duration = Duration::from_millis(25);

/// What happened to one step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepResult {
    /// What it was.
    pub describe: String,
    /// The exit code, if the program ran to completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// How long it took.
    pub duration_ms: u64,
    /// Output, redacted and truncated.
    pub output: String,
    /// Whether this step is considered to have worked.
    pub ok: bool,
    /// What went wrong, if something did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What happened to a run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum Outcome {
    /// The permission layer said no.
    Refused {
        /// What to tell the user.
        because: String,
    },
    /// A person has to say yes first.
    NeedsApproval {
        /// What to tell the user.
        because: String,
    },
    /// The definition itself is not valid.
    Invalid {
        /// What is wrong with it.
        because: String,
    },
    /// It ran.
    Ran {
        /// What each step did.
        steps: Vec<StepResult>,
        /// What the verification step found. `None` when the steps failed
        /// before verification could be reached, which is an unknown rather
        /// than a failed check.
        verified: Option<bool>,
        /// Whether the rollback ran.
        rolled_back: bool,
        /// How long the whole thing took.
        duration_ms: u64,
    },
}

impl Outcome {
    /// Whether this run did what it set out to do, as checked.
    pub fn succeeded(&self) -> bool {
        matches!(
            self,
            Outcome::Ran {
                verified: Some(true),
                ..
            }
        )
    }

    /// Whether anything actually ran.
    pub fn executed(&self) -> bool {
        matches!(self, Outcome::Ran { .. })
    }

    /// One line for the timeline.
    pub fn summary(&self) -> String {
        match self {
            Outcome::Refused { because } => format!("refused: {because}"),
            Outcome::NeedsApproval { because } => format!("waiting for you: {because}"),
            Outcome::Invalid { because } => format!("not a valid capability: {because}"),
            Outcome::Ran {
                verified: Some(true),
                duration_ms,
                ..
            } => format!("ran and verified in {}s", duration_ms / 1000),
            Outcome::Ran {
                verified: Some(false),
                rolled_back,
                ..
            } => {
                if *rolled_back {
                    "ran, verification failed, rolled back".into()
                } else {
                    "ran, verification failed".into()
                }
            }
            Outcome::Ran { steps, .. } => match steps.iter().find(|step| !step.ok) {
                Some(step) => format!("stopped at {}", step.describe),
                None => "ran".into(),
            },
        }
    }
}

/// Where a capability runs.
#[derive(Clone, Debug, Default)]
pub struct Context {
    /// The folder the workspace lives in.
    pub root: Option<PathBuf>,
    /// Why this is being run, for the audit record.
    pub reason: String,
}

/// Runs capabilities.
#[derive(Debug, Default)]
pub struct Runtime {
    /// Whether to actually start processes.
    ///
    /// A dry run walks the whole path, including validation and the permission
    /// ruling, and starts nothing. It is what the interface uses to show what
    /// would happen, and what the tests use to exercise the decisions without
    /// depending on what is installed.
    pub dry_run: bool,
}

impl Runtime {
    /// A runtime that runs things.
    pub fn live() -> Runtime {
        Runtime { dry_run: false }
    }

    /// A runtime that decides everything and starts nothing.
    pub fn dry() -> Runtime {
        Runtime { dry_run: true }
    }

    /// Run a capability.
    pub fn execute(
        &self,
        capability: &Capability,
        context: &Context,
        permissions: &Permissions,
        now_ns: u64,
    ) -> Outcome {
        if let Err(invalid) = capability.validate() {
            return Outcome::Invalid {
                because: invalid.to_string(),
            };
        }

        let request = Request {
            capability: capability.id.clone(),
            targets: capability.targets(),
            risk: capability.risk,
            reversible: capability.reversible,
            reason: if context.reason.is_empty() {
                capability.purpose.clone()
            } else {
                context.reason.clone()
            },
        };
        match permissions.rule(&request, now_ns) {
            Ruling::Refused { because } => return Outcome::Refused { because },
            Ruling::NeedsApproval { because } => return Outcome::NeedsApproval { because },
            Ruling::Allowed { .. } => {}
        }

        let started = Instant::now();
        let mut steps = Vec::new();
        let mut failed = false;

        for operation in &capability.steps {
            // Re-checked immediately before running, so a definition edited
            // underneath a run cannot widen what it touches mid-flight.
            if permissions
                .authority()
                .permits(&operation.targets())
                .is_err()
            {
                steps.push(StepResult {
                    describe: operation.describe(),
                    exit_code: None,
                    duration_ms: 0,
                    output: String::new(),
                    ok: false,
                    error: Some("this step is outside what CoreScout may touch".into()),
                });
                failed = true;
                break;
            }
            let result = self.perform(operation, context);
            let ok = result.ok;
            steps.push(result);
            if !ok {
                failed = true;
                break;
            }
        }

        // Verification is not attempted when the steps did not finish: the
        // check would be answering a question nobody asked, and a green
        // verification after a failed step reads as a success.
        let verified = if failed {
            None
        } else {
            let result = self.perform(&capability.verification, context);
            let ok = result.ok;
            steps.push(result);
            Some(ok)
        };

        let mut rolled_back = false;
        if verified == Some(false) || failed {
            for operation in &capability.rollback {
                rolled_back = true;
                steps.push(self.perform(operation, context));
            }
        }

        Outcome::Ran {
            steps,
            verified,
            rolled_back,
            duration_ms: started.elapsed().as_millis() as u64,
        }
    }

    fn perform(&self, operation: &Operation, context: &Context) -> StepResult {
        let describe = operation.describe();
        match operation {
            Operation::Wait { ms, .. } => {
                if !self.dry_run {
                    std::thread::sleep(Duration::from_millis((*ms).min(60_000)));
                }
                StepResult {
                    describe,
                    exit_code: None,
                    duration_ms: *ms,
                    output: String::new(),
                    ok: true,
                    error: None,
                }
            }
            Operation::Run {
                command,
                args,
                cwd,
                expect,
                ..
            } => {
                if self.dry_run {
                    return StepResult {
                        describe,
                        exit_code: Some(0),
                        duration_ms: 0,
                        output: "(dry run: nothing was started)".into(),
                        ok: true,
                        error: None,
                    };
                }
                let directory = working_directory(context.root.as_deref(), cwd.as_deref());
                let started = Instant::now();
                match run(command, args, directory.as_deref(), operation.timeout_ms()) {
                    Ok((code, output)) => {
                        // `map_or(true, ..)` rather than `is_none_or`, which
                        // is newer than this workspace's minimum Rust.
                        let matched = expect
                            .as_ref()
                            .map_or(true, |needle| output.contains(needle.as_str()));
                        StepResult {
                            describe,
                            exit_code: code,
                            duration_ms: started.elapsed().as_millis() as u64,
                            output: trim(&corescout_agent_observation::redact::scrub(&output)),
                            ok: code == Some(0) && matched,
                            error: (!matched).then(|| {
                                format!(
                                    "the output did not contain {:?}",
                                    expect.clone().unwrap_or_default()
                                )
                            }),
                        }
                    }
                    Err(error) => StepResult {
                        describe,
                        exit_code: None,
                        duration_ms: started.elapsed().as_millis() as u64,
                        output: String::new(),
                        ok: false,
                        error: Some(error),
                    },
                }
            }
        }
    }
}

/// Where a step runs: the workspace root, plus any relative directory.
fn working_directory(root: Option<&Path>, cwd: Option<&Path>) -> Option<PathBuf> {
    match (root, cwd) {
        (Some(root), Some(cwd)) => Some(root.join(cwd)),
        (Some(root), None) => Some(root.to_path_buf()),
        (None, _) => None,
    }
}

/// Start a program and wait for it, with a ceiling.
///
/// The standard library has no timeout on `wait`, so this polls `try_wait` and
/// kills what is still running when the ceiling is reached. A build that hangs
/// has to be a failure with a reason rather than a service that never returns.
fn run(
    command: &str,
    args: &[String],
    cwd: Option<&Path>,
    timeout_ms: u64,
) -> Result<(Option<i32>, String), String> {
    let mut builder = Command::new(command);
    builder
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        builder.current_dir(cwd);
    }
    let mut child = builder
        .spawn()
        .map_err(|error| format!("could not start {command}: {error}"))?;

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child
                    .wait_with_output()
                    .map(|out| {
                        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                        text.push_str(&String::from_utf8_lossy(&out.stderr));
                        text
                    })
                    .unwrap_or_default();
                return Ok((status.code(), output));
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("{command} did not finish within {timeout_ms} ms"));
                }
                std::thread::sleep(POLL);
            }
            Err(error) => return Err(format!("{command} could not be waited on: {error}")),
        }
    }
}

/// Keep the start and the end of a long output, and say what was dropped.
fn trim(text: &str) -> String {
    if text.len() <= OUTPUT_LIMIT {
        return text.to_string();
    }
    let half = OUTPUT_LIMIT / 2;
    let head: String = text.chars().take(half).collect();
    let tail: String = text
        .chars()
        .rev()
        .take(half)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    format!(
        "{head}\n... [{} characters omitted] ...\n{tail}",
        text.len() - OUTPUT_LIMIT
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Provenance;
    use corescout_permissions::{Autonomy, Grant, Risk};

    fn capability() -> Capability {
        Capability {
            id: "cap-1".into(),
            name: "Regenerate schema, then build".into(),
            purpose: "Builds here read a generated file that goes stale.".into(),
            workspace: None,
            preconditions: Vec::new(),
            steps: vec![Operation::run("cargo", &["build"])],
            verification: Operation::run("cargo", &["build"]),
            rollback: Vec::new(),
            risk: Risk::Low,
            reversible: true,
            provenance: Provenance::BuiltIn,
            uses: 0,
            successes: 0,
            created_ms: 0,
            last_used_ms: 0,
        }
    }

    fn permissive() -> Permissions {
        let mut permissions = Permissions::new();
        permissions.set_autonomy(Autonomy::Autopilot);
        permissions.set_grant("cap-1", Grant::automatic());
        permissions.authority_mut().grant_command("cargo");
        permissions
    }

    #[test]
    fn nothing_runs_while_corescout_is_paused() {
        let mut permissions = permissive();
        permissions.pause();
        let outcome = Runtime::dry().execute(&capability(), &Context::default(), &permissions, 0);
        assert!(matches!(outcome, Outcome::Refused { .. }));
        assert!(!outcome.executed());
    }

    #[test]
    fn a_capability_whose_command_was_never_granted_does_not_run() {
        let mut permissions = permissive();
        permissions.authority_mut().grant_command("npm");
        // Rebuilt without cargo.
        let mut fresh = Permissions::new();
        fresh.set_autonomy(Autonomy::Autopilot);
        fresh.set_grant("cap-1", Grant::automatic());
        fresh.authority_mut().grant_command("npm");
        let outcome = Runtime::dry().execute(&capability(), &Context::default(), &fresh, 0);
        match outcome {
            Outcome::Refused { because } => assert!(because.contains("cargo"), "{because}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
        let _ = permissions;
    }

    #[test]
    fn an_invalid_definition_is_refused_before_the_permission_layer_sees_it() {
        // A definition that would be refused anyway should still fail as
        // invalid, so the message says what is actually wrong with it.
        let broken = Capability {
            steps: vec![Operation::run("cargo build && rm x", &[])],
            ..capability()
        };
        let outcome = Runtime::dry().execute(&broken, &Context::default(), &permissive(), 0);
        match outcome {
            Outcome::Invalid { because } => assert!(because.contains("shell"), "{because}"),
            other => panic!("expected invalid, got {other:?}"),
        }
    }

    #[test]
    fn suggest_mode_asks_rather_than_running() {
        let mut permissions = permissive();
        permissions.set_autonomy(Autonomy::Suggest);
        let outcome = Runtime::dry().execute(&capability(), &Context::default(), &permissions, 0);
        assert!(matches!(outcome, Outcome::NeedsApproval { .. }));
    }

    #[test]
    fn a_dry_run_decides_everything_and_starts_nothing() {
        let outcome = Runtime::dry().execute(&capability(), &Context::default(), &permissive(), 0);
        match outcome {
            Outcome::Ran {
                steps,
                verified,
                rolled_back,
                ..
            } => {
                assert_eq!(steps.len(), 2, "one step and one verification");
                assert_eq!(verified, Some(true));
                assert!(!rolled_back);
                assert!(steps.iter().all(|step| step.output.contains("dry run")));
            }
            other => panic!("expected a run, got {other:?}"),
        }
    }

    /// A program that exists on Windows, is not a shell, and whose exit code
    /// and output are predictable.
    ///
    /// `cmd` would be easier and is refused by validation, which is the point:
    /// these tests exercise the same path a generated capability would take.
    fn ping(args: &[&str], timeout_ms: u64, expect: Option<&str>) -> Operation {
        Operation::Run {
            command: "ping".into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: None,
            timeout_ms: Some(timeout_ms),
            expect: expect.map(str::to_string),
        }
    }

    #[test]
    fn a_real_run_of_a_real_program_reports_what_happened() {
        // Actually starts a process.
        if !cfg!(windows) {
            return;
        }
        let capability = Capability {
            steps: vec![ping(&["-n", "1", "127.0.0.1"], 20_000, None)],
            verification: ping(&["-n", "1", "127.0.0.1"], 20_000, Some("127.0.0.1")),
            ..capability()
        };
        let mut permissions = permissive();
        permissions.authority_mut().grant_command("ping");
        let outcome = Runtime::live().execute(&capability, &Context::default(), &permissions, 0);
        assert!(outcome.succeeded(), "{outcome:?}");
    }

    #[test]
    fn a_step_that_fails_stops_the_run_and_verification_is_not_attempted() {
        // A green verification after a failed step reads as a success, which
        // is the exact failure this product exists to catch.
        if !cfg!(windows) {
            return;
        }
        let capability = Capability {
            // A documentation address that never answers, so this exits
            // nonzero without depending on the network.
            steps: vec![ping(&["-n", "1", "-w", "200", "192.0.2.1"], 20_000, None)],
            verification: ping(&["-n", "1", "127.0.0.1"], 20_000, None),
            ..capability()
        };
        let mut permissions = permissive();
        permissions.authority_mut().grant_command("ping");
        match Runtime::live().execute(&capability, &Context::default(), &permissions, 0) {
            Outcome::Ran {
                steps, verified, ..
            } => {
                assert_eq!(verified, None, "an unknown, not a failed check");
                assert_eq!(steps.len(), 1, "verification was not attempted");
                assert_ne!(steps[0].exit_code, Some(0));
            }
            other => panic!("expected a run, got {other:?}"),
        }
    }

    #[test]
    fn steps_that_pass_and_a_verification_that_fails_is_a_failure() {
        if !cfg!(windows) {
            return;
        }
        let capability = Capability {
            steps: vec![ping(&["-n", "1", "127.0.0.1"], 20_000, None)],
            verification: ping(
                &["-n", "1", "127.0.0.1"],
                20_000,
                Some("the thing that would prove it worked"),
            ),
            rollback: vec![ping(&["-n", "1", "127.0.0.1"], 20_000, None)],
            reversible: true,
            ..capability()
        };
        let mut permissions = permissive();
        permissions.authority_mut().grant_command("ping");
        let outcome = Runtime::live().execute(&capability, &Context::default(), &permissions, 0);
        assert!(!outcome.succeeded(), "{outcome:?}");
        match outcome {
            Outcome::Ran {
                verified,
                rolled_back,
                ..
            } => {
                assert_eq!(verified, Some(false));
                assert!(rolled_back, "a failed verification runs the rollback");
            }
            other => panic!("expected a run, got {other:?}"),
        }
    }

    #[test]
    fn a_step_that_hangs_is_killed_rather_than_waited_on_forever() {
        if !cfg!(windows) {
            return;
        }
        let capability = Capability {
            // Sixty pings, a second apart: far longer than the ceiling below.
            steps: vec![ping(&["-n", "60", "127.0.0.1"], 400, None)],
            ..capability()
        };
        let mut permissions = permissive();
        permissions.authority_mut().grant_command("ping");
        let started = Instant::now();
        let outcome = Runtime::live().execute(&capability, &Context::default(), &permissions, 0);
        assert!(started.elapsed() < Duration::from_secs(20), "it hung");
        match outcome {
            Outcome::Ran { steps, .. } => {
                assert!(!steps[0].ok);
                assert!(
                    steps[0]
                        .error
                        .as_deref()
                        .unwrap_or_default()
                        .contains("did not finish"),
                    "{:?}",
                    steps[0].error
                );
            }
            other => panic!("expected a run, got {other:?}"),
        }
    }

    #[test]
    fn a_program_that_does_not_exist_is_a_failure_with_a_reason() {
        let capability = Capability {
            steps: vec![Operation::run("thisprogramdoesnotexistanywhere", &[])],
            ..capability()
        };
        let mut permissions = permissive();
        permissions
            .authority_mut()
            .grant_command("thisprogramdoesnotexistanywhere");
        match Runtime::live().execute(&capability, &Context::default(), &permissions, 0) {
            Outcome::Ran { steps, .. } => {
                assert!(!steps[0].ok);
                assert!(steps[0].error.is_some());
            }
            other => panic!("expected a run, got {other:?}"),
        }
    }

    #[test]
    fn a_long_output_is_trimmed_at_both_ends_and_says_so() {
        let long = "x".repeat(OUTPUT_LIMIT * 3);
        let trimmed = trim(&long);
        assert!(trimmed.len() < long.len());
        assert!(trimmed.contains("characters omitted"));
    }

    #[test]
    fn a_short_output_is_left_alone() {
        assert_eq!(
            trim("error: schema out of date"),
            "error: schema out of date"
        );
    }

    #[test]
    fn the_working_directory_is_the_workspace_root_plus_any_relative_part() {
        assert_eq!(
            working_directory(
                Some(Path::new("C:\\Projects\\app")),
                Some(Path::new("crates"))
            ),
            Some(PathBuf::from("C:\\Projects\\app\\crates"))
        );
        assert_eq!(
            working_directory(Some(Path::new("C:\\Projects\\app")), None),
            Some(PathBuf::from("C:\\Projects\\app"))
        );
        assert_eq!(working_directory(None, Some(Path::new("crates"))), None);
    }

    #[test]
    fn every_outcome_has_a_line_worth_showing() {
        let outcomes = [
            Outcome::Refused {
                because: "CoreScout is paused".into(),
            },
            Outcome::NeedsApproval {
                because: "waiting".into(),
            },
            Outcome::Invalid {
                because: "no steps".into(),
            },
            Outcome::Ran {
                steps: Vec::new(),
                verified: Some(true),
                rolled_back: false,
                duration_ms: 4000,
            },
        ];
        for outcome in outcomes {
            assert!(outcome.summary().len() > 5, "{outcome:?}");
        }
    }
}
