//! Where does the machine end?
//!
//! # The question, stated as engineering
//!
//! The mirror contains entities. Some of them are "the machine" in whatever
//! sense that phrase has; some are equipment it is attached to; some are
//! workload passing through. Nothing in the reflection says which is which, and
//! **the boundary is deliberately not hard-coded**.
//!
//! Instead, evidence is gathered per entity along four axes, and candidate
//! boundaries are constructed from it:
//!
//! | axis | question | what it is measured from |
//! |---|---|---|
//! | coupling | does it move with the rest? | behavioural correlation |
//! | controllability | does it respond to my actions? | action outcomes |
//! | persistence | is it continuously there? | presence across epochs |
//! | observability | can I see it at all? | filled cells |
//!
//! A candidate is a *hypothesis*, named `self_candidate_a`, not an answer. The
//! project's position is that "is RAM part of the self" is an experimental
//! question and this crate is the apparatus, not the conclusion.
//!
//! # Controllability is the interesting axis
//!
//! Coupling alone would put a busy neighbour process inside the boundary: it
//! moves with everything else because it is *causing* everything else. What
//! distinguishes self from environment is that acting changes it. An entity
//! that responds to this system's own actuators is a candidate for "me" in a
//! way that a merely correlated entity is not.
//!
//! That criterion has a consequence worth stating: **the boundary depends on
//! what the system can do.** Add an actuator and the self grows. This is
//! treated as a finding rather than a flaw; see `RESEARCH.md`.
//!
//! # Not anthropomorphised
//!
//! Nothing here concludes anything about awareness. A boundary is a partition
//! of an entity set supported by measurements, and the vocabulary stays at that
//! level.

pub mod boundary;
pub mod description;

pub use boundary::{
    BoundaryCriterion, BoundaryInference, CandidateId, EntityEvidence, SelfCandidate,
};
pub use description::{describe, Evidence, Ground, Proposition, SelfDescription, Topic};
