//! OpenAI Codex CLI session logs, read into the same claim/check model as
//! Claude Code transcripts.
//!
//! Codex writes one "rollout" per session, as JSON lines, to
//! `~/.codex/sessions/YYYY/MM/DD/rollout-<time>-<id>.jsonl`. The format is
//! defined by `codex-rs/protocol` and `codex-rs/rollout` in
//! <https://github.com/openai/codex>, and it has changed shape three times, so
//! this reads all three:
//!
//! * **Paginated** (current): each line is `{"timestamp", "type", "payload"}`.
//!   What the user typed, what the agent said and every command it ran are
//!   `event_msg` lines whose payload is `{"type": "item_completed", "item":
//!   {...}}`, with `item.type` `UserMessage`, `AgentMessage` or
//!   `CommandExecution` (`command` as an argument vector, `exit_code`,
//!   `aggregated_output`, `status`, `source`). The raw model items
//!   (`response_item`) are there too and are ignored, because the completed
//!   items say the same thing without the injected instructions.
//! * **Legacy** (2025 to mid 2026): the same wrapper, but the conversation is
//!   `event_msg` `user_message` and `agent_message`, and commands are only in
//!   the raw items: a `function_call` to `shell` (`{"command": [argv]}`),
//!   `shell_command` (`{"command": "..."}`) or `exec_command` (`{"cmd": "..."}`),
//!   a `local_shell_call`, or a code-mode `custom_tool_call` named `exec` whose
//!   script calls `tools.exec_command({cmd: ...})`, each answered by a
//!   `*_output` item with the same `call_id`. The exit code is in the output:
//!   `{"metadata": {"exit_code": N}}`, an `Exit code: N` or `Process exited
//!   with code N` header line, or an `exit_code` field.
//! * **Original** (spring 2025): a bare header line `{"id", "timestamp",
//!   "instructions"}`, then bare response items and `{"record_type": "state"}`
//!   lines, with no timestamps of their own.
//!
//! A command the user ran themselves (`!cmd`, `source: "user_shell"`) is not
//! the agent checking its work and is left out, as is input written to a
//! command that was already running. A command still running when its call
//! returned has no exit code, so it never backs a claim.

use std::collections::HashMap;

use serde_json::Value;

use crate::agents::{argv, command_line, parts_text, Agent};
use crate::claims::{fold, Event, Run, Transcript};

/// Read one Codex rollout.
pub fn analyse(text: &str) -> Transcript {
    let (events, project) = events(text);
    let mut transcript = fold(Agent::Codex, events);
    transcript.project = project;
    transcript
}

/// One rollout line, unwrapped.
struct Line {
    kind: String,
    payload: Value,
    timestamp: Option<String>,
}

fn lines(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let Ok(Value::Object(mut entry)) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        let timestamp = entry
            .get("timestamp")
            .and_then(Value::as_str)
            .map(str::to_string);
        if entry.contains_key("record_type") {
            continue;
        }
        let kind = entry
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let (Some(kind), Some(payload)) = (kind.clone(), entry.remove("payload")) {
            out.push(Line {
                kind,
                payload,
                timestamp,
            });
        } else if kind.is_none() && entry.contains_key("instructions") {
            out.push(Line {
                kind: "session_meta".into(),
                payload: Value::Object(entry),
                timestamp,
            });
        } else if kind.is_some() {
            out.push(Line {
                kind: "response_item".into(),
                payload: Value::Object(entry),
                timestamp,
            });
        }
    }
    out
}

fn payload_type(line: &Line) -> &str {
    line.payload["type"].as_str().unwrap_or_default()
}

fn item_type(line: &Line) -> &str {
    if line.kind == "event_msg" && payload_type(line) == "item_completed" {
        line.payload["item"]["type"].as_str().unwrap_or_default()
    } else {
        ""
    }
}

