//! Capabilities: what CoreScout has learned to do, and the machinery to do it.
//!
//! # The three properties a capability has to have
//!
//! **Evidence.** Where it came from, and on what grounds. A capability with no
//! provenance is a script someone left lying around.
//!
//! **Verification.** A step whose job is to disagree with the others. Without
//! it the only thing a run can report is what it was told, and the whole
//! product exists because what a tool reports and what happened are different
//! things.
//!
//! **A way back.** Not always possible, and when it is not, the definition
//! says so rather than claiming to be reversible.
//!
//! # Boundaries are checked twice
//!
//! Once when the run is authorised, and again immediately before each step
//! starts. The second check is not redundant: a definition can be edited while
//! a run is in flight, and a capability that was permitted must not become
//! able to touch something new halfway through.

#![deny(missing_docs)]

pub mod capability;
pub mod runtime;

pub use capability::{Capability, Invalid, Operation, Provenance};
pub use runtime::{Context, Outcome, Runtime, StepResult};
