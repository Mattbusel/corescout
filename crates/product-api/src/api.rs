//! The method table. One door for the desktop app, the CLI and the AI.
//!
//! # Why there is only one of these
//!
//! Three transports, one dispatch. A permission check written here cannot be
//! missing from the MCP route, and a capability the interface can run is
//! exactly the set an agent can run. The alternative — a handler per transport
//! — is how a product ends up with an admin endpoint nobody remembered to
//! guard.
//!
//! # What `ask` actually is
//!
//! The `ask` method routes a question by keyword to a query over what CoreScout
//! has recorded. It is not language understanding and does not claim to be:
//! the answer always carries the structured evidence beside the sentence, so a
//! caller that disagrees with the routing can read the data directly. When
//! nothing matches, it says so rather than guessing.

use std::sync::{Arc, Mutex, MutexGuard};

use corescout_agent_observation::{AgentKind, Raw};
use corescout_core::error::{Error, Result};
use corescout_permissions::{Autonomy, Grant};
use serde_json::{json, Value};

use crate::engine::Engine;
use crate::setup;

/// The methods this API answers, with a line each.
///
/// Also what the MCP bridge advertises, so a tool cannot exist there without
/// existing here.
pub const METHODS: &[(&str, &str)] = &[
    (
        "status",
        "Is CoreScout running, what is connected, what has it seen",
    ),
    ("home", "The one-screen summary a person reads first"),
    ("learned", "Everything CoreScout has learned, as cards"),
    ("explain", "The evidence behind one learned item"),
    ("activity", "The event timeline"),
    ("agents", "Which AI systems have connected"),
    ("setup", "How to connect one AI"),
    ("configure", "Write CoreScout into an AI's configuration"),
    ("computer", "What this machine is, and what the mirror sees"),
    (
        "states",
        "The recurring states this machine was found to have",
    ),
    ("live", "The live mirror, for the visualisation"),
    ("knowledge", "What this computer knows, grouped for reading"),
    ("privacy", "Exactly what is stored, and where"),
    ("forget", "Delete what CoreScout has recorded"),
    ("diagnostics", "What CoreScout costs to run"),
    ("autonomy", "Set how much CoreScout may do"),
    ("pause", "Stop everything now"),
    ("resume", "Start again"),
    ("settings", "The current settings and permissions"),
    ("grant", "Give CoreScout access to a folder or a command"),
    ("revoke", "Take access away again"),
    ("capabilities", "Verified procedures CoreScout can run"),
    ("decide", "Approve, disable, or automate a capability"),
    ("rename", "Rename a capability"),
    ("delete", "Delete a capability"),
    ("run", "Run a capability"),
    ("hello", "Identify a connecting AI"),
    ("observe", "Report something an AI did"),
    ("goodbye", "End an AI session"),
    ("ask", "Ask CoreScout what it knows about something"),
    ("failures", "Operations that recur and go wrong here"),
    ("hypotheses", "What CoreScout is currently testing"),
    (
        "briefing",
        "What a connecting AI should know about CoreScout",
    ),
    (
        "advise",
        "What to do before an operation, and the trial that goes with it",
    ),
];

/// The dispatcher.
#[derive(Clone)]
pub struct Api {
    engine: Arc<Mutex<Engine>>,
}

impl Api {
    /// Wrap an engine.
    pub fn new(engine: Arc<Mutex<Engine>>) -> Api {
        Api { engine }
    }

    /// The engine, for the observation loop.
    pub fn engine(&self) -> &Arc<Mutex<Engine>> {
        &self.engine
    }

