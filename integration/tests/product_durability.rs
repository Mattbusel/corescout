//! Storage: what survives, what does not, and what happens when it goes wrong.
//!
//! A background service on a desktop machine is killed constantly — by
//! updates, by sleep, by Task Manager, by the user closing something they did
//! not recognise. Losing what was learned each time would make the product
//! pointless, and corrupting it would make it harmful.

use corescout_product_api::{Api, Engine};
use corescout_storage::docs::Kind;
use corescout_storage::events::{Event, EventKind, Severity};
use corescout_storage::{Ring, Store, SCHEMA_VERSION};
use serde_json::json;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Open an engine over one directory, use it, and close it.
fn session<T>(dir: &Path, work: impl FnOnce(&Api) -> T) -> T {
    let store = Store::open(&dir.join("corescout.redb")).expect("a store");
    let ring = Ring::open(&dir.join("mirror.ring"), 512).expect("a ring");
    let engine = Engine::open(store, ring).expect("an engine");
    let api = Api::new(Arc::new(Mutex::new(engine)));
    work(&api)
}

#[test]
fn a_restart_keeps_what_was_learned_and_what_was_decided() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path();

    session(path, |api| {
        api.call("autonomy", &json!({ "mode": "assist" }))
            .expect("mode");
        api.call("grant", &json!({ "folder": "C:\\Projects\\app" }))
            .expect("grant");
        for round in 0..10u64 {
            api.call(
                "observe",
                &json!({
                    "session": "s1",
                    "name": "cargo build",
                    "workspace": "app",
                    "reported": if round < 6 { "failure" } else { "success" },
                    "verified": if round < 6 { "contradicted" } else { "confirmed" },
                    "started_ms": 1_700_000_000_000u64 + round * 900_000,
                }),
            )
            .expect("observe");
        }
    });

    session(path, |api| {
        let status = api.call("status", &json!({})).expect("status");
        assert_eq!(status["autonomy"], "assist");

        let settings = api.call("settings", &json!({})).expect("settings");
        assert_eq!(
            settings["folders"].as_array().expect("a list").len(),
            1,
            "a granted folder must survive: {settings}"
        );

        let failures = api.call("failures", &json!({})).expect("failures");
        let modes = failures.as_array().expect("a list");
        assert_eq!(modes.len(), 1, "{failures}");
        assert_eq!(modes[0]["attempts"], 10);
        assert_eq!(modes[0]["failures"], 6);
    });
}

#[test]
fn a_pause_survives_a_restart() {
    // The one setting where getting this wrong is unsafe rather than annoying.
    // Someone who paused CoreScout because it was doing something they did not
    // want must not find it running again after a reboot.
    let dir = tempfile::tempdir().expect("a temporary directory");
    session(dir.path(), |api| {
        api.call("pause", &json!({})).expect("pause");
    });
    session(dir.path(), |api| {
        let status = api.call("status", &json!({})).expect("status");
        assert_eq!(status["paused"], true, "{status}");
    });
}

#[test]
fn an_interrupted_write_does_not_lose_what_was_written_before_it() {
    // Simulating the kill: the engine is dropped without a clean shutdown,
    // which is what happens when Windows terminates a process during an
    // update. Anything that had been persisted has to still be there.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path();

    for round in 0..5u64 {
        let store = Store::open(&path.join("corescout.redb")).expect("a store");
        let ring = Ring::open(&path.join("mirror.ring"), 512).expect("a ring");
        let engine = Engine::open(store, ring).expect("an engine");
        let api = Api::new(Arc::new(Mutex::new(engine)));
        api.call(
            "observe",
            &json!({
                "session": format!("s{round}"),
                "name": "cargo test",
                "workspace": "app",
                "reported": "failure",
                "verified": "contradicted",
                "started_ms": 1_700_000_000_000u64 + round * 900_000,
            }),
        )
        .expect("observe");
        // No shutdown, no flush, no goodbye. Just gone.
        drop(api);
    }

    session(path, |api| {
        let failures = api.call("failures", &json!({})).expect("failures");
        let modes = failures.as_array().expect("a list");
        assert_eq!(modes.len(), 1, "{failures}");
        assert_eq!(
            modes[0]["attempts"], 5,
            "every observation before each kill must have survived it"
        );
    });
}

