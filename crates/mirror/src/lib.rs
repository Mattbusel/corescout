//! `M(t)`: the computational mirror.
//!
//! > A computational mirror is a persistent, read-only, causally accessible
//! > representation of a computational substrate's current observable state and
//! > relationships, expressed in a form optimised for machine consumption rather
//! > than human interpretation.
//!
//! ```text
//! physical machine state H(t)
//!         |
//!      observation
//!         |
//!         v
//!     mirror M(t)
//! ```
//!
//! # What is here, and what deliberately is not
//!
//! This crate is the *representation* and its transport. It contains no
//! sensors, no syscalls that reach hardware, and no way to produce a snapshot
//! from a real machine. That work lives in `corescout-substrate`, which depends
//! on this crate and not the other way around.
//!
//! The split is what lets a consumer link `corescout-mirror` alone and be
//! physically incapable of reading `/sys`, rather than merely discouraged from
//! it. An observer's isolation is a property of the dependency graph.
//!
//! ```text
//! M(t) != recommendation      that is analysis
//! M(t) != prediction          that is selfmodel
//! M(t) != historical summary  that is memory
//! M(t) != benchmark result    that is experiment
//! ```
//!
//! # The representation
//!
//! ```text
//! G(t) = (V, E, X(t))
//!
//! V     entities with persistent identity      entity
//! E     relations between them                 relation
//! X(t)  a dense entities x channels matrix     state
//! ```
//!
//! Assembled into a [`MirrorSnapshot`] and published through the [`plane`], a
//! fixed-layout shared memory region consumers map read-only.
//!
//! # Nothing here is CPU-specific
//!
//! [`entity::EntityClass`] names CPUs and caches today and is an open
//! enumeration with an `Unclassified` variant; the schema has no notion of a
//! core. A GPU, a memory controller or a NIC becomes an entity with channels
//! and edges like anything else. See `ONTOLOGY.md`.

pub mod entity;
pub mod nanjson;
pub mod plane;
pub mod relation;
pub mod schema;
pub mod snapshot;
pub mod state;

#[doc(hidden)]
pub mod test_support;

pub use entity::{Entity, EntityClass, EntityId};
pub use plane::{PlaneReader, PlaneWriter};
pub use relation::{Relation, RelationKind, RelationView};
pub use schema::{Availability, AvailabilityMatrix, Perturbation, SensorId, SensorReport};
pub use snapshot::{MirrorSnapshot, FORMAT_VERSION};
pub use state::{ChannelId, ChannelSpec, Semantics, StateMatrix, Unit};
