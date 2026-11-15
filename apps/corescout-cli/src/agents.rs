//! Which AI coding agent wrote a transcript, where each one keeps them, and
//! the reader for each format.
//!
//! `corescout report` and `corescout gate` judge every agent the same way:
//! each format is read into the events `crate::claims::fold` judges. This
//! module knows the agents:
//!
//! * **Claude Code** writes JSON lines to `~/.claude/projects/<project>/<session>.jsonl`
//!   (`$CLAUDE_CONFIG_DIR/projects` when set).
//! * **OpenAI Codex CLI** writes a "rollout" of JSON lines per session to
//!   `~/.codex/sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl`, and moves archived
//!   ones to `~/.codex/archived_sessions` (`$CODEX_HOME` instead of `~/.codex`
//!   when set). See `crate::codex`.
//! * **Gemini CLI** writes each session to
//!   `~/.gemini/tmp/<project hash>/chats/session-<time>-<id>.jsonl` (`.json`
//!   before it switched to JSON lines; `$GEMINI_CLI_HOME` instead of the home
//!   directory when set). See `crate::gemini`.
//!
//! Cursor is not here: its hooks hand over a `transcript_path`, but Cursor
//! does not document the transcript's format, and a reader guessed from one
//! version's files would break silently on the next.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::claims::Transcript;

/// An AI coding agent whose transcripts CoreScout can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    /// Anthropic's Claude Code.
    Claude,
    /// OpenAI's Codex CLI (and the Codex IDE extension and app, which write
    /// the same rollout files).
    Codex,
    /// Google's Gemini CLI.
    Gemini,
}

impl Agent {
    /// Every agent, in the order reports list them.
    pub const ALL: [Agent; 3] = [Agent::Claude, Agent::Codex, Agent::Gemini];

    /// The name `--source` takes and `--json` prints: `claude`, `codex`,
    /// `gemini`.
    pub fn key(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Gemini => "gemini",
        }
    }

    /// The product's name, for people.
    pub fn label(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Gemini => "Gemini CLI",
        }
    }

    /// Read one transcript in this agent's format.
    pub fn analyse(self, text: &str) -> Transcript {
        match self {
            Agent::Claude => crate::claims::analyse(text),
            Agent::Codex => crate::codex::analyse(text),
            Agent::Gemini => crate::gemini::analyse(text),
        }
    }

    /// The folders this agent keeps its transcripts in on this machine,
    /// whether or not they exist.
    pub fn default_dirs(self) -> Vec<PathBuf> {
        let home = || std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        match self {
            Agent::Claude => crate::report::claude_projects_dir().into_iter().collect(),
            Agent::Codex => {
                let root = match std::env::var_os("CODEX_HOME") {
                    Some(dir) => Some(PathBuf::from(dir)),
                    None => home().map(|h| PathBuf::from(h).join(".codex")),
                };
                root.map(|root| vec![root.join("sessions"), root.join("archived_sessions")])
                    .unwrap_or_default()
            }
            Agent::Gemini => {
                let base = std::env::var_os("GEMINI_CLI_HOME").or_else(home);
                base.map(|b| vec![PathBuf::from(b).join(".gemini").join("tmp")])
                    .unwrap_or_default()
            }
        }
    }

    /// Whether a file under one of [`Agent::default_dirs`] is one of this
    /// agent's transcripts, judged by its path.
    pub fn owns_path(self, relative: &Path) -> bool {
        let extension = relative.extension().and_then(|e| e.to_str());
        match self {
            Agent::Claude | Agent::Codex => extension == Some("jsonl"),
            // Gemini CLI keeps other JSON (logs, checkpoints, shell history)
            // beside its chats; only `chats/` holds sessions.
            Agent::Gemini => {
                matches!(extension, Some("jsonl") | Some("json"))
                    && relative
                        .parent()
                        .and_then(Path::file_name)
                        .is_some_and(|name| name == "chats")
            }
        }
    }
}

impl std::str::FromStr for Agent {
    type Err = String;

    fn from_str(name: &str) -> Result<Agent, String> {
        match name.to_ascii_lowercase().as_str() {
            "claude" | "claude-code" | "claudecode" => Ok(Agent::Claude),
            "codex" | "openai-codex" => Ok(Agent::Codex),
            "gemini" | "gemini-cli" => Ok(Agent::Gemini),
            other => Err(format!(
                "{other:?} is not a source; try claude, codex, gemini or all"
            )),
        }
    }
}

/// Which of `--source claude|codex|gemini|all` was asked for.
pub fn parse_sources(value: &str) -> Result<Vec<Agent>, String> {
    if value.eq_ignore_ascii_case("all") {
        return Ok(Agent::ALL.to_vec());
    }
    let mut out = Vec::new();
    for name in value.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        let agent: Agent = name.parse()?;
        if !out.contains(&agent) {
            out.push(agent);
        }
    }
    if out.is_empty() {
        return Err("--source needs claude, codex, gemini or all".to_string());
    }
    out.sort();
    Ok(out)
}

/// Which agent wrote this transcript, from its contents.
///
/// It looks at the first lines that parse as JSON: a Codex rollout's entries
/// are wrapped as `{"timestamp", "type": "session_meta" | "response_item" |
/// "event_msg" | ..., "payload"}`; a Gemini CLI session is one JSON object
/// with `sessionId` and `messages`, or JSON lines that start with a record
/// carrying `sessionId` and `projectHash`. Anything else is read as Claude
/// Code, which is what every file was read as before other agents existed.
pub fn detect(text: &str) -> Agent {
    let start = text.trim_start();
    if start.starts_with('{') {
        // A legacy Gemini session is one pretty-printed object; parsing the
        // whole text is the only way to know, so look for its keys first.
        let head: String = start.chars().take(4096).collect();
        if head.contains("\"sessionId\"")
            && (head.contains("\"projectHash\"") || head.contains("\"messages\""))
        {
            if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(start) {
                if map.get("messages").is_some_and(Value::is_array) {
                    return Agent::Gemini;
                }
            }
        }
    }
    for line in text.lines().take(50) {
        let Ok(Value::Object(entry)) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(found) = detect_entry(&entry) {
            return found;
        }
    }
    Agent::Claude
}

