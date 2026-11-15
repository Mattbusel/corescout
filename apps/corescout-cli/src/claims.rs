//! Done-claims in Claude Code transcripts, and whether anything backed them.
//!
//! An assistant that writes "all tests pass" has made a claim about the world.
//! The transcript says whether it ran anything that could have told it so. This
//! module reads one transcript (Claude Code's JSON-lines format, one entry per
//! line) and answers, turn by turn:
//!
//! * did the assistant claim success ("tests pass", "done", "fixed",
//!   "deployed", "all green", and close variants), and
//! * did a check pass in that turn or in the turn just before it: a `Bash` or
//!   `PowerShell` tool call that ran tests, a build or a linter and came back
//!   with exit code 0.
//!
//! A *turn* starts at a prompt the user typed and runs until the next one. Tool
//! results arrive as `user` entries too, so they do not start a turn. Only the
//! last claim in a turn is counted, because that is the one the user reads, and
//! a turn that narrates "Fixed. Now let me run the tests." and then runs them
//! is judged by the whole turn, not by the sentence.
//!
//! Nothing here touches the filesystem; [`analyse`] takes the transcript text,
//! which is what lets the tests use synthetic transcripts.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

/// One shell command the assistant ran, and what came of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Run {
    /// The command line, as the assistant wrote it.
    pub command: String,
    /// The exit code, when the transcript says. `None` for a call that was
    /// refused, interrupted, sent to the background or never answered.
    pub exit: Option<i64>,
    /// Whether the command runs tests, a build or a linter.
    pub check: bool,
    /// The exit code was 0 but the output carries an unambiguous failure
    /// marker, which is what `cargo test | tail` does to an exit code.
    pub failure_in_output: bool,
}

impl Run {
    /// A check that ran and passed.
    pub fn passed(&self) -> bool {
        self.check && self.exit == Some(0) && !self.failure_in_output
    }
}

/// One assistant turn that claimed success.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Claim {
    /// The Claude Code session the turn belongs to.
    pub session: String,
    /// The timestamp of the message that made the claim, as written.
    pub timestamp: String,
    /// The same, in milliseconds since the Unix epoch, when it parses.
    pub at_ms: Option<i64>,
    /// The sentence that made the claim.
    pub text: String,
    /// Which kind of claim it was: "tests pass", "done", "fixed", ...
    pub phrase: &'static str,
    /// Whether a check passed in this turn or the one before it.
    pub backed: bool,
    /// The last command run before the claim, in this turn or the one before.
    pub last_command: Option<String>,
    /// That command's exit code.
    pub last_exit: Option<i64>,
}

/// The state of the last turn in a transcript, which is what a Stop hook
/// judges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalTurn {
    /// The last text the assistant wrote in the turn.
    pub last_text: Option<String>,
    /// Whether a check passed anywhere in the turn.
    pub passed: bool,
    /// The last command run in the turn.
    pub last_run: Option<Run>,
}

/// What one transcript says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Transcript {
    /// Every turn that claimed success, in order.
    pub claims: Vec<Claim>,
    /// The last turn, finished or not.
    pub final_turn: FinalTurn,
}

#[derive(Default)]
struct Turn {
    runs: Vec<Run>,
    claim: Option<Claim>,
    last_text: Option<String>,
}

impl Turn {
    fn passed(&self) -> bool {
        self.runs.iter().any(Run::passed)
    }
}

