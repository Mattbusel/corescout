//! Gemini CLI session logs, read into the same claim/check model as Claude
//! Code transcripts.
//!
//! Gemini CLI records every session under
//! `~/.gemini/tmp/<project hash>/chats/`. The format is defined by
//! `packages/core/src/services/chatRecordingService.ts` and
//! `chatRecordingTypes.ts` in <https://github.com/google-gemini/gemini-cli>:
//!
//! * **JSON lines** (current, `session-<time>-<id>.jsonl`): the first line is
//!   the session's metadata (`sessionId`, `projectHash`, `startTime`, ...).
//!   Each later line is a message record (`id`, `timestamp`, `type` of `user`,
//!   `gemini`, `info`, `error` or `warning`, `content`, and for `gemini`
//!   messages `toolCalls`), or an edit to what came before: a message record
//!   whose `id` was already written replaces it in place, `{"$rewindTo": id}`
//!   drops that message and everything after it, `{"$patch": ...}` updates a
//!   message's content or a tool call's result, removes messages or reorders
//!   them, and `{"$set": ...}` updates the metadata. They are replayed in
//!   order, the way Gemini CLI does when it resumes a session.
//! * **JSON** (before the switch, `session-<time>-<id>.json`): one object,
//!   the metadata plus `messages`.
//!
//! A shell command is a tool call named `run_shell_command` with
//! `args.command`. Its result is a `functionResponse` whose
//! `response.output` reads `Output: ...` and, when the command failed, an
//! `Exit Code: N` line (older versions always wrote `Exit Code:`, with
//! `(none)` when a signal ended it). A command that was cancelled, sent to
//! the background or could not start has no exit code.

use std::collections::HashMap;

use serde_json::Value;

use crate::agents::{parts_text, Agent};
use crate::claims::{fold, Event, Run, Transcript};

/// Read one Gemini CLI session.
pub fn analyse(text: &str) -> Transcript {
    let session = replay(text);
    let mut transcript = fold(Agent::Gemini, session.events());
    transcript.project = session.project;
    transcript
}

/// A session as Gemini CLI would load it: the messages in order, after every
/// rewind and patch.
#[derive(Default)]
struct Session {
    id: String,
    project: Option<String>,
    order: Vec<String>,
    messages: HashMap<String, Value>,
}

impl Session {
    fn metadata(&mut self, record: &Value) {
        if let Some(id) = record["sessionId"].as_str() {
            self.id = id.to_string();
        }
        if let Some(hash) = record["projectHash"].as_str() {
            self.project = Some(hash.to_string());
        }
        if let Some(messages) = record["messages"].as_array() {
            for message in messages {
                self.put(message.clone());
            }
        }
    }

    /// Add a message, or replace the one with its id where it stands.
    fn put(&mut self, message: Value) {
        let Some(id) = message["id"].as_str().map(str::to_string) else {
            return;
        };
        if !self.messages.contains_key(&id) {
            self.order.push(id.clone());
        }
        self.messages.insert(id, message);
    }

    fn rewind(&mut self, id: &str) {
        match self.order.iter().position(|known| known == id) {
            Some(at) => {
                for removed in self.order.drain(at..) {
                    self.messages.remove(&removed);
                }
            }
            None => {
                self.order.clear();
                self.messages.clear();
            }
        }
    }

    fn patch_one(&mut self, patch: &Value) {
        let Some(message) = patch["id"]
            .as_str()
            .and_then(|id| self.messages.get_mut(id))
        else {
            return;
        };
        if !patch["content"].is_null() {
            message["content"] = patch["content"].clone();
        }
        let (Some(updates), Some(calls)) = (
            patch["toolCalls"].as_array(),
            message["toolCalls"].as_array_mut(),
        ) else {
            return;
        };
        for update in updates {
            let Some(call) = calls.iter_mut().find(|c| c["id"] == update["id"]) else {
                continue;
            };
            if let Some(result) = update.get("result") {
                call["result"] = result.clone();
            }
        }
    }

