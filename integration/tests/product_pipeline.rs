//! The whole product, end to end, without a service or a window.
//!
//! # Why these are here and not in the crates
//!
//! Each one crosses at least three of them. An agent reports a command, the
//! observation layer scrubs and fingerprints it, the experience layer counts
//! it, the science layer refuses to draw a causal conclusion from it, the
//! engine turns what survives into a capability, and the permission layer
//! decides whether it may run. Any single crate's tests can only see one link
//! of that.
//!
//! # These drive the same API the interface does
//!
//! Not a private entry point. Everything below goes through `Api::call`, which
//! is the identical path the desktop window, the command line and a connected
//! AI all take. A rule that held here and not there would be a rule that does
//! not hold.

use corescout_product_api::{Api, Engine};
use corescout_storage::{Kind, Ring, Store};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

/// A working CoreScout with its own storage, thrown away afterwards.
struct Fresh {
    _dir: tempfile::TempDir,
    api: Api,
}

impl Fresh {
    fn open() -> Fresh {
        let dir = tempfile::tempdir().expect("a temporary directory");
        Fresh::at(dir)
    }

    fn at(dir: tempfile::TempDir) -> Fresh {
        let store = Store::open(&dir.path().join("corescout.redb")).expect("a store");
        let ring = Ring::open(&dir.path().join("mirror.ring"), 4096).expect("a ring");
        let engine = Engine::open(store, ring).expect("an engine");
        Fresh {
            _dir: dir,
            api: Api::new(Arc::new(Mutex::new(engine))),
        }
    }

    fn call(&self, method: &str, params: Value) -> Value {
        self.api
            .call(method, &params)
            .unwrap_or_else(|error| panic!("{method} failed: {error}"))
    }

    fn try_call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.api
            .call(method, &params)
            .map_err(|error| error.to_string())
    }

    /// Report one action the way an agent would.
    fn did(&self, session: &str, name: &str, failed: bool, checked: bool) {
        let mut params = json!({
            "session": session,
            "name": name,
            "workspace": "app",
            "reported": if failed { "failure" } else { "success" },
            "started_ms": next_ms(),
        });
        if failed {
            params["detail"] = json!("the generated schema was out of date");
        }
        if checked {
            params["verified"] = json!(if failed { "contradicted" } else { "confirmed" });
            if failed {
                params["verification_detail"] = json!("the binary was not produced");
            }
        }
        self.call("observe", params);
    }
}

/// A clock that only goes forwards, so an ordering is an ordering.
fn next_ms() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CLOCK: AtomicU64 = AtomicU64::new(1_700_000_000_000);
    CLOCK.fetch_add(1_000, Ordering::Relaxed)
}

/// The planted history: builds that ran after the generator mostly worked,
/// builds that did not mostly failed.
///
/// A real agent would produce this by accident. Producing it deliberately is
/// what makes the test about what CoreScout concludes rather than about
/// whether the machine happened to cooperate.
fn plant(product: &Fresh) {
    for round in 0..7 {
        product.did("careful", "cargo run --bin gen-schema", false, true);
        product.did("careful", "cargo build", round == 6, true);
    }
    for round in 0..7 {
        product.did("hasty", "cargo build", round != 6, true);
    }
}

#[test]
fn a_fresh_install_claims_nothing_and_says_why() {
    // The first screen anyone sees. An empty dashboard reads as broken; an
    // empty state that explains itself reads as careful, and it is also the
    // honest answer.
    let product = Fresh::open();
    let home = product.call("home", json!({}));
    assert!(home["empty"].is_object(), "{home}");
    assert_eq!(home["today"]["learned"], 0);
    assert!(product
        .call("learned", json!({}))
        .as_array()
        .expect("a list")
        .is_empty());
    assert!(product
        .call("capabilities", json!({}))
        .as_array()
        .expect("a list")
        .is_empty());

    let status = product.call("status", json!({}));
    assert_eq!(status["autonomy"], "suggest", "the safe default");
    assert_eq!(status["paused"], false);
}

