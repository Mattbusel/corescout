//! `mirror-learn`: learn to predict a machine's next state, having never seen
//! the machine.
//!
//! # What it is given
//!
//! A stream of reflections. Nothing else. It does not know what a CPU is, what
//! a temperature is, or which of the numbers are counters and which are gauges.
//! It knows that there are entities, that each has variables, that some are
//! related, and that time passes.
//!
//! # What it does
//!
//! Fits a scaling, recognises recurring states, and learns a per-cell predictor
//! of the next reflection. Then it scores itself against the only baseline that
//! matters here: **assuming nothing changes**. On a quiet machine that baseline
//! is very strong, and beating it is not guaranteed. The score is printed
//! whichever way it comes out, because a learner that only reports its wins is
//! not reporting.
//!
//! # Why the scaling is fitted on a prefix
//!
//! Fitting the normaliser on the whole trace and then scoring predictions
//! against that same trace lets information from the future leak into the
//! scaling. The effect is small and it always flatters the model, so the
//! normaliser is fitted on an early slice and the rest is scored honestly.
//!
//! This program links no hardware-facing crate. See `Cargo.toml`.

use std::process::ExitCode;

use corescout_core::error::{Error, Result};
use corescout_memory::source::Source;
use corescout_mirror::plane::default_path;
use corescout_mirror::MirrorSnapshot;
use corescout_represent::latent::LatentCatalogue;
use corescout_represent::normalize::Normalizer;
use corescout_selfmodel::SelfModel;

const USAGE: &str = "\
mirror-learn - learn to predict a machine's next state from its reflection alone

USAGE:
    mirror-learn [OPTIONS]

SOURCE (one of):
    --plane <PATH>     Watch a live self-state plane
                       (default: $XDG_RUNTIME_DIR/corescout/mirror.plane)
    --replay <PATH>    Replay a trace written by `corescout record`

OPTIONS:
    --frames <N>       Reflections to learn from (default 1000)
    --interval <MS>    Milliseconds between reads of a live plane (default 100)
    --states           List the states discovered, not just the count
    --json             Emit the learned model as JSON
    -h, --help         Show this help

The score is against assuming nothing changes. On an idle machine that is a
strong baseline, and this program will say so when it loses to it.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mirror-learn: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }

    let mut frames_wanted = 1000usize;
    let mut interval_ms = 100u64;
    let mut plane: Option<String> = None;
    let mut replay: Option<String> = None;
    let mut list_states = false;
    let mut json = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--states" => list_states = true,
            "--json" => json = true,
            "--frames" | "--interval" | "--plane" | "--replay" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| Error::invalid(format!("{} needs a value", args[index])))?;
                match args[index].as_str() {
                    "--frames" => {
                        frames_wanted = value
                            .parse()
                            .map_err(|_| Error::invalid("--frames expects a number"))?
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
                    "unexpected argument {other:?}; run mirror-learn --help"
                )))
            }
        }
        index += 1;
    }

    // Live or recorded, the code below is identical and is not told which.
    let mut source = match (&replay, &plane) {
        (Some(path), _) => Source::replay(std::path::Path::new(path))?,
        (None, Some(path)) => Source::live(
            std::path::Path::new(path),
            std::time::Duration::from_millis(interval_ms.max(1)),
        )?,
        (None, None) => Source::live(
            &default_path(),
            std::time::Duration::from_millis(interval_ms.max(1)),
        )?,
    };

    let frames = source.take(frames_wanted)?;
    if frames.len() < 8 {
        return Err(Error::invalid(format!(
            "learning needs more than {} reflections",
            frames.len()
        )));
    }

    let (model, catalogue, scored, skill) = learn(&frames)?;

    if json {
        let text = serde_json::to_string_pretty(&model)
            .map_err(|error| Error::invalid(format!("could not serialise the model: {error}")))?;
        println!("{text}");
        return Ok(());
    }

    println!(
        "{} reflections, {} entities x {} variables",
        frames.len(),
        frames[0].entities.len(),
        frames[0].channels.len()
    );
    println!(
        "{} recurring states discovered, {} cells tracked",
        catalogue.len(),
        model.tracked_cells()
    );
    println!("{scored} predictions scored");
    println!("mean skill against assuming no change: {skill:+.3}");
    println!(
        "{}",
        if skill > 0.05 {
            "the model beats assuming nothing changes"
        } else if skill > -0.05 {
            // The usual answer on a quiet machine, and not a failure.
            "the model is about level with assuming nothing changes"
        } else {
            "the model is worse than assuming nothing changes"
        }
    );

    if list_states {
        for state in catalogue.ranked() {
            println!(
                "  {}  entered {} times, mean dwell {:.0} ms",
                state.id,
                state.entries,
                state.mean_dwell_ns() / 1e6
            );
        }
        println!("those names are the machine's own; this program named nothing");
    }
    Ok(())
}

/// Fit, recognise and predict, in one pass over the reflections.
fn learn(frames: &[MirrorSnapshot]) -> Result<(SelfModel, LatentCatalogue, u64, f64)> {
    let cols = frames[0].channels.len().max(1);
    // An early slice, not the whole trace: see the note at the top of the file.
    let fit_frames = (frames.len() / 5).clamp(4, 200);
    let rows: Vec<Vec<f64>> = frames[..fit_frames]
        .iter()
        .map(|frame| frame.state.as_slice().to_vec())
        .collect();
    let normalizer = Normalizer::fit(&rows, cols);

    let mut catalogue = LatentCatalogue::new(0.75, 32);
    let mut model = SelfModel::default();
    let mut scored = 0u64;
    let mut skill_total = 0.0;

    for frame in frames {
        let (point, _filled) = normalizer.apply_filled(frame.state.as_slice(), cols);
        catalogue.observe(&point, frame.monotonic_ns);
        let before = model.scored();
        model.observe(frame);
        if model.scored() > before {
            scored += 1;
            skill_total += model.skill();
        }
    }

    let skill = if scored > 0 {
        skill_total / scored as f64
    } else {
        0.0
    };
    Ok((model, catalogue, scored, skill))
}
