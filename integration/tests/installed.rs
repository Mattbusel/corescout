//! What has to be true of a build before it is worth shipping.
//!
//! # Why these are ignored by default
//!
//! They inspect release artefacts that a plain `cargo test` has not built.
//! Running them without those present would fail for the wrong reason, so they
//! are opt-in and the release script runs them:
//!
//! ```text
//! cargo build --release -p corescout-service -p corescout-cli -p corescout-mcp
//! cargo test -p corescout-integration --test installed -- --ignored --nocapture
//! ```
//!
//! # What they check
//!
//! Not that the code is correct — everything else does that. That the *things
//! a user receives* are the things this repository intends them to receive:
//! the four programs exist, they start, they answer `--help` rather than
//! hanging, they do not print protocol to a terminal, and the bridge survives
//! having no service behind it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The programs a user ends up with.
const SHIPPED: &[&str] = &["corescout", "corescout-service", "corescout-mcp"];

fn release_dir() -> PathBuf {
    // `target/release`, from this crate's manifest directory.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workspace root")
        .join("target")
        .join("release")
}

fn program(name: &str) -> PathBuf {
    release_dir().join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    })
}

/// Run something with a ceiling, so a hang is a failure rather than a wait.
fn run(path: &Path, args: &[&str], stdin: Option<&str>) -> (bool, String) {
    let mut command = Command::new(path);
    command
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return (false, format!("could not start: {error}")),
    };
    if let Some(text) = stdin {
        use std::io::Write;
        if let Some(mut pipe) = child.stdin.take() {
            let _ = pipe.write_all(text.as_bytes());
        }
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return (false, "did not finish within 30 seconds".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(error) => return (false, error.to_string()),
        }
    }
    let output = child.wait_with_output().expect("output");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

#[test]
#[ignore = "inspects release artefacts; run after cargo build --release"]
fn every_program_a_user_receives_is_present() {
    for name in SHIPPED {
        let path = program(name);
        assert!(
            path.exists(),
            "{} is missing; build the release profile first",
            path.display()
        );
        let size = std::fs::metadata(&path).expect("metadata").len();
        assert!(
            size > 100_000,
            "{name} is {size} bytes, which is not a program"
        );
    }
}

#[test]
#[ignore = "inspects release artefacts; run after cargo build --release"]
fn every_program_explains_itself_without_hanging() {
    // A binary that blocks on `--help` is one somebody will run in a terminal
    // and have to kill, which is the first impression this product makes.
    for name in SHIPPED {
        let (ok, text) = run(&program(name), &["--help"], None);
        assert!(ok, "{name} --help failed: {text}");
        assert!(
            text.to_lowercase().contains("corescout"),
            "{name} --help said nothing useful: {text}"
        );
        assert!(text.len() > 100, "{name} --help is too terse: {text}");
    }
}

#[test]
#[ignore = "inspects release artefacts; run after cargo build --release"]
fn the_command_line_says_how_to_start_corescout_rather_than_reporting_a_missing_file() {
    // Someone who has installed CoreScout and not opened it yet types
    // `corescout status` first. "No such file or directory" is a useless thing
    // to show them.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let mut command = Command::new(program("corescout"));
    command
        .arg("status")
        .env("CORESCOUT_DATA_DIR", dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = command.output().expect("run");
    let text = String::from_utf8_lossy(&output.stderr).to_lowercase();
    assert!(
        text.contains("not running") || text.contains("corescout service"),
        "it should say what to do: {text}"
    );
}

#[test]
#[ignore = "inspects release artefacts; run after cargo build --release"]
fn the_bridge_speaks_the_protocol_with_no_service_behind_it() {
    // The failure that matters most on someone's first day: they connect their
    // AI before opening the application. The handshake has to complete and the
    // tools have to list, or the client reports a broken server and the user
    // concludes CoreScout does not work.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let script = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"corescout_status","arguments":{}}}"#,
    ]
    .join("\n");

    let mut command = Command::new(program("corescout-mcp"));
    command
        .arg("--no-start")
        .env("CORESCOUT_DATA_DIR", dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().expect("start the bridge");
    {
        use std::io::Write;
        let mut pipe = child.stdin.take().expect("stdin");
        pipe.write_all(script.as_bytes()).expect("write");
        pipe.write_all(b"\n").expect("write");
    }
    let output = child.wait_with_output().expect("output");
    let text = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();

    assert_eq!(
        lines.len(),
        3,
        "one reply per request and none for the notification: {text}"
    );
    for (index, line) in lines.iter().enumerate() {
        let reply: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|_| panic!("line {index} is not JSON: {line}"));
        assert_eq!(reply["id"], serde_json::json!(index + 1));
        assert_eq!(reply["jsonrpc"], "2.0");
    }
    assert!(lines[0].contains("protocolVersion"), "{}", lines[0]);
    assert!(lines[1].contains("corescout_ask"), "{}", lines[1]);
    // The tool fails, and says so as a result rather than as a transport
    // error, so the model can tell the user what to do.
    assert!(lines[2].contains("not running"), "{}", lines[2]);
}