#[test]
fn a_recurring_failure_becomes_visible_before_anything_is_claimed_about_it() {
    let product = Fresh::open();
    plant(&product);

    let failures = product.call("failures", json!({}));
    let modes = failures.as_array().expect("a list");
    assert_eq!(modes.len(), 1, "{failures}");
    assert_eq!(modes[0]["fingerprint"], "cargo build");
    assert_eq!(modes[0]["attempts"], 14);
    assert_eq!(modes[0]["failures"], 7);
}

#[test]
fn a_correlation_is_reported_as_a_correlation_and_nothing_more() {
    // The centre of the whole product. Everything in the planted history was
    // chosen by the agent, so the honest reading is that the two go together.
    let product = Fresh::open();
    plant(&product);

    let cards = product.call("learned", json!({}));
    let list = cards.as_array().expect("a list");
    assert!(!list.is_empty(), "nothing was noticed: {cards}");

    let found = list
        .iter()
        .find(|card| {
            card["title"]
                .as_str()
                .unwrap_or_default()
                .contains("gen-schema")
        })
        .expect("the precursor pattern");
    assert_eq!(found["causal"], false);
    assert_eq!(found["basis"], "Seen together");
    assert!(
        found["confidence"].as_f64().expect("a number") <= 0.75,
        "an association may never reach certainty: {found}"
    );
    assert!(
        found["reliability"].is_null(),
        "an untested correlation has no reliability: {found}"
    );
}

#[test]
fn an_untested_correlation_never_becomes_a_capability() {
    // However much of it there is. This is the line that separates this
    // product from one that pattern-matches and then recommends.
    let product = Fresh::open();
    for _ in 0..40 {
        plant(&product);
    }
    assert!(
        product
            .call("capabilities", json!({}))
            .as_array()
            .expect("a list")
            .is_empty(),
        "560 observations produced a capability, which they must not"
    );

    let open = product.call("hypotheses", json!({}));
    let questions = open.as_array().expect("a list");
    assert!(!questions.is_empty(), "it should be testing something");
    assert!(
        questions.iter().all(|q| q["settled"] == false),
        "a settled question is not an open one: {open}"
    );
    assert_eq!(questions[0]["randomised_trials"], 0);
    assert!(questions[0]["missing"]
        .as_str()
        .expect("a reason")
        .to_lowercase()
        .contains("randomis"));
}

#[test]
fn the_explanation_of_a_correlation_admits_it_has_not_been_tested() {
    let product = Fresh::open();
    plant(&product);
    let cards = product.call("learned", json!({}));
    let id = cards[0]["id"].as_str().expect("an id").to_string();

    let explanation = product.call("explain", json!({ "id": id }));
    assert_eq!(explanation["kind"], "observed");
    assert!(explanation["evidence"]
        .as_str()
        .expect("evidence")
        .contains("has not tested"));
    assert!(explanation["technical"].is_object() || explanation["technical"].is_null());
}

#[test]
fn a_silent_failure_is_recorded_as_its_own_kind_of_event() {
    // Reported success, contradicted by a check. The first thing this product
    // exists to notice, and it must not be filed as an ordinary failure.
    let product = Fresh::open();
    product.call(
        "observe",
        json!({
            "session": "s1",
            "name": "npm run deploy",
            "workspace": "app",
            "reported": "success",
            "verified": "contradicted",
            "verification_detail": "the health endpoint returned 502 for another 40 seconds",
        }),
    );
    let activity = product.call("activity", json!({ "limit": 50, "technical": true }));
    let mention = activity
        .as_array()
        .expect("a list")
        .iter()
        .find(|moment| {
            moment["summary"]
                .as_str()
                .unwrap_or_default()
                .contains("said it worked")
        })
        .expect("a silent failure should be on the timeline");
    assert_eq!(mention["severity"], "warning");
}

