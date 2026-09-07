//! The two ways CoreScout actually sees work happen.
//!
//! # `corescout hook`
//!
//! Reads one tool call from an agent's hook on stdin and records it. It must
//! be fast and it must never fail visibly: it runs inside somebody's agent
//! loop, several times a minute, and a hook that is slow or noisy is one they
//! will remove within the hour. So it exits zero whatever happens, prints
//! nothing on the happy path, and gives up rather than waiting if CoreScout is
//! not running.
//!
//! # `corescout run`
//!
//! Owns the process, and therefore owns its exit code. A hook watches and can
//! only sometimes tell what happened; this runs the thing and knows.
//!
//! It is also where CoreScout can act. When the machine has cores of more than
//! one kind, `--placement` confines the child to some of them, and asking
//! CoreScout which is what turns the whole research layer into something with
//! a point: real work, placed deliberately, measured exactly, and sometimes
//! placed by a coin so the difference can be attributed.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use corescout_core::error::{Error, Result};
use corescout_product_api::hook::{self, HookEvent};
use corescout_product_api::Client;
use serde_json::json;

/// How long a hook may spend before it gives up and lets the agent get on.
///
/// A hook is not allowed to be the reason somebody's build felt slow.
const HOOK_BUDGET: Duration = Duration::from_millis(1500);

/// Record one tool call reported by an agent's hook.
///
/// Always returns success. A hook that reports a failure makes the agent
/// display an error about a tool the user did not ask for and cannot fix from
/// there, and a hook that CoreScout can break is a hook nobody keeps.
pub fn hook() -> std::process::ExitCode {
    // A hook that fails silently is a hook nobody can diagnose, and the first
    // question anybody asks is "is it even running". `CORESCOUT_HOOK_DEBUG=1`
    // makes it say, on stderr, where an agent will show it.
    let loud = std::env::var_os("CORESCOUT_HOOK_DEBUG").is_some();
    let say = |what: &str| {
        if loud {
            eprintln!("corescout hook: {what}");
        }
    };

    let mut input = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut input) {
        say(&format!("could not read stdin: {error}"));
        return std::process::ExitCode::SUCCESS;
    }
    let event = match serde_json::from_str::<HookEvent>(&input) {
        Ok(event) => event,
        Err(error) => {
            say(&format!("that is not a hook payload: {error}"));
            return std::process::ExitCode::SUCCESS;
        }
    };
    let Some(raw) = hook::to_raw(&event, 0) else {
        say(&format!(
            "nothing worth recording in a {} call",
            event.tool_name
        ));
        return std::process::ExitCode::SUCCESS;
    };

    let started = Instant::now();
    match Client::connect() {
        Ok(client) => {
            if started.elapsed() >= HOOK_BUDGET {
                say("gave up rather than hold the agent up");
                return std::process::ExitCode::SUCCESS;
            }
            match client.call("observe", serde_json::to_value(&raw).unwrap_or(json!({}))) {
                Ok(_) => say(&format!("recorded {}", raw.name)),
                Err(error) => say(&format!("CoreScout refused it: {error}")),
            }
        }
        Err(error) => say(&format!("CoreScout is not running: {error}")),
    }
    std::process::ExitCode::SUCCESS
}

/// What a run should be confined to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Wherever the operating system puts it. The honest default.
    Anywhere,
    /// The performance cores, on a machine that has more than one kind.
    Fast,
    /// The efficiency cores.
    Efficient,
    /// Whatever CoreScout currently believes, which is sometimes a coin flip.
    Ask,
}

impl Placement {
    /// Parse the `--placement` value.
    pub fn parse(name: &str) -> Option<Placement> {
        match name {
            "any" | "anywhere" | "default" => Some(Placement::Anywhere),
            "fast" | "performance" | "p" => Some(Placement::Fast),
            "efficient" | "efficiency" | "e" => Some(Placement::Efficient),
            "ask" | "auto" => Some(Placement::Ask),
            _ => None,
        }
    }

    /// The name CoreScout records this under, so two runs on the same
    /// placement are comparable.
    pub fn label(self) -> &'static str {
        match self {
            Placement::Anywhere => "anywhere",
            Placement::Fast => "performance cores",
            Placement::Efficient => "efficiency cores",
            Placement::Ask => "chosen by CoreScout",
        }
    }
}

