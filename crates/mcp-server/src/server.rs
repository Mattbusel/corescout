//! The protocol: JSON-RPC 2.0 over stdio, one message per line.
//!
//! # Why the backend is a trait
//!
//! [`Backend`] is what a tool call reaches. In the shipped binary it is a
//! client of the local service; in the tests it is a closure. That means the
//! protocol can be exercised — malformed frames, unknown methods, the
//! initialise handshake, a tool that fails — without a service running, and
//! the service can be exercised without a protocol.
//!
//! # Errors are results, not transport failures
//!
//! When a tool fails, the reply is a successful JSON-RPC response whose
//! content says what went wrong and carries `isError`. That is what the
//! protocol asks for, and it is also the useful behaviour: a model that gets a
//! transport error learns nothing, and a model that is told "CoreScout is
//! paused" can say so to the user.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

use crate::tools;

/// The protocol version this speaks.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// Where a tool call goes.
pub trait Backend {
    /// Call a product API method.
    fn call(&self, method: &str, params: &Value) -> Result<Value, String>;

    /// What to tell the client about CoreScout when it connects.
    ///
    /// Returned in the initialise handshake, where a client puts it in front
    /// of the model. This is how the AI comes to know CoreScout exists without
    /// anyone having written it into a prompt.
    fn instructions(&self) -> String;

    /// A client has identified itself.
    ///
    /// Called before [`Backend::instructions`], so the briefing a client
    /// receives is generated after CoreScout knows who is asking. Without
    /// this, an AI can connect and work for an hour and the AI screen still
    /// says nothing has ever connected.
    ///
    /// The default does nothing, so a backend that only answers questions does
    /// not have to care.
    fn connected(&self, _name: &str, _version: Option<&str>) {}
}

/// The stdio server.
pub struct Server<B: Backend> {
    backend: B,
    initialised: bool,
}

impl<B: Backend> Server<B> {
    /// Wrap a backend.
    pub fn new(backend: B) -> Server<B> {
        Server {
            backend,
            initialised: false,
        }
    }

    /// Read messages until the input ends.
    pub fn run(&mut self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Some(reply) = self.handle_line(&line) {
                writeln!(output, "{reply}")?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// Handle one line, returning what to write back.
    ///
    /// `None` for a notification, which by the protocol gets no reply. Sending
    /// one anyway is the classic way to hang a client that is waiting for a
    /// response it will match by id.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let message: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            // A parse error has no id to answer, so it is reported against the
            // null id rather than dropped: a client that sent malformed JSON
            // should be told, not left waiting.
            Err(error) => {
                return Some(
                    error_reply(Value::Null, -32700, &format!("that is not JSON: {error}"))
                        .to_string(),
                )
            }
        };
        let id = message.get("id").cloned();
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let params = message.get("params").cloned().unwrap_or(json!({}));

        let Some(id) = id else {
            // A notification. `initialized` is the one that matters, and it
            // wants no reply.
            if method == "notifications/initialized" {
                self.initialised = true;
            }
            return None;
        };

        Some(self.handle(id, &method, &params).to_string())
    }

    fn handle(&mut self, id: Value, method: &str, params: &Value) -> Value {
        match method {
            "initialize" => {
                self.initialised = true;
                // Announced before the briefing is asked for, so the briefing
                // is generated knowing who it is for.
                if let Some(client) = params.get("clientInfo") {
                    let name = client.get("name").and_then(Value::as_str).unwrap_or("");
                    let version = client.get("version").and_then(Value::as_str);
                    self.backend.connected(name, version);
                }
                reply(
                    id,
                    json!({
                        "protocolVersion": PROTOCOL_VERSION,
                        "capabilities": { "tools": { "listChanged": false } },
                        "serverInfo": {
                            "name": "corescout",
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                        "instructions": self.backend.instructions(),
                    }),
                )
            }
            "ping" => reply(id, json!({})),
            "tools/list" => reply(id, json!({ "tools": tools::describe() })),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                let Some(tool) = tools::find(name) else {
                    return error_reply(
                        id,
                        -32602,
                        &format!("CoreScout has no tool called {name:?}"),
                    );
                };
                match self.backend.call(tool.method, &tool.params(&arguments)) {
                    Ok(value) => reply(id, content(&structured(tool.name, value), false)),
                    // A failed tool is a successful response carrying an
                    // error, so the model reads it and can tell the user.
                    Err(message) => reply(id, content(&json!({ "error": message }), true)),
                }
            }
            // Advertised as absent in the handshake, so a client should not ask.
            "resources/list" => reply(id, json!({ "resources": [] })),
            "prompts/list" => reply(id, json!({ "prompts": [] })),
            other => error_reply(id, -32601, &format!("no method {other:?}")),
        }
    }
}

