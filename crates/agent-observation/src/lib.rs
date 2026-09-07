//! What the AI did, as the machine saw it.
//!
//! # The mirror, extended to software
//!
//! The hardware mirror answers "what is this machine doing". This crate
//! answers "what is the AI doing to it", using the same discipline: persistent
//! entities, observable channels, and an explicit representation of what could
//! not be observed rather than a guess.
//!
//! Nothing here reads a model's thoughts, its prompts, or its reasoning. It
//! records externally visible behaviour: a tool was invoked, a command ran, it
//! exited 1, these files changed, it was tried again forty seconds later.
//! That is both the ethical line and the useful one, because it is the only
//! part reality can contradict.
//!
//! # The distinction the whole product rests on
//!
//! [`Report`] is what the tool *said* happened. [`Verification`] is what was
//! *checked*. They are separate fields because they disagree, and every
//! disagreement is exactly the thing worth learning:
//!
//! ```text
//! reported Success + verified Contradicted  -> a silent failure
//! reported Success + verified None          -> nobody checked
//! reported Success + verified Confirmed     -> it worked
//! ```
//!
//! A schema that collapsed these into one `success: bool` could not represent
//! "the deploy said it worked and the service was not reachable", which is the
//! first thing this product exists to notice.
//!
//! # Recurrence needs a fingerprint
//!
//! Two runs of `cargo build --release --target-dir C:\tmp\a1b2` are the same
//! operation. [`fingerprint`] strips the parts that vary so recurrence can be
//! counted, and it is deliberately conservative: over-normalising merges
//! operations that are genuinely different, which invents patterns.

#![deny(missing_docs)]

pub mod action;
pub mod agent;
pub mod fingerprint;
pub mod ingest;
pub mod redact;
pub mod session;
pub mod workspace;

pub use action::{Action, ActionKind, Report, Verification};
pub use agent::{Agent, AgentKind};
pub use fingerprint::Fingerprint;
pub use ingest::{Ingest, Raw};
pub use session::{Session, Task, TaskOutcome};
pub use workspace::Workspace;
