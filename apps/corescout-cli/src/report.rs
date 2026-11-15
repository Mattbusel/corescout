//! `corescout report` and `corescout gate`: did the AI check before it said
//! it was done?
//!
//! Both read Claude Code transcripts directly from disk and never talk to the
//! CoreScout service, so they work whether or not it is running.
//!
//! # `corescout report`
//!
//! Reads every transcript under `~/.claude/projects` (or `--from <DIR>`), finds
//! the assistant turns that claimed success, and says how many of them in the
//! last `--days` days (30 by default) had no passing test, build or lint run
//! in that turn or the one before. It lists the ten most recent unbacked
//! claims with the command that ran last. `--card` prints a one-screen card
//! with no paths, session ids or message text in it, for sharing; `--json`
//! prints the whole answer for a program.
//!
//! Claude Code deletes transcripts after 30 days by default. On every run the
//! report first copies any new or grown transcript into CoreScout's own data
//! directory (`<data dir>/claude-transcripts`, see
//! `corescout_storage::paths::claude_transcripts`) and reads from there, so
//! the history outlives the cleanup. The first copy is all of the history,
//! whatever `--days` says, so it is announced with its size before it starts.
//! A transcript that cannot be copied is read where it is instead. `--no-store`
//! reads the transcripts where they are and copies nothing, and so does
//! `--from <DIR>`: the store mirrors `~/.claude/projects` only, so another
//! folder can never overwrite a copy it holds or leak into a later report.
//!
//! # `corescout gate`
//!
//! A Claude Code `Stop` hook. It reads the hook's JSON from stdin, opens the
//! transcript it names, and when the last assistant message claims success and
//! no check passed in that turn it exits 2 with a message on stderr, which
//! Claude Code shows to the model and which keeps it working. Anything it
//! cannot read lets the stop through: a gate that breaks somebody's session
//! because of its own bug is a gate they uninstall. When the hook payload says
//! the stop is already the result of a block (`stop_hook_active`), it lets the
//! stop through too, so it can never loop.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use corescout_core::error::{Error, Result};
use serde_json::{json, Value};

use crate::claims::{self, Claim};

/// Options for `corescout report`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportOptions {
    pub days: u64,
    pub card: bool,
    pub json: bool,
    pub store: bool,
    pub from: Option<PathBuf>,
    pub top: usize,
}

impl ReportOptions {
    /// Parse the arguments after `report`.
    pub fn parse(args: &[String]) -> Result<ReportOptions> {
        let mut options = ReportOptions {
            days: 30,
            card: false,
            json: false,
            store: true,
            from: None,
            top: 10,
        };
        let mut at = 0;
        while at < args.len() {
            let value = |at: usize| -> Result<&String> {
                args.get(at + 1)
                    .ok_or_else(|| Error::invalid(format!("{} needs a value", args[at])))
            };
            match args[at].as_str() {
                "--card" => options.card = true,
                "--json" => options.json = true,
                "--no-store" => options.store = false,
                "--days" => {
                    options.days = value(at)?
                        .parse()
                        .map_err(|_| Error::invalid("--days needs a whole number".to_string()))?;
                    at += 1;
                }
                "--top" => {
                    options.top = value(at)?
                        .parse()
                        .map_err(|_| Error::invalid("--top needs a whole number".to_string()))?;
                    at += 1;
                }
                "--from" => {
                    options.from = Some(PathBuf::from(value(at)?));
                    at += 1;
                }
                other => {
                    return Err(Error::invalid(format!(
                        "{other:?} is not an option for report; try --days, --top, --card, --json, --no-store or --from"
                    )))
                }
            }
            at += 1;
        }
        if options.days == 0 {
            return Err(Error::invalid("--days must be at least 1".to_string()));
        }
        // The store mirrors `~/.claude/projects`. Copying another folder into
        // it would mix that folder into every later report, and a file there
        // whose relative path matched an archived transcript would overwrite
        // the archived copy. So `--from` reads in place, and only that folder.
        if options.from.is_some() {
            options.store = false;
        }
        Ok(options)
    }
}

