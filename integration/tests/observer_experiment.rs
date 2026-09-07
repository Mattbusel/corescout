//! The observer experiment.
//!
//! > Can a program with no `/proc`, `/sys`, CPUID, perf or hardware access learn
//! > meaningful things about a computer purely by looking into the mirror?
//!
//! These tests run the real observer over a machine whose true structure is
//! known, and check what it found against that truth. Nothing here gives the
//! observer anything except mirror snapshots.
//!
//! # What this proves and what it does not
//!
//! The machine is synthetic (see `common/synthetic.rs`), and it makes SMT
//! siblings covary by construction. So "the observer found the SMT pairs" is
//! **not** evidence about real CPUs. It is evidence that:
//!
//! - the discovery pipeline finds structure that is present,
//! - it does not find structure that is absent, which the frequency-domain
//!   edges are the control for,
//! - the mirror's representation carries this class of signal intact from
//!   producer to consumer,
//! - and the labelled and unlabelled observers can be compared on identical
//!   data.
//!
//! The real experiment is an hour of `corescout mirror --record` on hardware,
//! followed by exactly this analysis. This is the harness for that, run first
//! against a machine whose answers are known.

mod common;

use common::synthetic::SyntheticMachine;
use corescout_observer::{Findings, Lens, Observer, ObserverConfig};

/// Watch a synthetic machine and return what each observer concluded.
fn run_experiment(frames: usize, seed: u64) -> (SyntheticMachine, Findings, Findings) {
    let mut machine = SyntheticMachine::new(seed);
    let snapshots = machine.run(frames);

    let observe = |lens: Lens| {
        let mut observer = Observer::new(lens, ObserverConfig::default());
        for snapshot in &snapshots {
            observer.observe(snapshot);
        }
        observer
            .findings()
            .expect("enough reflections to say something")
    };

    let unlabelled = observe(Lens::Unlabelled);
    let labelled = observe(Lens::Labelled);
    (machine, unlabelled, labelled)
}

#[test]
fn the_unlabelled_observer_rediscovers_the_smt_pairs() {
    // The headline. An observer told nothing but "entity 3", "x0" and
    // "edge_type_2" partitions the CPUs into exactly the pairs that share a
    // physical core.
    let (machine, findings, _) = run_experiment(400, 7);
    let truth = machine.truth();

    let mut found: Vec<(u32, u32)> = findings
        .entity_clusters
        .iter()
        .filter(|cluster| cluster.rows.len() == 2)
        .map(|cluster| {
            let a = cluster.rows[0] as u32;
            let b = cluster.rows[1] as u32;
            (a.min(b), a.max(b))
        })
        .collect();
    found.sort_unstable();

    let mut expected = truth.smt_pairs.clone();
    expected.sort_unstable();

    assert_eq!(
        found, expected,
        "the observer should have grouped the CPUs into their true SMT pairs.\n\
         found:    {found:?}\n\
         expected: {expected:?}"
    );
    assert_eq!(
        findings.entity_clusters.len(),
        4,
        "four cores, four pairs, and no spurious groups"
    );
    for cluster in &findings.entity_clusters {
        assert!(
            cluster.cohesion > 0.8,
            "a discovered pair should be strongly coupled, got {}",
            cluster.cohesion
        );
    }
}

#[test]
fn the_observer_distinguishes_a_real_edge_type_from_a_vacuous_one() {
    // The control. Both edge types really exist in the mirror and join CPUs.
    // One reflects a physical coupling; the other joins everything to
    // everything. An observer that reports lift for both has learned nothing.
    let (machine, findings, _) = run_experiment(400, 7);
    let truth = machine.truth();

    let real = findings
        .relation_lifts
        .iter()
        .find(|lift| lift.kind == truth.real_edge_kind)
        .expect("the SMT edge type should have comparable pairs");
    let vacuous = findings
        .relation_lifts
        .iter()
        .find(|lift| lift.kind == truth.vacuous_edge_kind)
        .expect("the frequency-domain edge type should have comparable pairs");

    assert!(
        real.lift > 0.3,
        "the real edge type should predict shared behaviour, lift was {:+.3}",
        real.lift
    );
    assert!(
        real.separation > 1.0,
        "and by a large effect size, d was {:.2}",
        real.separation
    );
    assert!(
        vacuous.lift.abs() < 0.1,
        "the all-to-all edge type carries no information, but lift was {:+.3}",
        vacuous.lift
    );
    assert_eq!(
        findings.relation_lifts[0].kind, truth.real_edge_kind,
        "the real edge type should rank first"
    );
}