/// Run a command, measure it exactly, and tell CoreScout what happened.
///
/// Returns the child's own exit code, so this can be dropped in front of any
/// command without changing what the caller sees. That property is the whole
/// reason anybody would use it, and there is a test for it.
pub fn run(args: &[String], quiet: bool) -> Result<std::process::ExitCode> {
    let placement = args
        .iter()
        .position(|a| a == "--placement")
        .and_then(|at| args.get(at + 1))
        .map(|value| {
            Placement::parse(value).ok_or_else(|| {
                Error::invalid(format!(
                    "{value:?} is not a placement; choose any, fast, efficient or ask"
                ))
            })
        })
        .transpose()?
        .unwrap_or(Placement::Anywhere);

    let separator = args
        .iter()
        .position(|a| a == "--")
        .ok_or_else(|| Error::invalid("say which command to run, after --".to_string()))?;
    let command: Vec<String> = args[separator + 1..].to_vec();
    let (program, rest) = command
        .split_first()
        .ok_or_else(|| Error::invalid("there is no command after --".to_string()))?;

    let session = std::env::var("CORESCOUT_SESSION").unwrap_or_else(|_| "run".into());
    let workspace = std::env::current_dir().ok();
    let client = Client::connect().ok();

    // Ask before doing, so the answer can be a coin flip and the trial is
    // recorded before the outcome is known. Asking afterwards would be asking
    // a question whose answer could no longer change anything.
    let line = command.join(" ");
    let advice = client.as_ref().and_then(|client| {
        client
            .call(
                "advise",
                json!({
                    "session": session,
                    "operation": line,
                    "workspace": workspace.as_ref().map(|p| p.display().to_string()),
                }),
            )
            .ok()
    });
    if !quiet {
        if let Some(advice) = &advice {
            if advice["has_advice"] == json!(true) {
                eprintln!("corescout: {}", advice["because"].as_str().unwrap_or(""));
                for step in advice["steps"].as_array().cloned().unwrap_or_default() {
                    eprintln!("           first: {}", step.as_str().unwrap_or(""));
                }
            }
        }
    }

    let cpus = confine(placement)?;
    let started = Instant::now();

    let mut builder = Command::new(program);
    builder
        .args(rest)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let status = match &cpus {
        Some(cpus) => {
            let platform = corescout_substrate::platform::detect();
            let mut child = platform.spawn_with_affinity(&mut builder, cpus)?;
            child
                .wait()
                .map_err(|source| Error::io(program.clone(), source))?
        }
        None => builder
            .status()
            .map_err(|source| Error::io(program.clone(), source))?,
    };
    let elapsed = started.elapsed();
    let code = status.code().unwrap_or(-1);

    if let Some(client) = &client {
        // The exit code is not a claim, it is the process status. This is the
        // one path in the product that can honestly say it verified something,
        // because it is the one that owned the process.
        let _ = client.call(
            "observe",
            json!({
                "session": session,
                "name": line,
                "workspace": workspace.as_ref().map(|p| p.display().to_string()),
                "duration_ms": elapsed.as_millis() as u64,
                "exit_code": code,
                "reported": if code == 0 { "success" } else { "failure" },
                "detail": if code == 0 { String::new() } else { format!("exit code {code}") },
                "verified": if code == 0 { "confirmed" } else { "contradicted" },
                "verification_detail": if code == 0 {
                    String::new()
                } else {
                    format!("the process exited {code}")
                },
            }),
        );
    }

    if !quiet {
        eprintln!(
            "corescout: {} in {:.1}s on {}{}",
            if code == 0 { "finished" } else { "failed" },
            elapsed.as_secs_f64(),
            placement.label(),
            if client.is_none() {
                " (CoreScout is not running, so nothing was recorded)"
            } else {
                ""
            }
        );
    }

    // The child's own code, so this can sit in front of anything without
    // changing what the caller sees.
    Ok(std::process::ExitCode::from(code.clamp(0, 255) as u8))
}