/// The [`Event`]s in a rollout, and the working directory it ran in.
pub fn events(text: &str) -> (Vec<Event>, Option<String>) {
    let lines = lines(text);

    // Which generation of the format this is decides where the conversation
    // and the commands are read from, so nothing is counted twice.
    let items_text = lines
        .iter()
        .any(|l| matches!(item_type(l), "UserMessage" | "AgentMessage"));
    let item_commands = lines.iter().any(|l| item_type(l) == "CommandExecution");
    let end_events = lines
        .iter()
        .any(|l| l.kind == "event_msg" && payload_type(l) == "exec_command_end");
    let legacy_text = lines.iter().any(|l| {
        l.kind == "event_msg" && matches!(payload_type(l), "user_message" | "agent_message")
    });

    let mut session = String::new();
    let mut project = None;
    let mut clock = String::new();
    let mut events = Vec::new();
    // call_id -> the commands that call runs
    let mut pending: HashMap<String, Vec<String>> = HashMap::new();

    for line in &lines {
        if let Some(stamp) = &line.timestamp {
            clock = stamp.clone();
        }
        let payload = &line.payload;
        let text_event = |text: String, clock: &str, session: &str| Event::Text {
            text,
            session: session.to_string(),
            timestamp: clock.to_string(),
        };
        match (line.kind.as_str(), payload_type(line)) {
            ("session_meta", _) => {
                let meta = if payload["meta"].is_object() {
                    &payload["meta"]
                } else {
                    payload
                };
                if let Some(id) = meta["id"].as_str().or(meta["session_id"].as_str()) {
                    session = id.to_string();
                }
                if let Some(cwd) = meta["cwd"].as_str() {
                    project = Some(cwd.to_string());
                }
                if clock.is_empty() {
                    if let Some(stamp) = meta["timestamp"].as_str() {
                        clock = stamp.to_string();
                    }
                }
            }
            ("event_msg", "item_completed") => {
                let item = &payload["item"];
                match item["type"].as_str().unwrap_or_default() {
                    "UserMessage" if items_text => events.push(Event::Prompt),
                    "AgentMessage" if items_text => {
                        events.push(text_event(parts_text(&item["content"]), &clock, &session));
                    }
                    "CommandExecution" => {
                        if let Some(run) = executed(item) {
                            events.push(Event::Run(run));
                        }
                    }
                    _ => {}
                }
            }
            ("event_msg", "user_message") if !items_text => {
                // Early 2025 rollouts tagged injected instructions as user
                // messages of another kind.
                let kind = payload["kind"].as_str().unwrap_or("plain");
                if kind == "plain" {
                    events.push(Event::Prompt);
                }
            }
            ("event_msg", "agent_message") if !items_text => {
                let message = payload["message"].as_str().unwrap_or_default();
                events.push(text_event(message.to_string(), &clock, &session));
            }
            ("event_msg", "exec_command_end") if !item_commands => {
                if let Some(run) = executed(payload) {
                    events.push(Event::Run(run));
                }
            }
            ("response_item", "message") if !items_text && !legacy_text => {
                let text = parts_text(&payload["content"]);
                match payload["role"].as_str() {
                    Some("user") if !injected(&text) => events.push(Event::Prompt),
                    Some("assistant") => events.push(text_event(text, &clock, &session)),
                    _ => {}
                }
            }
            ("response_item", "function_call" | "custom_tool_call" | "local_shell_call")
                if !item_commands && !end_events =>
            {
                let id = payload["call_id"]
                    .as_str()
                    .or(payload["id"].as_str())
                    .unwrap_or_default();
                let commands = commands_called(payload);
                if !id.is_empty() && !commands.is_empty() {
                    pending.insert(id.to_string(), commands);
                }
            }
            ("response_item", "function_call_output" | "custom_tool_call_output")
                if !item_commands && !end_events =>
            {
                let id = payload["call_id"].as_str().unwrap_or_default();
                let Some(commands) = pending.remove(id) else {
                    continue;
                };
                let (codes, output) = exit_codes(&payload["output"]);
                // A script that ran several commands reports one code each;
                // when the counts disagree nothing says which is which.
                let paired = commands.len() == 1 || codes.len() == commands.len();
                for (at, command) in commands.into_iter().enumerate() {
                    let exit = if paired {
                        codes.get(at).copied().flatten()
                    } else {
                        None
                    };
                    events.push(Event::Run(Run::new(command, exit, &output)));
                }
            }
            _ => {}
        }
    }
    (events, project)
}

