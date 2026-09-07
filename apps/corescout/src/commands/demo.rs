//! `corescout demo self`: the whole architecture, in one run, on this machine.
//!
//! # What it does, in order
//!
//! 1. Observes the real machine and publishes a real self-state plane.
//! 2. Remembers the reflections.
//! 3. Consumes them the way a program with no hardware access would.
//! 4. Discovers structure without being told what anything is called.
//! 5. Learns to predict its own next state, and scores the predictions.
//! 6. Forms a hypothesis about which parts of the machine are "itself".
//! 7. Imagines what an action would do.
//! 8. Decides, under an intent, with the watchdog watching.
//! 9. Explains afterwards what it did and why, from what it recorded at the
//!    time rather than from a story assembled at the end.
//!
//! # It is allowed to be unimpressive
//!
//! On an idle machine with few sensors, the honest output is a handful of
//! states, mediocre prediction skill and no action worth taking. The demo
//! prints that. A demonstration that always produces an exciting result is not
//! demonstrating the system, it is demonstrating the demo.

use std::time::Duration;

use corescout_agency::{ActuatorSet, Bounds, Scope};
use corescout_autonomy::curiosity::Curiosity;
use corescout_autonomy::loop_::{ControlLoop, LoopConfig};
use corescout_autonomy::policy::{Policy, PolicyConfig};
use corescout_autonomy::safeguards::{Watchdog, WatchdogConfig};
use corescout_core::error::{Error, Result};
use corescout_human::Format;
use corescout_identity::{describe, Evidence};
use corescout_intent::Intent;
use corescout_memory::source::Source;
use corescout_mirror::MirrorSnapshot;

use crate::commands::consume;
use crate::{objectives, runtime};

/// How many reflections the demo gathers before it starts reasoning.
const FRAMES: usize = 240;
/// Gap between observations. Short enough that the demo finishes, long enough
/// that the mirror is not the dominant load on the machine it is observing.
const INTERVAL: Duration = Duration::from_millis(25);

pub fn run(what: &str, format: Format) -> Result<()> {
    if what != "self" {
        return Err(Error::invalid(format!(
            "unknown demo {what:?}; the only demo is `corescout demo self`"
        )));
    }

    step(1, "observing this machine", format);
    let frames = observe(format)?;

    step(
        2,
        "remembering, then reading it back as a consumer would",
        format,
    );
    let mut source = Source::from_memory(frames.clone());
    let replayed = source.take(frames.len())?;
    if format == Format::Human {
        println!(
            "  {} reflections in, {} out, over {:.1} s of machine time",
            frames.len(),
            replayed.len(),
            span_seconds(&replayed)
        );
        println!("  the code below cannot tell this from a live plane, and is not told");
    }

    step(
        3,
        "finding structure, without being told what anything is",
        format,
    );
    let learned = consume::learn_from(&replayed)?;
    if format == Format::Human {
        if learned.catalogue.is_empty() {
            println!("  no recurring states were distinguishable");
        } else {
            println!(
                "  {} states discovered over {} observations:",
                learned.catalogue.len(),
                learned.catalogue.observations()
            );
            for state in learned.catalogue.ranked().into_iter().take(5) {
                println!(
                    "    {}  entered {} times, mean dwell {:.0} ms",
                    state.id,
                    state.entries,
                    state.mean_dwell_ns() / 1e6
                );
            }
            println!("  those names are the machine's own; nothing here named them");
        }
    }

    step(
        4,
        "predicting its own next state, and scoring itself",
        format,
    );
    if format == Format::Human {
        println!(
            "  {} predictions scored, mean skill {:+.3} against assuming no change",
            learned.scored, learned.skill
        );
        println!(
            "  {}",
            if learned.skill > 0.05 {
                "the model beats the assumption that nothing changes"
            } else if learned.skill > -0.05 {
                "the model is about as good as assuming nothing changes, which on an \
                 idle machine is the correct answer"
            } else {
                "the model is worse than assuming nothing changes"
            }
        );
    }

    step(
        5,
        "deciding, under an intent, with the watchdog watching",
        format,
    );
    let ticks = decide(&replayed, format)?;

    step(6, "explaining, from what it recorded at the time", format);
    let latest = replayed.last().ok_or_else(|| {
        Error::invalid("the demo gathered no reflections; is this a supported platform?")
    })?;
    let evidence = Evidence {
        frames: replayed.len(),
        scored_predictions: learned.scored,
        model_skill: (learned.scored > 0).then_some(learned.skill),
        actions_taken: ticks.actions,
        responsive_entities: ticks.responsive,
        largest_variable_group: None,
    };
    let description = describe(Some(latest), Some(&learned.catalogue), &[], &evidence);

    if format == Format::Json {
        println!(
            "{}",
            runtime::to_json(&serde_json::json!({
                "frames": replayed.len(),
                "states": learned.catalogue.len(),
                "scored": learned.scored,
                "skill": learned.skill,
                "ticks": ticks.count,
                "actions": ticks.actions,
                "description": description,
            }))?
        );
    } else {
        print!("{}", description.render());
        println!();
        println!(
            "Everything above was derived from {} reflections of this machine. \
             Nothing was asserted that a person with the recording could not check.",
            replayed.len()
        );
    }
    Ok(())
}

