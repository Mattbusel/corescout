//! Black-box tests of the built binary.
//!
//! These check the things a user hits in the first thirty seconds: help, exit
//! codes, and whether an unsupported platform says so clearly instead of
//! panicking.

use std::process::Command;

fn corescout(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_corescout"))
        .args(args)
        .output()
        .expect("run corescout");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[test]
fn help_lists_the_commands_and_exits_zero() {
    let (code, stdout, _) = corescout(&["--help"]);
    assert_eq!(code, 0);
    for command in ["info", "benchmark", "analyze", "run"] {
        assert!(stdout.contains(command), "help omits `{command}`");
    }
}

#[test]
fn bare_invocation_shows_help_rather_than_an_error() {
    let (code, stdout, _) = corescout(&[]);
    assert_eq!(code, 0);
    assert!(stdout.contains("USAGE"));
}

#[test]
fn version_is_reported() {
    let (code, stdout, _) = corescout(&["--version"]);
    assert_eq!(code, 0);
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn an_unknown_command_fails_with_a_message_not_a_panic() {
    let (code, _, stderr) = corescout(&["overclock"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("unknown command"));
    assert!(
        !stderr.contains("panicked"),
        "a bad argument must not panic"
    );
}

#[test]
fn run_without_a_program_is_rejected_before_benchmarking_anything() {
    // The program is the one argument `run` cannot default. Rejecting it here
    // matters because the alternative is a full benchmark pass followed by the
    // discovery that there was nothing to launch.
    let (code, _, stderr) = corescout(&["run", "--profile", "latency"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("needs a program"), "{stderr}");
}

#[test]
fn an_unknown_profile_names_the_ones_that_exist() {
    let (code, _, stderr) = corescout(&["run", "--profile", "fastest", "--", "/bin/true"]);
    assert_eq!(code, 1);
    // On a machine where the benchmark cannot run, the platform error comes
    // first; either way it must not launch anything or panic.
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(
        stderr.contains("fastest") || stderr.contains("unsupported"),
        "{stderr}"
    );
}

#[test]
#[cfg(not(target_os = "linux"))]
fn unsupported_platforms_say_so_and_point_at_the_port() {
    // The stub backend exists so this is a clear message rather than a
    // link-time failure or a panic.
    let (code, _, stderr) = corescout(&["info"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("Linux"), "got: {stderr}");
}

#[test]
#[cfg(target_os = "linux")]
fn info_describes_the_real_machine() {
    let (code, stdout, stderr) = corescout(&["info"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("Physical cores"));
    assert!(stdout.contains("Logical CPUs"));
}

#[test]
#[cfg(target_os = "linux")]
fn info_json_is_valid_json() {
    let (code, stdout, _) = corescout(&["info", "--json"]);
    assert_eq!(code, 0);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert!(!value["logical_cpus"].as_array().unwrap().is_empty());
}