/// Where Claude Code keeps its transcripts: `$CLAUDE_CONFIG_DIR/projects`,
/// else `~/.claude/projects`.
pub fn claude_projects_dir() -> Option<PathBuf> {
    if let Some(config) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(config).join("projects"));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".claude").join("projects"))
}

/// Where CoreScout keeps its copy of the transcripts.
pub fn store_dir() -> PathBuf {
    corescout_storage::paths::claude_transcripts()
}

/// Every `.jsonl` file under a directory, at any depth, with its path
/// relative to that directory.
fn transcripts_under(root: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "jsonl") {
                if let Ok(relative) = path.strip_prefix(root) {
                    out.push((path.clone(), relative.to_path_buf()));
                }
            }
        }
    }
    out.sort();
    out
}

/// What copying into the store did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Copied {
    pub files: usize,
    pub bytes: u64,
    /// Relative paths of the transcripts that could not be copied. The
    /// report reads those from the source instead.
    pub failed: Vec<PathBuf>,
}

/// One transcript that is new or has grown since it was last copied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub source: PathBuf,
    pub relative: PathBuf,
    pub bytes: u64,
}

/// The transcripts under `source` that are new or have grown since they were
/// last copied into `store`.
///
/// Transcripts only ever grow, so "the source is longer" is the whole test.
pub fn plan_copy(source: &Path, store: &Path) -> Vec<Pending> {
    let mut out = Vec::new();
    for (path, relative) in transcripts_under(source) {
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let stored = std::fs::metadata(store.join(&relative))
            .map(|m| m.len())
            .ok();
        if stored.is_some_and(|len| len >= meta.len()) {
            continue;
        }
        out.push(Pending {
            source: path,
            relative,
            bytes: meta.len(),
        });
    }
    out
}

/// Copy one transcript into the store without ever leaving a half-written
/// file in place of a good copy: it is written beside the target and renamed
/// over it. The copy keeps the source's modification time, which is what lets
/// the report skip files too old to matter without opening them.
fn copy_one(item: &Pending, store: &Path) -> std::io::Result<u64> {
    let target = store.join(&item.relative);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut partial = target.clone().into_os_string();
    partial.push(".part");
    let partial = PathBuf::from(partial);
    let copied = std::fs::copy(&item.source, &partial).and_then(|bytes| {
        if let Ok(modified) = std::fs::metadata(&item.source).and_then(|m| m.modified()) {
            if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&partial) {
                let _ = file.set_modified(modified);
            }
        }
        std::fs::rename(&partial, &target)?;
        Ok(bytes)
    });
    if copied.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    copied
}

/// Copy what [`plan_copy`] found. A file that cannot be copied is skipped and
/// named in [`Copied::failed`]: one bad file never stops the rest. `progress`
/// is told the bytes copied so far after each file.
pub fn copy_pending(pending: &[Pending], store: &Path, mut progress: impl FnMut(u64)) -> Copied {
    let mut copied = Copied::default();
    for item in pending {
        match copy_one(item, store) {
            Ok(bytes) => {
                copied.files += 1;
                copied.bytes += bytes;
            }
            Err(_) => copied.failed.push(item.relative.clone()),
        }
        progress(copied.bytes);
    }
    copied
}

