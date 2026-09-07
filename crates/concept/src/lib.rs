//! Concepts the machine coined about itself, and the resources it built from
//! them.
//!
//! # The threshold this crate exists to cross
//!
//! > The system constructs a concept that was not explicitly supplied by its
//! > designers, uses that concept to predict its own behavior, and acts
//! > successfully because of it.
//!
//! Three clauses, each of which can fail on its own, and each of which is
//! checked somewhere different:
//!
//! | clause | where it is enforced |
//! |---|---|
//! | not supplied by designers | [`concept::Origin`]: every concept traces to a discovery |
//! | used to predict | [`concept::CoinageRules::min_utility`]: coinage requires predictive utility |
//! | acts successfully because of it | [`resource::VirtualResource::reliability`] |
//!
//! The middle one is what stops this being renaming. A recurring state is not a
//! concept until knowing you are in it improves prediction, and a concept that
//! stops predicting is retired.
//!
//! # Three things this crate will not do
//!
//! **It will not rename a discovered concept to a human word.** If `concept_7`
//! coincides with what we call thermal throttling, that is a finding, recorded
//! as [`concept::Concept::resembles`] and read by nothing.
//!
//! **It will not let coinage justify itself.** Predictive utility is measured by
//! the self-model and passed in. A registry that computed its own justification
//! would coin everything.
//!
//! **It will not offer a resource it cannot deliver.** Reliability is `None`
//! until there is evidence, and a resource that falls below its promise is
//! withdrawn.

pub mod concept;
pub mod resource;
pub mod signature;

pub use concept::{CoinageRules, Concept, ConceptId, Origin, Refusal, Registry};
pub use resource::{Attempt, Catalogue, Recipe, Step, VirtualResource};
pub use signature::{find_analogue, Analogue, Signature};
