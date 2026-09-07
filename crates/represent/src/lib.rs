//! Learned representations of machine state.
//!
//! ```text
//! [M(t-n) .. M(t)]  ->  structure, and Z: a vocabulary the machine derived
//! ```
//!
//! # What this crate is for
//!
//! Turning a series of reflections into things that can be *named and reused*:
//! groups of entities that behave alike, edges that predict shared behaviour,
//! and recurring whole-machine configurations.
//!
//! Nothing here knows what a core, a cache or a temperature is. Every function
//! takes numbers indexed by `(entity row, variable column, time)` plus an edge
//! list. That is not an aesthetic choice: if the discovery code could look up a
//! channel called `cpu.frequency.current`, then whatever it found would be a
//! restatement of what we already told it.
//!
//! # Two levels
//!
//! | module | question |
//! |---|---|
//! | [`discover`] | which things move together, right now, in this window |
//! | [`latent`] | which *configurations* recur, persist, and predict |
//!
//! The first is statistics. The second is the beginning of an ontology: a
//! latent state that recurs, is stable, and improves prediction is a candidate
//! concept, and it gets an identifier of its own rather than being translated
//! back into human vocabulary. See `ONTOLOGY.md`.

pub mod discover;
pub mod features;
pub mod latent;
pub mod normalize;

pub use discover::{
    accumulators, cluster_entities, correlation, differences, mean_similarity, regimes,
    relation_lift, remove_common_mode, similarity_matrix, standardise, variable_groups,
    Accumulator, EntityCluster, Regime, RelationLift, VariableGroup,
};
pub use latent::{LatentCatalogue, LatentState, LatentStateId};
pub use normalize::{Normalizer, Scaling};
