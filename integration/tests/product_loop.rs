//! The whole claim, run end to end: correlation, experiment, capability,
//! inheritance.
//!
//! # What is simulated and what is not
//!
//! The *agent* is simulated: there is no model here, and a loop stands in for
//! one. Everything the agent touches is real — the same MCP server a client
//! connects to, the same product API the window calls, the same experience
//! layer, the same science crate that refuses to compute an effect without
//! randomised trials, the same permission layer.
//!
//! The *world* is simulated too, and this is the part worth being precise
//! about. A planted repository decides whether a build fails, according to a
//! rule this file states in one place. That is what makes it a test: the
//! ground truth is known, so it is possible to check that CoreScout arrives at
//! it and, more importantly, that it does not arrive at it on evidence that
//! could not support it.
//!
//! A demonstration on a real repository is a different thing and lives in
//! `demo/`. This is the reproducible version.

use corescout_mcp_server::{Backend, Server};
use corescout_product_api::{Api, Engine};
use corescout_storage::{Ring, Store};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

/// The rule the planted repository obeys.
///
/// Regenerating the schema first genuinely fixes the build most of the time,
/// and does not always. Both halves matter: a procedure that always works is
/// too easy, and a world with no signal is a different test.
fn build_fails(regenerated_first: bool, round: u64) -> bool {
    if regenerated_first {
        round % 14 == 0
    } else {
        round % 10 != 0
    }
}

/// CoreScout, plus an MCP server in front of it, in a directory of its own.
struct World {
    _dir: tempfile::TempDir,
    api: Api,
}

impl World {
    fn open() -> World {
        World::seeded(0x5EED_C0DE_1234_5678)
    }

    /// A world whose exploration is reproducible.
    ///
    /// CoreScout picks its exploration seed from the clock on first run, so
    /// that two machines do not randomise in lockstep. That is right for the
    /// product and wrong for a test: several of the tests below assert on how
    /// often CoreScout chose to explore, and with a fresh seed every run those
    /// assertions are a coin flip that fails a few times in a hundred. Fixing
    /// the seed keeps the property under test, which is the policy, and drops
    /// the part that is not under test, which is the draw.
    ///
    /// The seed is written the same way the engine would write it, so nothing
    /// in the product needs a test-only path.
    fn seeded(seed: u64) -> World {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let store = Store::open(&dir.path().join("corescout.redb")).expect("a store");
        store
            .set_meta("seed", &seed.to_string())
            .expect("a fixed seed");
        let ring = Ring::open(&dir.path().join("mirror.ring"), 1024).expect("a ring");
        let engine = Engine::open(store, ring).expect("an engine");
        World {
            _dir: dir,
            api: Api::new(Arc::new(Mutex::new(engine))),
        }
    }

    fn call(&self, method: &str, params: Value) -> Value {
        self.api
            .call(method, &params)
            .unwrap_or_else(|error| panic!("{method} failed: {error}"))
    }
}

/// The MCP backend, so an agent reaches CoreScout the way a real one does.
struct Bridge(Api);

impl Backend for Bridge {
    fn connected(&self, name: &str, version: Option<&str>) {
        let _ = self
            .0
            .call("hello", &json!({ "name": name, "version": version }));
    }

    fn call(&self, method: &str, params: &Value) -> Result<Value, String> {
        self.0.call(method, params).map_err(|e| e.to_string())
    }

    fn instructions(&self) -> String {
        self.0
            .call("briefing", &json!({}))
            .ok()
            .and_then(|value| {
                value
                    .get("briefing")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default()
    }
}

/// One agent, working through the tools an MCP client exposes.
struct Agent {
    server: Server<Bridge>,
    session: String,
    id: u64,
    /// Builds attempted, and how many of them failed.
    pub attempts: u64,
    pub failures: u64,
}

impl Agent {
    fn connect(api: &Api, name: &str, session: &str) -> Agent {
        let mut agent = Agent {
            server: Server::new(Bridge(api.clone())),
            session: session.to_string(),
            id: 0,
            attempts: 0,
            failures: 0,
        };
        // The handshake announces the client, so nothing here has to call
        // `hello` on its behalf. That is what a real client does too.
        agent.rpc(
            "initialize",
            json!({ "clientInfo": { "name": name, "version": "1.0" } }),
        );
        agent.tool("corescout_status", json!({}));
        agent
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let request =
            json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params}).to_string();
        let reply = self
            .server
            .handle_line(&request)
            .unwrap_or_else(|| panic!("{method} produced no reply"));
        serde_json::from_str(&reply).expect("a JSON reply")
    }