    /// Take the lock, recovering from a panic in another thread.
    ///
    /// A poisoned lock here means some other request panicked. The state
    /// behind it is a set of counters and maps, not a half-written invariant,
    /// so continuing is better than refusing every request until restart.
    fn lock(&self) -> MutexGuard<'_, Engine> {
        self.engine
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Call a method.
    pub fn call(&self, method: &str, params: &Value) -> Result<Value> {
        match method {
            "status" => to_value(&self.lock().status()),
            "home" => to_value(&self.lock().home()),
            "learned" => to_value(&self.lock().learned()),
            "explain" => {
                let id = string(params, "id")?;
                match self.lock().explain(&id) {
                    Some(explanation) => to_value(&explanation),
                    None => Err(Error::invalid(format!(
                        "CoreScout knows nothing called {id}"
                    ))),
                }
            }
            "activity" => {
                let limit = number(params, "limit").unwrap_or(50.0) as usize;
                let technical = flag(params, "technical");
                to_value(&self.lock().activity(limit, technical)?)
            }
            "agents" => to_value(&self.lock().agent_summaries()),
            "setup" => match params.get("agent").and_then(Value::as_str) {
                Some(name) => to_value(&setup::setup(&AgentKind::recognise(name))),
                None => to_value(&setup::all()),
            },
            "configure" => {
                let name = string(params, "agent")?;
                let kind = AgentKind::recognise(&name);
                let path = setup::apply(&kind)?;
                Ok(json!({
                    "configured": kind.title(),
                    "path": path.display().to_string(),
                    "restart_required": true,
                    "note": format!("Restart {} for it to pick this up.", kind.title()),
                }))
            }
            "computer" => to_value(&self.lock().machine()),
            "states" => to_value(&self.lock().states()),
            "live" => to_value(&self.lock().live()),
            "knowledge" => to_value(&self.lock().knowledge()),
            "privacy" => to_value(&self.lock().privacy()?),
            "forget" => {
                let everything = flag(params, "everything");
                let removed = self.lock().forget(everything)?;
                Ok(json!({ "removed": removed, "everything": everything }))
            }
            "diagnostics" => to_value(&self.lock().diagnostics()?),
            "autonomy" => {
                let name = string(params, "mode")?;
                let mode = Autonomy::parse(&name).ok_or_else(|| {
                    Error::invalid(format!(
                        "{name:?} is not a mode; choose observe, suggest, assist or autopilot"
                    ))
                })?;
                let mut engine = self.lock();
                engine.set_autonomy(mode);
                engine.persist()?;
                Ok(json!({ "autonomy": mode.as_str(), "summary": mode.summary() }))
            }
            "pause" => {
                let mut engine = self.lock();
                engine.pause();
                engine.persist()?;
                Ok(json!({ "paused": true }))
            }
            "resume" => {
                let mut engine = self.lock();
                engine.resume();
                engine.persist()?;
                Ok(json!({ "paused": false }))
            }
            "settings" => {
                let engine = self.lock();
                let permissions = engine.permissions();
                Ok(json!({
                    "autonomy": permissions.autonomy().as_str(),
                    "autonomy_title": permissions.autonomy().title(),
                    "autonomy_summary": permissions.autonomy().summary(),
                    "paused": permissions.is_paused(),
                    "authority": permissions.authority().describe(),
                    "folders": permissions.authority().roots()
                        .iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                    "commands": permissions.authority().commands(),
                    "limits": permissions.limiter().limits(),
                    "modes": Autonomy::all().iter().map(|mode| json!({
                        "id": mode.as_str(),
                        "title": mode.title(),
                        "summary": mode.summary(),
                    })).collect::<Vec<_>>(),
                }))
            }
            "grant" => {
                let mut engine = self.lock();
                let mut granted = Vec::new();
                if let Some(folder) = params.get("folder").and_then(Value::as_str) {
                    if !engine.permissions_mut().authority_mut().grant_root(folder) {
                        return Err(Error::invalid(format!(
                            "{folder} is off limits to CoreScout and cannot be granted"
                        )));
                    }
                    granted.push(folder.to_string());
                }
                if let Some(command) = params.get("command").and_then(Value::as_str) {
                    engine
                        .permissions_mut()
                        .authority_mut()
                        .grant_command(command);
                    granted.push(command.to_string());
                }
                engine.persist()?;
                Ok(json!({ "granted": granted }))
            }
            "revoke" => {
                let folder = string(params, "folder")?;
                let mut engine = self.lock();
                engine
                    .permissions_mut()
                    .authority_mut()
                    .revoke_root(std::path::Path::new(&folder));
                engine.persist()?;
                Ok(json!({ "revoked": folder }))
            }
            "capabilities" => to_value(
                &self
                    .lock()
                    .learned()
                    .into_iter()
                    .filter(|card| card.is_capability)
                    .collect::<Vec<_>>(),
            ),
            "decide" => {
                let id = string(params, "id")?;
                let grant = Grant {
                    approved: params
                        .get("approved")
                        .and_then(Value::as_bool)
                        .unwrap_or(true),
                    auto_use: flag(params, "auto_use"),
                    disabled: flag(params, "disabled"),
                };
                let mut engine = self.lock();
                engine.decide_capability(&id, grant.clone())?;
                engine.persist()?;
                to_value(&grant)
            }
            "rename" => {
                let id = string(params, "id")?;
                let name = string(params, "name")?;
                let mut engine = self.lock();
                engine.rename_capability(&id, &name)?;
                engine.persist()?;
                Ok(json!({ "id": id, "name": name }))
            }
            "delete" => {
                let id = string(params, "id")?;
                let mut engine = self.lock();
                engine.delete_capability(&id)?;
                engine.persist()?;
                Ok(json!({ "deleted": id }))
            }
            "run" => {
                let id = string(params, "id")?;
                let dry_run = flag(params, "dry_run");
                let mut engine = self.lock();
                let outcome = engine.run_capability(&id, dry_run)?;
                engine.persist()?;
                Ok(json!({ "summary": outcome.summary(), "outcome": outcome }))
            }
            "hello" => {
                let name = string(params, "name")?;
                let version = params
                    .get("version")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let mut engine = self.lock();
                let id = engine.agent_hello(&name, version);
                let briefing = briefing(&engine);
                engine.persist()?;
                Ok(json!({ "agent": id, "briefing": briefing }))
            }
            "observe" => {
                let raw: Raw = serde_json::from_value(params.clone()).map_err(|error| {
                    Error::invalid(format!(
                        "that is not something CoreScout can record: {error}"
                    ))
                })?;
                if raw.session.trim().is_empty() {
                    return Err(Error::invalid("every report needs a session".to_string()));
                }
                let mut engine = self.lock();
                let action = engine.agent_observe(&raw);
                // `recurring_failure_here` is a fact about history; `failed` is
                // a fact about this one action. Reporting the second under a
                // name that sounds like the first is how an agent comes to
                // believe CoreScout knows more than it does.
                let recurring = engine
                    .failures()
                    .iter()
                    .any(|mode| mode.fingerprint == action.fingerprint);
                // Written now rather than on the next timer tick. An agent
                // reports an action every few seconds at most, so a
                // transaction each is cheap, and the alternative is losing the
                // last twenty seconds of a session every time the service is
                // killed -- which on a desktop is constantly.
                engine.persist()?;
                Ok(json!({
                    "recorded": action.id,
                    "fingerprint": action.fingerprint,
                    "failed": action.visibly_failed(),
                    "recurring_failure_here": recurring,
                    "note": if action.is_silent_failure() {
                        "CoreScout recorded this as a silent failure: it reported success and \
                         something contradicted it."
                    } else if action.unverified() {
                        "CoreScout recorded this as unverified. Nothing checked whether it worked."
                    } else {
                        "recorded"
                    },
                }))
            }
            "advise" => {
                let session = string(params, "session")?;
                let operation = string(params, "operation")?;
                let workspace = params.get("workspace").and_then(Value::as_str);
                let mut engine = self.lock();
                to_value(&engine.advise(&session, &operation, workspace))
            }
            "goodbye" => {
                let session = string(params, "session")?;
                let mut engine = self.lock();
                engine.agent_goodbye(&session);
                engine.persist()?;
                Ok(json!({ "closed": session }))
            }
            "ask" => self.ask(&string(params, "question")?, params),
            "failures" => {
                let engine = self.lock();
                to_value(&engine.failures())
            }
            "hypotheses" => {
                let engine = self.lock();
                to_value(&engine.open_questions())
            }
            "briefing" => {
                let engine = self.lock();
                Ok(json!({ "briefing": briefing(&engine) }))
            }
            other => Err(Error::invalid(format!(
                "CoreScout has no method called {other:?}"
            ))),
        }
    }