#[test]
fn the_observer_works_out_which_variables_accumulate() {
    // Without being told. `Semantics::Cumulative` is a human label the
    // unlabelled observer never sees, and monotonicity is observable.
    let (machine, unlabelled, labelled) = run_experiment(400, 7);
    let truth = machine.truth();

    let mut inferred = unlabelled.inferred_accumulators.clone();
    inferred.sort_unstable();
    let mut expected = truth.accumulator_columns.clone();
    expected.sort_unstable();
    assert_eq!(
        inferred, expected,
        "the observer should have identified the counters by watching them"
    );

    // And the labelled observer, which was told, agrees exactly. So on this
    // machine that particular label was worth a name and nothing else.
    let agreement = labelled
        .accumulator_agreement()
        .expect("the labelled observer was told");
    assert!(
        agreement.is_exact(),
        "inference and declaration disagreed: {agreement:?}"
    );
}

#[test]
fn both_observers_discover_the_same_structure() {
    // The A/B result. Identical data, different vocabulary, same findings.
    let (_, unlabelled, labelled) = run_experiment(400, 7);

    assert!(
        unlabelled.discovered_the_same_structure(&labelled),
        "the two observers partitioned the machine differently:\n{}\n{}",
        unlabelled.render(),
        labelled.render()
    );
    assert_eq!(unlabelled.relation_lifts, labelled.relation_lifts);
    assert_eq!(
        unlabelled.prediction.cells, labelled.prediction.cells,
        "both scored the same cells"
    );
}

#[test]
fn the_observer_can_predict_the_next_reflection_better_than_chance() {
    // "Can you predict the next reflection?" Scored against baselines chosen
    // to be hard: persistence for readings, drift for counters.
    let (_, findings, _) = run_experiment(400, 7);

    assert!(findings.prediction.cells > 0, "nothing was scorable");
    assert!(
        findings.prediction.median_skill > 0.05,
        "median skill was {:+.3}; the observer did no better than copying the \
         last value, which would mean the reflection carries no usable dynamics",
        findings.prediction.median_skill
    );
    assert!(
        findings.prediction.cells_with_skill * 2 >= findings.prediction.cells,
        "only {}/{} cells beat the baseline",
        findings.prediction.cells_with_skill,
        findings.prediction.cells
    );
}

#[test]
fn the_observer_finds_the_recurring_whole_machine_states() {
    // The synthetic machine alternates between a quiet and a busy phase.
    let (_, findings, _) = run_experiment(400, 7);

    assert!(
        findings.regimes.len() >= 2,
        "the machine has two phases; the observer found {}",
        findings.regimes.len()
    );
    let occupied = findings
        .regimes
        .iter()
        .filter(|regime| regime.occupancy > 20)
        .count();
    assert!(
        occupied >= 2,
        "at least two regimes should be substantially occupied"
    );
    // And it should have seen the machine move between them.
    let transitions: usize = findings
        .regime_transitions
        .iter()
        .enumerate()
        .map(|(from, row)| {
            row.iter()
                .enumerate()
                .filter(|(to, _)| *to != from)
                .map(|(_, count)| *count)
                .sum::<usize>()
        })
        .sum();
    assert!(transitions > 0, "the observer never saw a phase change");
}

#[test]
fn the_unlabelled_observers_report_contains_no_human_vocabulary() {
    // The experiment is void if the report quietly reaches past the lens.
    let (_, findings, _) = run_experiment(400, 7);
    let text = findings.render().to_lowercase();
    // Whole words, not substrings: "scored" contains "core", and a substring
    // check would fail on the report's own prose rather than on a real leak.
    let words: Vec<&str> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    for leak in [
        "cpu",
        "core",
        "cores",
        "frequency",
        "temperature",
        "smt",
        "sibling",
        "cache",
        "numa",
        "interrupts",
        "cycles",
        "celsius",
        "khz",
    ] {
        assert!(
            !words.contains(&leak),
            "the unlabelled report leaked `{leak}`:\n{text}"
        );
    }
}