/// Copy every transcript that is new or has grown since the last copy, with
/// no progress and no fallback: what the tests use.
#[cfg(test)]
pub fn sync_store(source: &Path, store: &Path) -> Copied {
    copy_pending(&plan_copy(source, store), store, |_| {})
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The start of a window `days` long that ends at `now_ms`. It saturates
/// rather than overflowing, so `--days 999999999999` means "everything".
pub fn window_start(now_ms: i64, days: u64) -> i64 {
    let span = i64::try_from(days)
        .unwrap_or(i64::MAX)
        .saturating_mul(86_400_000);
    now_ms.saturating_sub(span)
}

/// Read every transcript under `root` and keep the claims made since
/// `since_ms`, each with the project folder it came from.
#[cfg(test)]
pub fn collect(root: &Path, since_ms: i64) -> (Vec<(String, Claim)>, usize) {
    collect_files(transcripts_under(root), since_ms)
}

/// Read the given transcripts, each a `(path, path relative to its projects
/// folder)` pair, and keep the claims made since `since_ms`, each with the
/// project folder it came from.
pub fn collect_files(
    files: Vec<(PathBuf, PathBuf)>,
    since_ms: i64,
) -> (Vec<(String, Claim)>, usize) {
    let mut out = Vec::new();
    let mut read = 0;
    for (path, relative) in files {
        // A file last written before the window holds nothing inside it.
        let modified_ms = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64);
        if modified_ms.is_some_and(|ms| ms < since_ms) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        read += 1;
        let text = String::from_utf8_lossy(&bytes);
        let project = relative
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_default();
        for claim in claims::analyse(&text).claims {
            if claim.at_ms.is_some_and(|at| at >= since_ms) {
                out.push((project.clone(), claim));
            }
        }
    }
    (out, read)
}

/// The answer `report` gives, as one JSON object.
pub fn summarise(claims: &[(String, Claim)], options: &ReportOptions, read: usize) -> Value {
    let total = claims.len();
    let unbacked: Vec<&(String, Claim)> = claims.iter().filter(|(_, c)| !c.backed).collect();
    let percent = if total == 0 {
        0.0
    } else {
        unbacked.len() as f64 * 100.0 / total as f64
    };
    let mut by_phrase: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for (_, claim) in claims {
        let entry = by_phrase.entry(claim.phrase).or_default();
        entry.0 += 1;
        if !claim.backed {
            entry.1 += 1;
        }
    }
    let sessions: BTreeSet<&str> = claims.iter().map(|(_, c)| c.session.as_str()).collect();
    let projects: BTreeSet<&str> = claims.iter().map(|(p, _)| p.as_str()).collect();

    let mut recent = unbacked.clone();
    recent.sort_by_key(|(_, claim)| std::cmp::Reverse(claim.at_ms));
    let top: Vec<Value> = recent
        .iter()
        .take(options.top)
        .map(|(project, claim)| {
            json!({
                "session": claim.session,
                "project": project,
                "timestamp": claim.timestamp,
                "claim": claim.text,
                "phrase": claim.phrase,
                "last_command": claim.last_command,
                "last_exit": claim.last_exit,
            })
        })
        .collect();

    json!({
        "days": options.days,
        "transcripts_read": read,
        "claims": total,
        "unbacked": unbacked.len(),
        "unbacked_percent": (percent * 10.0).round() / 10.0,
        "sessions": sessions.len(),
        "projects": projects.len(),
        "by_phrase": by_phrase
            .iter()
            .map(|(phrase, (all, unbacked))| json!({"phrase": phrase, "claims": all, "unbacked": unbacked}))
            .collect::<Vec<_>>(),
        "top_unbacked": top,
    })
}

fn one_line(text: &str, width: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= width {
        return flat;
    }
    let mut cut: String = flat.chars().take(width.saturating_sub(3)).collect();
    cut.push_str("...");
    cut
}