    /// Call a tool, the way a model would.
    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        let reply = self.rpc(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        assert!(
            reply["error"].is_null(),
            "{name} failed at the protocol level: {reply}"
        );
        // The protocol says this is an object, and a strict client throws the
        // whole message away when it is not. Asserted on every tool call the
        // integration suite makes, because the tools that got this wrong were
        // the ones nothing here happened to exercise.
        let structured = reply["result"]["structuredContent"].clone();
        assert!(
            structured.is_object(),
            "{name} answered with something that is not an object: {structured}"
        );
        structured
    }

    /// A tool whose answer is a list, unwrapped from the object carrying it.
    fn list(&mut self, name: &str, arguments: Value) -> Vec<Value> {
        let answer = self.tool(name, arguments);
        let key = name.strip_prefix("corescout_").unwrap_or(name);
        answer[key]
            .as_array()
            .unwrap_or_else(|| panic!("{name} should answer with a list under {key:?}: {answer}"))
            .clone()
    }

    /// The briefing this agent was given when it connected.
    fn briefing(&mut self) -> String {
        self.rpc("initialize", json!({}))["result"]["instructions"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    /// Do one build, asking CoreScout first and reporting afterwards.
    ///
    /// This is exactly the loop the MCP tool descriptions ask an agent to
    /// follow, and nothing here reaches past those tools.
    fn build(&mut self, round: u64) {
        let advice = self.tool(
            "corescout_before",
            json!({
                "session": self.session,
                "operation": "cargo build",
                "workspace": "app",
            }),
        );

        // A careful agent that has been told nothing regenerates the schema
        // half the time, out of its own habits. That is what produces the
        // correlation CoreScout later has to be sceptical about.
        let suggested = advice["has_advice"] == json!(true);
        let regenerated = suggested || round % 2 == 0;

        if regenerated {
            self.tool(
                "corescout_observe",
                json!({
                    "session": self.session,
                    "name": "cargo run --bin gen-schema",
                    "workspace": "app",
                    "kind": "build",
                    "reported": "success",
                    "verified": "confirmed",
                    "started_ms": at(round, 0),
                }),
            );
        }

        let failed = build_fails(regenerated, round);
        self.attempts += 1;
        if failed {
            self.failures += 1;
        }
        self.tool(
            "corescout_observe",
            json!({
                "session": self.session,
                "name": "cargo build",
                "workspace": "app",
                "kind": "build",
                "reported": if failed { "failure" } else { "success" },
                "detail": if failed { "the generated schema was out of date" } else { "" },
                // Verified either way: this agent actually checks, which is
                // what makes its reports usable as evidence.
                "verified": if failed { "contradicted" } else { "confirmed" },
                "verification_detail": if failed { "no binary was produced" } else { "" },
                "started_ms": at(round, 1),
            }),
        );
    }
}

/// A monotonic clock for the planted history.
///
/// Fifteen minutes between rounds, which is roughly how often somebody
/// actually runs a build, and further apart than the ten-minute window
/// CoreScout looks back over for a precursor. That matters: with rounds a
/// minute apart, the previous round's schema generation is still inside the
/// window, so every build looks like it had one, and there is no comparison
/// left to make. The window is bounded by time as well as by count for exactly
/// that reason.
fn at(round: u64, step: u64) -> u64 {
    1_700_000_000_000 + round * 15 * 60_000 + step * 5_000
}

#[test]
fn a_correlation_becomes_a_capability_only_by_being_tested() {
    // The central claim of the product, start to finish, through the tools an
    // AI actually has.
    let world = World::open();
    let mut claude = Agent::connect(&world.api, "claude-code", "claude-1");

    // Phase one: work, without CoreScout knowing anything.
    for round in 0..16 {
        claude.build(round);
    }

    // It should have noticed the correlation, and said no more than that.
    let cards = world.call("learned", json!({}));
    let noticed = cards
        .as_array()
        .expect("a list")
        .iter()
        .find(|card| {
            card["title"]
                .as_str()
                .unwrap_or_default()
                .contains("gen-schema")
        })
        .expect("the correlation should be noticed");
    assert_eq!(noticed["causal"], false, "not yet: {noticed}");
    assert_eq!(noticed["basis"], "Seen together");
    assert!(
        world
            .call("capabilities", json!({}))
            .as_array()
            .expect("a list")
            .is_empty(),
        "a correlation is not a capability"
    );

    // Phase two: keep working. Now CoreScout is running the experiment, which
    // means sometimes withholding its own suggestion.
    for round in 16..400 {
        claude.build(round);
    }

    let capabilities = world.call("capabilities", json!({}));
    let list = capabilities.as_array().expect("a list");
    assert_eq!(
        list.len(),
        1,
        "one procedure is one thing, not a capability plus the finding behind it plus          the association it grew out of: {capabilities}"
    );
    let capability = &list[0];
    assert_eq!(capability["causal"], true, "{capability}");
    assert_eq!(capability["basis"], "Verified");
    assert_eq!(
        capability["approved"], false,
        "a capability arrives unapproved; creating one and enabling one are different things"
    );

    // And the evidence behind it names the randomised trials.
    let id = capability["id"].as_str().expect("an id");
    let explanation = world.call("explain", json!({ "id": id }));
    assert_eq!(explanation["kind"], "randomised");
    assert!(
        explanation["evidence"]
            .as_str()
            .expect("evidence")
            .contains("randomised trials"),
        "{explanation}"
    );
}

#[test]
fn corescout_sometimes_withholds_its_own_suggestion_and_says_so() {
    // The cost of the evidence, made visible. If this ever stops happening,
    // every causal claim downstream is unsupported.
    let world = World::open();
    let mut claude = Agent::connect(&world.api, "claude-code", "claude-1");
    for round in 0..40 {
        claude.build(round);
    }

    let mut withheld = 0;
    let mut offered = 0;
    for _ in 0..300 {
        let advice = claude.tool(
            "corescout_before",
            json!({ "session": "probe", "operation": "cargo build", "workspace": "app" }),
        );
        if advice["has_advice"] == json!(true) {
            offered += 1;
        } else if advice["because"]
            .as_str()
            .unwrap_or_default()
            .contains("checking whether")
        {
            withheld += 1;
        }
    }
    assert!(offered > 0, "it should usually offer what it believes");
    assert!(
        withheld > 0,
        "it must sometimes withhold, or nothing is ever tested"
    );
    assert!(
        withheld < offered,
        "{withheld} withheld against {offered} offered is too much of a user's real work"
    );
}

#[test]
fn what_was_learned_belongs_to_the_machine_rather_than_to_the_model() {
    // The inheritance claim, stated as narrowly as the evidence allows.
    //
    // This shows that a second AI, connecting after the first has gone, can
    // reach the same operational knowledge through the same tools. It does
    // *not* show that the second one performs better; that needs a real
    // workload and is not something a simulated agent can demonstrate.
    let world = World::open();

    let procedure = {
        let mut claude = Agent::connect(&world.api, "claude-code", "claude-1");
        for round in 0..400 {
            claude.build(round);
        }
        world.call("goodbye", json!({ "session": "claude-1" }));
        let capabilities = world.call("capabilities", json!({}));
        let list = capabilities.as_array().expect("a list");
        assert!(
            !list.is_empty(),
            "Claude's session should have produced one"
        );
        list[0]["id"].as_str().expect("an id").to_string()
    };

    // Claude is gone. Codex arrives, having never seen this machine.
    let mut codex = Agent::connect(&world.api, "codex", "codex-1");

    let briefing = codex.briefing();
    assert!(
        briefing.contains("verified procedure"),
        "the new agent should be told what is available: {briefing}"
    );

    let inherited = codex.list("corescout_capabilities", json!({}));
    assert_eq!(inherited.len(), 1, "{inherited:?}");
    assert_eq!(inherited[0]["id"], procedure);
    assert_eq!(inherited[0]["causal"], true);

    // And the reasoning, not only the conclusion.
    let explanation = codex.tool("corescout_explain", json!({ "id": procedure }));
    assert_eq!(explanation["kind"], "randomised");

    // And the failure history, which was recorded while a different model was
    // at the keyboard.
    let failures = codex.list("corescout_failures", json!({}));
    assert!(
        !failures.is_empty(),
        "the failure history should outlive the session that produced it"
    );

    let agents = world.call("agents", json!({}));
    let names: Vec<&str> = agents
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|agent| agent["name"].as_str())
        .collect();
    assert!(names.contains(&"Claude Code"), "{names:?}");
    assert!(names.contains(&"Codex"), "{names:?}");
}

#[test]
fn an_agent_that_never_verifies_anything_teaches_corescout_nothing_false() {
    // The adversarial case. An agent that reports success from an exit code
    // and never checks would, in a naive system, teach it that everything
    // works. Here those reports are unknowns and produce no failure mode, no
    // pattern, and no capability.
    let world = World::open();
    let mut agent = Agent::connect(&world.api, "over-confident", "s1");
    for round in 0..200u64 {
        agent.tool(
            "corescout_observe",
            json!({
                "session": "s1",
                "name": "cargo run --bin gen-schema",
                "workspace": "app",
                "reported": "success",
                "started_ms": at(round, 0),
            }),
        );
        agent.tool(
            "corescout_observe",
            json!({
                "session": "s1",
                "name": "cargo build",
                "workspace": "app",
                "reported": "success",
                "started_ms": at(round, 1),
            }),
        );
    }

    assert!(
        world
            .call("failures", json!({}))
            .as_array()
            .expect("a list")
            .is_empty(),
        "unverified claims are unknowns, not successes"
    );
    assert!(
        world
            .call("capabilities", json!({}))
            .as_array()
            .expect("a list")
            .is_empty(),
        "nothing may be promoted on unverified reports"
    );

    // What it *does* learn is that nobody is checking, which is the useful
    // thing to notice about this agent.
    let knowledge = world.call("knowledge", json!({}));
    let noticed = knowledge["about_workspaces"]
        .as_array()
        .expect("a list")
        .iter()
        .any(|line| {
            line.as_str()
                .unwrap_or_default()
                .contains("never been checked")
        });
    assert!(noticed, "{knowledge}");
}

#[test]
fn an_agent_reporting_success_that_reality_contradicts_is_believed_about_reality() {
    // Deploys that report success and are contradicted. The report is not
    // taken at face value, and the contradiction is what counts.
    let world = World::open();
    let mut agent = Agent::connect(&world.api, "claude-code", "s1");
    for round in 0..10u64 {
        agent.tool(
            "corescout_observe",
            json!({
                "session": "s1",
                "name": "npm run deploy",
                "workspace": "app",
                "reported": "success",
                "verified": if round < 7 { "contradicted" } else { "confirmed" },
                "verification_detail": "the service returned 502 for another 40 seconds",
                "started_ms": at(round, 0),
            }),
        );
    }

    let failures = world.call("failures", json!({}));
    let modes = failures.as_array().expect("a list");
    assert_eq!(modes.len(), 1, "{failures}");
    assert_eq!(modes[0]["attempts"], 10);
    assert_eq!(modes[0]["failures"], 7, "reality, not the report");
    assert_eq!(
        modes[0]["silent_failures"], 7,
        "a success that reality contradicted is its own kind of event"
    );
}

#[test]
fn the_agent_is_told_when_the_operation_it_is_about_to_run_has_a_history() {
    let world = World::open();
    let mut agent = Agent::connect(&world.api, "claude-code", "s1");
    for round in 0..12 {
        agent.build(round);
    }

    let advice = agent.tool(
        "corescout_before",
        json!({ "session": "s2", "operation": "cargo build", "workspace": "app" }),
    );
    let history = advice["history"].as_str().expect("a history line");
    assert!(history.contains("cargo build"), "{history}");
    assert!(
        history.contains('%') || history.contains("reported success"),
        "{history}"
    );
}

#[test]
fn an_operation_corescout_has_never_seen_produces_no_advice_and_no_invention() {
    let world = World::open();
    let mut agent = Agent::connect(&world.api, "claude-code", "s1");
    let advice = agent.tool(
        "corescout_before",
        json!({ "session": "s1", "operation": "terraform apply", "workspace": "infra" }),
    );
    assert_eq!(advice["has_advice"], false);
    assert!(advice["history"].is_null());
    assert!(advice["because"]
        .as_str()
        .expect("a reason")
        .contains("knows nothing about this operation"));
}