/// Make a tool's answer into something `structuredContent` may carry.
///
/// The protocol says `structuredContent` is an object. Several of these tools
/// naturally answer with a list -- the failures, the hypotheses, the recent
/// activity -- and a bare array there is not a lax reading of the schema but a
/// message a strict client discards whole, which is what Claude Code does. The
/// tools were unusable rather than degraded, and the text content went with
/// them, so nothing arrived at all.
///
/// The key is the tool's own name with its prefix removed, so `corescout_
/// failures` answers `{"failures": [...]}`. Deriving it beats a table nobody
/// updates when a tool is added, and `count` is here because a model that
/// wants to know whether the list is empty should not have to measure it.
fn structured(tool: &str, value: Value) -> Value {
    if value.is_object() {
        return value;
    }
    let key = tool.strip_prefix("corescout_").unwrap_or(tool);
    let mut object = serde_json::Map::new();
    // A scalar is as unusable as an array, and rarer, so it gets the same
    // treatment rather than a second convention to remember.
    if let Value::Array(items) = &value {
        object.insert("count".into(), json!(items.len()));
    }
    object.insert(key.to_string(), value);
    Value::Object(object)
}

/// Wrap a value as tool content.
///
/// Both a readable rendering and the structured value, because clients differ
/// in which they surface and a model reading the text should see the same
/// facts as one reading the structure.
fn content(value: &Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": value,
        "isError": is_error,
    })
}

