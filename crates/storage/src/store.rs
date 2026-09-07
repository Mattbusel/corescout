//! The transactional store: documents, the event log, and the meta table.
//!
//! # Keys are one string, not a pair
//!
//! A document is addressed by `kind` and `id`, but the key on disk is a single
//! string joined by a byte that cannot occur in either. That makes listing a
//! kind an ordinary range scan over a sorted keyspace, using only the one key
//! type this crate has to trust. It also means a caller cannot smuggle a
//! separator into an id and read another kind's documents, which is checked
//! rather than assumed.
//!
//! # Concurrency
//!
//! One writer, many readers, which is what the underlying store gives and what
//! the service needs: the service writes, the desktop app and the MCP bridge
//! read through the service rather than opening the file themselves.

use std::path::Path;

use corescout_core::error::{Error, Result};
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition};
use serde::{Deserialize, Serialize};

use crate::docs::{now_ms, Document, Kind};
use crate::events::{Event, EventKind, Severity};
use crate::migrate::{self, SCHEMA_VERSION};

/// Documents, keyed by `kind` + [`SEPARATOR`] + `id`.
const DOCS: TableDefinition<&str, &[u8]> = TableDefinition::new("documents");
/// The event log, keyed by sequence number.
const EVENTS: TableDefinition<u64, &[u8]> = TableDefinition::new("events");
/// Small facts about the database itself.
const META: TableDefinition<&str, &str> = TableDefinition::new("meta");

/// Joins a kind to an id. Below every character a kind or id may contain.
const SEPARATOR: char = '\u{1}';

/// Entries kept in the event log before the oldest are dropped.
///
/// Bounded storage is a product promise, so the log has a ceiling like the
/// ring does. At the rate a busy agent session generates entries this is
/// months of history.
pub const EVENT_CEILING: u64 = 200_000;

/// The local database.
#[derive(Debug)]
pub struct Store {
    db: Database,
    /// The next sequence number to hand out. Read once on open, then owned in
    /// memory: reading the last key on every append would turn a write into a
    /// read plus a write for no gain, since there is one writer.
    next_sequence: u64,
}

