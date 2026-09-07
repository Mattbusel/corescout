//! Deliberate perturbation: benchmarks, probes and the comparison harness.
//!
//! # Why this is not the mirror
//!
//! Everything here changes the machine on purpose. A benchmark pins a thread it
//! does not own, saturates a core, raises package temperature, drains turbo
//! budget and evicts other tenants' cache lines. It does all of that *in order
//! to find something out*, which is the definition of an experiment and the
//! opposite of a reflection.
//!
//! The original CoreScout was almost entirely this crate. It still works
//! exactly as it did; it has simply stopped being the centre of the project.
//!
//! # What the harness is for
//!
//! A self-modelling controller is only interesting if it beats the
//! alternatives, and the alternatives are strong. The Linux scheduler is very
//! good. Static pinning to a well-chosen core is very good. "Whatever CoreScout
//! 0.1 ranked first" is a real baseline, and beating it is the minimum bar for
//! the whole project to have been worth building.
//!
//! [`baselines`] enumerates the policies to compare, [`workloads`] the
//! workloads to compare them on, and [`harness`] runs the matrix and reports
//! it. Where a result is inconclusive, it is reported as inconclusive.

pub mod baselines;
pub mod benchmarks;
pub mod harness;
pub mod workloads;

pub use baselines::{Baseline, Placement};
pub use harness::{Comparison, Harness, Measurement, ScenarioResult, Verdict};
pub use workloads::{Metric, Scenario, ScenarioKind};
