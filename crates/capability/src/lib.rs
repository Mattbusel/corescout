//! What the machine has learned it can make itself become.
//!
//! # Three layers, and the map above them
//!
//! | layer | the claim | type |
//! |---|---|---|
//! | concept | *this state of me exists* | `corescout_concept::Concept` |
//! | affordance | *from here, this action tends to take me there* | [`affordance::Affordance`] |
//! | capability | *I can invoke that reliably enough to treat it as a primitive* | [`capability::Capability`] |
//!
//! Knowledge, then controllability, then executable ability. Each is strictly
//! harder to establish than the one above it, and the crate is arranged so a
//! claim cannot be promoted without the evidence its layer requires.
//!
//! Above all three sits the [`atlas::Atlas`]: the graph `G = (S, A, T)` of
//! states the machine has found in itself, actions it can take, and transitions
//! it has measured between them.
//!
//! # Why the atlas is the object and not the capabilities
//!
//! A capability is a route. The atlas is the map, and a map is worth more than
//! the routes drawn on it, because it says what is *not* reachable, what is
//! reachable only expensively, and where the unexplored edges are.
//!
//! A computer out of the box has an enormous space of physically possible
//! configurations and a tiny human-given vocabulary for talking about them. The
//! atlas is the record of converting the first into the second:
//!
//! ```text
//! physical possibility  ->  discovered structure  ->  measured control
//! ```
//!
//! # What is accumulated
//!
//! Two physically identical machines, one of which has run this for a long time,
//! differ in what they can *intentionally* do. The difference is not a file or a
//! cache or a log. It is that one of them has measured its own transitions and
//! the other has not.
//!
//! [`atlas::Atlas::command`] puts a number on that, with all the caveats its
//! documentation carries.

pub mod affordance;
pub mod atlas;
pub mod capability;
pub mod frontier;

pub use affordance::{Affordance, AffordanceId, Transition};
pub use atlas::{Atlas, Command, Route};
pub use capability::{Attempt, Capability, CapabilityId, Failure, Step};
pub use frontier::{compose, Admission, Frontier, Reach, Via};
