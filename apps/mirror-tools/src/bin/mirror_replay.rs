//! `mirror-replay`: read a recorded trace as though it were happening now.
//!
//! # The property being demonstrated
//!
//! A recording and a live plane are the same thing to a consumer. This program
//! exists to make that checkable: point it at a trace, and it produces exactly
//! what `mirror-inspect` produces against a live plane, because both are reading
//! a [`corescout_memory::source::Source`] and neither asks which kind it has.
//!
//! That matters beyond tidiness. It means a model can be developed against a
//! recording, a bug can be reproduced from one, and an experiment can be re-run
//! on the identical data months later, all with the code that runs in
//! production. A system that can only be studied while it is running cannot be
//! studied carefully.
//!
//! # Time
//!
//! By default the trace is read as fast as it can be. `--paced` reproduces the
//! original intervals, which matters when something downstream is sensitive to
//! rate rather than to order.
//!
//! This program links no hardware-facing crate. See `Cargo.toml`.

use std::process::ExitCode;

use corescout_core::error::{Error, Result};
use corescout_memory::recording;
use corescout_memory::source::Source;

const USAGE: &str = "\
mirror-replay - read a recorded trace as though it were live

USAGE:
    mirror-replay <TRACE> [OPTIONS]

OPTIONS:
    --frames <N>       Reflections to read (default: all of them)
    --paced            Reproduce the original intervals rather than reading flat out
    --summary          Print what the recording contains and stop
    --json             Emit each reflection as JSON, one per line
    -h, --help         Show this help

A consumer cannot tell this from a live mirror, and is not told.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mirror-replay: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<()> {
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }

    let mut path: Option<String> = None;
    let mut frames_wanted: Option<usize> = None;
    let mut paced = false;
    let mut summary = false;
    let mut json = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--paced" => paced = true,
            "--summary" => summary = true,
            "--json" => json = true,
            "--frames" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| Error::invalid("--frames needs a value"))?;
                frames_wanted = Some(
                    value
                        .parse()
                        .map_err(|_| Error::invalid("--frames expects a number"))?,
                );
                index += 1;
            }
            other if !other.starts_with('-') && path.is_none() => path = Some(other.to_string()),
            other => {
                return Err(Error::invalid(format!(
                    "unexpected argument {other:?}; run mirror-replay --help"
                )))
            }
        }
        index += 1;
    }

    let path = path
        .ok_or_else(|| Error::invalid("mirror-replay needs a trace: mirror-replay trace.jsonl"))?;
    let path = std::path::Path::new(&path);

    if summary {
        let summary = recording::summarise(path)?;
        println!("{summary:#?}");
        return Ok(());
    }

    let mut source = if paced {
        Source::replay_paced(path)?
    } else {
        Source::replay(path)?
    };
    let frames = source.take(frames_wanted.unwrap_or(usize::MAX))?;
    if frames.is_empty() {
        return Err(Error::invalid(format!(
            "{} contains no readable reflections",
            path.display()
        )));
    }

    if json {
        for frame in &frames {
            println!("{}", corescout_human::json::snapshot(frame)?);
        }
        return Ok(());
    }

    let first = &frames[0];
    let last = &frames[frames.len() - 1];
    println!(
        "{} reflections, sequence {} to {}, spanning {:.1} s of machine time",
        frames.len(),
        first.sequence,
        last.sequence,
        last.monotonic_ns.saturating_sub(first.monotonic_ns) as f64 / 1e9
    );
    if first.epoch != last.epoch {
        // Worth saying loudly: rows do not mean the same thing across an epoch
        // boundary, and anything averaging over one is averaging over two
        // different machines.
        println!(
            "the machine changed shape during this recording (epoch {} to {}); \
             rows before and after are not the same entities",
            first.epoch, last.epoch
        );
    }
    print!("{}", corescout_human::mirror_debug::summary(last));
    Ok(())
}
