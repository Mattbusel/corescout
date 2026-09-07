//! `mirror-inspect`: look into the mirror, and nothing else.
//!
//! # What this program proves
//!
//! It collects no hardware data. It opens no file under `/proc` or `/sys`, calls
//! no `perf_event_open`, reads no MSR, and knows nothing about CPUs beyond what
//! the reflection tells it. Its entire contact with the machine is:
//!
//! ```text
//! open(plane, O_RDONLY) -> mmap(PROT_READ) -> memcpy
//! ```
//!
//! If it can describe the machine anyway, then the abstraction exists
//! independently of its observer, which is the point of Mirror v0.1. A mirror
//! that only its own author can read is not a representation, it is an internal
//! data structure with a nice name.
//!
//! An architectural test in `tests/mirror_boundary.rs` asserts this by
//! inspecting the source of this file: it must not mention `/proc`, `/sys`,
//! `observation::`, `substrate::` or `platform::`. That is a crude check and it
//! catches the thing that actually goes wrong, which is someone reaching for a
//! sysfs read because one field was easier to get that way.

use std::process::ExitCode;

use corescout_human::mirror_debug;
use corescout_mirror::plane::{default_path, PlaneReader};

const USAGE: &str = "\
mirror-inspect - read a CoreScout self-state plane

USAGE:
    mirror-inspect [OPTIONS]

OPTIONS:
    --plane <PATH>     Plane to read (default: $XDG_RUNTIME_DIR/corescout/mirror.plane)
    --detail           Print every observed value, entity by entity
    --filter <KEY>     With --detail, only entities whose key contains KEY
    --entity <KEY>     Print one entity's neighbourhood in the graph
    --json             Print the snapshot as JSON
    --watch <N>        Read N times, 200 ms apart, reporting the sequence number
    -h, --help         Show this help

This program has no hardware access. Everything it prints came from the plane.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("mirror-inspect: {message}");
            ExitCode::from(1)
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let mut path = default_path();
    let mut detail = false;
    let mut json = false;
    let mut filter: Option<String> = None;
    let mut entity: Option<String> = None;
    let mut watch: Option<u32> = None;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "--detail" => detail = true,
            "--json" => json = true,
            "--plane" => {
                path = iter.next().ok_or("--plane needs a path")?.into();
            }
            "--filter" => filter = Some(iter.next().ok_or("--filter needs a key")?.clone()),
            "--entity" => entity = Some(iter.next().ok_or("--entity needs a key")?.clone()),
            "--watch" => {
                watch = Some(
                    iter.next()
                        .ok_or("--watch needs a count")?
                        .parse()
                        .map_err(|_| "--watch expects a whole number")?,
                )
            }
            other => return Err(format!("unknown option `{other}`; try --help")),
        }
    }

    let reader = PlaneReader::open(&path)
        .map_err(|e| format!("{e}\n\nIs the mirror running? Start it with `corescout mirror`."))?;

    if let Some(count) = watch {
        return watch_plane(&reader, count);
    }

    let snapshot = reader.read_snapshot().map_err(|e| e.to_string())?;

    if json {
        let text = corescout_human::json::snapshot(&snapshot).map_err(|e| e.to_string())?;
        println!("{text}");
        return Ok(());
    }

    if let Some(key) = entity {
        print!("{}", mirror_debug::neighbourhood(&snapshot, &key));
        return Ok(());
    }

    print!("{}", mirror_debug::summary(&snapshot));
    println!(
        "\nplane: {} ({} bytes, written by pid {})",
        path.display(),
        reader.layout().total_bytes,
        reader.writer_pid()
    );

    if detail {
        print!("{}", mirror_debug::detail(&snapshot, filter.as_deref()));
    }
    Ok(())
}

/// Read repeatedly, showing that the reflection is live and that reading it is
/// cheap.
fn watch_plane(reader: &PlaneReader, count: u32) -> Result<(), String> {
    println!(
        "{:>6}  {:>12}  {:>10}  coverage",
        "read", "sequence", "read us"
    );
    for index in 0..count {
        let started = std::time::Instant::now();
        let state = reader.read_state().map_err(|e| e.to_string())?;
        let elapsed = started.elapsed();

        let observed = state.values.iter().filter(|v| !v.is_nan()).count();
        let coverage = if state.values.is_empty() {
            0.0
        } else {
            observed as f64 / state.values.len() as f64
        };
        println!(
            "{:>6}  {:>12}  {:>10.1}  {:.0}%",
            index + 1,
            state.sequence,
            elapsed.as_secs_f64() * 1e6,
            coverage * 100.0
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Ok(())
}
