//! `corescout-demo`: the same work, twice, with and without an experienced
//! computer.
//!
//! # What is real here
//!
//! The repository is real and it is really built, by `cargo`, on this machine.
//! The trap is real: a source file is generated from a schema, and when the
//! schema changes and that file is not regenerated, the crate still compiles
//! and its test fails, because a constant is stale rather than absent. The
//! failures are real and the timings are real wall clock.
//!
//! That shape is deliberate. A missing file is a failure anybody notices; a
//! stale constant is a program that builds cleanly and is wrong, which is the
//! class of failure this whole product is about.
//!
//! CoreScout is real: the same engine, the same experience layer, the same
//! science crate that refuses to compute an effect without randomised trials.
//!
//! # What is not
//!
//! The agent. There is no model in this loop; a small program stands in for
//! one, and it behaves the way a competent agent that has not been told about
//! the trap behaves — it builds, it fails, it tries again, and after a couple
//! of attempts it works out that something needs regenerating.
//!
//! That means **token usage is not measured here, because nothing spends any**.
//! What is measured is what this harness can actually observe: tasks that
//! ended in a working build, how many builds it took, how many of them failed,
//! and how long it all took. A demonstration with a real model attached would
//! measure more, and would be a different and less reproducible thing.
//!
//! # The comparison
//!
//! Both conditions run the identical agent against the identical repository.
//! The only difference is whether the CoreScout it is talking to has worked on
//! this repository before.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use corescout_product_api::{Api, Engine};
use corescout_storage::{Ring, Store};
use serde_json::{json, Value};

const USAGE: &str = "\
corescout-demo - the same work, twice, with and without an experienced computer

USAGE:
    corescout-demo [OPTIONS]

OPTIONS:
    --tasks <N>      Tasks in the measured run (default 12)
    --warmup <N>     Tasks the experienced CoreScout gets first (default 260)
    --keep           Leave the scratch repository behind for inspection
    -h, --help       Show this help

This really builds a really broken crate, many times. It takes a few minutes
and it uses a core while it does.
";

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return std::process::ExitCode::SUCCESS;
    }
    let tasks = number(&args, "--tasks").unwrap_or(12);
    // Enough tasks for the randomised arms to fill. The exploration rate is
    // deliberately small -- every randomised trial is one where CoreScout may
    // withhold something that would have helped -- so reaching a verified
    // conclusion takes a couple of hundred occasions, and a demo that stopped
    // before then would only ever show the correlation.
    let warmup = number(&args, "--warmup").unwrap_or(260);
    let keep = args.iter().any(|a| a == "--keep");

    match run(tasks, warmup, keep) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("corescout-demo: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(tasks: u64, warmup: u64, keep: bool) -> Result<(), String> {
    let scratch = std::env::temp_dir().join(format!("corescout-demo-{}", std::process::id()));
    let repo = scratch.join("trapped");
    write_repository(&repo)?;
    println!("A real crate with a real trap, at {}", repo.display());
    println!("Building it once to warm the compiler cache, so the comparison is fair.\n");
    // Without this the first condition pays for downloading and compiling
    // whatever cargo needs, and the second does not, and the whole comparison
    // is about that instead.
    regenerate(&repo)?;
    let _ = build(&repo);

    println!("Condition A: a fresh CoreScout that has never seen this repository.");
    let fresh = Harness::open("fresh")?;
    let a = fresh.work(&repo, tasks)?;
    report("fresh", &a);

    println!("\nCondition B: a CoreScout that has worked here before.");
    println!("  Warming up over {warmup} tasks; this is the part that takes a while.");
    let experienced = Harness::open("experienced")?;
    experienced.work(&repo, warmup)?;
    let learned = experienced.learned();
    println!("  It came out of that with {learned}.\n");
    let b = experienced.work(&repo, tasks)?;
    report("experienced", &b);

    println!("\n{}", compare(&a, &b));
    println!("{}", experienced.what_it_knows());

    if keep {
        println!("\nThe scratch repository is at {}", repo.display());
    } else {
        let _ = std::fs::remove_dir_all(&scratch);
    }
    Ok(())
}

/// What one condition produced.
#[derive(Debug, Default, Clone)]
struct Result_ {
    tasks: u64,
    /// Tasks that ended with a working build.
    succeeded: u64,
    /// Builds run, including the ones that failed.
    builds: u64,
    /// Builds that failed.
    failed: u64,
    /// Builds that were a second or later attempt at the same thing.
    retries: u64,
    elapsed: Duration,
}