/// What the decision phase did.
struct Ticks {
    count: u64,
    actions: u64,
    responsive: usize,
}

fn observe(format: Format) -> Result<Vec<MirrorSnapshot>> {
    let mut reflector = runtime::reflector()?;
    reflector.observe();
    let first = reflector.snapshot();
    if format == Format::Human {
        println!(
            "  {} entities, {} channels, {} relations",
            first.entities.len(),
            first.channels.len(),
            first.relations.len()
        );
        let gaps = first.privilege_gaps();
        if gaps > 0 {
            // Stated up front: the demo's ceiling is set by what this process
            // is allowed to see, and pretending otherwise would misrepresent
            // every number that follows.
            println!(
                "  {gaps} readings need privilege this process does not have; \
                 they are recorded as gaps, not guessed at"
            );
        }
        println!(
            "  observation costs {} us per pass",
            reflector.last_observation_cost_ns() / 1000
        );
    }

    let mut frames = Vec::with_capacity(FRAMES);
    frames.push(first);
    for _ in 1..FRAMES {
        std::thread::sleep(INTERVAL);
        reflector.observe();
        frames.push(reflector.snapshot());
    }
    Ok(frames)
}

fn decide(frames: &[MirrorSnapshot], format: Format) -> Result<Ticks> {
    // Dry run, always. The demo exercises the entire decision path including
    // scope, bounds, rate limiting and the audit trail, and skips only the
    // final syscall. A demo that repinned the reader's threads to prove a point
    // would be a bad neighbour.
    let actuators = ActuatorSet::dry_run(Scope::own_process(), Bounds::default());
    let policy = Policy::new(
        Intent::preset("latency-critical").expect("a preset intent"),
        PolicyConfig::default(),
    );
    let mut control = ControlLoop::new(
        LoopConfig::default(),
        policy,
        actuators,
        Watchdog::new(WatchdogConfig::default()),
        Curiosity::default(),
    );

    let intent = Intent::preset("latency-critical").expect("a preset intent");
    let measure = objectives::measure(&intent);
    let permitted = runtime::permitted_cpus();

    let mut actions = 0u64;
    let mut decided = 0u64;
    for frame in frames {
        let candidates = objectives::candidates(&permitted, frame);
        let tick = control.tick(frame, &candidates, &measure);
        if tick.decision.is_some() {
            decided += 1;
        }
        if tick.disposition.is_some() {
            actions += 1;
        }
    }

    if format == Format::Human {
        println!(
            "  {} ticks, {} decisions, {} would-be actions (dry run: nothing was changed)",
            control.ticks(),
            decided,
            actions
        );
        if actions == 0 {
            // The common and correct outcome on an idle machine.
            println!("  no action was worth taking, which on a quiet machine is the right answer");
        }
        println!(
            "  the watchdog is {}",
            if control.watchdog().is_stopped() {
                "stopped, and the loop obeyed it"
            } else {
                "satisfied"
            }
        );
        println!(
            "  {} audit events recorded",
            control.actuators().log().len()
        );
    }

    Ok(Ticks {
        count: control.ticks(),
        actions,
        responsive: 0,
    })
}

fn span_seconds(frames: &[MirrorSnapshot]) -> f64 {
    match (frames.first(), frames.last()) {
        (Some(first), Some(last)) => {
            last.monotonic_ns.saturating_sub(first.monotonic_ns) as f64 / 1e9
        }
        _ => 0.0,
    }
}

fn step(number: u32, what: &str, format: Format) {
    if format == Format::Human {
        println!("\n[{number}] {what}");
    }
}
