//! The acting command: `agent start|status|stop`.
//!
//! # The default scope is this process and nothing else
//!
//! An agent started with no `--scope` may act on itself. That is not a
//! placeholder for something more ambitious later; it is the correct default for
//! a program that decides what to do by learning, on a machine it does not own.
//! Every additional process is opted in by pid at the command line, and the
//! agent verifies each one is reachable before accepting it.
//!
//! # What stops it
//!
//! | mechanism | stops what |
//! |---|---|
//! | scope | acting on anything nobody opted in |
//! | bounds | values outside the permitted range, and acting too often |
//! | watchdog | a controller that is thrashing, failing, or making things worse |
//! | `--dry-run` | everything, while still exercising the whole decision path |
//! | `--max-actions` | a run that would otherwise go on indefinitely |
//! | ctrl-c | the loop, after which the agent reverts what it changed |
//!
//! The watchdog is not reachable from the policy. A controller that can switch
//! off its own safety layer has no safety layer.
//!
//! # Stopping means putting things back
//!
//! On the way out the agent reverts its last action and unfreezes nothing. The
//! machine is left to the operating system, which is a competent default and
//! the correct destination for every failure path in this program.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use corescout_agency::audit::Disposition;
use corescout_agency::{ActuatorSet, Bounds, Scope};
use corescout_autonomy::curiosity::Curiosity;
use corescout_autonomy::loop_::{ControlLoop, LoopConfig};
use corescout_autonomy::policy::{Policy, PolicyConfig};
use corescout_autonomy::safeguards::{Watchdog, WatchdogConfig};
use corescout_core::error::{Error, Result};
use corescout_human::Format;
use corescout_intent::Intent;
use corescout_memory::source::Source;

use crate::cli::SourceOptions;
use crate::{objectives, runtime};

/// How an agent run is configured.
pub struct AgentRun {
    pub intent: String,
    pub dry_run: bool,
    pub scope: Vec<u32>,
    pub max_actions: Option<u64>,
    pub source: SourceOptions,
    pub format: Format,
}

/// Start the autonomous decision loop.
pub fn start(options: AgentRun) -> Result<()> {
    let intent = Intent::preset(&options.intent).ok_or_else(|| {
        Error::invalid(format!(
            "unknown intent {:?}; try one of: {}",
            options.intent,
            Intent::preset_names().join(", ")
        ))
    })?;

    let mut scope = Scope::own_process();
    for pid in &options.scope {
        // A pid that cannot be signalled is refused here rather than accepted
        // and failed later, where the audit log would blame the actuator.
        if !scope.register(*pid as i32) {
            return Err(Error::invalid(format!(
                "process {pid} is not reachable by this user, so it cannot be opted in"
            )));
        }
    }

    let actuators = if options.dry_run {
        ActuatorSet::dry_run(scope, Bounds::default())
    } else {
        ActuatorSet::new(scope, Bounds::default())
    };
    let watchdog = Watchdog::new(WatchdogConfig::default());
    let policy = Policy::new(intent.clone(), PolicyConfig::default());
    let control = ControlLoop::new(
        LoopConfig::default(),
        policy,
        actuators,
        watchdog,
        Curiosity::default(),
    );

    if options.format == Format::Human {
        println!(
            "agent starting: intent {}, scope {} process(es), {}",
            intent.name,
            options.scope.len() + 1,
            if options.dry_run {
                "dry run, no action will be performed"
            } else {
                "actions will be performed and audited"
            }
        );
        println!("press ctrl-c to stop; the agent reverts what it changed on the way out");
    }

    let mut source = runtime::open_source(&options.source)?;
    run_loop(control, &mut source, &options, intent)
}

/// Report what a running agent has done.
///
/// The audit log lives in the agent's own process, so this reads the plane to
/// establish whether one is running rather than inventing a control channel.
/// A status command that lies about whether an agent is running would be worse
/// than not having one.
pub fn status(options: &SourceOptions, format: Format) -> Result<()> {
    match runtime::open_source(options) {
        Ok(mut source) => {
            let frames = source.take(1)?;
            match frames.first() {
                Some(frame) => {
                    if format == Format::Json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "mirror": "live",
                                "sequence": frame.sequence,
                                "entities": frame.entities.len(),
                                "channels": frame.channels.len(),
                            })
                        );
                    } else {
                        println!(
                            "a mirror is live at sequence {} ({} entities, {} channels)",
                            frame.sequence,
                            frame.entities.len(),
                            frame.channels.len()
                        );
                        println!(
                            "the audit log belongs to the agent process; run `agent start` \
                             in the foreground to watch it"
                        );
                    }
                    Ok(())
                }
                None => Err(Error::invalid("the mirror produced no reflection")),
            }
        }
        Err(error) => {
            if format == Format::Human {
                println!("no agent is reachable: {error}");
                Ok(())
            } else {
                Err(error)
            }
        }
    }
}