/// The report as text for a terminal.
pub fn render_report(summary: &Value) -> String {
    let mut out = String::new();
    let days = summary["days"].as_u64().unwrap_or(0);
    let total = summary["claims"].as_u64().unwrap_or(0);
    let unbacked = summary["unbacked"].as_u64().unwrap_or(0);
    out.push_str(&format!("Claude Code success claims, last {days} days\n\n"));
    if total == 0 {
        out.push_str(&format!(
            "No assistant turn claimed success in {} transcripts read.\n",
            summary["transcripts_read"].as_u64().unwrap_or(0)
        ));
        return out;
    }
    out.push_str(&format!(
        "  {:.1}% unbacked: {unbacked} of {total} turns that claimed success had no passing\n  test, build or lint run in that turn or the one before.\n",
        summary["unbacked_percent"].as_f64().unwrap_or(0.0)
    ));
    out.push_str(&format!(
        "  {} sessions in {} projects, {} transcripts read.\n",
        summary["sessions"].as_u64().unwrap_or(0),
        summary["projects"].as_u64().unwrap_or(0),
        summary["transcripts_read"].as_u64().unwrap_or(0),
    ));
    let top = summary["top_unbacked"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if top.is_empty() {
        return out;
    }
    out.push_str(&format!("\nMost recent unbacked claims ({})\n", top.len()));
    for item in top {
        let session = item["session"].as_str().unwrap_or_default();
        let stamp = item["timestamp"].as_str().unwrap_or_default();
        out.push_str(&format!(
            "\n  {} UTC  session {}  {}\n    \"{}\"\n",
            stamp.get(..16).unwrap_or(stamp).replace('T', " "),
            session.get(..8).unwrap_or(session),
            item["project"].as_str().unwrap_or_default(),
            one_line(item["claim"].as_str().unwrap_or_default(), 100),
        ));
        out.push_str(&match item["last_command"].as_str() {
            Some(command) => format!(
                "    last command: {}  (exit {})\n",
                one_line(command, 80),
                item["last_exit"]
                    .as_i64()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".into())
            ),
            None => "    no command run in this turn or the one before\n".to_string(),
        });
    }
    out
}

/// A one-screen card with nothing in it that identifies a project, a session
/// or what anybody was working on: counts and the kinds of claim only.
pub fn render_card(summary: &Value) -> String {
    const WIDTH: usize = 52;
    let mut lines = vec![
        "CoreScout: did the AI check before it said done?".to_string(),
        format!(
            "Claude Code, last {} days",
            summary["days"].as_u64().unwrap_or(0)
        ),
        String::new(),
        format!(
            "  {:.0}% of success claims had no passing check",
            summary["unbacked_percent"].as_f64().unwrap_or(0.0)
        ),
        format!(
            "  {} of {} turns that claimed success",
            summary["unbacked"].as_u64().unwrap_or(0),
            summary["claims"].as_u64().unwrap_or(0)
        ),
        String::new(),
        format!("  {:<16}{:>8}{:>11}", "claim", "turns", "unbacked"),
    ];
    for row in summary["by_phrase"].as_array().cloned().unwrap_or_default() {
        let all = row["claims"].as_u64().unwrap_or(0);
        let unbacked = row["unbacked"].as_u64().unwrap_or(0);
        let share = (unbacked * 100).checked_div(all).unwrap_or(0);
        lines.push(format!(
            "  {:<16}{:>8}{:>10}%",
            row["phrase"].as_str().unwrap_or_default(),
            all,
            share
        ));
    }
    lines.push(String::new());
    lines.push(format!(
        "  {} sessions, {} projects",
        summary["sessions"].as_u64().unwrap_or(0),
        summary["projects"].as_u64().unwrap_or(0)
    ));
    lines.push("  check = tests, build or lint exiting 0".to_string());

    let rule = format!("+{}+\n", "-".repeat(WIDTH + 2));
    let mut out = rule.clone();
    for line in lines {
        out.push_str(&format!(
            "| {:<WIDTH$} |\n",
            line.chars().take(WIDTH).collect::<String>()
        ));
    }
    out.push_str(&rule);
    out
}

/// Copy new and grown transcripts from `source` into `store` and return the
/// files the report should read: the store's, with anything that failed to
/// copy read from the source instead.
///
/// The first copy and any large one are announced on stderr before they
/// start, with their size, and report progress as they go, so several
/// gigabytes of history do not look like a hang.
fn sync_and_list(source: &Path, store: &Path, quiet: bool) -> Vec<(PathBuf, PathBuf)> {
    const LOUD_BYTES: u64 = 64 * 1_048_576;
    let first = !store.exists();
    let pending = plan_copy(source, store);
    let total: u64 = pending.iter().map(|p| p.bytes).sum();
    let loud = !quiet && !pending.is_empty() && (first || total >= LOUD_BYTES);
    if loud {
        eprintln!(
            "Copying {} transcripts ({:.1} MB) into {} so they outlive Claude Code's 30-day cleanup.",
            pending.len(),
            total as f64 / 1_048_576.0,
            store.display()
        );
        eprintln!(
            "This keeps all of your history, not just the last --days; later runs copy only what is new. --no-store skips it."
        );
    }
    // A line every tenth of the way, and at least every 256 MB.
    let step = (total / 10).clamp(1, 256 * 1_048_576);
    let mut next = step;
    let copied = copy_pending(&pending, store, |done| {
        if loud && done >= next && done < total {
            eprintln!(
                "  {:.1} of {:.1} MB copied",
                done as f64 / 1_048_576.0,
                total as f64 / 1_048_576.0
            );
            next = done.saturating_add(step);
        }
    });
    if loud {
        eprintln!(
            "Kept {} transcripts ({:.1} MB).",
            copied.files,
            copied.bytes as f64 / 1_048_576.0
        );
    }
    if !copied.failed.is_empty() {
        eprintln!(
            "Could not copy {} transcripts into {}; read them where they are instead.",
            copied.failed.len(),
            store.display()
        );
    }
    let failed: BTreeSet<&PathBuf> = copied.failed.iter().collect();
    let mut files: Vec<(PathBuf, PathBuf)> = transcripts_under(store)
        .into_iter()
        .filter(|(_, relative)| !failed.contains(relative))
        .collect();
    files.extend(
        copied
            .failed
            .iter()
            .map(|relative| (source.join(relative), relative.clone())),
    );
    files
}

/// `corescout report`.
pub fn report(args: &[String]) -> Result<()> {
    let options = ReportOptions::parse(args)?;
    let source = match &options.from {
        Some(dir) => dir.clone(),
        None => claude_projects_dir()
            .ok_or_else(|| Error::invalid("cannot find a home directory".to_string()))?,
    };
    let store = store_dir();
    let files = if options.store && source.is_dir() {
        sync_and_list(&source, &store, options.json)
    } else if options.store && store.is_dir() {
        // Claude Code's folder is gone; the copies are what is left of it.
        transcripts_under(&store)
    } else if source.is_dir() {
        transcripts_under(&source)
    } else {
        return Err(Error::invalid(format!(
            "no transcripts at {}; is Claude Code installed? --from <DIR> points somewhere else",
            source.display()
        )));
    };

    let since = window_start(now_ms(), options.days);
    let (claims, read) = collect_files(files, since);
    let summary = summarise(&claims, &options, read);
    if options.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).unwrap_or_else(|_| summary.to_string())
        );
    } else if options.card {
        print!("{}", render_card(&summary));
    } else {
        print!("{}", render_report(&summary));
    }
    Ok(())
}