#[test]
fn findings_are_reproducible() {
    // A learner that reached different conclusions each time it considered the
    // same evidence would not be discovering anything.
    let (_, first, _) = run_experiment(300, 11);
    let (_, second, _) = run_experiment(300, 11);
    assert_eq!(first, second);
}

#[test]
fn a_different_machine_yields_different_findings() {
    // The counterpart: the observer is reading the data, not reciting a
    // constant.
    let (_, seed_a, _) = run_experiment(300, 11);
    let (_, seed_b, _) = run_experiment(300, 12);
    assert_ne!(
        seed_a.prediction.median_skill, seed_b.prediction.median_skill,
        "two different machines produced identical predictions"
    );
    // The true structure is the same in both, so that part should agree.
    assert!(seed_a.discovered_the_same_structure(&seed_b));
}

#[test]
fn the_observer_refuses_to_speak_too_early() {
    // Correlation over a handful of samples produces confident nonsense.
    let mut machine = SyntheticMachine::new(3);
    let mut observer = Observer::new(Lens::Unlabelled, ObserverConfig::default());
    for snapshot in machine.run(10) {
        observer.observe(&snapshot);
    }
    assert!(observer.findings().is_none());
}

/// Print the full experiment. Run with:
///
/// ```text
/// cargo test --release --test observer_experiment -- --nocapture show_the_experiment
/// ```
#[test]
fn show_the_experiment() {
    let (machine, unlabelled, labelled) = run_experiment(400, 7);

    println!("\n{}", unlabelled.render());
    println!("{}", labelled.render());
    println!("{}", unlabelled.compare(&labelled));

    println!("Ground truth (which neither observer saw)");
    let truth = machine.truth();
    println!("  SMT pairs by entity row: {:?}", truth.smt_pairs);
    println!("  accumulator columns:     {:?}", truth.accumulator_columns);
    println!(
        "  interrupt-heavy entity:  row {}",
        truth.interrupt_heavy_row
    );
    println!(
        "  real edge type {} joins SMT siblings; edge type {} joins everything",
        truth.real_edge_kind, truth.vacuous_edge_kind
    );
}

/// Locate a binary built by another package in this workspace.
///
/// `CARGO_BIN_EXE_*` only exists for binaries in the *same* package, and
/// `mirror-observer` deliberately lives in a package this one does not and must
/// not depend on. Walking up from this test's own executable finds it in the
/// same profile directory, which is where Cargo has just put it.
fn binary(name: &str) -> std::path::PathBuf {
    let mut path = std::env::current_exe().expect("this test's own path");
    path.pop(); // deps/
    if path.ends_with("deps") {
        path.pop();
    }
    path.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

#[test]
fn the_observer_binary_replays_a_recorded_trace() {
    let mut machine = SyntheticMachine::new(7);
    let snapshots = machine.run(400);

    let path = std::env::temp_dir().join(format!(
        "corescout-observer-trace-{}.jsonl",
        std::process::id()
    ));
    let mut trace = String::new();
    for snapshot in &snapshots {
        trace.push_str(&serde_json::to_string(snapshot).expect("serialise reflection"));
        trace.push('\n');
    }
    std::fs::write(&path, trace).expect("write trace");

    let output = std::process::Command::new(binary("mirror-observer"))
        .args([
            "--replay",
            path.to_str().unwrap(),
            "--both",
            "--frames",
            "400",
        ])
        .output()
        .expect("run mirror-observer");
    let _ = std::fs::remove_file(&path);

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "mirror-observer failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("Observer comparison"), "got:\n{stdout}");
    assert!(
        stdout.contains("grouped the machine identically"),
        "the two observers should agree:\n{stdout}"
    );
    // Four SMT pairs, found from a file, by a program with no hardware access.
    assert!(stdout.contains("entity groups found"), "got:\n{stdout}");
}