fn report(name: &str, result: &Result_) {
    println!(
        "  {name:<14} {}/{} tasks worked, {} builds ({} failed, {} retries), {:.1}s",
        result.succeeded,
        result.tasks,
        result.builds,
        result.failed,
        result.retries,
        result.elapsed.as_secs_f64()
    );
}

fn compare(a: &Result_, b: &Result_) -> String {
    let rate = |r: &Result_| {
        if r.builds == 0 {
            0.0
        } else {
            r.failed as f64 / r.builds as f64
        }
    };
    let mut out = String::from("Same agent, same repository, same trap.\n\n");
    out.push_str(&format!(
        "                     fresh    experienced\n  \
         tasks completed  {:>8}   {:>12}\n  \
         builds run       {:>8}   {:>12}\n  \
         builds failed    {:>8}   {:>12}\n  \
         failure rate     {:>7.0}%   {:>11.0}%\n  \
         seconds          {:>8.1}   {:>12.1}\n",
        a.succeeded,
        b.succeeded,
        a.builds,
        b.builds,
        a.failed,
        b.failed,
        rate(a) * 100.0,
        rate(b) * 100.0,
        a.elapsed.as_secs_f64(),
        b.elapsed.as_secs_f64(),
    ));
    // Stated as what it is. A single paired run on one machine is a
    // demonstration, not a measurement, and calling it one would be exactly
    // the kind of claim this project spent its research phase refusing to make.
    out.push_str(
        "\nThis is one paired run on one machine. It shows the mechanism working; it is not \
         a measurement of how much CoreScout helps in general, and the numbers will move \
         between runs.\n",
    );
    out
}

/// A CoreScout, and the agent that works against it.
struct Harness {
    _dir: tempfile::TempDir,
    api: Api,
    session: String,
}

impl Harness {
    fn open(session: &str) -> Result<Harness, String> {
        let dir = tempfile::tempdir().map_err(|error| error.to_string())?;
        let store = Store::open(&dir.path().join("corescout.redb")).map_err(|e| e.to_string())?;
        let ring = Ring::open(&dir.path().join("mirror.ring"), 512).map_err(|e| e.to_string())?;
        let engine = Engine::open(store, ring).map_err(|e| e.to_string())?;
        let api = Api::new(Arc::new(Mutex::new(engine)));
        api.call("hello", &json!({ "name": "demo-agent" }))
            .map_err(|e| e.to_string())?;
        Ok(Harness {
            _dir: dir,
            api,
            session: session.to_string(),
        })
    }

    fn call(&self, method: &str, params: Value) -> Value {
        self.api.call(method, &params).unwrap_or(Value::Null)
    }

    /// Run `tasks` tasks, the way an agent would.
    ///
    /// Each task changes the schema, the way a person editing the project
    /// would, and then the agent has to get the crate building again.
    fn work(&self, repo: &Path, tasks: u64) -> Result<Result_, String> {
        let mut result = Result_ {
            tasks,
            ..Result_::default()
        };
        let started = Instant::now();

        for task in 0..tasks {
            change_schema(repo, task)?;
            let mut attempt = 0;
            loop {
                let advice = self.call(
                    "advise",
                    json!({
                        "session": self.session,
                        "operation": "cargo build",
                        "workspace": "trapped",
                    }),
                );
                let suggested = advice["has_advice"] == json!(true);

                // What a competent agent does with no information: try it,
                // and after it fails, work out what is stale. What it does
                // with information: the thing that works, first time.
                let regenerate_first = suggested || attempt > 0;
                if regenerate_first {
                    regenerate(repo)?;
                    self.report("cargo run --bin generate", false, task, attempt);
                }

                let ok = build(repo);
                result.builds += 1;
                if !ok {
                    result.failed += 1;
                }
                if attempt > 0 {
                    result.retries += 1;
                }
                self.report("cargo build", !ok, task, attempt);

                if ok {
                    result.succeeded += 1;
                    break;
                }
                attempt += 1;
                if attempt > 3 {
                    // Given up. A real agent would ask the user at this point,
                    // which is the intervention this is standing in for.
                    break;
                }
            }
        }
        result.elapsed = started.elapsed();
        Ok(result)
    }