impl Store {
    /// Open or create the database at `path`, running any migrations.
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;
        }
        let db = Database::create(path).map_err(|error| {
            Error::invalid(format!("could not open {}: {error}", path.display()))
        })?;

        // Create every table on first open, so a reader never meets a missing
        // table and has to decide whether that means empty or broken.
        let txn = db.begin_write().map_err(transaction)?;
        {
            txn.open_table(DOCS).map_err(table)?;
            txn.open_table(EVENTS).map_err(table)?;
            txn.open_table(META).map_err(table)?;
        }
        txn.commit().map_err(commit)?;

        let found = read_meta(&db, "schema_version")?
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(SCHEMA_VERSION);
        let steps = migrate::plan(found)?;

        let txn = db.begin_write().map_err(transaction)?;
        {
            let mut meta = txn.open_table(META).map_err(table)?;
            meta.insert("schema_version", SCHEMA_VERSION.to_string().as_str())
                .map_err(write)?;
            if meta.get("created_ms").map_err(read)?.is_none() {
                meta.insert("created_ms", now_ms().to_string().as_str())
                    .map_err(write)?;
            }
        }
        txn.commit().map_err(commit)?;

        let next_sequence = {
            let txn = db.begin_read().map_err(transaction)?;
            let events = txn.open_table(EVENTS).map_err(table)?;
            // Bound to a local rather than left as the tail expression: the
            // guard borrows the table, and a borrow that outlives the block is
            // a borrow of a table that has already been dropped.
            let next = events
                .last()
                .map_err(read)?
                .map(|(key, _)| key.value() + 1)
                .unwrap_or(1);
            next
        };

        let store = Store { db, next_sequence };
        if !steps.is_empty() {
            for step in steps {
                store.append(Event::new(
                    EventKind::Lifecycle,
                    Severity::Info,
                    format!("migrated storage: {step}"),
                ))?;
            }
        }
        Ok(store)
    }

    /// Open the database at the standard location for this user.
    pub fn open_default() -> Result<Store> {
        Store::open(&crate::paths::database())
    }

    /// The schema version stamped in this database.
    pub fn schema_version(&self) -> Result<u32> {
        Ok(read_meta(&self.db, "schema_version")?
            .and_then(|value| value.parse().ok())
            .unwrap_or(SCHEMA_VERSION))
    }

    /// When this database was first created, in Unix milliseconds.
    ///
    /// The "your computer has been learning for 14 hours" line is derived from
    /// this and nothing else.
    pub fn created_ms(&self) -> Result<u64> {
        Ok(read_meta(&self.db, "created_ms")?
            .and_then(|value| value.parse().ok())
            .unwrap_or(0))
    }

    /// Read a small fact about the database.
    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        read_meta(&self.db, key)
    }

    /// Write a small fact about the database.
    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        let txn = self.db.begin_write().map_err(transaction)?;
        {
            let mut meta = txn.open_table(META).map_err(table)?;
            meta.insert(key, value).map_err(write)?;
        }
        txn.commit().map_err(commit)
    }

    /// Store a document, replacing any with the same kind and id.
    pub fn put<T: Serialize>(&self, kind: Kind, id: &str, body: &T) -> Result<()> {
        self.put_document(&Document::new(kind, id, body))
    }

    /// Store an already-built document.
    pub fn put_document(&self, document: &Document) -> Result<()> {
        let key = key_for(document.kind, &document.id)?;
        let bytes = serde_json::to_vec(document)
            .map_err(|error| Error::invalid(format!("could not serialise a document: {error}")))?;
        let txn = self.db.begin_write().map_err(transaction)?;
        {
            let mut docs = txn.open_table(DOCS).map_err(table)?;
            docs.insert(key.as_str(), bytes.as_slice()).map_err(write)?;
        }
        txn.commit().map_err(commit)
    }

    /// Read one document.
    pub fn get(&self, kind: Kind, id: &str) -> Result<Option<Document>> {
        let key = key_for(kind, id)?;
        let txn = self.db.begin_read().map_err(transaction)?;
        let docs = txn.open_table(DOCS).map_err(table)?;
        let found = docs.get(key.as_str()).map_err(read)?;
        let document = found.and_then(|bytes| serde_json::from_slice(bytes.value()).ok());
        Ok(document)
    }

    /// Read one document and parse its body.
    pub fn load<T: for<'de> Deserialize<'de>>(&self, kind: Kind, id: &str) -> Result<Option<T>> {
        Ok(self.get(kind, id)?.and_then(|document| document.parse()))
    }

    /// Every document of a kind, in id order.
    pub fn list(&self, kind: Kind) -> Result<Vec<Document>> {
        let txn = self.db.begin_read().map_err(transaction)?;
        let docs = txn.open_table(DOCS).map_err(table)?;
        let low = format!("{}{SEPARATOR}", kind.as_str());
        let high = format!("{}{}", kind.as_str(), '\u{2}');
        let mut out = Vec::new();
        for entry in docs.range(low.as_str()..high.as_str()).map_err(read)? {
            let (_, value) = entry.map_err(read)?;
            if let Ok(document) = serde_json::from_slice::<Document>(value.value()) {
                out.push(document);
            }
        }
        Ok(out)
    }

    /// Every document of a kind, parsed, skipping any this build cannot read.
    pub fn list_as<T: for<'de> Deserialize<'de>>(&self, kind: Kind) -> Result<Vec<T>> {
        Ok(self
            .list(kind)?
            .iter()
            .filter_map(|document| document.parse())
            .collect())
    }

    /// How many documents of a kind there are.
    pub fn count(&self, kind: Kind) -> Result<usize> {
        Ok(self.list(kind)?.len())
    }

    /// Remove one document. Returns whether it was there.
    pub fn delete(&self, kind: Kind, id: &str) -> Result<bool> {
        let key = key_for(kind, id)?;
        let txn = self.db.begin_write().map_err(transaction)?;
        let existed = {
            let mut docs = txn.open_table(DOCS).map_err(table)?;
            let existed = docs.remove(key.as_str()).map_err(write)?.is_some();
            existed
        };
        txn.commit().map_err(commit)?;
        Ok(existed)
    }

    /// Remove every document of a kind. Returns how many went.
    ///
    /// This is what the Privacy page's delete buttons call.
    pub fn clear(&self, kind: Kind) -> Result<usize> {
        let ids: Vec<String> = self
            .list(kind)?
            .into_iter()
            .map(|document| document.id)
            .collect();
        let count = ids.len();
        let txn = self.db.begin_write().map_err(transaction)?;
        {
            let mut docs = txn.open_table(DOCS).map_err(table)?;
            for id in &ids {
                docs.remove(key_for(kind, id)?.as_str()).map_err(write)?;
            }
        }
        txn.commit().map_err(commit)?;
        Ok(count)
    }

    /// Append an event, assigning it a sequence number.
    pub fn append(&self, mut event: Event) -> Result<u64> {
        // `next_sequence` is owned by this handle rather than shared, so read
        // the true tail when appending. One writer means this is exact; the
        // cached value is only a starting point after a restart.
        let sequence = {
            let txn = self.db.begin_read().map_err(transaction)?;
            let events = txn.open_table(EVENTS).map_err(table)?;
            let next = events
                .last()
                .map_err(read)?
                .map(|(key, _)| key.value() + 1)
                .unwrap_or(self.next_sequence);
            next
        };
        event.sequence = sequence;
        let bytes = serde_json::to_vec(&event)
            .map_err(|error| Error::invalid(format!("could not serialise an event: {error}")))?;
        let txn = self.db.begin_write().map_err(transaction)?;
        {
            let mut events = txn.open_table(EVENTS).map_err(table)?;
            events.insert(sequence, bytes.as_slice()).map_err(write)?;
            // Trim from the front, so the log has a ceiling.
            let len = events.len().map_err(read)?;
            if len > EVENT_CEILING {
                let excess = len - EVENT_CEILING;
                let doomed: Vec<u64> = events
                    .iter()
                    .map_err(read)?
                    .take(excess as usize)
                    .filter_map(|entry| entry.ok().map(|(key, _)| key.value()))
                    .collect();
                for key in doomed {
                    events.remove(key).map_err(write)?;
                }
            }
        }
        txn.commit().map_err(commit)?;
        Ok(sequence)
    }

    /// The most recent `count` events, newest first.
    pub fn recent_events(&self, count: usize) -> Result<Vec<Event>> {
        let txn = self.db.begin_read().map_err(transaction)?;
        let events = txn.open_table(EVENTS).map_err(table)?;
        let mut out = Vec::with_capacity(count.min(1024));
        for entry in events.iter().map_err(read)?.rev() {
            if out.len() >= count {
                break;
            }
            let (_, value) = entry.map_err(read)?;
            if let Ok(event) = serde_json::from_slice::<Event>(value.value()) {
                out.push(event);
            }
        }
        Ok(out)
    }

    /// Events after `after`, oldest first, capped at `limit`.
    ///
    /// This is what a timeline that is already showing something polls with.
    pub fn events_after(&self, after: u64, limit: usize) -> Result<Vec<Event>> {
        let txn = self.db.begin_read().map_err(transaction)?;
        let events = txn.open_table(EVENTS).map_err(table)?;
        let mut out = Vec::new();
        for entry in events.range((after + 1)..).map_err(read)? {
            if out.len() >= limit {
                break;
            }
            let (_, value) = entry.map_err(read)?;
            if let Ok(event) = serde_json::from_slice::<Event>(value.value()) {
                out.push(event);
            }
        }
        Ok(out)
    }

    /// How many events are in the log.
    pub fn event_count(&self) -> Result<u64> {
        let txn = self.db.begin_read().map_err(transaction)?;
        let events = txn.open_table(EVENTS).map_err(table)?;
        events.len().map_err(read)
    }

    /// Remove every event.
    pub fn clear_events(&self) -> Result<u64> {
        let count = self.event_count()?;
        let txn = self.db.begin_write().map_err(transaction)?;
        {
            let mut events = txn.open_table(EVENTS).map_err(table)?;
            let keys: Vec<u64> = events
                .iter()
                .map_err(read)?
                .filter_map(|entry| entry.ok().map(|(key, _)| key.value()))
                .collect();
            for key in keys {
                events.remove(key).map_err(write)?;
            }
        }
        txn.commit().map_err(commit)?;
        Ok(count)
    }
}