    /// Answer a question about what CoreScout knows.
    ///
    /// Keyword routing, and honest about it: every answer carries the
    /// structured evidence next to the sentence, and a question that matches
    /// nothing gets told so rather than being answered from the nearest thing.
    fn ask(&self, question: &str, params: &Value) -> Result<Value> {
        let text = question.to_lowercase();
        let engine = self.lock();
        let subject = params
            .get("operation")
            .and_then(Value::as_str)
            .or_else(|| params.get("workspace").and_then(Value::as_str));

        let has = |words: &[&str]| words.iter().any(|word| text.contains(word));

        if has(&["unusual", "strange", "different", "weird", "odd"]) {
            let mood = engine.home().machine;
            return Ok(json!({
                "answer": if mood.unfamiliar {
                    "Yes. The machine is in a way of running CoreScout has not seen before, so \
                     anything it tells you about this state is weaker than usual."
                } else {
                    "No. The machine is in a state CoreScout recognises."
                },
                "machine": mood,
            }));
        }
        if has(&["state", "machine", "computer doing", "right now"]) {
            let mood = engine.home().machine;
            return Ok(json!({ "answer": mood.plain.clone(), "machine": mood }));
        }
        if has(&[
            "verified",
            "reliable way",
            "better way",
            "capability",
            "procedure",
        ]) {
            let capabilities: Vec<_> = engine
                .learned()
                .into_iter()
                .filter(|card| card.is_capability)
                .filter(|card| {
                    subject.map_or(true, |s| card.title.contains(s) || card.detail.contains(s))
                })
                .collect();
            return Ok(json!({
                "answer": if capabilities.is_empty() {
                    "CoreScout has no verified procedure for that yet.".to_string()
                } else {
                    format!(
                        "CoreScout has {} verified procedure(s). Prefer them over doing it by hand.",
                        capabilities.len()
                    )
                },
                "capabilities": capabilities,
            }));
        }
        if has(&["fail", "failure", "wrong", "broken", "retry", "error"]) {
            let failures = engine.failures();
            let relevant: Vec<_> = failures
                .into_iter()
                .filter(|mode| subject.map_or(true, |s| mode.fingerprint.contains(s)))
                .collect();
            return Ok(json!({
                "answer": match relevant.first() {
                    Some(first) => format!(
                        "Yes. {}. CoreScout has seen this enough times to call it recurring.",
                        first.headline()
                    ),
                    None => "CoreScout has not seen a recurring failure for that here.".to_string(),
                },
                "failures": relevant,
            }));
        }
        if has(&[
            "learned",
            "know about",
            "knows about",
            "repository",
            "repo",
            "project",
        ]) {
            let knowledge = engine.knowledge();
            return Ok(json!({
                "answer": format!(
                    "CoreScout has been learning here for {} hours. It distinguishes {} states of \
                     this machine, has {} operational patterns, and {} verified procedures.",
                    knowledge.learning_for_ms / 3_600_000,
                    knowledge.machine_states,
                    knowledge.workflow_patterns,
                    knowledge.verified_capabilities
                ),
                "knowledge": knowledge,
            }));
        }
        if has(&["testing", "uncertain", "unsure", "hypothes", "experiment"]) {
            let open = engine.open_questions();
            return Ok(json!({
                "answer": if open.is_empty() {
                    "CoreScout is not testing anything right now.".to_string()
                } else {
                    format!("CoreScout is testing {} thing(s).", open.len())
                },
                "open": open,
            }));
        }

        Ok(json!({
            "answer": "CoreScout did not recognise that question. Everything it knows is below; \
                       the other methods return it in structured form.",
            "matched": false,
            "knowledge": engine.knowledge(),
            "methods": METHODS.iter().map(|(name, about)| json!({
                "method": name, "about": about
            })).collect::<Vec<_>>(),
        }))
    }
}

