//! The machine as a scientist of itself.
//!
//! # What this adds to the discovery layer
//!
//! `corescout-represent` finds structure: states that recur, variables that
//! move together. `corescout-selfmodel` predicts. Neither can be **wrong** in
//! the way that matters, because neither commits to anything in advance. A
//! predictor that misses simply updates its coefficients.
//!
//! This crate commits first, then looks:
//!
//! ```text
//! observation -> hypothesis -> prediction -> experiment -> falsification
//!                     ^                                          |
//!                     +------------- revised theory -------------+
//! ```
//!
//! A [`hypothesis::Hypothesis`] cannot be constructed unless some possible
//! observation would refute it, and a [`theory::Theory`] keeps its graveyard so
//! the rate at which it is wrong stays visible.
//!
//! # Why the refutation rate is the number to watch
//!
//! A theory full of supported claims is not evidence of understanding. It is
//! equally consistent with a generator that only makes safe statements, which
//! is the easier thing to build and the harder thing to notice. A healthy
//! refutation rate means the machine is making claims bold enough to be wrong,
//! which is the only kind worth making.
//!
//! [`theory::Theory::render`] says so out loud when the rate falls too low.

pub mod causal;
pub mod hypothesis;
pub mod theory;

pub use causal::{Attribution, CausalEffect, CausalEstimate, Exploration, Insufficient};
pub use hypothesis::{Claim, Expectation, Hypothesis, Verdict};
pub use theory::{Confrontation, Refutation, Theory, TheoryConfig};