fn detect_entry(entry: &serde_json::Map<String, Value>) -> Option<Agent> {
    let kind = entry.get("type").and_then(Value::as_str);
    if entry.contains_key("payload")
        && matches!(
            kind,
            Some(
                "session_meta"
                    | "response_item"
                    | "event_msg"
                    | "turn_context"
                    | "compacted"
                    | "world_state"
            )
        )
    {
        return Some(Agent::Codex);
    }
    // Codex's first rollout format: a bare header line, then bare items.
    if entry.contains_key("instructions") && entry.contains_key("id") && kind.is_none() {
        return Some(Agent::Codex);
    }
    if entry.contains_key("record_type") {
        return Some(Agent::Codex);
    }
    if entry.contains_key("sessionId") && entry.contains_key("projectHash") {
        return Some(Agent::Gemini);
    }
    if entry.contains_key("$set") || entry.contains_key("$rewindTo") || entry.contains_key("$patch")
    {
        return Some(Agent::Gemini);
    }
    if kind == Some("gemini") {
        return Some(Agent::Gemini);
    }
    if matches!(kind, Some("user" | "assistant")) && entry.contains_key("message") {
        return Some(Agent::Claude);
    }
    None
}

/// The text of a list of `{ "text": ... }` parts, a bare string, or one part.
pub fn parts_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|part| part["thought"].as_bool() != Some(true))
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(_) => value["text"].as_str().unwrap_or_default().to_string(),
        _ => String::new(),
    }
}

/// One command line from an argument vector. A shell wrapper
/// (`["bash", "-lc", "cargo test"]`, `["powershell.exe", "-Command", "..."]`)
/// becomes the script it runs; anything else is joined with spaces, quoting
/// arguments that contain them.
pub fn command_line(argv: &[String]) -> String {
    if argv.len() >= 2 {
        let first = argv[0].rsplit(['/', '\\']).next().unwrap_or(&argv[0]);
        let first = first.to_ascii_lowercase();
        let shell = first.trim_end_matches(".exe");
        if matches!(
            shell,
            "bash" | "sh" | "zsh" | "dash" | "fish" | "powershell" | "pwsh" | "cmd"
        ) {
            if let Some(script) = argv.last() {
                return script.clone();
            }
        }
    }
    argv.iter()
        .map(|arg| {
            if arg.chars().any(char::is_whitespace) {
                format!("\"{arg}\"")
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// An argument vector from a JSON array of strings.
pub fn argv(value: &Value) -> Option<Vec<String>> {
    let items = value.as_array()?;
    Some(
        items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_parse() {
        assert_eq!(parse_sources("all").unwrap(), Agent::ALL.to_vec());
        assert_eq!(parse_sources("codex").unwrap(), vec![Agent::Codex]);
        assert_eq!(
            parse_sources("gemini,claude").unwrap(),
            vec![Agent::Claude, Agent::Gemini]
        );
        assert!(parse_sources("cursor")
            .unwrap_err()
            .contains("claude, codex"));
        assert!(parse_sources("").is_err());
    }

    #[test]
    fn shell_wrappers_become_the_script_they_run() {
        let v = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            command_line(&v(&["bash", "-lc", "cargo test"])),
            "cargo test"
        );
        assert_eq!(
            command_line(&v(&[
                "C:\\WINDOWS\\System32\\WindowsPowerShell\\v1.0\\powershell.exe",
                "-Command",
                "npm test"
            ])),
            "npm test"
        );
        assert_eq!(
            command_line(&v(&["cargo", "test", "--", "a b"])),
            "cargo test -- \"a b\""
        );
    }

    #[test]
    fn gemini_owns_only_files_in_chats() {
        assert!(Agent::Gemini.owns_path(Path::new("abc/chats/session-1.jsonl")));
        assert!(Agent::Gemini.owns_path(Path::new("abc/chats/session-1.json")));
        assert!(!Agent::Gemini.owns_path(Path::new("abc/logs.json")));
        assert!(Agent::Codex.owns_path(Path::new("2026/11/14/rollout-x.jsonl")));
        assert!(!Agent::Codex.owns_path(Path::new("2026/11/14/rollout-x.json")));
    }

    #[test]
    fn detection_tells_the_formats_apart() {
        assert_eq!(
            detect(include_str!("../fixtures/codex/paginated.jsonl")),
            Agent::Codex
        );
        assert_eq!(
            detect(include_str!("../fixtures/codex/legacy.jsonl")),
            Agent::Codex
        );
        assert_eq!(
            detect(include_str!("../fixtures/codex/original.jsonl")),
            Agent::Codex
        );
        assert_eq!(
            detect(include_str!("../fixtures/gemini/session.jsonl")),
            Agent::Gemini
        );
        assert_eq!(
            detect(include_str!("../fixtures/gemini/legacy.json")),
            Agent::Gemini
        );
        let claude = crate::claims::tests::Script::new()
            .user("go")
            .say("Done.")
            .text();
        assert_eq!(detect(&claude), Agent::Claude);
        assert_eq!(detect("not json at all"), Agent::Claude);
    }
}