/// Text Codex put into a user message itself: instructions, AGENTS.md, the
/// environment. Only consulted for rollouts old enough to have nothing else.
fn injected(text: &str) -> bool {
    let start = text.trim_start();
    start.is_empty() || start.starts_with('<') || start.starts_with("# AGENTS.md")
}

/// A finished command from a `CommandExecution` item or an
/// `exec_command_end` event. `None` for one the agent did not run.
fn executed(item: &Value) -> Option<Run> {
    if matches!(
        item["source"].as_str(),
        Some("user_shell" | "unified_exec_interaction")
    ) {
        return None;
    }
    let command = match &item["command"] {
        Value::String(line) => line.clone(),
        other => command_line(&argv(other)?),
    };
    if command.trim().is_empty() {
        return None;
    }
    let finished = !matches!(
        item["status"].as_str(),
        Some("declined" | "in_progress" | "inProgress")
    );
    let exit = if finished {
        item["exit_code"].as_i64()
    } else {
        None
    };
    let output = match item["aggregated_output"].as_str() {
        Some(output) => output.to_string(),
        None => format!(
            "{}{}",
            item["stdout"].as_str().unwrap_or_default(),
            item["stderr"].as_str().unwrap_or_default()
        ),
    };
    Some(Run::new(command, exit, &output))
}

/// The shell commands a tool call runs, if it is a shell tool at all.
fn commands_called(payload: &Value) -> Vec<String> {
    let name = payload["name"].as_str().unwrap_or_default();
    match payload["type"].as_str().unwrap_or_default() {
        "local_shell_call" => argv(&payload["action"]["command"])
            .map(|a| vec![command_line(&a)])
            .unwrap_or_default(),
        "custom_tool_call" if name == "exec" => {
            exec_command_literals(payload["input"].as_str().unwrap_or_default())
        }
        "function_call" => {
            let arguments = match &payload["arguments"] {
                Value::String(json) => serde_json::from_str(json).unwrap_or(Value::Null),
                other => other.clone(),
            };
            let command = match name {
                "shell" | "container.exec" | "local_shell" | "shell_command" => {
                    match &arguments["command"] {
                        Value::String(line) => Some(line.clone()),
                        other => argv(other).map(|a| command_line(&a)),
                    }
                }
                "exec_command" => arguments["cmd"].as_str().map(str::to_string),
                _ => None,
            };
            command
                .into_iter()
                .filter(|c| !c.trim().is_empty())
                .collect()
        }
        _ => Vec::new(),
    }
}

/// The `cmd` string literals of every `exec_command({cmd: ...})` call in a
/// code-mode script, in order.
fn exec_command_literals(script: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = script;
    while let Some(at) = rest.find("exec_command(") {
        rest = &rest[at + "exec_command(".len()..];
        let Some(key) = rest.find("cmd") else {
            break;
        };
        let after = rest[key + 3..].trim_start_matches(['"', '\'']).trim_start();
        let Some(value) = after.strip_prefix(':') else {
            continue;
        };
        if let Some((literal, remaining)) = string_literal(value.trim_start()) {
            out.push(literal);
            rest = remaining;
        }
    }
    out
}

/// A JavaScript string literal at the start of `text`, unescaped, and what
/// follows it.
fn string_literal(text: &str) -> Option<(String, &str)> {
    let quote = text
        .chars()
        .next()
        .filter(|c| matches!(c, '"' | '\'' | '`'))?;
    let mut out = String::new();
    let mut chars = text.char_indices().skip(1);
    while let Some((at, ch)) = chars.next() {
        match ch {
            '\\' => {
                let (_, escaped) = chars.next()?;
                out.push(match escaped {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    other => other,
                });
            }
            c if c == quote => return Some((out, &text[at + 1..])),
            c => out.push(c),
        }
    }
    None
}