/// Which CPUs a placement means on this machine.
///
/// `None` means "do not confine it", which is what every placement collapses
/// to on a machine whose cores are all the same. Pinning work on a uniform
/// part is a change with no upside and a real downside, so it is not made.
fn confine(placement: Placement) -> Result<Option<corescout_core::CpuSet>> {
    let wanted = match placement {
        Placement::Anywhere | Placement::Ask => return Ok(None),
        Placement::Fast => corescout_substrate::topology::CoreType::Performance,
        Placement::Efficient => corescout_substrate::topology::CoreType::Efficiency,
    };
    let platform = corescout_substrate::platform::detect();
    let topology = platform.discover_topology()?;
    if !topology.hybrid {
        return Ok(None);
    }
    let permitted = platform.process_affinity().unwrap_or_default();
    let cpus: corescout_core::CpuSet = topology
        .logical_cpus
        .iter()
        .filter(|cpu| cpu.core_type == wanted && cpu.online && permitted.contains(cpu.id))
        .map(|cpu| cpu.id)
        .collect();
    if cpus.is_empty() {
        // Asked for cores this process may not use. Running unconfined is the
        // right answer; refusing would turn a placement preference into a
        // reason somebody's build did not run at all.
        return Ok(None);
    }
    Ok(Some(cpus))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_placement_name_a_person_would_type_is_understood() {
        for name in ["any", "anywhere", "default"] {
            assert_eq!(Placement::parse(name), Some(Placement::Anywhere));
        }
        for name in ["fast", "performance", "p"] {
            assert_eq!(Placement::parse(name), Some(Placement::Fast));
        }
        for name in ["efficient", "efficiency", "e"] {
            assert_eq!(Placement::parse(name), Some(Placement::Efficient));
        }
        assert_eq!(Placement::parse("ask"), Some(Placement::Ask));
        assert_eq!(Placement::parse("sideways"), None);
    }

    #[test]
    fn every_placement_has_a_name_it_is_recorded_under() {
        // Two runs on the same placement have to be comparable, so the label
        // is the identity of the arm rather than decoration.
        let labels: Vec<&str> = [
            Placement::Anywhere,
            Placement::Fast,
            Placement::Efficient,
            Placement::Ask,
        ]
        .iter()
        .map(|p| p.label())
        .collect();
        let mut sorted = labels.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), labels.len());
    }

    #[test]
    fn a_run_with_no_command_says_so_rather_than_doing_nothing_quietly() {
        let error = run(&["--placement".into(), "fast".into()], true)
            .expect_err("should fail")
            .to_string();
        assert!(error.contains("after --"), "{error}");

        let error = run(&["--".into()], true)
            .expect_err("should fail")
            .to_string();
        assert!(error.contains("no command"), "{error}");
    }

    #[test]
    fn an_unknown_placement_lists_the_real_ones() {
        let error = run(
            &[
                "--placement".into(),
                "sideways".into(),
                "--".into(),
                "x".into(),
            ],
            true,
        )
        .expect_err("should fail")
        .to_string();
        assert!(error.contains("fast"), "{error}");
        assert!(error.contains("efficient"), "{error}");
    }

    #[test]
    fn a_run_returns_the_exit_code_of_what_it_ran() {
        // The property that makes this safe to put in front of anything. If it
        // ever swallows a failure, every script wrapping it starts lying.
        if !cfg!(windows) {
            return;
        }
        let ok = run(
            &[
                "--".into(),
                "ping".into(),
                "-n".into(),
                "1".into(),
                "127.0.0.1".into(),
            ],
            true,
        )
        .expect("it ran");
        assert_eq!(
            format!("{ok:?}"),
            format!("{:?}", std::process::ExitCode::SUCCESS)
        );

        let bad = run(
            &[
                "--".into(),
                "ping".into(),
                "-n".into(),
                "1".into(),
                "-w".into(),
                "200".into(),
                "192.0.2.1".into(),
            ],
            true,
        )
        .expect("it ran");
        assert_ne!(
            format!("{bad:?}"),
            format!("{:?}", std::process::ExitCode::SUCCESS),
            "a failure must survive the wrapper"
        );
    }

    #[test]
    fn a_command_that_does_not_exist_is_an_error_and_not_a_panic() {
        assert!(run(&["--".into(), "thisdoesnotexistanywhere".into()], true).is_err());
    }

    #[test]
    fn asking_for_a_placement_on_a_uniform_machine_confines_nothing() {
        // Pinning work on a part whose cores are all the same is a change with
        // no upside and a real downside. Whatever this machine is, the call
        // must not fail.
        for placement in [Placement::Fast, Placement::Efficient, Placement::Anywhere] {
            let confined = confine(placement).expect("it should decide, not fail");
            if let Some(cpus) = confined {
                assert!(!cpus.is_empty(), "an empty confinement would run nowhere");
            }
        }
    }

    #[test]
    fn the_hook_budget_is_short_enough_not_to_be_noticed() {
        // A hook is not allowed to be the reason somebody's build felt slow.
        assert!(HOOK_BUDGET <= Duration::from_secs(2));
    }
}
