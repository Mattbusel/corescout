//! `mirror-daemon`: keep a reflection of this machine continuously current.
//!
//! # The only writer
//!
//! One process observes and publishes; every other program maps the plane
//! read-only. That is not a convention, it is enforced by the kernel: consumers
//! map with `PROT_READ`, so a consumer that tried to write its own reflection
//! would take a fault. The mirror is read-only to everything except the thing
//! whose reflection it is.
//!
//! An intelligent controller may change the machine. It may not change its own
//! reflection. Keeping the writer in a separate program from the agent is how
//! that stays true as the code grows.
//!
//! # It publishes gaps
//!
//! A sensor that cannot read its source marks its cells with a reason code and
//! the pass continues. The plane then carries "permission denied" where a
//! reading would have been, and a consumer can tell the difference between a
//! machine that is idle and a machine that will not say. Substituting a
//! plausible number there would corrupt every model downstream, permanently and
//! invisibly.
//!
//! # Cost
//!
//! Observation is not free and the daemon does not pretend it is: each pass is
//! timed, and the per-sensor cost is published in the plane alongside the
//! readings. A consumer can therefore see what its own visibility is costing.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use corescout_core::error::{Error, Result};
use corescout_memory::recording::Recorder;
use corescout_mirror::plane::{default_path, PlaneMemory, PlaneWriter};
use corescout_substrate::observation::default_sensors;
use corescout_substrate::{platform, Reflector, Substrate};

const USAGE: &str = "\
mirror-daemon - keep a reflection of this machine continuously current

USAGE:
    mirror-daemon [OPTIONS]

OPTIONS:
    --plane <PATH>     Where to publish (default: $XDG_RUNTIME_DIR/corescout/mirror.plane)
    --interval <MS>    Milliseconds between observations (default 100)
    --ticks <N>        Stop after N observations (default: run until killed)
    --record <PATH>    Also append every reflection to a replayable trace
    --quiet            Say nothing on startup
    -h, --help         Show this help

This is the only program that writes the plane. Everything else maps it
read-only, which the kernel enforces.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("mirror-daemon: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }

    let mut plane = default_path();
    let mut interval_ms = 100u64;
    let mut ticks: Option<u64> = None;
    let mut record: Option<String> = None;
    let mut quiet = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--quiet" => quiet = true,
            "--plane" | "--interval" | "--ticks" | "--record" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| Error::invalid(format!("{} needs a value", args[index])))?;
                match args[index].as_str() {
                    "--plane" => plane = std::path::PathBuf::from(value),
                    "--interval" => {
                        interval_ms = value
                            .parse()
                            .map_err(|_| Error::invalid("--interval expects a number"))?
                    }
                    "--ticks" => {
                        ticks = Some(
                            value
                                .parse()
                                .map_err(|_| Error::invalid("--ticks expects a number"))?,
                        )
                    }
                    _ => record = Some(value.clone()),
                }
                index += 1;
            }
            other => {
                return Err(Error::invalid(format!(
                    "unexpected argument {other:?}; run mirror-daemon --help"
                )))
            }
        }
        index += 1;
    }

    let interval = Duration::from_millis(interval_ms.max(1));
    let mut reflector = build()?;
    reflector.observe();
    let mut snapshot = reflector.snapshot();

    let target = plane.clone();
    let mut writer = PlaneWriter::new(move |len| PlaneMemory::create(&target, len), &snapshot)?;
    let mut recorder = match &record {
        Some(path) => Some(Recorder::create(path)?),
        None => None,
    };

    if !quiet {
        eprintln!(
            "publishing {} entities x {} channels to {} ({} bytes), every {} ms",
            snapshot.entities.len(),
            snapshot.channels.len(),
            plane.display(),
            writer.size(),
            interval_ms
        );
        let gaps = snapshot.privilege_gaps();
        if gaps > 0 {
            eprintln!(
                "{gaps} readings need privilege this process does not have; they are \
                 published as gaps, not guessed at"
            );
        }
        eprintln!(
            "observation costs {} us per pass",
            reflector.last_observation_cost_ns() / 1000
        );
    }

    let mut published = 0u64;
    loop {
        let started = Instant::now();

        // A machine that changes shape is a different machine as far as the
        // plane's row indices are concerned. Rebuilding moves the epoch forward,
        // and every reader is required to notice.
        if reflector.shape_changed() {
            reflector = build()?;
            reflector.observe();
            snapshot = reflector.snapshot();
            let target = plane.clone();
            writer = PlaneWriter::new(move |len| PlaneMemory::create(&target, len), &snapshot)?;
            if !quiet {
                eprintln!(
                    "the machine changed shape; the plane moved to epoch {}",
                    writer.epoch()
                );
            }
        } else {
            reflector.observe();
            snapshot = reflector.snapshot();
        }

        writer.publish(&snapshot)?;
        if let Some(recorder) = &mut recorder {
            recorder.record(&snapshot)?;
        }
        published += 1;
        if ticks.is_some_and(|limit| published >= limit) {
            break;
        }

        // The requested interval is between observations, not after them, so a
        // slow pass does not silently halve the sampling rate.
        if let Some(remaining) = interval.checked_sub(started.elapsed()) {
            std::thread::sleep(remaining);
        }
    }

    if let Some(recorder) = &mut recorder {
        recorder.flush()?;
        if !quiet {
            eprintln!(
                "recorded {} reflections to {}",
                recorder.written(),
                recorder.path().display()
            );
        }
    }
    Ok(())
}

fn build() -> Result<Reflector> {
    let platform = platform::detect();
    let substrate = Substrate::discover(platform.as_ref())?;
    Reflector::build(substrate, default_sensors())
}
