//! End-to-end proof that the mirror is a real abstraction.
//!
//! Each test here writes a trace of reflections that *no hardware produced*,
//! then runs the shipped binaries against it. The programs cannot tell, because
//! there is nothing in a reflection that says where it came from, and that is
//! the whole claim: the mirror is consumable by software that had no part in
//! producing it and no way to reach the machine it describes.
//!
//! The trace is a test fixture and is labelled as one. Nothing here is a
//! substitute for a real reading in a shipping code path; these are synthetic
//! inputs to real consumers, which is the opposite arrangement.

use std::path::{Path, PathBuf};
use std::process::Command;

use corescout_memory::recording::Recorder;

/// Write a synthetic trace and return its path.
///
/// The reflections come from the mirror crate's own fixture generator, so they
/// have the exact shape a real one does: entities with stable identity,
/// channels, relations, an epoch, and a permission-denied cell.
fn trace(name: &str, frames: u64) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "corescout-test-{name}-{}.jsonl",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let mut recorder = Recorder::create(&path).expect("a recorder");
    for snapshot in corescout_mirror::test_support::series(frames) {
        recorder.record(&snapshot).expect("recorded");
    }
    recorder.flush().expect("flushed");
    path
}

fn run(binary: &str, args: &[&str]) -> (bool, String) {
    let output = Command::new(binary)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("could not run {binary}: {error}"));
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

#[test]
fn mirror_replay_reads_a_trace_no_machine_produced() {
    let path = trace("replay", 40);
    let (ok, text) = run(
        env!("CARGO_BIN_EXE_mirror-replay"),
        &[path.to_str().unwrap()],
    );
    assert!(ok, "{text}");
    assert!(text.contains("40 reflections"), "{text}");
    // It reported the span, which means it read the timestamps rather than
    // just counting lines.
    assert!(text.contains("spanning"), "{text}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn mirror_learn_learns_from_a_trace_and_reports_its_score_either_way() {
    let path = trace("learn", 200);
    let (ok, text) = run(
        env!("CARGO_BIN_EXE_mirror-learn"),
        &["--replay", path.to_str().unwrap(), "--states"],
    );
    assert!(ok, "{text}");
    assert!(text.contains("predictions scored"), "{text}");
    assert!(text.contains("mean skill"), "{text}");
    // The verdict is present whichever way it came out.
    assert!(
        text.contains("beats assuming nothing changes")
            || text.contains("level with assuming nothing changes")
            || text.contains("worse than assuming nothing changes"),
        "{text}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn discovered_states_carry_machine_chosen_names() {
    // The project's rule: what the machine finds keeps the machine's names.
    // `latent_state_3` is a discovery; "high power state" would be us telling it
    // what it found.
    let path = trace("names", 200);
    let (ok, text) = run(
        env!("CARGO_BIN_EXE_mirror-learn"),
        &["--replay", path.to_str().unwrap(), "--states"],
    );
    assert!(ok, "{text}");
    if text.contains("states discovered") && !text.contains("0 recurring states") {
        assert!(text.contains("latent_state_"), "{text}");
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_replay_and_a_second_replay_of_the_same_trace_agree() {
    // Determinism is what makes a recording useful for studying a bug: two runs
    // over the same reflections must reach the same conclusions.
    let path = trace("determinism", 150);
    let args = ["--replay", path.to_str().unwrap()];
    let (ok_a, first) = run(env!("CARGO_BIN_EXE_mirror-learn"), &args);
    let (ok_b, second) = run(env!("CARGO_BIN_EXE_mirror-learn"), &args);
    assert!(ok_a && ok_b);
    assert_eq!(first, second, "two replays of one trace disagreed");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_empty_or_missing_trace_is_an_error_not_an_empty_success() {
    let (ok, text) = run(
        env!("CARGO_BIN_EXE_mirror-replay"),
        &["/nonexistent/trace.jsonl"],
    );
    assert!(!ok, "a missing trace must fail: {text}");

    let empty = std::env::temp_dir().join(format!("corescout-empty-{}.jsonl", std::process::id()));
    std::fs::write(&empty, "").expect("wrote an empty file");
    let (ok, text) = run(
        env!("CARGO_BIN_EXE_mirror-replay"),
        &[empty.to_str().unwrap()],
    );
    assert!(
        !ok,
        "an empty trace must fail rather than report nothing: {text}"
    );
    let _ = std::fs::remove_file(&empty);
}

#[test]
fn too_few_reflections_is_refused_rather_than_answered() {
    // Learning from four frames would produce a number, and the number would be
    // meaningless. Refusing is the correct behaviour.
    let path = trace("tiny", 4);
    let (ok, text) = run(
        env!("CARGO_BIN_EXE_mirror-learn"),
        &["--replay", path.to_str().unwrap()],
    );
    assert!(!ok, "{text}");
    assert!(text.contains("needs more than"), "{text}");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn every_consumer_rejects_an_unrecognised_flag() {
    for binary in [
        env!("CARGO_BIN_EXE_mirror-learn"),
        env!("CARGO_BIN_EXE_mirror-replay"),
    ] {
        let (ok, text) = run(binary, &["--frmes", "10"]);
        assert!(!ok, "{binary} accepted a typo: {text}");
    }
}

#[test]
fn no_consumer_source_file_mentions_a_hardware_interface() {
    // A crude check that catches the thing that actually goes wrong: someone
    // reaches for a sysfs read because one field was easier to get that way.
    // The link-time guarantee in Cargo.toml is the real defence; this is the
    // one that fires during review.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).expect("the bin directory") {
        let path = entry.expect("an entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("readable");
        // Skip doc comments: they discuss these names precisely because the
        // code must not use them.
        let code: String = source
            .lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                !trimmed.starts_with("//")
            })
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "/proc",
            "/sys",
            "perf_event_open",
            "corescout_substrate",
            "cpuid",
            "rdmsr",
        ] {
            assert!(
                !code.contains(forbidden),
                "{} reaches for {forbidden}",
                path.display()
            );
        }
        checked += 1;
    }
    assert!(
        checked >= 4,
        "expected to check every consumer, saw {checked}"
    );
}
