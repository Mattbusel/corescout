//! Imagination: what would happen to me if I did this.
//!
//! ```text
//! F(M(t), A)  ->  M_hat(t + dt | A)     with uncertainty
//! ```
//!
//! # This is not the mirror
//!
//! A counterfactual produces something shaped exactly like a reflection and
//! which is *not one*. It is a claim about a machine that does not exist. So:
//!
//! - it is never written into the plane,
//! - it never carries a real observation timestamp,
//! - and [`Outcome`] is a distinct type from `MirrorSnapshot`, so the two
//!   cannot be confused by accident.
//!
//! # How it learns
//!
//! From paired observations: the state before an action, the action, and the
//! state that followed. Nothing else can teach it, and in particular the
//! passive self-model cannot: a model of how the machine evolves on its own
//! says nothing about how it evolves when pushed.
//!
//! This has an uncomfortable consequence that the design takes seriously. A
//! counterfactual model can only know about actions that have been **tried**.
//! Until the system has moved a thread to entity 13, its prediction about
//! moving a thread to entity 13 is an extrapolation, and it says so through
//! [`Prediction::support`]. That is what makes bounded self-experimentation
//! necessary rather than merely interesting: it is the only way the model
//! acquires the evidence it needs.
//!
//! # Deliberately simple
//!
//! Effects are learned per `(action family, target entity, channel)` as a
//! distribution of observed changes. No neural network, no latent dynamics
//! model. With tens of trials per combination, a mean and a spread is what the
//! data can support, and anything more expressive would be fitting noise while
//! looking authoritative.

pub mod effects;
pub mod outcome;

pub use effects::{Candidate, EffectKey, EffectModel, Measure, ObservedEffect};
pub use outcome::{Confidence, Outcome, Prediction, Support};