    /// Tell CoreScout what just happened, exactly as the MCP tool would.
    fn report(&self, name: &str, failed: bool, task: u64, attempt: u64) {
        self.call(
            "observe",
            json!({
                "session": self.session,
                "name": name,
                "workspace": "trapped",
                "kind": "build",
                "reported": if failed { "failure" } else { "success" },
                "detail": if failed { "the generated source did not match the schema" } else { "" },
                // Verified either way: the compiler either produced a binary
                // or it did not, and this harness checks which.
                "verified": if failed { "contradicted" } else { "confirmed" },
                "verification_detail": if failed { "cargo exited non-zero" } else { "" },
                // Spaced past the window CoreScout looks back over, so a
                // precursor from the previous task is not counted for this one.
                "started_ms": 1_700_000_000_000u64 + task * 15 * 60_000 + attempt * 60_000,
            }),
        );
    }

    fn learned(&self) -> String {
        let capabilities = self.call("capabilities", json!({}));
        let verified = capabilities.as_array().map(Vec::len).unwrap_or(0);
        let cards = self.call("learned", json!({}));
        let noticed = cards.as_array().map(Vec::len).unwrap_or(0);
        format!(
            "{noticed} thing{} noticed, {verified} of which verified",
            if noticed == 1 { "" } else { "s" }
        )
    }

    fn what_it_knows(&self) -> String {
        let mut out = String::from("\nWhat the experienced CoreScout ended up with:\n");
        for card in self
            .call("learned", json!({}))
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            out.push_str(&format!(
                "  [{}] {}\n      {}\n",
                card["basis"].as_str().unwrap_or("?"),
                card["title"].as_str().unwrap_or(""),
                card["detail"].as_str().unwrap_or(""),
            ));
        }
        for question in self
            .call("hypotheses", json!({}))
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            out.push_str(&format!(
                "  [still testing] {}\n      {}\n",
                question["name"].as_str().unwrap_or(""),
                question["missing"].as_str().unwrap_or(""),
            ));
        }
        out
    }
}

/* ------------------------------------------------------------ the repository */

/// A crate with a real trap in it.
///
/// `src/generated.rs` is produced from `schema.txt` by `src/bin/generate.rs`.
/// When the schema changes and the file is not regenerated, the crate compiles
/// perfectly and its test fails, because `LATEST` still holds what the schema
/// used to say.
///
/// The generated file is seeded here to match the initial schema, so the crate
/// is valid from the start. A generator that cannot run until it has already
/// run is not a trap, it is a bootstrapping problem, and it took one real run
/// of this demo to find that out.
fn write_repository(repo: &Path) -> Result<(), String> {
    let write = |path: PathBuf, body: &str| -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(&path, body).map_err(|error| format!("{}: {error}", path.display()))
    };

    write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"trapped\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [workspace]\n\n[[bin]]\nname = \"generate\"\npath = \"src/bin/generate.rs\"\n\n\
         [lib]\npath = \"src/lib.rs\"\n",
    )?;
    write(repo.join("schema.txt"), "field_0\n")?;
    write(
        repo.join("src/generated.rs"),
        "pub const FIELDS: [&str; 1] = [\"field_0\"];\npub const LATEST: &str = \"field_0\";\n",
    )?;
    write(
        repo.join("src/lib.rs"),
        "//! A crate whose source is partly generated from `schema.txt`.\n\
         //!\n\
         //! Change the schema without running the generator and this still\n\
         //! compiles: `LATEST` is simply stale, and the test is what notices.\n\
         \n\
         mod generated;\n\n\
         pub fn check() -> usize {\n    generated::FIELDS.len()\n}\n\n\
         pub use generated::LATEST;\n",
    )?;
    write(
        repo.join("src/bin/generate.rs"),
        "//! Writes `src/generated.rs` from `schema.txt`.\n\
         \n\
         fn main() {\n    \
             let schema = std::fs::read_to_string(\"schema.txt\").expect(\"schema.txt\");\n    \
             let fields: Vec<&str> = schema.lines().filter(|l| !l.trim().is_empty()).collect();\n    \
             let latest = fields.last().copied().unwrap_or(\"none\");\n    \
             let body = format!(\n        \
                 \"pub const FIELDS: [&str; {}] = [{}];\\npub const LATEST: &str = \\\"{}\\\";\\n\",\n        \
                 fields.len(),\n        \
                 fields.iter().map(|f| format!(\"\\\"{f}\\\"\")).collect::<Vec<_>>().join(\", \"),\n        \
                 latest\n    \
             );\n    \
             std::fs::write(\"src/generated.rs\", body).expect(\"write generated.rs\");\n\
         }\n",
    )?;
    write(
        repo.join("tests/schema.rs"),
        "//! The check that makes a stale generated file a compile error rather\n\
         //! than a silently wrong program.\n\
         \n\
         #[test]\n\
         fn the_generated_source_matches_the_schema() {\n    \
             let schema = std::fs::read_to_string(\"schema.txt\").expect(\"schema.txt\");\n    \
             let last = schema.lines().filter(|l| !l.trim().is_empty()).last().unwrap_or(\"none\");\n    \
             assert_eq!(trapped::LATEST, last);\n\
         }\n",
    )?;
    Ok(())
}