#[test]
#[ignore = "inspects release artefacts; run after cargo build --release"]
fn the_bridge_writes_nothing_but_protocol_to_its_output() {
    // stdout is the transport. One stray line desynchronises the client for
    // the rest of the session, and the symptom is an AI that mysteriously
    // stops being able to use any tool.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let mut command = Command::new(program("corescout-mcp"));
    command
        .arg("--no-start")
        .env("CORESCOUT_DATA_DIR", dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("start the bridge");
    {
        use std::io::Write;
        let mut pipe = child.stdin.take().expect("stdin");
        pipe.write_all(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#)
            .expect("write");
        pipe.write_all(b"\n").expect("write");
    }
    let output = child.wait_with_output().expect("output");
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if line.trim().is_empty() {
            continue;
        }
        serde_json::from_str::<serde_json::Value>(line)
            .unwrap_or_else(|_| panic!("something that is not protocol reached stdout: {line}"));
    }
}

#[test]
#[ignore = "inspects release artefacts; run after cargo build --release"]
fn the_service_runs_a_bounded_pass_and_leaves_a_usable_store_behind() {
    // The closest thing to an installer smoke test that does not need an
    // installer: start the real service against a fresh data directory, let it
    // observe, and check what it left.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let mut command = Command::new(program("corescout-service"));
    command
        .args(["--ticks", "8"])
        .env("CORESCOUT_DATA_DIR", dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let output = command.output().expect("run the service");
    assert!(
        output.status.success(),
        "the service failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    for name in ["corescout.redb", "mirror.ring"] {
        let path = dir.path().join(name);
        assert!(path.exists(), "{name} was not created");
        assert!(std::fs::metadata(&path).expect("metadata").len() > 0);
    }
    // The endpoint file is withdrawn on a clean stop, so a client does not
    // find a port nobody is listening on.
    assert!(
        !dir.path().join("endpoint.json").exists(),
        "a stale endpoint file was left behind"
    );

    // And what it wrote can be read back.
    let store = corescout_storage::Store::open(&dir.path().join("corescout.redb")).expect("reopen");
    assert!(store.event_count().expect("events") > 0);
    assert_eq!(
        store.schema_version().expect("schema"),
        corescout_storage::SCHEMA_VERSION
    );
}

#[test]
#[ignore = "inspects release artefacts; run after the desktop bundle"]
fn the_installers_are_the_size_a_person_would_expect() {
    // Tauri uses the system web view, so this should be about two megabytes.
    // A hundred means something bundled a browser, and the download page is
    // then telling people something false.
    let bundle = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workspace root")
        .join("apps/corescout-desktop/src-tauri/target/release/bundle");
    if !bundle.exists() {
        eprintln!(
            "no bundle at {}; run scripts/release.ps1 first",
            bundle.display()
        );
        return;
    }
    let mut found = 0;
    for (directory, extension) in [("nsis", "exe"), ("msi", "msi")] {
        let Ok(entries) = std::fs::read_dir(bundle.join(directory)) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            if entry.path().extension().and_then(|e| e.to_str()) != Some(extension) {
                continue;
            }
            found += 1;
            let size = entry.metadata().expect("metadata").len();
            assert!(size > 500_000, "{:?} is {size} bytes", entry.file_name());
            assert!(
                size < 40_000_000,
                "{:?} is {size} bytes, which means something bundled a browser",
                entry.file_name()
            );
        }
    }
    assert!(
        found > 0,
        "the bundle directory exists and holds no installer"
    );
}