/// The exit codes reported in a tool call's output, in order, and the
/// output's text. A command still running when the call returned reports
/// none.
fn exit_codes(output: &Value) -> (Vec<Option<i64>>, String) {
    let parts: Vec<String> = match output {
        Value::String(text) => vec![text.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item["text"].as_str().map(str::to_string))
            .collect(),
        Value::Object(_) => vec![output["content"]
            .as_str()
            .or(output["output"].as_str())
            .unwrap_or_default()
            .to_string()],
        _ => Vec::new(),
    };
    let mut codes = Vec::new();
    let mut texts = Vec::new();
    for part in &parts {
        if let Ok(Value::Object(json)) = serde_json::from_str::<Value>(part.trim()) {
            let json = Value::Object(json);
            let code = json["metadata"]["exit_code"]
                .as_i64()
                .or(json["exit_code"].as_i64());
            if let Some(code) = code {
                codes.push(Some(code));
            }
            texts.push(json["output"].as_str().unwrap_or_default().to_string());
            continue;
        }
        if let Some(code) = header_exit_code(part) {
            codes.push(Some(code));
        }
        texts.push(part.clone());
    }
    (codes, texts.join("\n"))
}

/// `Exit code: N` or `Process exited with code N` in the header Codex puts
/// before a command's output. Only the header is read, so a line in the
/// command's own output cannot pass for it.
fn header_exit_code(text: &str) -> Option<i64> {
    for line in text.lines().take(6) {
        let line = line.trim();
        if line == "Output:" {
            break;
        }
        let rest = line
            .strip_prefix("Exit code:")
            .or_else(|| line.strip_prefix("Process exited with code"));
        if let Some(rest) = rest {
            return rest.trim().parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGINATED: &str = include_str!("../fixtures/codex/paginated.jsonl");
    const LEGACY: &str = include_str!("../fixtures/codex/legacy.jsonl");
    const ORIGINAL: &str = include_str!("../fixtures/codex/original.jsonl");

    #[test]
    fn a_paginated_rollout_is_read_from_its_completed_items() {
        let t = analyse(PAGINATED);
        assert_eq!(t.project.as_deref(), Some("/home/dev/parser"));
        // One claim per turn, though each final answer is also in the raw
        // response items.
        assert_eq!(t.claims.len(), 2, "{:#?}", t.claims);

        let first = &t.claims[0];
        assert_eq!(first.agent, Agent::Codex);
        assert_eq!(first.session, "019a0c1e-0000-7000-8000-00000000c0de");
        assert_eq!(first.phrase, "tests pass");
        assert!(first.backed);
        assert_eq!(first.last_command.as_deref(), Some("cargo test"));
        assert_eq!(first.last_exit, Some(0));
        assert!(first.at_ms.is_some());

        // The second turn only ran `git status`; the user's own `!cargo
        // test` does not count as the agent checking, and the turn before
        // passed, so the report counts it as backed while the gate, which
        // only looks at this turn, would not.
        let second = &t.claims[1];
        assert_eq!(second.phrase, "done");
        assert_eq!(second.last_command.as_deref(), Some("git status --short"));
        assert!(second.backed);
        assert!(!t.final_turn.passed);
        assert_eq!(
            t.final_turn.last_run.as_ref().map(|r| r.command.as_str()),
            Some("git status --short")
        );
    }

    #[test]
    fn a_legacy_rollout_is_read_from_events_and_tool_calls() {
        let t = analyse(LEGACY);
        assert_eq!(t.project.as_deref(), Some("/home/dev/web"));
        assert_eq!(t.claims.len(), 3, "{:#?}", t.claims);

        // `shell` with an argv, exit code in JSON metadata: npm test failed.
        assert_eq!(t.claims[0].phrase, "tests pass");
        assert!(!t.claims[0].backed);
        assert_eq!(t.claims[0].last_command.as_deref(), Some("npm test"));
        assert_eq!(t.claims[0].last_exit, Some(1));

        // `shell_command`, exit code in an `Exit code:` header: it passed.
        assert!(t.claims[1].backed);
        assert_eq!(t.claims[1].last_exit, Some(0));

        // `exec_command` still running when it returned: no exit code.
        let third = &t.claims[2];
        assert_eq!(third.phrase, "build passes");
        assert_eq!(third.last_command.as_deref(), Some("npm run build"));
        assert_eq!(third.last_exit, None);
        // Backed by the turn before in the report; not by its own turn.
        assert!(third.backed);
        assert!(!t.final_turn.passed);
    }

    #[test]
    fn the_original_format_is_read_from_bare_items() {
        let t = analyse(ORIGINAL);
        assert_eq!(t.claims.len(), 1, "{:#?}", t.claims);
        let claim = &t.claims[0];
        assert_eq!(claim.phrase, "build passes");
        assert!(claim.backed);
        assert_eq!(claim.last_command.as_deref(), Some("cargo build --release"));
        // The items carry no timestamps; the header's stands in.
        assert_eq!(claim.timestamp, "2025-05-02T09:00:00.000Z");
        assert_eq!(claim.session, "5973b6c0-94b8-487b-a530-2aeb6098ae0e");
    }

    #[test]
    fn code_mode_scripts_yield_their_commands() {
        assert_eq!(
            exec_command_literals(
                "text(await tools.exec_command({cmd:\"cargo test -p a\",max_output_tokens:1000}));"
            ),
            vec!["cargo test -p a"]
        );
        assert_eq!(
            exec_command_literals("await tools.exec_command({ \"cmd\": 'npm run \\'lint\\'' });"),
            vec!["npm run 'lint'"]
        );
        assert!(exec_command_literals("await tools.apply_patch(\"x\")").is_empty());
    }

    #[test]
    fn exit_codes_come_only_from_the_header() {
        let (codes, _) = exit_codes(&Value::String(
            "Exit code: 3\nWall time: 1 seconds\nOutput:\nExit code: 0".into(),
        ));
        assert_eq!(codes, vec![Some(3)]);
        let (codes, _) = exit_codes(&Value::String(
            "Chunk ID: 1\nWall time: 1 seconds\nProcess running with session ID 4\nOutput:\nExit code: 0".into(),
        ));
        assert!(codes.is_empty());
        let (codes, text) = exit_codes(&serde_json::json!(
            "{\"output\":\"test result: FAILED\",\"metadata\":{\"exit_code\":0}}"
        ));
        assert_eq!(codes, vec![Some(0)]);
        assert_eq!(text, "test result: FAILED");
    }

    #[test]
    fn a_code_mode_call_takes_its_exit_code_from_the_chunk() {
        let rollout = [
            r#"{"timestamp":"2026-11-14T10:00:00Z","type":"session_meta","payload":{"id":"s","cwd":"/p"}}"#,
            r#"{"timestamp":"2026-11-14T10:00:01Z","type":"event_msg","payload":{"type":"user_message","message":"test it"}}"#,
            r#"{"timestamp":"2026-11-14T10:00:02Z","type":"response_item","payload":{"type":"custom_tool_call","call_id":"c1","name":"exec","input":"text(await tools.exec_command({cmd:\"pytest -q\"}));"}}"#,
            r#"{"timestamp":"2026-11-14T10:00:03Z","type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"c1","output":[{"type":"input_text","text":"Script completed\nWall time 1.0 seconds\nOutput:\n"},{"type":"input_text","text":"{\"chunk_id\":\"a\",\"exit_code\":0,\"output\":\"3 passed\"}"}]}}"#,
            r#"{"timestamp":"2026-11-14T10:00:04Z","type":"event_msg","payload":{"type":"agent_message","message":"Tests pass."}}"#,
        ]
        .join("\n");
        let t = analyse(&rollout);
        assert_eq!(t.claims.len(), 1);
        assert!(t.claims[0].backed);
        assert!(t.final_turn.passed);
    }
}