/// Read one transcript.
///
/// Lines that are not JSON, entries that are neither `user` nor `assistant`,
/// and subagent entries mixed into a main session are skipped. A file that is
/// all subagent entries (Claude Code writes those to their own files) is read
/// as a conversation of its own.
pub fn analyse(transcript: &str) -> Transcript {
    let mut claims = Vec::new();
    let mut previous = Turn::default();
    let mut current = Turn::default();
    // tool_use id -> (command, is check, sent to background)
    let mut pending: HashMap<String, (String, bool, bool)> = HashMap::new();
    let mut sidechain: Option<bool> = None;

    for line in transcript.lines() {
        // Most of a transcript's bytes are in entries this never reads, and
        // parsing them is most of the cost. A top-level `"type":"user"` or
        // `"type":"assistant"` is never escaped, so this cannot skip one.
        if !line.contains("\"type\":\"user\"") && !line.contains("\"type\":\"assistant\"") {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = entry["type"].as_str().unwrap_or_default();
        if kind != "user" && kind != "assistant" {
            continue;
        }
        let side = entry["isSidechain"].as_bool().unwrap_or(false);
        match sidechain {
            None => sidechain = Some(side),
            Some(first) if first != side => continue,
            Some(_) => {}
        }
        if entry["isMeta"].as_bool() == Some(true)
            || entry["isCompactSummary"].as_bool() == Some(true)
        {
            continue;
        }
        let content = &entry["message"]["content"];

        if kind == "user" {
            let blocks = content.as_array();
            let results: Vec<&Value> = blocks
                .map(|b| {
                    b.iter()
                        .filter(|block| block["type"] == "tool_result")
                        .collect()
                })
                .unwrap_or_default();
            if results.is_empty() {
                // Something the user typed: a new turn.
                previous = close(std::mem::take(&mut current), &previous, &mut claims);
                continue;
            }
            for block in results {
                let id = block["tool_use_id"].as_str().unwrap_or_default();
                let Some((command, check, background)) = pending.remove(id) else {
                    continue;
                };
                let output = content_text(&block["content"]);
                let exit = if background {
                    None
                } else {
                    exit_code(block, &entry, &output)
                };
                let failure_in_output = check && exit == Some(0) && shows_failure(&output);
                current.runs.push(Run {
                    command,
                    exit,
                    check,
                    failure_in_output,
                });
            }
            continue;
        }

        // An assistant entry: tool calls and text, in order.
        let Some(blocks) = content.as_array() else {
            continue;
        };
        for block in blocks {
            match block["type"].as_str() {
                Some("tool_use") => {
                    let name = block["name"].as_str().unwrap_or_default();
                    if !is_shell_tool(name) {
                        continue;
                    }
                    let command = block["input"]["command"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    let background = block["input"]["run_in_background"].as_bool() == Some(true);
                    let check = is_check_command(&command);
                    if let Some(id) = block["id"].as_str() {
                        pending.insert(id.to_string(), (command, check, background));
                    }
                }
                Some("text") => {
                    let text = block["text"].as_str().unwrap_or_default();
                    if text.trim().is_empty() {
                        continue;
                    }
                    current.last_text = Some(text.to_string());
                    if let Some((sentence, phrase)) = find_claim(text) {
                        let last = current.runs.last().or(previous.runs.last());
                        current.claim = Some(Claim {
                            session: entry["sessionId"].as_str().unwrap_or_default().to_string(),
                            timestamp: entry["timestamp"].as_str().unwrap_or_default().to_string(),
                            at_ms: entry["timestamp"].as_str().and_then(parse_timestamp_ms),
                            text: sentence,
                            phrase,
                            backed: false,
                            last_command: last.map(|run| run.command.clone()),
                            last_exit: last.and_then(|run| run.exit),
                        });
                    }
                }
                _ => {}
            }
        }
    }

    let final_turn = FinalTurn {
        last_text: current.last_text.clone(),
        passed: current.passed(),
        last_run: current.runs.last().cloned(),
    };
    close(current, &previous, &mut claims);
    Transcript { claims, final_turn }
}

/// End a turn: its claim, if it made one, is backed when a check passed in it
/// or in the turn before. Returns the turn so it can become `previous`.
fn close(mut turn: Turn, previous: &Turn, claims: &mut Vec<Claim>) -> Turn {
    if let Some(mut claim) = turn.claim.take() {
        claim.backed = turn.passed() || previous.passed();
        claims.push(claim);
    }
    turn
}

/// The tools whose calls are shell commands.
fn is_shell_tool(name: &str) -> bool {
    matches!(name, "Bash" | "PowerShell")
}

/// The text of a tool result, which is a string or a list of text blocks.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The exit code of a shell tool result.
///
/// Claude Code marks a non-zero exit with `is_error` and an `Exit code N`
/// first line. A result without `is_error` exited 0, unless it was
/// interrupted. An error without an exit code (a hook refused the call, the
/// user rejected it) never ran, so it has none.
fn exit_code(block: &Value, entry: &Value, output: &str) -> Option<i64> {
    if block["is_error"].as_bool() != Some(true) {
        if entry["toolUseResult"]["interrupted"].as_bool() == Some(true) {
            return None;
        }
        return Some(0);
    }
    let first = output.trim_start().lines().next().unwrap_or_default();
    first
        .strip_prefix("Exit code ")
        .and_then(|rest| rest.trim().parse().ok())
}

/// Output that says a check failed whatever the exit code says.
fn shows_failure(output: &str) -> bool {
    const MARKERS: &[&str] = &[
        "test result: FAILED",
        "error: could not compile",
        "npm ERR!",
        "ERR_PNPM_",
        "short test summary info",
        "Tests failed",
        "BUILD FAILED",
        "Build FAILED",
    ];
    MARKERS.iter().any(|marker| output.contains(marker))
}

/// Split a command line into words, keeping quoted runs together.
fn words(segment: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    for ch in segment.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => word.push(ch),
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch.is_whitespace() || ch == '(' || ch == ')' || ch == '{' || ch == '}' => {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
            }
            None => word.push(ch),
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// A program's name without its directory or Windows extension.
fn program(word: &str) -> &str {
    let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
    for ext in [".exe", ".cmd", ".bat", ".ps1", ".sh"] {
        if let Some(stripped) = base.strip_suffix(ext) {
            return stripped;
        }
    }
    base
}

/// Whether a command line runs tests, a build or a linter.
///
/// Each segment between `&&`, `||`, `;` and `|` is judged on its own, so
/// `cd app && npm test` counts and `echo cargo test` does not.
pub fn is_check_command(command: &str) -> bool {
    let lowered = command.to_lowercase();
    lowered
        .split(['&', '|', ';', '\n'])
        .any(|segment| segment_is_check(&words(segment)))
}

fn script_is_check(script: &str) -> bool {
    [
        "test",
        "build",
        "lint",
        "check",
        "typecheck",
        "tsc",
        "verify",
        "e2e",
    ]
    .iter()
    .any(|prefix| script.starts_with(prefix))
}

fn segment_is_check(tokens: &[String]) -> bool {
    let mut at = 0;
    // Environment assignments and wrappers that change nothing about what runs.
    while let Some(token) = tokens.get(at) {
        let name = program(token);
        let assignment = token.contains('=') && !token.starts_with('-');
        let wrapper = matches!(name, "env" | "time" | "sudo" | "nice" | "exec" | "command");
        if assignment || wrapper {
            at += 1;
        } else if name == "timeout" {
            at += 1;
            if tokens
                .get(at)
                .is_some_and(|t| t.chars().any(|c| c.is_ascii_digit()))
            {
                at += 1;
            }
        } else if name == "corescout" && tokens.get(at + 1).map(String::as_str) == Some("run") {
            match tokens[at..].iter().position(|t| t == "--") {
                Some(dash) => at += dash + 1,
                None => return false,
            }
        } else {
            break;
        }
    }
    let Some(first) = tokens.get(at) else {
        return false;
    };
    let prog = program(first);
    let all: Vec<&str> = tokens[at + 1..].iter().map(String::as_str).collect();
    let rest: Vec<&str> = all
        .iter()
        .copied()
        .filter(|t| !t.starts_with('-') && !t.starts_with('+'))
        .collect();
    let sub = rest.first().copied().unwrap_or_default();
    let has = |flag: &str| all.contains(&flag);

    match prog {
        "cargo" => {
            matches!(
                sub,
                "test" | "t" | "build" | "b" | "check" | "c" | "clippy" | "nextest" | "miri"
            ) || (sub == "fmt" && has("--check"))
        }
        "npm" => match sub {
            "test" | "t" | "tst" => true,
            "run" | "run-script" => rest.get(1).is_some_and(|s| script_is_check(s)),
            _ => false,
        },
        "pnpm" | "yarn" | "bun" => match sub {
            "run" => rest.get(1).is_some_and(|s| script_is_check(s)),
            "dlx" | "x" | "exec" => rest.get(1).is_some_and(|tool| direct_check(tool, &all)),
            other => script_is_check(other),
        },
        "npx" | "pnpx" | "bunx" => direct_check(program(sub), &all),
        "go" => matches!(sub, "test" | "build" | "vet"),
        "make" | "gmake" | "nmake" | "mingw32-make" | "just" | "ninja" => {
            !matches!(sub, "clean" | "install" | "distclean" | "uninstall")
        }
        "gradle" | "gradlew" | "mvn" | "mvnw" => rest.iter().any(|t| {
            matches!(
                *t,
                "test" | "build" | "check" | "verify" | "compile" | "assemble" | "package"
            )
        }),
        "dotnet" | "swift" => matches!(sub, "test" | "build"),
        "deno" => matches!(sub, "test" | "check" | "lint"),
        "mix" => matches!(sub, "test" | "compile"),
        "bazel" | "bazelisk" => matches!(sub, "test" | "build"),
        "cmake" => has("--build"),
        "node" => has("--test"),
        "invoke-pester" => true,
        name if name.starts_with("python") || name == "py" => all
            .iter()
            .position(|t| *t == "-m")
            .and_then(|m| all.get(m + 1))
            .is_some_and(|module| {
                matches!(
                    *module,
                    "pytest"
                        | "unittest"
                        | "mypy"
                        | "ruff"
                        | "pyright"
                        | "flake8"
                        | "pylint"
                        | "tox"
                        | "nox"
                        | "compileall"
                        | "py_compile"
                        | "build"
                )
            }),
        other => direct_check(other, &all),
    }
}

/// Programs that are checks by themselves.
fn direct_check(name: &str, all: &[&str]) -> bool {
    match name {
        "pytest" | "tox" | "nox" | "jest" | "vitest" | "mocha" | "ava" | "tsc" | "vue-tsc"
        | "svelte-check" | "eslint" | "ruff" | "mypy" | "pyright" | "flake8" | "pylint"
        | "rspec" | "phpunit" | "ctest" | "xcodebuild" | "msbuild" | "golangci-lint"
        | "shellcheck" | "playwright" | "cypress" | "biome" | "clippy-driver" => true,
        "black" | "prettier" | "rustfmt" => all.contains(&"--check"),
        _ => false,
    }
}

/// The kinds of claim, each with the word sequences that make it.
const PHRASES: &[(&str, &[&[&str]])] = &[
    (
        "tests pass",
        &[
            &["tests", "pass"],
            &["tests", "passed"],
            &["tests", "passing"],
            &["tests", "are", "passing"],
            &["tests", "now", "pass"],
            &["tests", "all", "pass"],
            &["tests", "are", "all", "passing"],
            &["tests", "are", "now", "passing"],
            &["tests", "now", "passing"],
            &["test", "passes"],
            &["test", "passed"],
            &["suite", "passes"],
            &["suite", "passed"],
            &["tests", "green"],
            &["tests", "are", "green"],
            &["tests", "are", "all", "green"],
            &["tests", "are", "now", "green"],
            &["tests", "all", "green"],
            &["tests", "now", "green"],
            &["tests", "were", "green"],
        ],
    ),
    ("all green", &[&["all", "green"], &["everything", "green"]]),
    (
        "build passes",
        &[
            &["build", "passes"],
            &["build", "succeeds"],
            &["build", "succeeded"],
            &["builds", "cleanly"],
            &["compiles", "cleanly"],
            &["build", "is", "clean"],
        ],
    ),
    ("deployed", &[&["deployed"]]),
    ("fixed", &[&["fixed"]]),
    ("done", &[&["done"]]),
];

/// Words that, anywhere in the sentence before the claim, make it something
/// other than a report of success: a condition ("once the tests pass"), a
/// plan ("I'll make sure it's done"), a request or a failure ("two tests
/// failed, so ..."). After the claim they are only qualifiers: "All 52 tests
/// pass, 0 failed." and "Fixed the crash when input is empty." are claims.
const NOT_A_CLAIM_BEFORE: &[&str] = &[
    "if", "once", "until", "unless", "before", "when", "whenever", "should", "would", "will",
    "might", "may", "need", "needs", "fail", "fails", "failed", "failing", "want", "wants",
    "please", "i'll", "we'll", "it'll",
];

/// Negations. They undo a claim when they come before it in the same clause
/// ("it is not deployed", "nothing has been done"), but not from another
/// clause ("No warnings; all tests pass.") and not after it ("All tests pass
/// with no warnings.").
const NEGATIONS: &[&str] = &[
    "not", "no", "never", "cannot", "unable", "nothing", "without",
];

/// Words that, after a negation following the claim, admit the claim was not
/// checked: "Done, but I haven't run the tests." That is the honest answer the
/// gate asks for, so it is not counted as a claim.
const CHECKING: &[&str] = &[
    "run",
    "ran",
    "running",
    "test",
    "tests",
    "tested",
    "testing",
    "verify",
    "verified",
    "verifying",
    "check",
    "checked",
    "checking",
    "confirm",
    "confirmed",
    "build",
    "built",
    "compile",
    "compiled",
    "try",
    "tried",
];

fn is_negation(token: &str) -> bool {
    NEGATIONS.contains(&token) || token.ends_with("n't")
}

/// Words that, just before "done", "fixed" or "deployed", make it something
/// other than a report on the work: "work done", "to be done", "gets done".
const NOT_ASSERTING: &[&str] = &["work", "be", "get", "gets", "getting", "well", "half"];

/// Words that make "fixed" an adjective about money, size or time rather than
/// a repair: "prices are fixed for the term", "a fixed fee".
const FIXED_AS_ADJECTIVE: &[&str] = &[
    "price", "prices", "pricing", "priced", "fee", "fees", "cost", "costs", "rate", "rates",
    "term", "budget", "per", "salary", "income", "interest", "amount", "size", "width", "length",
    "point", "schedule", "monthly", "yearly", "annual",
];

/// Words that, just before "done", "fixed" or "deployed", make it a statement
/// about the work rather than an adjective ("fixed-size", "done with it").
const ASSERTING: &[&str] = &[
    "is",
    "are",
    "now",
    "all",
    "been",
    "was",
    "were",
    "it's",
    "that's",
    "everything's",
    "i've",
    "we've",
    "i",
    "we",
    "have",
    "has",
    "got",
    "successfully",
    "and",
    "also",
];

/// Find the last sentence in a message that claims success.
///
/// Fenced code is skipped; a question, a conditional or a negation is not a
/// claim. One-word claims ("done", "fixed", "deployed") count when they open
/// or close the sentence or follow a word that asserts them, which is the
/// difference between "Fixed the parser." and "a fixed-size buffer".
pub fn find_claim(text: &str) -> Option<(String, &'static str)> {
    let mut found = None;
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        for (sentence, question) in sentences(line) {
            if question {
                continue;
            }
            if let Some(phrase) = claim_in(&sentence) {
                found = Some((shorten(&sentence), phrase));
            }
        }
    }
    found
}

fn sentences(line: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = line.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        current.push(ch);
        let next_is_space = chars.get(i + 1).is_none_or(|c| c.is_whitespace());
        if (ch == '.' || ch == '!' || ch == '?' || ch == ':') && next_is_space {
            out.push((current.trim().to_string(), ch == '?'));
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        out.push((current.trim().to_string(), false));
    }
    out
}

fn claim_in(sentence: &str) -> Option<&'static str> {
    let lowered = sentence
        .to_lowercase()
        .replace(['\u{2019}', '\u{2018}'], "'");
    let (tokens, clauses) = tokenize(&lowered);
    for (label, sequences) in PHRASES {
        for sequence in *sequences {
            if sequence.len() > tokens.len() {
                continue;
            }
            for start in 0..=tokens.len() - sequence.len() {
                if tokens[start..start + sequence.len()] != **sequence {
                    continue;
                }
                if undone(&tokens, &clauses, start, start + sequence.len()) {
                    continue;
                }
                if sequence.len() > 1 {
                    return Some(label);
                }
                let before = start.checked_sub(1).map(|at| tokens[at]);
                if before.is_some_and(|word| NOT_ASSERTING.contains(&word)) {
                    continue;
                }
                if *label == "fixed"
                    && (lowered.contains('$')
                        || tokens.iter().any(|t| FIXED_AS_ADJECTIVE.contains(t)))
                {
                    continue;
                }
                let opens = start == 0;
                let closes = start + 1 == tokens.len();
                let asserted = before.is_some_and(|word| ASSERTING.contains(&word));
                if opens || closes || asserted {
                    return Some(label);
                }
            }
        }
    }
    None
}

/// Split a lowercased sentence into words, each with the number of the clause
/// it is in. Commas, semicolons, brackets and dashes start a new clause.
fn tokenize(lowered: &str) -> (Vec<&str>, Vec<usize>) {
    let (mut tokens, mut clauses) = (Vec::new(), Vec::new());
    let mut clause = 0;
    let mut start = None;
    let mut flush = |from: usize, to: usize, clause: &mut usize| {
        let word = lowered[from..to].trim_matches('\'');
        if word.is_empty() {
            return;
        }
        if word.chars().all(|c| c == '-') {
            *clause += 1;
            return;
        }
        tokens.push(word);
        clauses.push(*clause);
    };
    for (at, ch) in lowered.char_indices() {
        if ch.is_alphanumeric() || ch == '\'' || ch == '-' || ch == '_' {
            start.get_or_insert(at);
            continue;
        }
        if let Some(from) = start.take() {
            flush(from, at, &mut clause);
        }
        if matches!(ch, ',' | ';' | '(' | ')' | '\u{2013}' | '\u{2014}') {
            clause += 1;
        }
    }
    if let Some(from) = start {
        flush(from, lowered.len(), &mut clause);
    }
    (tokens, clauses)
}

/// Whether the words around a match at `start..end` take the claim back.
fn undone(tokens: &[&str], clauses: &[usize], start: usize, end: usize) -> bool {
    if tokens[..start]
        .iter()
        .any(|t| NOT_A_CLAIM_BEFORE.contains(t))
    {
        return true;
    }
    if (0..start).any(|at| clauses[at] == clauses[start] && is_negation(tokens[at])) {
        return true;
    }
    // After the claim, only an admission that nothing checked it counts:
    // "not tested", "haven't run the tests", "without running them". A
    // negated noun ("with no warnings", "no test failures") is a qualifier.
    (end..tokens.len()).any(|at| {
        let word = tokens[at];
        if matches!(word, "unverified" | "untested") {
            return true;
        }
        let negates_a_verb = is_negation(word) && !matches!(word, "no" | "nothing");
        negates_a_verb
            && tokens[at + 1..]
                .iter()
                .take(3)
                .any(|t| CHECKING.contains(t))
    })
}

fn shorten(sentence: &str) -> String {
    let cleaned: String = sentence.replace("**", "").replace('`', "");
    let cleaned = cleaned.trim_start_matches(['-', '*', '#', ' ']).trim();
    if cleaned.chars().count() <= 160 {
        return cleaned.to_string();
    }
    let mut cut: String = cleaned.chars().take(157).collect();
    cut.push_str("...");
    cut
}

/// Parse an RFC 3339 timestamp ("2026-11-14T21:39:04.471Z") without a date
/// library. Offsets other than `Z` are honoured.
pub fn parse_timestamp_ms(stamp: &str) -> Option<i64> {
    let stamp = stamp.trim();
    let (date, rest) = stamp.split_once(['T', ' '])?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;

    let (clock, offset_minutes) = if let Some(clock) = rest.strip_suffix('Z') {
        (clock, 0)
    } else if let Some(at) = rest.rfind(['+', '-']) {
        let (clock, offset) = rest.split_at(at);
        let sign = if offset.starts_with('-') { -1 } else { 1 };
        let (h, m) = offset[1..].split_once(':').unwrap_or((&offset[1..], "0"));
        (
            clock,
            sign * (h.parse::<i64>().ok()? * 60 + m.parse::<i64>().ok()?),
        )
    } else {
        (rest, 0)
    };
    let mut clock_parts = clock.split(':');
    let hour: i64 = clock_parts.next()?.parse().ok()?;
    let minute: i64 = clock_parts.next()?.parse().ok()?;
    let seconds: f64 = clock_parts.next().unwrap_or("0").parse().ok()?;

    // Days from the civil calendar (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    let ms = ((days * 86_400 + hour * 3600 + minute * 60) as f64 + seconds) * 1000.0;
    Some(ms as i64 - offset_minutes * 60_000)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serde_json::json;

    /// Build a synthetic transcript, one entry per line, as Claude Code writes
    /// them.
    #[derive(Default)]
    pub struct Script {
        lines: Vec<String>,
        next: usize,
        clock: i64,
    }

    impl Script {
        pub fn new() -> Script {
            Script::default()
        }

        fn stamp(&mut self) -> String {
            self.clock += 1;
            format!(
                "2026-11-14T10:{:02}:{:02}.000Z",
                self.clock / 60,
                self.clock % 60
            )
        }

        pub fn user(mut self, text: &str) -> Script {
            let ts = self.stamp();
            self.lines.push(
                json!({"type":"user","sessionId":"s-1","timestamp":ts,
                       "message":{"role":"user","content":text}})
                .to_string(),
            );
            self
        }

        pub fn say(mut self, text: &str) -> Script {
            let ts = self.stamp();
            self.lines.push(
                json!({"type":"assistant","sessionId":"s-1","timestamp":ts,
                       "message":{"role":"assistant","content":[{"type":"text","text":text}]}})
                .to_string(),
            );
            self
        }

        /// A shell call and its result. `exit` 0 is a success.
        pub fn bash(mut self, command: &str, exit: i64, output: &str) -> Script {
            self.next += 1;
            let id = format!("toolu_{}", self.next);
            let ts = self.stamp();
            self.lines.push(
                json!({"type":"assistant","sessionId":"s-1","timestamp":ts,
                       "message":{"role":"assistant","content":[
                           {"type":"tool_use","id":id,"name":"Bash","input":{"command":command}}]}})
                .to_string(),
            );
            let ts = self.stamp();
            let result = if exit == 0 {
                json!({"type":"tool_result","tool_use_id":id,"content":output})
            } else {
                json!({"type":"tool_result","tool_use_id":id,"is_error":true,
                       "content":format!("Exit code {exit}\n{output}")})
            };
            self.lines.push(
                json!({"type":"user","sessionId":"s-1","timestamp":ts,
                       "message":{"role":"user","content":[result]}})
                .to_string(),
            );
            self
        }

        pub fn raw(mut self, line: &str) -> Script {
            self.lines.push(line.to_string());
            self
        }

        pub fn text(&self) -> String {
            self.lines.join("\n") + "\n"
        }
    }

    #[test]
    fn a_claim_after_a_passing_test_run_is_backed() {
        let t = analyse(
            &Script::new()
                .user("fix the parser")
                .bash("cargo test", 0, "test result: ok. 4 passed")
                .say("Fixed the parser. All tests pass.")
                .text(),
        );
        assert_eq!(t.claims.len(), 1);
        let claim = &t.claims[0];
        assert!(claim.backed, "{claim:?}");
        assert_eq!(claim.phrase, "tests pass");
        assert_eq!(claim.last_command.as_deref(), Some("cargo test"));
        assert_eq!(claim.last_exit, Some(0));
        assert!(t.final_turn.passed);
    }

    #[test]
    fn a_claim_with_no_check_at_all_is_unbacked() {
        let t = analyse(
            &Script::new()
                .user("fix the parser")
                .bash("git diff --stat", 0, " 1 file changed")
                .say("Done. The parser handles empty input now.")
                .text(),
        );
        assert_eq!(t.claims.len(), 1);
        assert!(!t.claims[0].backed);
        assert_eq!(t.claims[0].phrase, "done");
        assert_eq!(t.claims[0].last_command.as_deref(), Some("git diff --stat"));
    }

    #[test]
    fn a_failing_check_does_not_back_a_claim() {
        let t = analyse(
            &Script::new()
                .user("make it build")
                .bash("npm run build", 1, "error TS2304")
                .say("Fixed.")
                .text(),
        );
        assert!(!t.claims[0].backed);
        assert_eq!(t.claims[0].last_exit, Some(1));
    }

    #[test]
    fn exit_zero_with_a_failure_in_the_output_is_not_a_pass() {
        // `cargo test 2>&1 | tail` exits with tail's code.
        let t = analyse(
            &Script::new()
                .user("go")
                .bash(
                    "cargo test 2>&1 | tail -3",
                    0,
                    "test result: FAILED. 3 passed; 1 failed",
                )
                .say("All tests pass.")
                .text(),
        );
        assert!(!t.claims[0].backed);
    }

    #[test]
    fn mixed_turns_are_judged_separately() {
        let t = analyse(
            &Script::new()
                .user("one")
                .bash("pytest -q", 0, "5 passed")
                .say("Tests pass.")
                .user("two")
                .say("Answered.")
                .user("three")
                .bash("ls", 0, "a b")
                .say("Deployed to staging.")
                .user("four")
                .bash("python -m pytest", 2, "1 failed")
                .say("Everything is done.")
                .text(),
        );
        let verdicts: Vec<(&str, bool)> = t.claims.iter().map(|c| (c.phrase, c.backed)).collect();
        assert_eq!(
            verdicts,
            vec![("tests pass", true), ("deployed", false), ("done", false)]
        );
    }

    #[test]
    fn a_check_in_the_turn_just_before_backs_a_claim() {
        let t = analyse(
            &Script::new()
                .user("run the tests")
                .bash("go test ./...", 0, "ok")
                .say("Here is the output.")
                .user("so is it working?")
                .say("Yes, the fix is done.")
                .text(),
        );
        assert_eq!(t.claims.len(), 1);
        assert!(t.claims[0].backed);
        // But the gate only looks at the turn it is stopping.
        assert!(!t.final_turn.passed);
    }

    #[test]
    fn a_check_later_in_the_same_turn_backs_the_turn() {
        let t = analyse(
            &Script::new()
                .user("fix")
                .say("Fixed. Now let me run the tests.")
                .bash("npx vitest run", 0, "passed")
                .say("Tests pass.")
                .text(),
        );
        assert_eq!(t.claims.len(), 1, "one claim per turn");
        assert!(t.claims[0].backed);
        assert_eq!(t.claims[0].text, "Tests pass.");
    }

    #[test]
    fn negations_questions_conditionals_and_code_are_not_claims() {
        for text in [
            "The tests do not pass yet.",
            "Is it done?",
            "Once the tests pass, I will push.",
            "I haven't fixed the flaky test.",
            "This uses a fixed-size buffer.",
            "```\necho done\n```",
            "Two tests failed, so it is not deployed.",
            "Please confirm what you want done.",
            "Prices are fixed for the whole term, including renewals.",
            "Fixed fee $2,250/month.",
            "Payment is against an itemized statement of work done.",
            "Nothing has been done on any of these.",
            "I'll report when it's done.",
            "It is not deployed, and the tests do not pass.",
            "If you rerun it, the tests pass.",
            "Done, but I haven't run the tests.",
            "Fixed the parser; not tested yet.",
            "Fixed without running the tests.",
            "Deployed, but this is unverified.",
        ] {
            assert_eq!(find_claim(text), None, "{text}");
        }
    }

    #[test]
    fn claims_with_qualifiers_after_them_are_still_claims() {
        // A negation, a failure word or "when" after the claim qualifies it;
        // it does not take it back. These used to be missed, so the gate let
        // exactly these "tests pass" messages through.
        for (text, phrase) in [
            ("All 52 tests pass, 0 failed.", "tests pass"),
            ("All tests pass with no warnings.", "tests pass"),
            ("Fixed the crash when input is empty.", "fixed"),
            ("Tests are green.", "tests pass"),
            ("The tests are all green now.", "tests pass"),
            ("No warnings; all tests pass.", "tests pass"),
            (
                "All tests pass (no test failures, nothing skipped).",
                "tests pass",
            ),
            ("Deployed - nothing else changed.", "deployed"),
        ] {
            assert_eq!(find_claim(text).map(|(_, p)| p), Some(phrase), "{text}");
        }
    }

    #[test]
    fn the_gate_blocks_a_qualified_claim_with_nothing_run() {
        let t = analyse(
            &Script::new()
                .user("fix it")
                .say("All 52 tests pass, 0 failed.")
                .text(),
        );
        assert_eq!(t.claims.len(), 1);
        assert!(!t.claims[0].backed);
    }

    #[test]
    fn plain_claims_are_found() {
        for (text, phrase) in [
            ("All tests pass.", "tests pass"),
            ("Done.", "done"),
            ("**Fixed** the off-by-one in the loop.", "fixed"),
            ("I've fixed the crash on startup.", "fixed"),
            ("Deployed to production", "deployed"),
            ("CI is all green!", "all green"),
            (
                "The build succeeded and the binary is in target.",
                "build passes",
            ),
        ] {
            assert_eq!(find_claim(text).map(|(_, p)| p), Some(phrase), "{text}");
        }
    }

    #[test]
    fn check_commands_are_recognised_and_others_are_not() {
        for command in [
            "cargo test",
            "cargo +1.99.0 clippy --workspace -- -D warnings",
            "cd app && npm test",
            "npm run build",
            "pnpm lint",
            "yarn typecheck",
            "bun test",
            "npx tsc --noEmit",
            "\"C:/Program Files/Python312/python.exe\" -m pytest -q",
            "python3 -m unittest discover",
            "pytest tests/",
            "go test ./...",
            "make",
            "./gradlew test",
            "dotnet build",
            "CARGO_TARGET_DIR=/tmp/t cargo build --release",
            "corescout run -- cargo test",
            "cargo fmt --all -- --check",
            "ruff check .",
            "Invoke-Pester",
        ] {
            assert!(is_check_command(command), "{command}");
        }
        for command in [
            "git push origin main",
            "ls -la",
            "npm install",
            "npm ci",
            "echo cargo test",
            "cargo fmt",
            "python script.py",
            "make clean",
            "prettier --write .",
        ] {
            assert!(!is_check_command(command), "{command}");
        }
    }

    #[test]
    fn a_refused_or_background_call_has_no_exit_code() {
        let refused = json!({"type":"user","sessionId":"s-1","timestamp":"2026-11-14T10:00:09Z",
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_x",
            "is_error":true,"content":"PreToolUse:Bash hook error: no"}]}});
        let call = json!({"type":"assistant","sessionId":"s-1","timestamp":"2026-11-14T10:00:08Z",
            "message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_x",
            "name":"Bash","input":{"command":"cargo test"}}]}});
        let t = analyse(
            &Script::new()
                .user("go")
                .raw(&call.to_string())
                .raw(&refused.to_string())
                .say("Tests pass.")
                .text(),
        );
        assert!(!t.claims[0].backed);
        assert_eq!(t.claims[0].last_exit, None);

        let background = json!({"type":"assistant","sessionId":"s-1","timestamp":"2026-11-14T10:00:08Z",
            "message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_y",
            "name":"Bash","input":{"command":"cargo test","run_in_background":true}}]}});
        let started = json!({"type":"user","sessionId":"s-1","timestamp":"2026-11-14T10:00:09Z",
            "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_y",
            "content":"Command running in background with ID: b1"}]}});
        let t = analyse(
            &Script::new()
                .user("go")
                .raw(&background.to_string())
                .raw(&started.to_string())
                .say("Done.")
                .text(),
        );
        assert!(!t.claims[0].backed);
    }

    #[test]
    fn junk_lines_and_other_entry_types_are_skipped() {
        let t = analyse(
            &Script::new()
                .raw("not json")
                .raw(r#"{"type":"summary","summary":"x"}"#)
                .raw(r#"{"type":"api-request-blob","message":{"role":"user","content":[]}}"#)
                .user("go")
                .say("Done.")
                .text(),
        );
        assert_eq!(t.claims.len(), 1);
    }

    #[test]
    fn meta_entries_do_not_start_a_turn() {
        let meta = json!({"type":"user","isMeta":true,"sessionId":"s-1",
            "timestamp":"2026-11-14T10:00:05Z","message":{"role":"user","content":"<system-reminder>"}});
        let t = analyse(
            &Script::new()
                .user("go")
                .bash("cargo build", 0, "Finished")
                .raw(&meta.to_string())
                .say("Build passes.")
                .text(),
        );
        assert!(t.final_turn.passed);
        assert!(t.claims[0].backed);
    }

    #[test]
    fn timestamps_parse_to_epoch_milliseconds() {
        assert_eq!(parse_timestamp_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_timestamp_ms("1970-01-02T00:00:01.500Z"),
            Some(86_401_500)
        );
        assert_eq!(
            parse_timestamp_ms("2026-11-14T21:39:04.471Z"),
            Some(1_794_692_344_471)
        );
        assert_eq!(
            parse_timestamp_ms("2026-11-14T22:39:04.471+01:00"),
            Some(1_794_692_344_471)
        );
        assert_eq!(parse_timestamp_ms("yesterday"), None);
    }
}