/// What a connecting AI is told about CoreScout.
///
/// Generated from what is actually enabled, so it never advertises a
/// capability that is switched off or a mode that is not in force. An AI that
/// is told it may ask for experiments while CoreScout is in Observe mode will
/// ask, and be refused, and learn to distrust the whole thing.
pub fn briefing(engine: &Engine) -> String {
    let status = engine.status();
    let knowledge = engine.knowledge();
    let permissions = engine.permissions();
    let usable = engine
        .learned()
        .into_iter()
        .filter(|card| card.is_capability && card.approved)
        .count();

    let mut lines = vec![
        "You are operating inside a computer running CoreScout.".to_string(),
        String::new(),
        "CoreScout keeps an empirical model of this machine and of how AI tools have worked with \
         it. You can query it for operational knowledge that outlives your session."
            .to_string(),
        String::new(),
    ];

    lines.push(format!(
        "Right now it distinguishes {} recurring states of this machine, holds {} operational \
         patterns, and has {} verified procedures ({} approved for use).",
        knowledge.machine_states,
        knowledge.workflow_patterns,
        knowledge.verified_capabilities,
        usable
    ));
    if status.actions > 0 {
        lines.push(format!(
            "It has observed {} actions from AI tools on this machine.",
            status.actions
        ));
    }
    lines.push(String::new());
    lines.push(
        "CoreScout separates association from causal evidence. Anything labelled \"Seen together\" \
         is a correlation it has not tested; anything labelled \"Verified\" was measured under \
         randomised assignment. Check which before you rely on it."
            .to_string(),
    );
    lines.push(String::new());
    lines.push("Useful things to ask it:".to_string());
    lines.push("  - what it has learned about this repository".to_string());
    lines.push("  - whether an operation has a recurring failure mode here".to_string());
    lines
        .push("  - whether there is a verified procedure for what you are about to do".to_string());
    lines.push("  - what state the machine is currently in, and whether it is unusual".to_string());
    lines.push(String::new());
    lines.push(
        "Report what you do back to it. Tool calls, commands, exit codes, and above all whether \
         you actually checked the result. CoreScout learns from the difference between what a \
         tool reported and what turned out to be true."
            .to_string(),
    );
    lines.push(String::new());

    match permissions.autonomy() {
        Autonomy::Observe => lines.push(
            "CoreScout is in Observe mode: it will answer questions and change nothing. Do not \
             offer to run its capabilities."
                .to_string(),
        ),
        Autonomy::Suggest => lines.push(
            "CoreScout is in Suggest mode: it can propose changes, and the user approves each \
             one. You may ask it to propose."
                .to_string(),
        ),
        Autonomy::Assist => lines.push(
            "CoreScout is in Assist mode: it may make small reversible changes itself, and asks \
             about anything larger."
                .to_string(),
        ),
        Autonomy::Autopilot => lines.push(
            "CoreScout is in Autopilot mode: it may apply changes it has verified, within the \
             limits the user set."
                .to_string(),
        ),
    }
    if permissions.is_paused() {
        lines.push(
            "CoreScout is currently paused by the user. It will answer questions and do nothing \
             else."
                .to_string(),
        );
    }
    lines.push(String::new());
    lines.push(
        "Do not treat CoreScout's claims as certain. Every answer carries its evidence and its \
         confidence; read them."
            .to_string(),
    );
    lines.join("\n")
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<Value> {
    serde_json::to_value(value)
        .map_err(|error| Error::invalid(format!("could not describe the answer: {error}")))
}

fn string(params: &Value, key: &str) -> Result<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::invalid(format!("this needs a {key}")))
}