/// Edit the schema, the way somebody working on the project would.
fn change_schema(repo: &Path, task: u64) -> Result<(), String> {
    let mut body = String::new();
    for field in 0..=task {
        body.push_str(&format!("field_{field}\n"));
    }
    std::fs::write(repo.join("schema.txt"), body).map_err(|error| error.to_string())
}

fn regenerate(repo: &Path) -> Result<(), String> {
    cargo(repo, &["run", "--quiet", "--bin", "generate"])
        .then_some(())
        .ok_or_else(|| "the generator itself failed".to_string())
}

fn build(repo: &Path) -> bool {
    // `cargo test` rather than `cargo build`: the trap is a constant that is
    // stale rather than absent, so the crate compiles either way and the test
    // is what notices. A build that succeeds while the program is wrong is
    // exactly the shape of failure this whole product is about.
    cargo(repo, &["test", "--quiet"])
}

fn cargo(repo: &Path, args: &[&str]) -> bool {
    Command::new("cargo")
        .args(args)
        .current_dir(repo)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn number(args: &[String], name: &str) -> Option<u64> {
    args.iter()
        .position(|a| a == name)
        .and_then(|at| args.get(at + 1))
        .and_then(|value| value.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generated_file_is_seeded_so_the_crate_is_valid_from_the_start() {
        // A generator that cannot run until it has already run is not a trap,
        // it is a bootstrapping problem. One real run of the demo found it.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let repo = dir.path().join("trapped");
        write_repository(&repo).expect("write");
        let generated = std::fs::read_to_string(repo.join("src/generated.rs")).expect("read");
        assert!(generated.contains("field_0"), "{generated}");
        let schema = std::fs::read_to_string(repo.join("schema.txt")).expect("read");
        assert_eq!(schema.trim(), "field_0", "the seed matches the schema");
    }

    #[test]
    fn the_repository_it_writes_is_a_real_crate_with_a_real_trap() {
        // Not a fixture that pretends to fail. The generator produces a
        // constant, the test asserts on it, and a stale file makes the test
        // fail for a reason a compiler can see.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let repo = dir.path().join("trapped");
        write_repository(&repo).expect("write");

        for name in [
            "Cargo.toml",
            "schema.txt",
            "src/lib.rs",
            "src/bin/generate.rs",
        ] {
            assert!(repo.join(name).exists(), "{name} is missing");
        }
        let generator = std::fs::read_to_string(repo.join("src/bin/generate.rs")).expect("read");
        assert!(generator.contains("schema.txt"));
        assert!(generator.contains("generated.rs"));
    }

    #[test]
    fn changing_the_schema_changes_what_the_generator_would_produce() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let repo = dir.path().join("trapped");
        write_repository(&repo).expect("write");
        change_schema(&repo, 3).expect("change");
        let schema = std::fs::read_to_string(repo.join("schema.txt")).expect("read");
        assert_eq!(schema.lines().count(), 4);
        assert!(schema.contains("field_3"));
    }

    #[test]
    fn the_comparison_says_what_it_is_and_what_it_is_not() {
        // A single paired run is a demonstration. Reporting it as a
        // measurement is the claim this project exists to not make.
        let a = Result_ {
            tasks: 12,
            succeeded: 12,
            builds: 24,
            failed: 12,
            retries: 12,
            elapsed: Duration::from_secs(60),
        };
        let b = Result_ {
            tasks: 12,
            succeeded: 12,
            builds: 12,
            failed: 0,
            retries: 0,
            elapsed: Duration::from_secs(40),
        };
        let text = compare(&a, &b);
        assert!(text.contains("one paired run"), "{text}");
        assert!(text.contains("not a measurement"), "{text}");
        // Twelve failures out of twenty-four builds is a failure rate of 50%,
        // not of 100%: the rate is per build, because that is the number that
        // says how often the agent had to try again.
        assert!(text.contains("50%"), "{text}");
        assert!(text.contains("0%"), "{text}");
    }
}
