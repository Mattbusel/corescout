//! `mirror-observer`: learn about the machine from its reflection alone.
//!
//! # The experiment
//!
//! This program has no hardware access. It does not open `/proc` or `/sys`, does
//! not execute `CPUID`, does not call `perf_event_open`, and does not know what
//! a core, a cache or a temperature is. It maps the self-state plane read-only,
//! watches the numbers change, and reports what structure it can find.
//!
//! If it can say true things about the machine on that basis, the mirror is a
//! real abstraction rather than an internal data structure with a nice name.
//!
//! # Two observers
//!
//! By default it runs *unlabelled*: entities are `entity_7`, variables are `x2`,
//! edges are `edge_type_2`. With `--labels` it sees the mirror's own vocabulary.
//! With `--both` it runs one of each over the identical data and compares them,
//! which is how the project asks whether our ontology helps a machine understand
//! itself or constrains it.
//!
//! # `mirror-inspect` and this program
//!
//! `mirror-inspect` is deliberately stupid: it turns one reflection into text a
//! person can read. This one is the interesting case, and both prove the same
//! thing from opposite ends: the mirror is consumable by software that had no
//! part in producing it.

use std::process::ExitCode;

use corescout_mirror::plane::{default_path, PlaneReader};
use corescout_mirror::MirrorSnapshot;
use corescout_observer::{Lens, Observer, ObserverConfig};

const USAGE: &str = "\
mirror-observer - learn about a machine from its reflection alone

USAGE:
    mirror-observer [OPTIONS]

SOURCE (one of):
    --plane <PATH>     Watch a live self-state plane
                       (default: $XDG_RUNTIME_DIR/corescout/mirror.plane)
    --replay <PATH>    Replay a recorded trace written by `corescout mirror --record`

OPTIONS:
    --frames <N>       Reflections to watch before reporting (default 300)
    --interval <MS>    Milliseconds between reads of a live plane (default 100)
    --labels           Let the observer see the mirror's names (Observer A)
    --both             Run a labelled and an unlabelled observer, and compare
    --detail           Print each observer's full findings, not just the summary
    -h, --help         Show this help

This program has no hardware access. Everything it reports was derived from the
numbers in the mirror.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("mirror-observer: {message}");
            ExitCode::from(1)
        }
    }
}

enum Source {
    Live {
        path: std::path::PathBuf,
        interval_ms: u64,
    },
    Replay(std::path::PathBuf),
}

fn run(args: &[String]) -> Result<(), String> {
    let mut path = default_path();
    let mut replay: Option<std::path::PathBuf> = None;
    let mut frames = 300usize;
    let mut interval_ms = 100u64;
    let mut labels = false;
    let mut both = false;
    let mut detail = false;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "--labels" => labels = true,
            "--both" => both = true,
            "--detail" => detail = true,
            "--plane" => path = iter.next().ok_or("--plane needs a path")?.into(),
            "--replay" => replay = Some(iter.next().ok_or("--replay needs a path")?.into()),
            "--frames" => {
                frames = iter
                    .next()
                    .ok_or("--frames needs a count")?
                    .parse()
                    .map_err(|_| "--frames expects a whole number")?
            }
            "--interval" => {
                interval_ms = iter
                    .next()
                    .ok_or("--interval needs a value")?
                    .parse()
                    .map_err(|_| "--interval expects a whole number")?
            }
            other => return Err(format!("unknown option `{other}`; try --help")),
        }
    }

    let source = match replay {
        Some(path) => Source::Replay(path),
        None => Source::Live { path, interval_ms },
    };

    let snapshots = collect(&source, frames)?;
    if snapshots.is_empty() {
        return Err("no reflections were collected".to_string());
    }

    let config = ObserverConfig::default();
    if both {
        let unlabelled = observe_all(Lens::Unlabelled, config.clone(), &snapshots)?;
        let labelled = observe_all(Lens::Labelled, config, &snapshots)?;
        if detail {
            print!("{}", unlabelled.render());
            println!();
            print!("{}", labelled.render());
            println!();
        }
        print!("{}", unlabelled.compare(&labelled));
        return Ok(());
    }

    let lens = if labels {
        Lens::Labelled
    } else {
        Lens::Unlabelled
    };
    let findings = observe_all(lens, config, &snapshots)?;
    print!("{}", findings.render());
    Ok(())
}

/// Gather reflections from a live plane or a recorded trace.
fn collect(source: &Source, frames: usize) -> Result<Vec<MirrorSnapshot>, String> {
    match source {
        Source::Live { path, interval_ms } => {
            let reader = PlaneReader::open(path).map_err(|e| {
                format!("{e}\n\nIs the mirror running? Start it with `corescout mirror`.")
            })?;
            eprintln!(
                "mirror-observer: watching {} for {} reflections at {} ms",
                path.display(),
                frames,
                interval_ms
            );

            let mut snapshots = Vec::with_capacity(frames);
            let mut last_sequence = u64::MAX;
            let interval = std::time::Duration::from_millis(*interval_ms);
            while snapshots.len() < frames {
                let snapshot = reader.read_snapshot().map_err(|e| e.to_string())?;
                // Only take genuinely new reflections. Sampling the same
                // snapshot twice would fabricate a zero-change step and make
                // everything look more predictable than it is.
                if snapshot.sequence != last_sequence {
                    last_sequence = snapshot.sequence;
                    snapshots.push(snapshot);
                }
                std::thread::sleep(interval);
            }
            Ok(snapshots)
        }
        Source::Replay(path) => {
            let text =
                std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let mut snapshots = Vec::new();
            for (index, line) in text.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                let snapshot: MirrorSnapshot = serde_json::from_str(line)
                    .map_err(|e| format!("{}: line {}: {e}", path.display(), index + 1))?;
                snapshots.push(snapshot);
                if snapshots.len() >= frames {
                    break;
                }
            }
            Ok(snapshots)
        }
    }
}

fn observe_all(
    lens: Lens,
    config: ObserverConfig,
    snapshots: &[MirrorSnapshot],
) -> Result<corescout_observer::Findings, String> {
    let mut observer = Observer::new(lens, config);
    for snapshot in snapshots {
        observer.observe(snapshot);
    }
    observer.findings().ok_or_else(|| {
        format!(
            "not enough consistent reflections to say anything ({} seen). \
             Watch for longer, or check that the machine's shape is not changing \
             under the observer.",
            snapshots.len()
        )
    })
}
