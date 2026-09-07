//! The observing commands: `mirror`, `record`, `info`.
//!
//! These are the only commands that read hardware. `mirror` and `record` are
//! passive: they sample what the machine already exposes and publish it. `info`
//! is a one-shot structural read.
//!
//! # Why the mirror keeps running when a sensor fails
//!
//! A sensor that cannot read its source marks its cells with a reason and the
//! pass continues. Aborting the whole reflection because one file under
//! `/sys` needed privilege would mean a machine that reveals less also
//! reflects nothing, which is exactly backwards: the gap is information.

use std::io::Write;
use std::time::{Duration, Instant};

use corescout_core::error::Result;
use corescout_human::{json, mirror_debug, Format};
use corescout_memory::recording::Recorder;
use corescout_mirror::plane::{PlaneMemory, PlaneWriter};
use corescout_report::json as report_json;

use crate::runtime;

/// Options for a mirror run, shared by `mirror` and `record`.
pub struct MirrorRun {
    pub once: bool,
    pub interval: Duration,
    pub ticks: Option<u64>,
    pub plane: Option<std::path::PathBuf>,
    pub publish: bool,
    pub record: Option<std::path::PathBuf>,
    pub filter: Option<String>,
    pub format: Format,
}

/// Observe continuously, publishing and optionally recording.
pub fn mirror(options: MirrorRun) -> Result<()> {
    let mut reflector = runtime::reflector()?;
    reflector.observe();
    let mut snapshot = reflector.snapshot();

    let mut writer = if options.publish {
        let path = runtime::plane_path(options.plane.as_ref());
        let target = path.clone();
        let writer = PlaneWriter::new(move |len| PlaneMemory::create(&target, len), &snapshot)?;
        if !options.once && options.format == Format::Human {
            eprintln!(
                "publishing to {} ({} bytes, {} entities x {} channels)",
                path.display(),
                writer.size(),
                snapshot.entities.len(),
                snapshot.channels.len()
            );
        }
        Some(writer)
    } else {
        None
    };

    let mut recorder = match &options.record {
        Some(path) => Some(Recorder::create(path)?),
        None => None,
    };

    if options.once {
        if let Some(writer) = &mut writer {
            writer.publish(&snapshot)?;
        }
        if let Some(recorder) = &mut recorder {
            recorder.record(&snapshot)?;
            recorder.flush()?;
        }
        return print_snapshot(&snapshot, options.format, options.filter.as_deref());
    }

    let mut ticks = 0u64;
    loop {
        let started = Instant::now();

        // A shape change means the machine is not the machine it was: a CPU
        // went offline, or a device appeared. The plane's epoch has to change
        // with it, and every reader has to be told rather than silently reading
        // rows that now mean something else.
        if reflector.shape_changed() {
            let fresh = runtime::reflector()?;
            reflector = fresh;
            reflector.observe();
            snapshot = reflector.snapshot();
            if let Some(path) = options
                .plane
                .as_ref()
                .map(|p| p.to_path_buf())
                .or_else(|| options.publish.then(corescout_mirror::plane::default_path))
            {
                let target = path.clone();
                writer = Some(PlaneWriter::new(
                    move |len| PlaneMemory::create(&target, len),
                    &snapshot,
                )?);
            }
            if options.format == Format::Human {
                eprintln!("the machine changed shape; the plane moved to a new epoch");
            }
        } else {
            reflector.observe();
            snapshot = reflector.snapshot();
        }

        if let Some(writer) = &mut writer {
            writer.publish(&snapshot)?;
        }
        if let Some(recorder) = &mut recorder {
            recorder.record(&snapshot)?;
        }

        ticks += 1;
        if options.format == Format::Json {
            let mut out = std::io::stdout().lock();
            // A closed pipe is a normal way for `| head` to end a stream, not a
            // failure of the mirror; the loop stops rather than reporting it.
            if writeln!(out, "{}", json::snapshot(&snapshot)?).is_err() {
                break;
            }
        }
        if options.ticks.is_some_and(|limit| ticks >= limit) {
            break;
        }

        // Subtract the observation's own cost, so the requested interval is the
        // interval between observations rather than the gap after them.
        let spent = started.elapsed();
        if let Some(remaining) = options.interval.checked_sub(spent) {
            std::thread::sleep(remaining);
        }
    }

    if let Some(recorder) = &mut recorder {
        recorder.flush()?;
        if options.format == Format::Human {
            println!(
                "recorded {} reflections to {} ({} bytes)",
                recorder.written(),
                recorder.path().display(),
                recorder.bytes()
            );
        }
    }
    Ok(())
}

/// One-shot structural read of the machine.
pub fn info(format: Format) -> Result<()> {
    let platform = corescout_substrate::platform::detect();
    let topology = platform.discover_topology()?;
    match format {
        Format::Json => println!("{}", report_json::topology(&topology)?),
        Format::Human => print!("{}", corescout_report::human_debug::topology(&topology)),
    }
    Ok(())
}

fn print_snapshot(
    snapshot: &corescout_mirror::MirrorSnapshot,
    format: Format,
    filter: Option<&str>,
) -> Result<()> {
    match format {
        Format::Json => println!("{}", json::snapshot(snapshot)?),
        Format::Human => print!(
            "{}{}",
            mirror_debug::summary(snapshot),
            mirror_debug::detail(snapshot, filter)
        ),
    }
    Ok(())
}
