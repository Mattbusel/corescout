//! Durable local storage for everything CoreScout learns.
//!
//! # Three stores, because the data is three shapes
//!
//! | store | shape | lives in |
//! |---|---|---|
//! | [`docs`] | small structured records, read by key and by kind | `corescout.redb` |
//! | [`events`] | an append-only log of what happened, read by range | `corescout.redb` |
//! | [`ring`] | fixed-width telemetry at observation rate | `mirror.ring` |
//!
//! Forcing observation-rate telemetry through the transactional store would
//! make a product that idles at near-zero cost into one that writes a
//! transaction every hundred milliseconds forever. The ring is a fixed-size
//! file of fixed-size records that wraps, so a machine that has been running
//! for a year uses exactly as much disk as one that started this morning.
//!
//! # Local-first is a storage property, not a policy
//!
//! Nothing in this crate opens a socket. There is no sync, no upload, no
//! remote. If a future version transmits anything it will have to be written
//! somewhere else, which is the point: the boundary is visible in the
//! dependency graph rather than in a privacy policy.
//!
//! # Migrations
//!
//! [`SCHEMA_VERSION`] is stamped into the database on creation and checked on
//! every open. Opening a newer database with an older binary fails loudly
//! rather than silently misreading it. See [`migrate`].

#![deny(missing_docs)]

pub mod docs;
pub mod events;
pub mod ids;
pub mod migrate;
pub mod packaged;
pub mod paths;
pub mod ring;
pub mod store;

pub use docs::{Document, Kind};
pub use events::{Event, EventKind, Severity};
pub use ids::Id;
pub use migrate::SCHEMA_VERSION;
pub use ring::{Ring, Sample};
pub use store::Store;