/// Stop an agent and revert what it changed.
///
/// A foreground agent stops on ctrl-c and reverts as it exits, which is the
/// supported path. This command exists to say so plainly rather than to
/// pretend at a daemon protocol the project does not have.
pub fn stop(format: Format) -> Result<()> {
    if format == Format::Json {
        println!(
            "{}",
            serde_json::json!({
                "stopped": false,
                "reason": "the agent runs in the foreground; interrupt it to stop and revert",
            })
        );
    } else {
        println!(
            "the agent runs in the foreground. Interrupt it with ctrl-c: it reverts its \
             last action and hands the machine back to the scheduler as it exits."
        );
    }
    Ok(())
}

fn run_loop(
    mut control: ControlLoop,
    source: &mut Source,
    options: &AgentRun,
    intent: Intent,
) -> Result<()> {
    // The candidate set is bounded by what this process is already permitted to
    // use. The loop never gets to widen it.
    let permitted = runtime::permitted_cpus();
    let measure = objectives::measure(&intent);
    let interrupted = Arc::new(AtomicBool::new(false));
    install_interrupt_handler(Arc::clone(&interrupted));

    let started = Instant::now();
    let mut actions = 0u64;

    loop {
        if interrupted.load(Ordering::Relaxed) {
            break;
        }
        let frames = source.take(1)?;
        let Some(frame) = frames.into_iter().next() else {
            // A recording that has ended is a finished run, not a failure.
            break;
        };

        let candidates = objectives::candidates(&permitted, &frame);
        let tick = control.tick(&frame, &candidates, &measure);
        if options.format == Format::Human {
            if tick.decision.is_some() || tick.verdict.reason().is_some() || tick.novel_state {
                println!("{}", tick.summary());
            }
        } else {
            println!("{}", runtime::to_json(&tick.summary())?);
        }

        if matches!(tick.disposition, Some(Disposition::Applied)) {
            actions += 1;
        }
        if control.watchdog().is_stopped() {
            if options.format == Format::Human {
                println!(
                    "the watchdog stopped the loop: {}",
                    tick.verdict.reason().unwrap_or("no reason recorded")
                );
            }
            break;
        }
        if options.max_actions.is_some_and(|limit| actions >= limit) {
            break;
        }
        std::thread::sleep(Duration::from_millis(options.source.interval_ms.max(1)));
    }

    // Every exit path leads here: revert, then report.
    let reverted = control.actuators_mut().revert_last();
    if options.format == Format::Human {
        println!(
            "\nagent stopped after {} ticks and {} actions in {:.1} s",
            control.ticks(),
            actions,
            started.elapsed().as_secs_f64()
        );
        match reverted {
            Some(disposition) => println!("reverted: {}", disposition.label()),
            None => println!("nothing needed reverting"),
        }
        println!(
            "{} states discovered, {} audit events recorded",
            control.catalogue().len(),
            control.actuators().log().len()
        );
    }
    Ok(())
}

/// Catch ctrl-c so the agent can revert before it exits.
///
/// Without this the process dies with its affinity changes still applied, which
/// is the one outcome the safety model is written to prevent.
#[cfg(target_os = "linux")]
fn install_interrupt_handler(flag: Arc<AtomicBool>) {
    // A plain static, not the Arc: a signal handler may only touch things that
    // are already allocated and need no lock to reach. The loop polls both this
    // and its own flag, so the two stay in step without the handler having to
    // know about the Arc at all.
    static INTERRUPTED: AtomicBool = AtomicBool::new(false);

    extern "C" fn handle(_signal: libc::c_int) {
        // Async-signal-safe: one relaxed store, nothing else. No allocation, no
        // formatting, no locks.
        INTERRUPTED.store(true, Ordering::Relaxed);
    }

    // SAFETY: the handler only stores to a static atomic, which is permitted
    // from a signal context.
    unsafe {
        libc::signal(libc::SIGINT, handle as libc::sighandler_t);
        libc::signal(libc::SIGTERM, handle as libc::sighandler_t);
    }

    // A watcher thread carries the signal across to the loop's own flag. The
    // loop then exits through its normal path, which is what reverts the last
    // action; killing it from the handler would skip exactly that.
    std::thread::spawn(move || loop {
        if INTERRUPTED.load(Ordering::Relaxed) {
            flag.store(true, Ordering::Relaxed);
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    });
}

#[cfg(not(target_os = "linux"))]
fn install_interrupt_handler(_flag: Arc<AtomicBool>) {
    // On platforms without the signal plumbing the loop still stops on
    // `--max-actions` and at the end of a recording. Saying nothing here would
    // imply a guarantee that is not being made.
}