fn reply(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_reply(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backend that answers everything, and records what it was asked.
    struct Recording {
        calls: std::sync::Mutex<Vec<(String, Value)>>,
        fail: bool,
    }

    impl Recording {
        fn new() -> Recording {
            Recording {
                calls: std::sync::Mutex::new(Vec::new()),
                fail: false,
            }
        }

        fn failing() -> Recording {
            Recording {
                fail: true,
                ..Recording::new()
            }
        }
    }

    impl Backend for Recording {
        fn connected(&self, name: &str, version: Option<&str>) {
            self.calls.lock().expect("the lock").push((
                "connected".into(),
                json!({ "name": name, "version": version }),
            ));
        }

        fn call(&self, method: &str, params: &Value) -> Result<Value, String> {
            self.calls
                .lock()
                .expect("the lock")
                .push((method.to_string(), params.clone()));
            if self.fail {
                return Err("CoreScout is paused".into());
            }
            Ok(json!({ "method": method }))
        }

        fn instructions(&self) -> String {
            "You are operating inside a computer running CoreScout.".into()
        }
    }

    fn ask<B: Backend>(server: &mut Server<B>, message: Value) -> Value {
        let line = server
            .handle_line(&message.to_string())
            .expect("a reply was expected");
        serde_json::from_str(&line).expect("the reply is JSON")
    }

    #[test]
    fn the_handshake_tells_the_client_what_corescout_is() {
        // This is the mechanism by which the AI comes to know CoreScout
        // exists. Without it a user has to write it into a prompt themselves.
        let mut server = Server::new(Recording::new());
        let reply = ask(
            &mut server,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        );
        assert_eq!(reply["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(reply["result"]["serverInfo"]["name"], "corescout");
        assert!(reply["result"]["instructions"]
            .as_str()
            .expect("instructions")
            .contains("CoreScout"));
    }

    #[test]
    fn a_client_that_names_itself_is_announced_before_the_briefing_is_written() {
        // Otherwise an AI can connect, work for an hour, and the AI screen
        // still says nothing has ever connected.
        let mut server = Server::new(Recording::new());
        ask(
            &mut server,
            json!({
                "jsonrpc":"2.0","id":1,"method":"initialize",
                "params":{"clientInfo":{"name":"claude-code","version":"2.1"}}
            }),
        );
        let calls = server.backend.calls.lock().expect("the lock");
        assert_eq!(calls[0].0, "connected", "{calls:?}");
        assert_eq!(calls[0].1["name"], "claude-code");
        assert_eq!(calls[0].1["version"], "2.1");
    }

    #[test]
    fn a_client_that_names_nothing_still_completes_the_handshake() {
        let mut server = Server::new(Recording::new());
        let reply = ask(
            &mut server,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        );
        assert!(reply["error"].is_null(), "{reply}");
        assert!(server.backend.calls.lock().expect("the lock").is_empty());
    }

    #[test]
    fn a_notification_gets_no_reply() {
        // Answering one is the classic way to hang a client that matches
        // replies by id.
        let mut server = Server::new(Recording::new());
        assert!(server
            .handle_line(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string())
            .is_none());
    }

    #[test]
    fn tools_are_listed_with_their_schemas() {
        let mut server = Server::new(Recording::new());
        let reply = ask(
            &mut server,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        );
        let listed = reply["result"]["tools"].as_array().expect("tools");
        assert_eq!(listed.len(), tools::TOOLS.len());
        assert!(listed.iter().any(|tool| tool["name"] == "corescout_ask"));
    }

    #[test]
    fn calling_a_tool_reaches_the_method_it_names() {
        let mut server = Server::new(Recording::new());
        let reply = ask(
            &mut server,
            json!({
                "jsonrpc":"2.0","id":3,"method":"tools/call",
                "params":{"name":"corescout_status","arguments":{}}
            }),
        );
        assert_eq!(reply["result"]["isError"], false);
        assert_eq!(reply["result"]["structuredContent"]["method"], "status");
        let calls = server.backend.calls.lock().expect("the lock");
        assert_eq!(calls[0].0, "status");
    }

    #[test]
    fn a_tool_that_fails_returns_a_result_the_model_can_read() {
        // A transport error teaches a model nothing. "CoreScout is paused" it
        // can act on and repeat to the user.
        let mut server = Server::new(Recording::failing());
        let reply = ask(
            &mut server,
            json!({
                "jsonrpc":"2.0","id":4,"method":"tools/call",
                "params":{"name":"corescout_status","arguments":{}}
            }),
        );
        assert!(reply["error"].is_null(), "not a transport error: {reply}");
        assert_eq!(reply["result"]["isError"], true);
        assert!(reply["result"]["content"][0]["text"]
            .as_str()
            .expect("text")
            .contains("paused"));
    }

    #[test]
    fn an_unknown_tool_is_a_protocol_error_naming_it() {
        let mut server = Server::new(Recording::new());
        let reply = ask(
            &mut server,
            json!({
                "jsonrpc":"2.0","id":5,"method":"tools/call",
                "params":{"name":"corescout_do_anything","arguments":{}}
            }),
        );
        assert_eq!(reply["error"]["code"], -32602);
        assert!(reply["error"]["message"]
            .as_str()
            .expect("a message")
            .contains("corescout_do_anything"));
    }

    #[test]
    fn malformed_json_is_reported_rather_than_leaving_a_client_waiting() {
        let mut server = Server::new(Recording::new());
        let line = server.handle_line("{not json").expect("a reply");
        let reply: Value = serde_json::from_str(&line).expect("the reply is JSON");
        assert_eq!(reply["error"]["code"], -32700);
        assert!(reply["id"].is_null());
    }

    #[test]
    fn an_unknown_method_is_answered_rather_than_ignored() {
        let mut server = Server::new(Recording::new());
        let reply = ask(
            &mut server,
            json!({"jsonrpc":"2.0","id":6,"method":"something/else"}),
        );
        assert_eq!(reply["error"]["code"], -32601);
    }

    #[test]
    fn resources_and_prompts_are_answered_as_empty_rather_than_erroring() {
        // Clients ask regardless of what the handshake advertised. An error
        // here shows up in a user's log as a red line about a server that is
        // working correctly.
        let mut server = Server::new(Recording::new());
        for method in ["resources/list", "prompts/list"] {
            let reply = ask(&mut server, json!({"jsonrpc":"2.0","id":7,"method":method}));
            assert!(reply["error"].is_null(), "{method}: {reply}");
        }
    }

    #[test]
    fn a_ping_is_answered() {
        let mut server = Server::new(Recording::new());
        let reply = ask(&mut server, json!({"jsonrpc":"2.0","id":8,"method":"ping"}));
        assert!(reply["error"].is_null());
    }

    #[test]
    fn every_reply_carries_the_id_it_was_asked_with() {
        // A client matches replies by id. Getting this wrong makes every call
        // time out in a way that looks like the server is slow.
        let mut server = Server::new(Recording::new());
        for id in [json!(1), json!("abc"), json!(999_999)] {
            let reply = ask(
                &mut server,
                json!({"jsonrpc":"2.0","id":id,"method":"ping"}),
            );
            assert_eq!(reply["id"], id);
            assert_eq!(reply["jsonrpc"], "2.0");
        }
    }

    #[test]
    fn a_whole_conversation_runs_over_a_pipe() {
        // The transport as it is actually used: lines in, lines out.
        let script = [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}).to_string(),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string(),
            String::new(),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}).to_string(),
            json!({
                "jsonrpc":"2.0","id":3,"method":"tools/call",
                "params":{"name":"corescout_ask","arguments":{"question":"what do you know?"}}
            })
            .to_string(),
        ]
        .join("\n");

        let mut output = Vec::new();
        let mut server = Server::new(Recording::new());
        server
            .run(std::io::BufReader::new(script.as_bytes()), &mut output)
            .expect("the conversation runs");

        let text = String::from_utf8(output).expect("utf8");
        let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
        assert_eq!(
            lines.len(),
            3,
            "one reply per request, none for the notification"
        );
        for (index, line) in lines.iter().enumerate() {
            let reply: Value = serde_json::from_str(line).expect("each line is one JSON message");
            assert_eq!(reply["id"], json!(index + 1));
        }
    }

    #[test]
    fn a_reply_is_always_one_line() {
        // The framing is one message per line. A pretty-printed reply would
        // break every client reading this stream.
        let mut server = Server::new(Recording::new());
        let line = server
            .handle_line(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string())
            .expect("a reply");
        assert!(!line.contains('\n'), "a reply must not contain a newline");
    }

    /// A backend whose answers are lists, like the real one's are.
    struct Listing;

    impl Backend for Listing {
        fn connected(&self, _name: &str, _version: Option<&str>) {}
        fn call(&self, _method: &str, _params: &Value) -> Result<Value, String> {
            Ok(json!([{ "one": 1 }, { "two": 2 }]))
        }
        fn instructions(&self) -> String {
            String::new()
        }
    }

    #[test]
    fn a_tool_that_answers_with_a_list_still_answers_with_an_object() {
        // structuredContent is an object by the specification, and a strict
        // client discards the whole message when it is not, taking the text
        // content with it. Three tools here answer with lists, and all three
        // arrived as nothing at all.
        let mut server = Server::new(Listing);
        let reply = ask(
            &mut server,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                   "params":{"name":"corescout_failures","arguments":{}}}),
        );
        let structured = &reply["result"]["structuredContent"];
        assert!(
            structured.is_object(),
            "structuredContent must be an object"
        );
        assert!(structured["failures"].is_array(), "named for the tool");
        assert_eq!(structured["count"], 2, "so an empty list is obvious");
    }

    #[test]
    fn every_tool_answers_with_an_object_whatever_the_backend_says() {
        // The one above proves the wrapping works; this one proves no tool is
        // missed, so a tool added later cannot reintroduce the bug.
        for tool in tools::TOOLS {
            let mut server = Server::new(Listing);
            let reply = ask(
                &mut server,
                json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                       "params":{"name": tool.name, "arguments":{}}}),
            );
            assert!(
                reply["result"]["structuredContent"].is_object(),
                "{} answered with something that is not an object",
                tool.name
            );
        }
    }

    #[test]
    fn an_object_from_the_backend_is_passed_through_untouched() {
        // The wrapping must not rename the fields of the tools that were
        // already correct.
        let mut server = Server::new(Recording::new());
        let reply = ask(
            &mut server,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                   "params":{"name":"corescout_status","arguments":{}}}),
        );
        assert_eq!(reply["result"]["structuredContent"]["method"], "status");
        assert!(reply["result"]["structuredContent"]["status"].is_null());
    }
}