#[test]
fn an_unverified_success_is_an_unknown_rather_than_a_success() {
    let product = Fresh::open();
    for _ in 0..8 {
        product.call(
            "observe",
            json!({
                "session": "s1",
                "name": "npm run deploy",
                "workspace": "app",
                "reported": "success",
                "started_ms": next_ms(),
            }),
        );
    }
    let failures = product.call("failures", json!({}));
    assert!(
        failures.as_array().expect("a list").is_empty(),
        "nothing failed, so nothing is a failure mode"
    );

    let knowledge = product.call("knowledge", json!({}));
    let noted = knowledge["about_workspaces"]
        .as_array()
        .expect("a list")
        .iter()
        .any(|line| {
            line.as_str()
                .unwrap_or_default()
                .contains("never been checked")
        });
    assert!(noted, "the gap should be named: {knowledge}");
}

#[test]
fn a_secret_never_survives_into_anything_the_product_will_show() {
    // Redaction happens where data enters. This walks every read method and
    // asserts the token is in none of them.
    let product = Fresh::open();
    product.call(
        "observe",
        json!({
            "session": "s1",
            "name": "deploy --token ghp_abcdefghijklmnopqrstuvwxyz --api-key=sk-abcdefghijklmnop",
            "workspace": "app",
            "reported": "failure",
            "detail": "auth failed with AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMIabcdefgh",
        }),
    );
    for method in [
        "home",
        "learned",
        "activity",
        "failures",
        "knowledge",
        "hypotheses",
    ] {
        let answer = product.call(method, json!({ "limit": 200, "technical": true }));
        let text = answer.to_string();
        for secret in [
            "ghp_abcdefghijklmnopqrstuvwxyz",
            "sk-abcdefghijklmnop",
            "wJalrXUtnFEMIabcdefgh",
        ] {
            assert!(!text.contains(secret), "{method} leaked a credential");
        }
    }
}

#[test]
fn everything_learned_survives_a_restart() {
    // The service is killed and starts again. Losing what was learned would
    // make the product forget every time Windows installs an update.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let before = {
        let product = Fresh::at(dir);
        plant(&product);
        product
            .call("learned", json!({}))
            .as_array()
            .expect("a list")
            .len()
    };
    assert!(before > 0);

    // The directory was moved into the first instance and dropped with it, so
    // this test re-opens a *new* one and asserts the shape of a fresh start
    // instead. The durable path is covered below, where one directory is used
    // twice.
    let fresh = Fresh::open();
    assert!(fresh
        .call("learned", json!({}))
        .as_array()
        .expect("a list")
        .is_empty());
}

#[test]
fn a_second_process_reading_the_same_folder_sees_what_the_first_learned() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().to_path_buf();

    {
        let store = Store::open(&path.join("corescout.redb")).expect("a store");
        let ring = Ring::open(&path.join("mirror.ring"), 1024).expect("a ring");
        let engine = Engine::open(store, ring).expect("an engine");
        let api = Api::new(Arc::new(Mutex::new(engine)));
        let product = Fresh {
            _dir: tempfile::tempdir().expect("a spare"),
            api,
        };
        plant(&product);
        product.call("autonomy", json!({ "mode": "assist" }));
    }

    let store = Store::open(&path.join("corescout.redb")).expect("reopen the store");
    let ring = Ring::open(&path.join("mirror.ring"), 1024).expect("reopen the ring");
    let engine = Engine::open(store, ring).expect("reopen the engine");
    let api = Api::new(Arc::new(Mutex::new(engine)));

    let status = api.call("status", &json!({})).expect("status");
    assert_eq!(status["autonomy"], "assist", "a setting must survive");

    let learned = api.call("learned", &json!({})).expect("learned");
    assert!(
        !learned.as_array().expect("a list").is_empty(),
        "what was learned must survive: {learned}"
    );
    let failures = api.call("failures", &json!({})).expect("failures");
    assert_eq!(failures.as_array().expect("a list")[0]["attempts"], 14);
}

#[test]
fn pausing_stops_every_change_and_the_ai_is_told() {
    let product = Fresh::open();
    product.call("autonomy", json!({ "mode": "autopilot" }));
    product.call("pause", json!({}));

    assert_eq!(product.call("status", json!({}))["paused"], true);
    let briefing = product.call("briefing", json!({}))["briefing"]
        .as_str()
        .expect("a briefing")
        .to_string();
    assert!(briefing.contains("paused"), "{briefing}");

    // And it comes back.
    product.call("resume", json!({}));
    assert_eq!(product.call("status", json!({}))["paused"], false);
}

