//! `mirror-agent`: the decision loop, as a standalone program.
//!
//! # Why it is a separate program from the daemon
//!
//! The daemon writes the mirror. This one acts on the machine. Keeping them in
//! separate processes means the agent holds no writable mapping of the plane and
//! could not alter its own reflection if it tried: it reads the same read-only
//! mapping every other consumer gets.
//!
//! An intelligent controller may change the physical system. It may not change
//! its reflection of that system. Process separation is how that survives
//! contact with future code.
//!
//! # Defaults are the conservative ones
//!
//! Dry run is **on** unless `--act` is passed. The scope is this process alone
//! unless processes are opted in by pid. The watchdog is the default one, and it
//! is not reachable from the policy. Ctrl-c reverts.
//!
//! A run with no `--act` still exercises the entire path: scope check, bounds,
//! rate limit, prediction, decision, audit entry. Only the final syscall is
//! skipped. That is the intended way to evaluate a policy on a machine you do
//! not own.

use std::process::ExitCode;
use std::time::Duration;

use corescout_agency::audit::Disposition;
use corescout_agency::{ActuatorSet, Bounds, Scope};
use corescout_autonomy::curiosity::Curiosity;
use corescout_autonomy::loop_::{ControlLoop, LoopConfig};
use corescout_autonomy::policy::{Policy, PolicyConfig};
use corescout_autonomy::safeguards::{Watchdog, WatchdogConfig};
use corescout_core::error::{Error, Result};
use corescout_core::CpuSet;
use corescout_intent::{Achievement, Intent, Objective};
use corescout_memory::source::Source;
use corescout_mirror::plane::default_path;
use corescout_mirror::MirrorSnapshot;

const USAGE: &str = "\
mirror-agent - decide and act on this machine, under an intent

USAGE:
    mirror-agent [OPTIONS]

SOURCE (one of):
    --plane <PATH>     Read a live plane (default: the standard location)
    --replay <PATH>    Drive the loop from a recorded trace

OPTIONS:
    --intent <NAME>    latency-critical | throughput | efficient | balanced
    --act              Actually perform actions. Without this, nothing is changed.
    --scope <PID>      Also permit acting on this process (repeatable)
    --ticks <N>        Stop after N ticks
    --max-actions <N>  Stop after N actions
    --interval <MS>    Milliseconds between ticks on a live plane (default 100)
    -h, --help         Show this help