    fn patch(&mut self, patch: &Value) {
        if patch["id"].is_string() {
            self.patch_one(patch);
        }
        for update in patch["updates"].as_array().into_iter().flatten() {
            self.patch_one(update);
        }
        for id in patch["removeIds"].as_array().into_iter().flatten() {
            if let Some(id) = id.as_str() {
                self.messages.remove(id);
                self.order.retain(|known| known != id);
            }
        }
        if let Some(ids) = patch["orderIds"].as_array() {
            let listed: Vec<String> = ids
                .iter()
                .filter_map(Value::as_str)
                .filter(|id| self.messages.contains_key(*id))
                .map(str::to_string)
                .collect();
            let mut order: Vec<String> = self
                .order
                .iter()
                .filter(|id| !listed.contains(id))
                .cloned()
                .collect();
            order.extend(listed);
            self.order = order;
        }
    }

    fn events(&self) -> Vec<Event> {
        let mut events = Vec::new();
        for id in &self.order {
            let Some(message) = self.messages.get(id) else {
                continue;
            };
            match message["type"].as_str() {
                Some("user") => events.push(Event::Prompt),
                Some("gemini") => {
                    // Gemini CLI attaches a response's tool calls to the
                    // message that holds the text it wrote before them.
                    events.push(Event::Text {
                        text: parts_text(&message["content"]),
                        session: self.id.clone(),
                        timestamp: message["timestamp"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                    });
                    for call in message["toolCalls"].as_array().into_iter().flatten() {
                        if let Some(run) = shell_run(call) {
                            events.push(Event::Run(run));
                        }
                    }
                }
                _ => {}
            }
        }
        events
    }
}

fn replay(text: &str) -> Session {
    let mut session = Session::default();
    // The JSON format is one object; JSON lines never parse as one.
    if let Ok(whole @ Value::Object(_)) = serde_json::from_str::<Value>(text.trim()) {
        if whole["messages"].is_array() {
            session.metadata(&whole);
            return session;
        }
    }
    for line in text.lines() {
        let Ok(record @ Value::Object(_)) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(id) = record["$rewindTo"].as_str() {
            session.rewind(id);
        } else if record["$patch"].is_object() {
            session.patch(&record["$patch"]);
        } else if record["id"].is_string() {
            session.put(record);
        } else if record["$set"].is_object() {
            session.metadata(&record["$set"]);
        } else if record["sessionId"].is_string() {
            session.metadata(&record);
        }
    }
    session
}

/// The text a tool call returned.
fn result_text(call: &Value) -> String {
    let mut texts = Vec::new();
    let parts: Vec<&Value> = match &call["result"] {
        Value::Array(parts) => parts.iter().collect(),
        Value::Null => Vec::new(),
        other => vec![other],
    };
    for part in parts {
        if let Some(text) = part.as_str() {
            texts.push(text.to_string());
            continue;
        }
        let response = &part["functionResponse"]["response"];
        if let Some(output) = response["output"].as_str() {
            texts.push(output.to_string());
        } else if let Some(error) = response["error"].as_str() {
            texts.push(error.to_string());
        } else if let Some(text) = part["text"].as_str() {
            texts.push(text.to_string());
        }
    }
    if texts.is_empty() {
        if let Some(display) = call["resultDisplay"].as_str() {
            texts.push(display.to_string());
        }
    }
    texts.join("\n")
}

/// A shell tool call as a [`Run`], or `None` for any other tool.
fn shell_run(call: &Value) -> Option<Run> {
    let name = call["name"].as_str().unwrap_or_default();
    if !matches!(name, "run_shell_command" | "shell") {
        return None;
    }
    let command = call["args"]["command"].as_str()?.to_string();
    let output = result_text(call);
    let exit = shell_exit(call, &output);
    Some(Run::new(command, exit, &output))
}

fn shell_exit(call: &Value, output: &str) -> Option<i64> {
    if call["args"]["is_background"].as_bool() == Some(true) {
        return None;
    }
    let status = call["status"].as_str().unwrap_or_default();
    // A result patched in later (`$patch`) leaves the status the call had
    // when it was first written, so an unfinished status only means "no
    // exit code" when there is no result either.
    if status == "cancelled" || (call["result"].is_null() && status != "success") {
        return None;
    }
    // Gemini CLI writes the exit code after the output, so read from the end
    // and a line inside the output cannot pass for it.
    for line in output.lines().rev() {
        if let Some(code) = line.trim().strip_prefix("Exit Code:") {
            return code.trim().parse().ok();
        }
        if line.trim_start().starts_with("Output:") {
            break;
        }
    }
    let unfinished = [
        "Command moved to background",
        "Command is running in background",
        "Command was cancelled",
        "Command was automatically cancelled",
    ];
    if unfinished.iter().any(|marker| output.contains(marker)) {
        return None;
    }
    let spawn_error = output.lines().any(|line| {
        line.strip_prefix("Error: ")
            .is_some_and(|rest| rest.trim() != "(none)")
    });
    if status == "error" || spawn_error || output.trim().is_empty() {
        return None;
    }
    Some(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SESSION: &str = include_str!("../fixtures/gemini/session.jsonl");
    const LEGACY: &str = include_str!("../fixtures/gemini/legacy.json");

    #[test]
    fn a_session_is_replayed_with_its_rewinds_and_patches() {
        let t = analyse(SESSION);
        assert_eq!(t.project.as_deref(), Some("9c1f0e7a5b"));
        // The rewound "Done." is gone; the re-appended message counts once.
        assert_eq!(t.claims.len(), 3, "{:#?}", t.claims);
        for claim in &t.claims {
            assert_eq!(claim.agent, Agent::Gemini);
            assert_eq!(claim.session, "6f1d2a40-1c4e-4d0b-9a57-0e2b7c9d8a11");
        }

        // npm test exited 1.
        assert_eq!(t.claims[0].phrase, "tests pass");
        assert!(!t.claims[0].backed);
        assert_eq!(t.claims[0].last_command.as_deref(), Some("npm test"));
        assert_eq!(t.claims[0].last_exit, Some(1));

        // The second run's result arrived in a $patch: it passed.
        assert!(t.claims[1].backed);
        assert_eq!(t.claims[1].last_exit, Some(0));

        // `git push` is not a check; the report credits the turn before,
        // the gate would not.
        assert_eq!(t.claims[2].phrase, "deployed");
        assert_eq!(
            t.claims[2].last_command.as_deref(),
            Some("git push origin main")
        );
        assert!(t.claims[2].backed);
        assert!(!t.final_turn.passed);
        assert_eq!(
            t.final_turn.last_text.as_deref(),
            Some("Pushed to main. Deployed.")
        );
    }

    #[test]
    fn the_older_single_object_format_is_read() {
        let t = analyse(LEGACY);
        assert_eq!(t.project.as_deref(), Some("4e2b9d"));
        assert_eq!(t.claims.len(), 2, "{:#?}", t.claims);
        assert!(t.claims[0].backed);
        assert_eq!(t.claims[0].last_command.as_deref(), Some("cargo test"));
        assert_eq!(t.claims[0].last_exit, Some(0));
        assert!(!t.claims[1].backed);
        assert_eq!(t.claims[1].last_exit, Some(101));
        // A turn in between claimed nothing, so the one before cannot back it.
    }

    #[test]
    fn exit_codes_follow_gemini_cli_output() {
        let call = |status: &str, output: &str, background: bool| {
            json!({"name":"run_shell_command","status":status,
                   "args":{"command":"cargo test","is_background":background},
                   "result":[{"functionResponse":{"id":"x","name":"run_shell_command",
                              "response":{"output":output}}}]})
        };
        let exit = |c: Value| shell_run(&c).unwrap().exit;
        assert_eq!(
            exit(call("success", "Output: ok\nProcess Group PGID: 1", false)),
            Some(0)
        );
        assert_eq!(
            exit(call("success", "Output: bad\nExit Code: 2", false)),
            Some(2)
        );
        // Something in the output that looks like the trailer is not it.
        assert_eq!(
            exit(call(
                "success",
                "Output: Exit Code: 9\nok\nProcess Group PGID: 1",
                false
            )),
            Some(0)
        );
        assert_eq!(
            exit(call(
                "success",
                "Output: x\nExit Code: (none)\nSignal: 9",
                false
            )),
            None
        );
        assert_eq!(
            exit(call(
                "success",
                "Command moved to background (PID: 4). Output hidden.",
                false
            )),
            None
        );
        assert_eq!(exit(call("success", "Output: (empty)", true)), None);
        assert_eq!(exit(call("cancelled", "Output: (empty)", false)), None);
        assert_eq!(
            exit(call(
                "success",
                "Output: (empty)\nError: spawn ENOENT",
                false
            )),
            None
        );
        assert!(shell_run(&json!({"name":"read_file","args":{"path":"x"}})).is_none());
    }
}