fn number(params: &Value, key: &str) -> Option<f64> {
    params.get(key).and_then(Value::as_f64)
}

fn flag(params: &Value, key: &str) -> bool {
    params.get(key).and_then(Value::as_bool).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_storage::{Ring, Store};

    fn api() -> (tempfile::TempDir, Api) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let store = Store::open(&dir.path().join("t.redb")).expect("a store");
        let ring = Ring::open(&dir.path().join("t.ring"), 1024).expect("a ring");
        let engine = Engine::open(store, ring).expect("an engine");
        (dir, Api::new(Arc::new(Mutex::new(engine))))
    }

    #[test]
    fn every_advertised_method_answers() {
        // The method table is what the MCP bridge advertises. A name in it
        // with no handler is a tool an agent can call and always fail.
        let (_dir, api) = api();
        let params = json!({
            "id": "nothing",
            "name": "claude-code",
            "question": "what have you learned",
            "mode": "suggest",
            "session": "s1",
            "agent": "claude-code",
            "folder": "C:\\Projects\\app",
        });
        for (method, _) in METHODS {
            // `configure` and `forget` change the machine or the user's files,
            // so they are exercised on their own rather than in a sweep.
            if matches!(*method, "configure" | "forget") {
                continue;
            }
            let result = api.call(method, &params);
            if let Err(error) = &result {
                // A method that legitimately refuses these arguments must say
                // why, not fall through to "no such method".
                assert!(
                    !error.to_string().contains("no method called"),
                    "{method} is advertised and not implemented"
                );
            }
        }
    }

    #[test]
    fn an_unknown_method_says_so() {
        let (_dir, api) = api();
        let error = api.call("nonsense", &json!({})).expect_err("should fail");
        assert!(error.to_string().contains("no method called"));
    }

    #[test]
    fn a_fresh_install_says_it_has_learned_nothing_rather_than_showing_an_empty_screen() {
        let (_dir, api) = api();
        let home = api.call("home", &json!({})).expect("home");
        assert!(home["empty"].is_object(), "{home}");
        assert!(home["empty"]["body"]
            .as_str()
            .expect("a body")
            .contains("normal"));
        assert_eq!(home["today"]["learned"], 0);
    }

    #[test]
    fn the_default_mode_is_suggest_and_nothing_is_paused() {
        let (_dir, api) = api();
        let status = api.call("status", &json!({})).expect("status");
        assert_eq!(status["autonomy"], "suggest");
        assert_eq!(status["paused"], false);
    }

    #[test]
    fn pausing_and_resuming_is_visible_in_the_status() {
        let (_dir, api) = api();
        api.call("pause", &json!({})).expect("pause");
        assert_eq!(
            api.call("status", &json!({})).expect("status")["paused"],
            true
        );
        api.call("resume", &json!({})).expect("resume");
        assert_eq!(
            api.call("status", &json!({})).expect("status")["paused"],
            false
        );
    }

    #[test]
    fn an_invalid_autonomy_mode_lists_the_valid_ones() {
        let (_dir, api) = api();
        let error = api
            .call("autonomy", &json!({"mode": "full"}))
            .expect_err("should fail")
            .to_string();
        assert!(error.contains("autopilot"), "{error}");
    }

    #[test]
    fn a_forbidden_folder_cannot_be_granted_through_the_api() {
        // The permission layer refuses it. This checks the refusal survives
        // the trip through the API rather than being swallowed.
        let (_dir, api) = api();
        let error = api
            .call("grant", &json!({"folder": "C:\\Windows\\System32"}))
            .expect_err("should refuse")
            .to_string();
        assert!(error.contains("off limits"), "{error}");
    }

    #[test]
    fn an_agent_gets_a_briefing_when_it_says_hello() {
        let (_dir, api) = api();
        let hello = api
            .call("hello", &json!({"name": "claude-code", "version": "2.1"}))
            .expect("hello");
        assert_eq!(hello["agent"], "claude-code");
        let briefing = hello["briefing"].as_str().expect("a briefing");
        assert!(briefing.contains("CoreScout"));
        assert!(briefing.contains("association from causal"));
        assert!(
            briefing.contains("Suggest mode"),
            "the briefing states the mode in force"
        );
    }

    #[test]
    fn the_briefing_never_claims_a_mode_that_is_not_in_force() {
        // An agent told it may ask for experiments while CoreScout is in
        // Observe mode will ask, be refused, and learn to distrust all of it.
        let (_dir, api) = api();
        api.call("autonomy", &json!({"mode": "observe"}))
            .expect("set mode");
        let briefing = api.call("briefing", &json!({})).expect("briefing")["briefing"]
            .as_str()
            .expect("text")
            .to_string();
        assert!(briefing.contains("Observe mode"), "{briefing}");
        assert!(briefing.contains("change nothing"));
        assert!(!briefing.contains("Autopilot mode"));
    }

    #[test]
    fn the_briefing_says_when_corescout_is_paused() {
        let (_dir, api) = api();
        api.call("pause", &json!({})).expect("pause");
        let briefing = api.call("briefing", &json!({})).expect("briefing")["briefing"]
            .as_str()
            .expect("text")
            .to_string();
        assert!(briefing.contains("paused"), "{briefing}");
    }

    #[test]
    fn an_observed_action_that_nobody_checked_is_reported_back_as_unverified() {
        // The agent should be told, in the reply, that its claim was recorded
        // as a claim. That is the cheapest possible nudge towards checking.
        let (_dir, api) = api();
        api.call("hello", &json!({"name": "claude-code"}))
            .expect("hello");
        let recorded = api
            .call(
                "observe",
                &json!({"session": "s1", "name": "npm run deploy", "reported": "success"}),
            )
            .expect("observe");
        assert!(recorded["note"]
            .as_str()
            .expect("a note")
            .contains("unverified"));
    }

    #[test]
    fn a_silent_failure_is_named_as_one_in_the_reply() {
        let (_dir, api) = api();
        let recorded = api
            .call(
                "observe",
                &json!({
                    "session": "s1",
                    "name": "npm run deploy",
                    "reported": "success",
                    "verified": "contradicted",
                    "verification_detail": "the health endpoint returned 502",
                }),
            )
            .expect("observe");
        assert!(recorded["note"]
            .as_str()
            .expect("a note")
            .contains("silent failure"));
    }

    #[test]
    fn a_report_with_no_session_is_refused() {
        let (_dir, api) = api();
        let error = api
            .call("observe", &json!({"name": "x"}))
            .expect_err("should fail")
            .to_string();
        assert!(error.contains("session"), "{error}");
    }

    #[test]
    fn a_secret_in_a_reported_command_does_not_come_back_out() {
        let (_dir, api) = api();
        let recorded = api
            .call(
                "observe",
                &json!({"session": "s1", "name": "deploy --token sk-abcdefghijklmnop"}),
            )
            .expect("observe");
        let text = recorded.to_string();
        assert!(!text.contains("sk-abcdefghijklmnop"), "{text}");
    }

    #[test]
    fn a_question_corescout_cannot_route_says_so_rather_than_guessing() {
        let (_dir, api) = api();
        let answer = api
            .call("ask", &json!({"question": "qqq zzz mumble"}))
            .expect("ask");
        assert_eq!(answer["matched"], false);
        assert!(answer["answer"]
            .as_str()
            .expect("an answer")
            .contains("did not recognise"));
    }

    #[test]
    fn asking_about_failures_with_nothing_recorded_says_there_are_none() {
        let (_dir, api) = api();
        let answer = api
            .call(
                "ask",
                &json!({"question": "are there recurring failures for this?"}),
            )
            .expect("ask");
        assert!(answer["answer"]
            .as_str()
            .expect("an answer")
            .contains("not seen a recurring failure"));
    }

    #[test]
    fn asking_for_a_verified_procedure_that_does_not_exist_does_not_invent_one() {
        let (_dir, api) = api();
        let answer = api
            .call(
                "ask",
                &json!({"question": "is there a verified way to deploy?"}),
            )
            .expect("ask");
        assert!(answer["answer"]
            .as_str()
            .expect("an answer")
            .contains("no verified procedure"));
        assert_eq!(answer["capabilities"].as_array().expect("a list").len(), 0);
    }

    #[test]
    fn running_a_capability_that_does_not_exist_says_so() {
        let (_dir, api) = api();
        let error = api
            .call("run", &json!({"id": "cap-nothing"}))
            .expect_err("should fail")
            .to_string();
        assert!(error.contains("no capability"), "{error}");
    }

    #[test]
    fn the_privacy_page_lists_everything_and_reports_no_telemetry() {
        let (_dir, api) = api();
        let privacy = api.call("privacy", &json!({})).expect("privacy");
        assert_eq!(privacy["telemetry"], false);
        let holdings = privacy["holdings"].as_array().expect("holdings");
        assert_eq!(holdings.len(), corescout_storage::Kind::all().len());
        for holding in holdings {
            assert!(
                !holding["describes"].as_str().expect("words").is_empty(),
                "every kind needs a plain description: {holding}"
            );
        }
    }

    #[test]
    fn settings_offer_all_four_modes_with_words_a_person_can_read() {
        let (_dir, api) = api();
        let settings = api.call("settings", &json!({})).expect("settings");
        let modes = settings["modes"].as_array().expect("modes");
        assert_eq!(modes.len(), 4);
        for mode in modes {
            assert!(mode["summary"].as_str().expect("a summary").len() > 20);
        }
    }
}
