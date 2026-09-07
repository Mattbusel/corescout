//! Turning "what happened once" into "what happens repeatedly".
//!
//! # Conservative on purpose
//!
//! Normalisation has two failure modes and they are not symmetric. Normalise
//! too little and a recurring operation looks like fifty unique ones, so
//! nothing is ever learned. Normalise too much and two genuinely different
//! operations merge, so CoreScout learns a pattern that does not exist and
//! recommends it.
//!
//! The second failure is much worse than the first, because the first produces
//! silence and the second produces confident wrong advice. So the rules here
//! keep anything that plausibly changes behaviour: subcommands, flag names,
//! `--release`. They drop only what is obviously incidental: paths, numbers,
//! hashes, temporary directories, and the values attached to flags.

use serde::{Deserialize, Serialize};

/// A normalised form of an action, for counting recurrence.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Fingerprint(String);

impl Fingerprint {
    /// Normalise a command line or tool name.
    pub fn of(name: &str) -> Fingerprint {
        Fingerprint(normalise(name))
    }

    /// The normalised text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether nothing survived normalisation.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Normalise a command line.
pub fn normalise(name: &str) -> String {
    let tokens = split(name);
    let mut out: Vec<String> = Vec::new();
    let mut positional = 0usize;

    for (index, token) in tokens.iter().enumerate() {
        if index == 0 {
            out.push(program(token));
            continue;
        }
        if let Some(flag) = flag_name(token) {
            out.push(flag);
            continue;
        }
        // A bare value after a flag is that flag's argument. Most of those
        // vary between runs of the same operation: paths, numbers, hashes,
        // temporary directories. But some of them *are* the operation:
        // `cargo run --bin gen-schema` and `cargo run --bin server` are two
        // different things, and merging them is the failure that invents
        // patterns rather than the one that produces silence.
        //
        // So the value survives when it looks like a name rather than an
        // argument, by the same test a subcommand has to pass, and never for
        // the flags whose value is free text.
        if let Some(flag) = out.last().filter(|last| last.starts_with('-')) {
            if !FREE_TEXT.contains(&flag.as_str()) && looks_like_a_subcommand(token) {
                out.push(token.to_lowercase());
            }
            continue;
        }
        // Up to two subcommands survive: `cargo build`, `git remote add`.
        if positional < 2 && looks_like_a_subcommand(token) {
            out.push(token.to_lowercase());
            positional += 1;
        }
    }

    out.join(" ")
}

/// Reduce a task description to a class, so the same kind of task can be
/// counted across sessions and across agents.
///
/// Deliberately coarse. This is a bucket for grouping, not an understanding of
/// what the user asked for, and pretending otherwise would be the kind of
/// inference this project does not make.
pub fn classify_task(description: &str) -> String {
    let text = description.to_lowercase();
    // Order is the tie-break, and it is deliberate: "fix the failing test" is
    // a fix, not a test run. The more specific intent comes first.
    const CLASSES: &[(&str, &[&str])] = &[
        (
            "deploy",
            &["deploy", "release", "publish", "ship", "rollout"],
        ),
        ("migrate", &["migrate", "migration", "schema"]),
        ("fix", &["fix", "bug", "broken", "failing", "repair"]),
        (
            "refactor",
            &["refactor", "rename", "restructure", "clean up"],
        ),
        ("install", &["install", "dependency", "dependencies"]),
        (
            "configure",
            &["configure", "config", "setting", "environment"],
        ),
        ("document", &["document", "readme", "docs", "comment"]),
        (
            "investigate",
            &["investigate", "why", "debug", "diagnose", "trace"],
        ),
        ("review", &["review", "audit"]),
        ("build", &["build", "compile", "bundle"]),
        ("test", &["test", "spec", "coverage"]),
        ("implement", &["implement", "create", "write", "feature"]),
    ];
    for (class, words) in CLASSES {
        if words.iter().any(|word| text.contains(word)) {
            return (*class).to_string();
        }
    }
    "other".into()
}

/// Flags whose value is prose, and therefore different every time.
///
/// A commit message kept in a fingerprint would make every commit its own
/// unique operation, and nothing about committing would ever be learned.
const FREE_TEXT: &[&str] = &[
    "-m",
    "--message",
    "--msg",
    "-c",
    "--comment",
    "--description",
];

/// Split on whitespace, keeping quoted runs together.
fn split(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => current.push(c),
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            None => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// The program name, without its directory or extension.
fn program(token: &str) -> String {
    let without_dir = token
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(token)
        .to_lowercase();
    without_dir
        .strip_suffix(".exe")
        .or_else(|| without_dir.strip_suffix(".cmd"))
        .or_else(|| without_dir.strip_suffix(".bat"))
        .unwrap_or(&without_dir)
        .to_string()
}

/// The flag name, if this token is one, with any attached value removed.
fn flag_name(token: &str) -> Option<String> {
    if !token.starts_with('-') || token == "-" || token == "--" {
        return None;
    }
    let name = token.split('=').next().unwrap_or(token);
    Some(name.to_lowercase())
}

/// Whether a bare token looks like a subcommand rather than an argument.
///
/// Subcommands are short words. Paths, versions, hashes, urls and numbers are
/// arguments, and every one of them varies between runs of the same operation.
fn looks_like_a_subcommand(token: &str) -> bool {
    if token.is_empty() || token.len() > 24 {
        return false;
    }
    if token.contains(['/', '\\', '.', ':', '@', '*']) {
        return false;
    }
    if token.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return false;
    }
    // A commit hash reads as a word: short, alphanumeric, no punctuation. It
    // is also different on every run, so treating it as a subcommand splits
    // one recurring operation into a hundred singletons.
    if looks_like_a_hash(token) {
        return false;
    }
    token
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Whether a token is a hex string long enough to be an identifier.
fn looks_like_a_hash(token: &str) -> bool {
    token.len() >= 6
        && token.chars().all(|c| c.is_ascii_hexdigit())
        && token.chars().any(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(a: &str, b: &str) -> bool {
        normalise(a) == normalise(b)
    }

    #[test]
    fn the_same_operation_run_twice_looks_the_same() {
        assert!(same(
            "cargo build --release --target-dir C:\\tmp\\a1b2",
            "cargo build --release --target-dir C:\\tmp\\9f3c"
        ));
    }

    #[test]
    fn a_full_path_to_the_program_does_not_make_it_a_different_program() {
        assert!(same(
            "\"C:\\Program Files\\nodejs\\npm.cmd\" install",
            "npm install"
        ));
        assert!(same("/usr/bin/git status", "git status"));
        assert!(same(
            "C:\\Users\\someone\\.cargo\\bin\\cargo.exe build",
            "cargo build"
        ));
    }

    #[test]
    fn a_flag_value_that_names_the_operation_is_kept() {
        // The failure that matters most. Two different binaries built by one
        // command must not merge into one operation, because CoreScout would
        // then learn a pattern about neither of them.
        assert!(!same(
            "cargo run --bin gen-schema",
            "cargo run --bin server"
        ));
        assert_eq!(
            normalise("cargo run --bin gen-schema"),
            "cargo run --bin gen-schema"
        );
        assert!(!same("npm run build", "npm run dev"));
    }

    #[test]
    fn a_flag_value_that_is_merely_an_argument_is_not_kept() {
        assert!(same(
            "cargo build --target-dir /tmp/a1b2",
            "cargo build --target-dir /tmp/9f3c"
        ));
        assert!(same("ping -n 60 127.0.0.1", "ping -n 4 127.0.0.1"));
    }

    #[test]
    fn a_commit_message_does_not_become_part_of_the_operation() {
        // Otherwise every commit is its own unique operation and nothing about
        // committing is ever learned.
        assert!(same(
            "git commit -m fixed-the-build",
            "git commit -m added-a-test"
        ));
    }

    #[test]
    fn a_flag_that_changes_the_output_is_kept() {
        // A release build and a debug build are different operations, and
        // merging them would let CoreScout recommend a procedure measured on
        // one for the other.
        assert!(!same("cargo build --release", "cargo build"));
        assert!(!same("npm test", "npm test --watch"));
    }

    #[test]
    fn subcommands_are_kept_and_arguments_are_not() {
        assert_eq!(
            normalise("git commit -m \"a message here\""),
            "git commit -m"
        );
        assert_eq!(
            normalise("git remote add origin https://x/y"),
            "git remote add"
        );
        assert!(same("docker run myimage:v1", "docker run myimage:v2"));
    }

    #[test]
    fn only_two_subcommands_survive() {
        // Deeper than that and the tail is almost always arguments.
        assert_eq!(normalise("aws s3 cp source dest"), "aws s3 cp");
    }

    #[test]
    fn numbers_and_hashes_do_not_survive() {
        assert!(same("git checkout a1b2c3d", "git checkout 9f8e7d6"));
        assert!(same("kill 4821", "kill 9123"));
    }

    #[test]
    fn two_genuinely_different_commands_stay_different() {
        // The failure that matters. If this ever passes, CoreScout has started
        // inventing patterns.
        assert!(!same("cargo build", "cargo test"));
        assert!(!same("npm run build", "npm run dev"));
        assert!(!same("git push", "git pull"));
        assert!(!same("terraform plan", "terraform apply"));
    }

    #[test]
    fn quoted_arguments_do_not_split_into_fake_subcommands() {
        assert_eq!(normalise("sh -c \"cargo build && cargo test\""), "sh -c");
    }

    #[test]
    fn a_tool_name_normalises_to_itself() {
        assert_eq!(normalise("Read"), "read");
        assert_eq!(
            normalise("mcp__corescout__status"),
            "mcp__corescout__status"
        );
    }

    #[test]
    fn an_empty_command_produces_an_empty_fingerprint() {
        assert!(Fingerprint::of("").is_empty());
        assert!(Fingerprint::of("   ").is_empty());
    }

    #[test]
    fn a_fingerprint_is_stable_enough_to_be_a_storage_key() {
        let fingerprint = Fingerprint::of("cargo build --release");
        assert_eq!(fingerprint.as_str(), "cargo build --release");
        assert_eq!(fingerprint.to_string(), "cargo build --release");
        let json = serde_json::to_string(&fingerprint).expect("serialise");
        assert_eq!(json, "\"cargo build --release\"");
    }

    #[test]
    fn task_classes_group_the_obvious_cases_and_admit_when_they_cannot() {
        assert_eq!(classify_task("Deploy the API to production"), "deploy");
        assert_eq!(classify_task("fix the failing test"), "fix");
        assert_eq!(classify_task("Why is this slow?"), "investigate");
        assert_eq!(classify_task("mumble mumble"), "other");
    }

    #[test]
    fn task_classification_does_not_pretend_to_understand() {
        // A bucket, not a reading. If a description matches nothing, it says
        // so rather than guessing at the nearest class.
        assert_eq!(classify_task(""), "other");
        assert_eq!(classify_task("qqq zzz"), "other");
    }
}