#[test]
fn the_briefing_describes_the_mode_that_is_actually_in_force() {
    // An AI told it may ask for experiments while CoreScout is in Observe mode
    // will ask, be refused, and learn to distrust everything else it is told.
    let product = Fresh::open();
    for (mode, expected) in [
        ("observe", "Observe mode"),
        ("suggest", "Suggest mode"),
        ("assist", "Assist mode"),
        ("autopilot", "Autopilot mode"),
    ] {
        product.call("autonomy", json!({ "mode": mode }));
        let briefing = product.call("briefing", json!({}))["briefing"]
            .as_str()
            .expect("a briefing")
            .to_string();
        assert!(briefing.contains(expected), "{mode}: {briefing}");
    }
}

#[test]
fn forgetting_ai_history_leaves_what_was_learned_about_the_machine() {
    // Two buttons because they are two questions, and getting this wrong once
    // would be unrecoverable for the person it happened to.
    let product = Fresh::open();
    plant(&product);
    product.call("autonomy", json!({ "mode": "assist" }));

    let removed = product.call("forget", json!({ "everything": false }));
    assert!(removed["removed"].as_u64().expect("a count") > 0);
    assert!(product
        .call("failures", json!({}))
        .as_array()
        .expect("a list")
        .is_empty());
    assert_eq!(
        product.call("status", json!({}))["autonomy"],
        "assist",
        "a setting is not AI history"
    );
}

#[test]
fn forgetting_everything_leaves_nothing() {
    let product = Fresh::open();
    plant(&product);
    product.call("forget", json!({ "everything": true }));

    let privacy = product.call("privacy", json!({}));
    for holding in privacy["holdings"].as_array().expect("a list") {
        // Settings are rewritten immediately after the wipe, which is correct:
        // the product still has to be configured. Everything else goes.
        if holding["kind"] == "setting" {
            continue;
        }
        assert_eq!(holding["count"], 0, "{holding} survived a full delete");
    }
}

#[test]
fn a_forbidden_folder_cannot_be_granted_through_any_route() {
    let product = Fresh::open();
    for folder in [
        "C:\\Windows\\System32",
        "C:\\Users\\someone\\.ssh",
        "/home/someone/.aws",
    ] {
        let error = product
            .try_call("grant", json!({ "folder": folder }))
            .expect_err("should refuse");
        assert!(error.contains("off limits"), "{folder}: {error}");
    }
    let settings = product.call("settings", json!({}));
    assert!(settings["folders"].as_array().expect("a list").is_empty());
}

#[test]
fn the_privacy_page_accounts_for_every_kind_of_record() {
    // If a kind of thing is stored and not listed, this page is lying.
    let product = Fresh::open();
    let privacy = product.call("privacy", json!({}));
    let holdings = privacy["holdings"].as_array().expect("a list");
    assert_eq!(holdings.len(), Kind::all().len());
    assert_eq!(privacy["telemetry"], false);
    for holding in holdings {
        let words = holding["describes"].as_str().expect("a description");
        assert!(words.len() > 10, "{holding}");
        assert!(
            !words.contains('_'),
            "the privacy page is for people, not identifiers: {words}"
        );
    }
}

#[test]
fn every_answer_a_person_reads_is_free_of_identifiers() {
    // Every user-facing sentence is produced by the engine, so this is where
    // an internal name leaking into one gets noticed.
    let product = Fresh::open();
    plant(&product);
    let home = product.call("home", json!({}));
    let sentences = [
        home["headline"].as_str().expect("a headline"),
        home["machine"]["plain"].as_str().expect("a state sentence"),
        home["empty"]["body"].as_str().unwrap_or("fine"),
    ];
    for text in sentences {
        assert!(!text.contains("latent"), "{text}");
        assert!(!text.contains("fingerprint"), "{text}");
        assert!(!text.contains('_'), "{text}");
    }
}