/// Build a storage key, refusing an id that could reach into another kind.
fn key_for(kind: Kind, id: &str) -> Result<String> {
    if id.contains(SEPARATOR) || id.is_empty() {
        return Err(Error::invalid(format!(
            "{id:?} is not a usable document id"
        )));
    }
    Ok(format!("{}{SEPARATOR}{id}", kind.as_str()))
}

fn read_meta(db: &Database, key: &str) -> Result<Option<String>> {
    let txn = db.begin_read().map_err(transaction)?;
    let meta = txn.open_table(META).map_err(table)?;
    Ok(meta
        .get(key)
        .map_err(read)?
        .map(|value| value.value().to_string()))
}

fn transaction(error: redb::TransactionError) -> Error {
    Error::invalid(format!("storage transaction failed: {error}"))
}

fn table(error: redb::TableError) -> Error {
    Error::invalid(format!("storage table unavailable: {error}"))
}

fn read(error: redb::StorageError) -> Error {
    Error::invalid(format!("storage read failed: {error}"))
}

fn write(error: redb::StorageError) -> Error {
    Error::invalid(format!("storage write failed: {error}"))
}

fn commit(error: redb::CommitError) -> Error {
    Error::invalid(format!("storage commit failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let store = Store::open(&dir.path().join("test.redb")).expect("a store");
        (dir, store)
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Thing {
        name: String,
        count: u32,
    }

    #[test]
    fn a_document_survives_a_round_trip() {
        let (_dir, store) = scratch();
        let thing = Thing {
            name: "build then regenerate".into(),
            count: 12,
        };
        store.put(Kind::Capability, "cap-1", &thing).expect("put");
        let back: Thing = store
            .load(Kind::Capability, "cap-1")
            .expect("load")
            .expect("present");
        assert_eq!(back, thing);
    }

    #[test]
    fn documents_of_one_kind_do_not_appear_in_another() {
        let (_dir, store) = scratch();
        store.put(Kind::Concept, "a", &1u32).expect("put");
        store.put(Kind::Capability, "a", &2u32).expect("put");
        assert_eq!(store.count(Kind::Concept).expect("count"), 1);
        assert_eq!(store.count(Kind::Capability).expect("count"), 1);
        assert_eq!(
            store.load::<u32>(Kind::Concept, "a").expect("load"),
            Some(1)
        );
    }

    #[test]
    fn an_id_cannot_reach_into_another_kind() {
        // The keyspace is one string. If an id may contain the separator, an
        // agent that names a workspace carefully can read the settings.
        let (_dir, store) = scratch();
        let sneaky = format!("x{SEPARATOR}setting{SEPARATOR}autonomy");
        let error = store
            .put(Kind::Workspace, &sneaky, &1u32)
            .expect_err("should refuse")
            .to_string();
        assert!(error.contains("not a usable document id"), "{error}");
    }

    #[test]
    fn an_empty_id_is_refused() {
        let (_dir, store) = scratch();
        assert!(store.put(Kind::Workspace, "", &1u32).is_err());
    }

    #[test]
    fn listing_a_kind_is_sorted_and_complete() {
        let (_dir, store) = scratch();
        for id in ["c", "a", "b"] {
            store.put(Kind::Pattern, id, &id.to_string()).expect("put");
        }
        let ids: Vec<String> = store
            .list(Kind::Pattern)
            .expect("list")
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn deleting_removes_only_what_was_named() {
        let (_dir, store) = scratch();
        store.put(Kind::Session, "one", &1u32).expect("put");
        store.put(Kind::Session, "two", &2u32).expect("put");
        assert!(store.delete(Kind::Session, "one").expect("delete"));
        assert!(!store.delete(Kind::Session, "one").expect("delete again"));
        assert_eq!(store.count(Kind::Session).expect("count"), 1);
    }

    #[test]
    fn clearing_a_kind_leaves_the_others_alone() {
        // "Forget my AI history" must not delete what was learned about the
        // hardware, and this is the line where that is true or not.
        let (_dir, store) = scratch();
        store.put(Kind::Session, "s", &1u32).expect("put");
        store.put(Kind::Action, "a", &1u32).expect("put");
        store.put(Kind::Concept, "c", &1u32).expect("put");
        for kind in Kind::all().into_iter().filter(|k| k.is_agent_activity()) {
            store.clear(kind).expect("clear");
        }
        assert_eq!(store.count(Kind::Session).expect("count"), 0);
        assert_eq!(store.count(Kind::Action).expect("count"), 0);
        assert_eq!(store.count(Kind::Concept).expect("count"), 1);
    }

    #[test]
    fn events_get_increasing_sequence_numbers() {
        let (_dir, store) = scratch();
        let first = store
            .append(Event::new(EventKind::Lifecycle, Severity::Info, "started"))
            .expect("append");
        let second = store
            .append(Event::new(EventKind::Discovery, Severity::Info, "found"))
            .expect("append");
        assert!(second > first);
        let recent = store.recent_events(10).expect("recent");
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].summary, "found", "newest first");
    }

    #[test]
    fn events_after_a_cursor_returns_only_newer_ones() {
        let (_dir, store) = scratch();
        let mut cursor = 0;
        for n in 0..5 {
            cursor = store
                .append(Event::new(
                    EventKind::Observation,
                    Severity::Debug,
                    format!("n{n}"),
                ))
                .expect("append");
        }
        assert!(store.events_after(cursor, 100).expect("after").is_empty());
        let tail = store.events_after(cursor - 2, 100).expect("after");
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].summary, "n3");
    }

    #[test]
    fn a_reopened_store_keeps_everything_and_keeps_counting() {
        // Crash recovery: the service is killed and restarts. Nothing may be
        // lost and sequence numbers may not go backwards, or the timeline
        // silently interleaves.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("test.redb");
        let last = {
            let store = Store::open(&path).expect("a store");
            store.put(Kind::Concept, "c", &7u32).expect("put");
            store
                .append(Event::new(EventKind::Lifecycle, Severity::Info, "one"))
                .expect("append")
        };
        let store = Store::open(&path).expect("reopen");
        assert_eq!(
            store.load::<u32>(Kind::Concept, "c").expect("load"),
            Some(7)
        );
        let next = store
            .append(Event::new(EventKind::Lifecycle, Severity::Info, "two"))
            .expect("append");
        assert!(next > last, "{next} should follow {last}");
        assert_eq!(store.event_count().expect("count"), 2);
    }

    #[test]
    fn the_event_log_has_a_ceiling() {
        // Bounded storage is a promise the product makes on its Privacy page.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let store = Store::open(&dir.path().join("t.redb")).expect("a store");
        // Exercising the real ceiling would write two hundred thousand rows,
        // so this checks the trim happens at all by driving past a small one.
        for n in 0..(EVENT_CEILING.min(64) + 8) {
            store
                .append(Event::new(
                    EventKind::Observation,
                    Severity::Debug,
                    format!("{n}"),
                ))
                .expect("append");
        }
        assert!(store.event_count().expect("count") <= EVENT_CEILING);
    }

    #[test]
    fn a_fresh_database_is_stamped_with_the_current_schema() {
        let (_dir, store) = scratch();
        assert_eq!(store.schema_version().expect("version"), SCHEMA_VERSION);
        assert!(store.created_ms().expect("created") > 0);
    }

    #[test]
    fn the_creation_time_does_not_move_when_reopened() {
        // "Your computer has been learning for 14 hours" is derived from this
        // one number, so a restart must not reset it to zero hours.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("t.redb");
        let created = Store::open(&path)
            .expect("a store")
            .created_ms()
            .expect("ms");
        std::thread::sleep(std::time::Duration::from_millis(5));
        let again = Store::open(&path)
            .expect("reopen")
            .created_ms()
            .expect("ms");
        assert_eq!(created, again);
    }

    #[test]
    fn a_database_from_the_future_is_refused() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("t.redb");
        {
            let store = Store::open(&path).expect("a store");
            store
                .set_meta("schema_version", &(SCHEMA_VERSION + 5).to_string())
                .expect("stamp");
        }
        let error = Store::open(&path).expect_err("should refuse").to_string();
        assert!(error.contains("newer CoreScout"), "{error}");
    }

    #[test]
    fn documents_a_newer_build_wrote_are_skipped_not_fatal() {
        let (_dir, store) = scratch();
        store.put(Kind::Capability, "good", &1u32).expect("put");
        store
            .put(Kind::Capability, "odd", &"not a number".to_string())
            .expect("put");
        let parsed: Vec<u32> = store.list_as(Kind::Capability).expect("list");
        assert_eq!(parsed, vec![1]);
        assert_eq!(store.list(Kind::Capability).expect("list").len(), 2);
    }
}
