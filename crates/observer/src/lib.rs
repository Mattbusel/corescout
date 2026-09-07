//! A consumer that knows the machine only through its reflection.
//!
//! # The experiment this crate is
//!
//! It depends on `corescout-mirror`, `corescout-memory`, `corescout-represent`,
//! `corescout-selfmodel`, `corescout-identity`, `corescout-counterfactual` and
//! `corescout-intent`.
//!
//! It does **not** depend on `corescout-substrate`. There is no code path from
//! here to `/sys`, `/proc`, `CPUID` or `perf_event_open`, and that is a fact
//! about the build rather than a claim in a comment: the crates that could do
//! those things are not linked into a binary that uses only this one.
//!
//! If an observer built this way can say true things about a machine, the
//! mirror is a real abstraction. If it cannot, the mirror is an internal data
//! structure with a nice name.
//!
//! # The A/B question
//!
//! The same observer runs under two [`Lens`]es:
//!
//! ```text
//! Labelled     "cpu/3", "cpu.frequency.current", "smt_sibling", Celsius
//! Unlabelled   entity_7, x2, edge_type_2, no units
//! ```
//!
//! Both see identical numbers; only the vocabulary differs. The question is
//! whether our ontology helps a system understand itself or constrains it, and
//! it is only answerable if the same learner can be run both ways.
//!
//! # What it does, in order
//!
//! ```text
//! remember    keep a bounded history of reflections
//! normalise   put incommensurable channels on one scale
//! discover    which entities and variables move together
//! recognise   assign the reflection to a latent state, or found a new one
//! predict     what the next reflection will be, with confidence
//! bound       which entities look like "me"
//! ```

pub mod lens;
pub mod observer;
pub mod report;

pub use lens::Lens;
pub use observer::{Observer, ObserverConfig};
pub use report::Findings;