/// What the gate decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Let the stop through.
    Allow,
    /// Block it, with this message for the model.
    Block(String),
}

/// Decide a Stop hook payload.
///
/// `read` opens the transcript the payload names; it is a parameter so the
/// tests can hand one over without a file.
pub fn decide(payload: &Value, read: impl Fn(&Path) -> Option<String>) -> Verdict {
    if payload["stop_hook_active"].as_bool() == Some(true) {
        return Verdict::Allow;
    }
    let Some(path) = payload["transcript_path"].as_str() else {
        return Verdict::Allow;
    };
    let Some(text) = read(Path::new(path)) else {
        return Verdict::Allow;
    };
    let transcript = claims::analyse(&text);
    let last = &transcript.final_turn;
    // Newer Claude Code versions put the final message in the payload, which
    // is safer than trusting the transcript to have been flushed.
    let message = payload["last_assistant_message"]
        .as_str()
        .map(str::to_string)
        .or_else(|| last.last_text.clone());
    let Some(message) = message else {
        return Verdict::Allow;
    };
    let Some((sentence, _)) = claims::find_claim(&message) else {
        return Verdict::Allow;
    };
    if last.passed {
        return Verdict::Allow;
    }
    let ran = match &last.last_run {
        Some(run) => format!(
            " The last command in this turn was `{}` (exit {}), which is not a passing test, build or lint run.",
            one_line(&run.command, 80),
            run.exit
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".into())
        ),
        None => " No command ran in this turn.".to_string(),
    };
    Verdict::Block(format!(
        "CoreScout gate: your last message says \"{}\" but nothing checked it.{ran} Run the tests, build or linter that would show it (for example `cargo test`, `npm test` or `pytest`), or say plainly that the change is unverified.",
        one_line(&sentence, 120)
    ))
}

