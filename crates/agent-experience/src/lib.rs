//! What working with an AI has taught this computer.
//!
//! # The same discipline, one layer up
//!
//! The research layers of this project spent their effort on one rule: an
//! association is not an effect. This crate applies that rule to agent
//! operations, where breaking it is easy and expensive.
//!
//! CoreScout notices that builds fail here. It notices that they fail less
//! often when the schema was regenerated first. That is an *association*, and
//! at that point the honest thing to say is that it is an association. The
//! agent regenerating the schema first was probably already the more careful
//! run; the correlation may be entirely the agent and not the procedure.
//!
//! To say more, CoreScout has to randomise: on a small fraction of occasions,
//! suggest the procedure by coin flip rather than by belief. Only trials
//! assigned that way support a causal claim, and [`corescout_science`] refuses
//! to compute one without them.
//!
//! ```text
//! observed together           -> Basis::Association   "seen together 7 times"
//! randomised and measured     -> Basis::Causal        "43% -> 7% failure rate"
//! ```
//!
//! Every learned item the interface shows carries which of those it is. That
//! is not a technical detail exposed for its own sake; it is the difference
//! between a suggestion worth following and a coincidence.
//!
//! # Thresholds are deliberately unkind
//!
//! A pattern needs repetition before it is a pattern, a lift over the base
//! rate before it is worth mentioning, and randomised evidence before it is
//! offered as a procedure. Most candidates die at one of those gates, and the
//! count of what died is shown in the interface next to what survived.

#![deny(missing_docs)]

pub mod experience;
pub mod failure;
pub mod pattern;
pub mod procedure;

pub use experience::{Experience, Thresholds};
pub use failure::FailureMode;
pub use pattern::{Basis, Pattern, Subject};
pub use procedure::{Candidate, Procedure, Step};