Without --act nothing is changed, and the whole decision path still runs. The
watchdog can stop the loop and the loop cannot stop the watchdog. On exit the
agent reverts its last action.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mirror-agent: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }

    let mut intent_name = "balanced".to_string();
    let mut act = false;
    let mut scope_pids: Vec<i32> = Vec::new();
    let mut ticks_limit: Option<u64> = None;
    let mut action_limit: Option<u64> = None;
    let mut interval_ms = 100u64;
    let mut plane: Option<String> = None;
    let mut replay: Option<String> = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--act" => act = true,
            "--intent" | "--scope" | "--ticks" | "--max-actions" | "--interval" | "--plane"
            | "--replay" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| Error::invalid(format!("{} needs a value", args[index])))?;
                match args[index].as_str() {
                    "--intent" => intent_name = value.clone(),
                    "--scope" => scope_pids.push(
                        value
                            .parse()
                            .map_err(|_| Error::invalid("--scope expects a process id"))?,
                    ),
                    "--ticks" => {
                        ticks_limit = Some(
                            value
                                .parse()
                                .map_err(|_| Error::invalid("--ticks expects a number"))?,
                        )
                    }
                    "--max-actions" => {
                        action_limit = Some(
                            value
                                .parse()
                                .map_err(|_| Error::invalid("--max-actions expects a number"))?,
                        )
                    }
                    "--interval" => {
                        interval_ms = value
                            .parse()
                            .map_err(|_| Error::invalid("--interval expects a number"))?
                    }
                    "--plane" => plane = Some(value.clone()),
                    _ => replay = Some(value.clone()),
                }
                index += 1;
            }
            other => {
                return Err(Error::invalid(format!(
                    "unexpected argument {other:?}; run mirror-agent --help"
                )))
            }
        }
        index += 1;
    }

    let intent = Intent::preset(&intent_name).ok_or_else(|| {
        Error::invalid(format!(
            "unknown intent {intent_name:?}; try one of: {}",
            Intent::preset_names().join(", ")
        ))
    })?;

    let mut scope = Scope::own_process();
    for pid in &scope_pids {
        if !scope.register(*pid) {
            return Err(Error::invalid(format!(
                "process {pid} is not reachable by this user, so it cannot be opted in"
            )));
        }
    }

    let actuators = if act {
        ActuatorSet::new(scope, Bounds::default())
    } else {
        ActuatorSet::dry_run(scope, Bounds::default())
    };
    let mut control = ControlLoop::new(
        LoopConfig::default(),
        Policy::new(intent.clone(), PolicyConfig::default()),
        actuators,
        Watchdog::new(WatchdogConfig::default()),
        Curiosity::default(),
    );

    let interval = Duration::from_millis(interval_ms.max(1));
    let mut source = match (&replay, &plane) {
        (Some(path), _) => Source::replay(std::path::Path::new(path))?,
        (None, Some(path)) => Source::live(std::path::Path::new(path), interval)?,
        (None, None) => Source::live(&default_path(), interval)?,
    };

    let permitted = permitted_cpus();
    let objectives = intent.objectives();
    let measure = move |snapshot: &MirrorSnapshot, _cells: &[(usize, usize, f64)]| {
        let mut achievement = Achievement::new();
        for objective in &objectives {
            if let Some(value) = read_objective(snapshot, *objective) {
                achievement = achievement.with(*objective, value);
            }
            // An objective this machine cannot measure stays absent, and an
            // absent objective counts as unsatisfied.
        }
        achievement
    };

    eprintln!(
        "intent {}, {} permitted CPUs, {}",
        intent.name,
        permitted.len(),
        if act {
            "acting for real; every action is audited and reverted on exit"
        } else {
            "dry run; the whole path runs and nothing is changed"
        }
    );

    let mut actions = 0u64;
    loop {
        let frames = source.take(1)?;
        let Some(frame) = frames.into_iter().next() else {
            break;
        };
        let candidates = candidates(&permitted, &frame);
        let tick = control.tick(&frame, &candidates, &measure);

        if tick.decision.is_some() || tick.verdict.reason().is_some() {
            println!("{}", tick.summary());
        }
        if matches!(tick.disposition, Some(Disposition::Applied)) {
            actions += 1;
        }
        if control.watchdog().is_stopped() {
            println!(
                "the watchdog stopped the loop: {}",
                tick.verdict.reason().unwrap_or("no reason recorded")
            );
            break;
        }
        if ticks_limit.is_some_and(|limit| control.ticks() >= limit)
            || action_limit.is_some_and(|limit| actions >= limit)
        {
            break;
        }
    }

    // Every exit path reverts. A process that dies with its changes applied is
    // the one outcome the safety model exists to prevent.
    let reverted = control.actuators_mut().revert_last();
    println!(
        "\n{} ticks, {} actions, {} states discovered, {} audit events",
        control.ticks(),
        actions,
        control.catalogue().len(),
        control.actuators().log().len()
    );
    match reverted {
        Some(disposition) => println!("reverted: {}", disposition.label()),
        None => println!("nothing needed reverting"),
    }
    Ok(())
}

/// The candidate actions, bounded by what this process may already use.
fn candidates(
    permitted: &CpuSet,
    snapshot: &MirrorSnapshot,
) -> Vec<(corescout_agency::ActionKind, Option<usize>, String)> {
    use corescout_agency::{ActionKind, Target};
    let mut actions = vec![(ActionKind::Hold, None, "hold".to_string())];
    for cpu in permitted.iter() {
        let row = snapshot
            .entities
            .iter()
            .position(|entity| entity.key == format!("cpu/{cpu}"));
        actions.push((
            ActionKind::SetAffinity {
                target: Target::CurrentProcess,
                cpus: [cpu].into_iter().collect(),
            },
            row,
            format!("affinity/cpu{cpu}"),
        ));
    }
    actions
}

fn permitted_cpus() -> CpuSet {
    // Falling back to the empty set is deliberate: a controller that cannot
    // establish what it may touch should consider nothing, not everything.
    corescout_substrate::platform::detect()
        .process_affinity()
        .unwrap_or_else(|_| CpuSet::new())
}

fn read_objective(snapshot: &MirrorSnapshot, objective: Objective) -> Option<f64> {
    let keys: &[&str] = match objective {
        Objective::Latency => &["cpu.sched.wait_ns", "cpu.sched.latency_ns"],
        Objective::Jitter => &["cpu.sched.wait_ns", "cpu.irq.count"],
        Objective::Throughput => &["cpu.frequency.current", "cpu.time.user"],
        Objective::Energy => &["package.power.uw", "package.energy.uj"],
        Objective::Migration => &["cpu.sched.migrations", "cpu.sched.switches"],
    };
    for key in keys {
        if let Some(channel) = snapshot.channel(key) {
            let values: Vec<f64> = (0..snapshot.entities.len())
                .filter_map(|row| snapshot.value(row as u32, channel))
                .filter(|value| value.is_finite())
                .collect();
            if !values.is_empty() {
                return Some(values.iter().sum::<f64>() / values.len() as f64);
            }
        }
    }
    None
}