#[test]
fn the_ring_survives_a_restart_and_never_grows() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("mirror.ring");

    let size = {
        let mut ring = Ring::open(&path, 256).expect("a ring");
        for n in 0..100u64 {
            ring.push(&corescout_storage::ring::Sample::empty(n * 1_000_000))
                .expect("a push");
        }
        ring.bytes()
    };

    let mut ring = Ring::open(&path, 999_999).expect("reopen");
    assert_eq!(ring.written(), 100, "history survives");
    assert_eq!(ring.capacity(), 256, "an existing ring keeps its capacity");
    for n in 100..10_000u64 {
        ring.push(&corescout_storage::ring::Sample::empty(n * 1_000_000))
            .expect("a push");
    }
    assert_eq!(
        std::fs::metadata(&path).expect("metadata").len(),
        size,
        "a bounded store is bounded"
    );
    assert!(ring.wrapped());
}

#[test]
fn a_database_written_by_a_newer_build_is_refused_rather_than_misread() {
    // An older binary silently ignoring fields it does not understand, and
    // then writing the record back without them, destroys data belonging to a
    // version the user may go back to.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("corescout.redb");
    {
        let store = Store::open(&path).expect("a store");
        store
            .set_meta("schema_version", &(SCHEMA_VERSION + 1).to_string())
            .expect("stamp a future version");
    }
    let error = Store::open(&path).expect_err("should refuse").to_string();
    assert!(error.contains("newer CoreScout"), "{error}");
    assert!(error.contains("Update CoreScout"), "{error}");
}

#[test]
fn a_document_a_newer_build_wrote_is_skipped_rather_than_fatal() {
    // Forward compatibility in the other direction: a record this build cannot
    // parse should cost that record, not the whole startup.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("corescout.redb");
    {
        let store = Store::open(&path).expect("a store");
        store
            .put(
                Kind::Capability,
                "from-the-future",
                &json!({ "shape": "nothing this build knows" }),
            )
            .expect("put");
    }
    session(dir.path(), |api| {
        let capabilities = api.call("capabilities", &json!({})).expect("capabilities");
        assert!(
            capabilities.as_array().expect("a list").is_empty(),
            "the unreadable record is skipped"
        );
        assert!(
            api.call("status", &json!({})).is_ok(),
            "and startup survives"
        );
    });
}

#[test]
fn the_event_log_has_a_ceiling_and_keeps_the_newest() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = Store::open(&dir.path().join("corescout.redb")).expect("a store");
    for n in 0..500u64 {
        store
            .append(Event::new(
                EventKind::Observation,
                Severity::Debug,
                format!("entry {n}"),
            ))
            .expect("append");
    }
    assert!(store.event_count().expect("count") <= corescout_storage::store::EVENT_CEILING);
    let recent = store.recent_events(1).expect("recent");
    assert_eq!(recent[0].summary, "entry 499", "the newest is kept");
}

#[test]
fn every_audited_action_carries_what_an_audit_needs() {
    // An action recorded without a reason is an action nobody can review.
    let dir = tempfile::tempdir().expect("a temporary directory");
    session(dir.path(), |api| {
        api.call("autonomy", &json!({ "mode": "assist" }))
            .expect("mode");
        api.call("pause", &json!({})).expect("pause");
        api.call("resume", &json!({})).expect("resume");
    });

    let store = Store::open(&dir.path().join("corescout.redb")).expect("reopen");
    let events = store.recent_events(200).expect("recent");
    let audited: Vec<_> = events.iter().filter(|event| event.is_audited()).collect();
    assert!(!audited.is_empty(), "those three should all be audited");
    for event in audited {
        assert!(
            event.is_accountable(),
            "an audited entry missing its reason or its outcome: {event:?}"
        );
    }
}

#[test]
fn an_id_from_a_caller_cannot_reach_into_another_kind_of_record() {
    // The document keyspace is one string. If an id may contain its separator,
    // an agent that names a workspace carefully can read the settings.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let store = Store::open(&dir.path().join("corescout.redb")).expect("a store");
    let sneaky = format!("x{}setting{}permissions", '\u{1}', '\u{1}');
    assert!(store.put(Kind::Workspace, &sneaky, &1u32).is_err());
    assert!(store.get(Kind::Workspace, &sneaky).is_err());
    assert!(store.delete(Kind::Workspace, &sneaky).is_err());
}

#[test]
fn everything_lives_under_one_directory() {
    // The Privacy page names one path and says deleting it deletes everything.
    // If a file escapes it, that page is lying.
    let dir = corescout_storage::paths::data_dir();
    for path in [
        corescout_storage::paths::database(),
        corescout_storage::paths::ring(),
        corescout_storage::paths::endpoint(),
    ] {
        assert!(
            path.starts_with(&dir),
            "{} escaped {}",
            path.display(),
            dir.display()
        );
    }
}