/// `corescout gate`. Returns the exit code: 2 to block, 0 otherwise.
pub fn gate(args: &[String]) -> i32 {
    let loud = std::env::var_os("CORESCOUT_GATE_DEBUG").is_some();
    // `--transcript <PATH>` judges a file directly, for trying the gate out
    // by hand; Claude Code sends the hook payload on stdin.
    let payload = match args.iter().position(|a| a == "--transcript") {
        Some(at) => match args.get(at + 1) {
            Some(path) => json!({ "transcript_path": path }),
            None => {
                eprintln!("corescout gate: --transcript needs a path");
                return 0;
            }
        },
        None => {
            let mut input = String::new();
            if std::io::stdin().read_to_string(&mut input).is_err() {
                return 0;
            }
            match serde_json::from_str(&input) {
                Ok(value) => value,
                Err(error) => {
                    if loud {
                        eprintln!("corescout gate: that is not a hook payload: {error}");
                    }
                    return 0;
                }
            }
        }
    };
    match decide(&payload, |path| std::fs::read_to_string(path).ok()) {
        Verdict::Allow => {
            if loud {
                eprintln!("corescout gate: allowed");
            }
            0
        }
        Verdict::Block(message) => {
            eprintln!("{message}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::tests::Script;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    fn payload(text: String) -> (Value, impl Fn(&Path) -> Option<String>) {
        (
            json!({"transcript_path": "t.jsonl", "stop_hook_active": false}),
            move |_: &Path| Some(text.clone()),
        )
    }

    #[test]
    fn report_options_parse_and_refuse_nonsense() {
        let o = ReportOptions::parse(&args("--days 7 --card --no-store --from x")).unwrap();
        assert_eq!(o.days, 7);
        assert!(o.card && !o.store);
        assert_eq!(o.from, Some(PathBuf::from("x")));
        assert!(ReportOptions::parse(&args("--days")).is_err());
        assert!(ReportOptions::parse(&args("--days soon")).is_err());
        assert!(ReportOptions::parse(&args("--days 0")).is_err());
        assert!(ReportOptions::parse(&args("--frobnicate")).is_err());
        assert_eq!(ReportOptions::parse(&args("--top 3")).unwrap().top, 3);
        assert!(ReportOptions::parse(&args("--top many")).is_err());
        let error = ReportOptions::parse(&args("--frobnicate"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("--top"), "{error}");
    }

    #[test]
    fn from_reads_in_place_and_never_touches_the_store() {
        // Copying another folder into the store mixed it into every later
        // report and could overwrite an archived transcript that happened to
        // share its relative path.
        assert!(!ReportOptions::parse(&args("--from x")).unwrap().store);
        assert!(ReportOptions::parse(&[]).unwrap().store);
    }

    #[test]
    fn a_huge_window_saturates_instead_of_overflowing() {
        let now = 1_800_000_000_000;
        assert_eq!(window_start(now, 1), now - 86_400_000);
        assert_eq!(window_start(now, 999_999_999_999), now - i64::MAX);
        assert_eq!(window_start(now, u64::MAX), now - i64::MAX);
        assert_eq!(window_start(i64::MIN + 1, u64::MAX), i64::MIN);
        // A window that reaches back before every claim keeps all of them.
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "p/s.jsonl",
            &Script::new().user("go").say("Done.").text(),
        );
        assert_eq!(collect(dir.path(), window_start(now, u64::MAX)).0.len(), 1);
    }

    #[test]
    fn a_transcript_that_cannot_be_copied_is_skipped_and_read_in_place() {
        let source = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        // Longer than any directory's reported size, so it is always pending.
        let big = Script::new().user("go").say("Deployed.").text() + &" ".repeat(8192);
        write(source.path(), "proj-a/bad.jsonl", &big);
        write(source.path(), "proj-b/good.jsonl", &big);
        // A directory where the copy has to go makes that one copy fail.
        std::fs::create_dir_all(store.path().join("proj-a/bad.jsonl")).unwrap();

        let copied = sync_store(source.path(), store.path());
        assert_eq!(copied.files, 1);
        assert_eq!(
            copied.failed,
            vec![PathBuf::from("proj-a").join("bad.jsonl")]
        );
        assert!(store.path().join("proj-b/good.jsonl").is_file());
        assert!(!store.path().join("proj-a/bad.jsonl.part").exists());

        // The report still reads it, from the source.
        let files = sync_and_list(source.path(), store.path(), true);
        assert!(
            files.contains(&(
                source.path().join("proj-a").join("bad.jsonl"),
                PathBuf::from("proj-a").join("bad.jsonl")
            )),
            "{files:?}"
        );
        let (claims, read) = collect_files(files, 0);
        assert_eq!(read, 2);
        assert_eq!(claims.len(), 2);
    }

    #[test]
    fn the_gate_blocks_an_unbacked_claim() {
        let (p, read) = payload(
            Script::new()
                .user("fix it")
                .bash("git status", 0, "clean")
                .say("Fixed. All tests pass.")
                .text(),
        );
        match decide(&p, read) {
            Verdict::Block(message) => {
                assert!(message.contains("All tests pass."), "{message}");
                assert!(message.contains("git status"), "{message}");
            }
            Verdict::Allow => panic!("should block"),
        }
    }

    #[test]
    fn the_gate_allows_a_backed_claim() {
        let (p, read) = payload(
            Script::new()
                .user("fix it")
                .bash("cargo test", 0, "test result: ok")
                .say("Fixed. All tests pass.")
                .text(),
        );
        assert_eq!(decide(&p, read), Verdict::Allow);
    }

    #[test]
    fn the_gate_allows_a_turn_that_claims_nothing() {
        let (p, read) = payload(
            Script::new()
                .user("what does this do?")
                .say("It parses the config file and returns a map.")
                .text(),
        );
        assert_eq!(decide(&p, read), Verdict::Allow);
    }

    #[test]
    fn the_gate_only_credits_checks_in_the_turn_it_is_stopping() {
        // Mixed: the tests passed a turn ago, then something was edited.
        let (p, read) = payload(
            Script::new()
                .user("run the tests")
                .bash("pytest", 0, "3 passed")
                .say("Tests pass.")
                .user("now rename the function")
                .say("Done.")
                .text(),
        );
        assert!(matches!(decide(&p, read), Verdict::Block(_)));
    }

    #[test]
    fn the_gate_never_blocks_twice_in_a_row() {
        let text = Script::new().user("go").say("Done.").text();
        let p = json!({"transcript_path": "t", "stop_hook_active": true});
        assert_eq!(decide(&p, |_| Some(text.clone())), Verdict::Allow);
    }

    #[test]
    fn the_gate_lets_the_stop_through_when_it_cannot_read() {
        assert_eq!(decide(&json!({}), |_| None), Verdict::Allow);
        assert_eq!(
            decide(&json!({"transcript_path":"missing"}), |_| None),
            Verdict::Allow
        );
    }

    #[test]
    fn the_gate_prefers_the_message_in_the_payload() {
        // The transcript has not caught up with the final message yet.
        let text = Script::new().user("go").bash("ls", 0, "x").text();
        let p = json!({"transcript_path":"t","last_assistant_message":"Deployed."});
        assert!(matches!(
            decide(&p, |_| Some(text.clone())),
            Verdict::Block(_)
        ));
    }

    fn write(dir: &Path, relative: &str, text: &str) {
        let path = dir.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn the_store_copies_new_and_grown_transcripts_once() {
        let source = tempfile::tempdir().unwrap();
        let store = tempfile::tempdir().unwrap();
        write(source.path(), "proj-a/one.jsonl", "{}\n");
        write(source.path(), "proj-b/s/subagents/two.jsonl", "{}\n");
        write(source.path(), "proj-b/notes.txt", "not a transcript");

        let first = sync_store(source.path(), store.path());
        assert_eq!(first.files, 2);
        assert!(store.path().join("proj-b/s/subagents/two.jsonl").exists());
        assert!(!store.path().join("proj-b/notes.txt").exists());

        assert_eq!(sync_store(source.path(), store.path()).files, 0);
        write(source.path(), "proj-a/one.jsonl", "{}\n{}\n");
        assert_eq!(sync_store(source.path(), store.path()).files, 1);

        // Claude Code's cleanup deletes the source; the copy stays.
        std::fs::remove_file(source.path().join("proj-a/one.jsonl")).unwrap();
        sync_store(source.path(), store.path());
        assert_eq!(
            std::fs::read_to_string(store.path().join("proj-a/one.jsonl")).unwrap(),
            "{}\n{}\n"
        );
    }

    #[test]
    fn a_report_over_backed_unbacked_and_mixed_transcripts() {
        let dir = tempfile::tempdir().unwrap();
        // Backed.
        write(
            dir.path(),
            "proj-a/backed.jsonl",
            &Script::new()
                .user("go")
                .bash("cargo test", 0, "ok")
                .say("All tests pass.")
                .text(),
        );
        // Unbacked.
        write(
            dir.path(),
            "proj-a/unbacked.jsonl",
            &Script::new().user("go").say("Deployed.").text(),
        );
        // Mixed: one backed turn, one unbacked.
        write(
            dir.path(),
            "proj-b/mixed.jsonl",
            &Script::new()
                .user("one")
                .bash("npm test", 0, "ok")
                .say("Fixed.")
                .user("two")
                .say("Answered.")
                .user("three")
                .bash("npm install", 0, "added 3")
                .say("Done.")
                .text(),
        );
        let (claims, read) = collect(dir.path(), 0);
        assert_eq!(read, 3);
        let options = ReportOptions::parse(&[]).unwrap();
        let summary = summarise(&claims, &options, read);
        assert_eq!(summary["claims"], 4);
        assert_eq!(summary["unbacked"], 2);
        assert_eq!(summary["unbacked_percent"], 50.0);
        assert_eq!(summary["projects"], 2);
        let top = summary["top_unbacked"].as_array().unwrap();
        assert_eq!(top.len(), 2);
        let commands: BTreeSet<Option<&str>> =
            top.iter().map(|t| t["last_command"].as_str()).collect();
        assert!(commands.contains(&Some("npm install")));
        assert!(commands.contains(&None));

        let text = render_report(&summary);
        assert!(text.contains("50.0% unbacked: 2 of 4"), "{text}");
        assert!(text.contains("npm install  (exit 0)"), "{text}");

        // Nothing in the card names a session, a project or a message.
        let card = render_card(&summary);
        assert!(card.contains("50% of success claims"), "{card}");
        assert!(
            card.contains("  tests pass             1         0%"),
            "{card}"
        );
        for secret in ["s-1", "proj-a", "proj-b", "Deployed.", "npm install"] {
            assert!(!card.contains(secret), "card leaks {secret}:\n{card}");
        }
        assert!(card.lines().all(|l| l.chars().count() == 56), "{card}");
    }

    #[test]
    fn claims_outside_the_window_are_left_out() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "p/old.jsonl",
            &Script::new().user("go").say("Done.").text(),
        );
        // The synthetic transcript is dated 2026-11-14; a window starting
        // after it holds nothing, whatever the file's own mtime says.
        let after = claims::parse_timestamp_ms("2026-11-15T00:00:00Z").unwrap();
        let (claims, _) = collect(dir.path(), after);
        assert!(claims.is_empty());
    }

    #[test]
    fn an_empty_report_says_so() {
        let summary = summarise(&[], &ReportOptions::parse(&[]).unwrap(), 5);
        assert!(render_report(&summary).contains("No assistant turn claimed success in 5"));
        let _ = render_card(&summary);
    }
}
