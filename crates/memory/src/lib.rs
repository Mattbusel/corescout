//! `H(t)`: what the machine has been.
//!
//! ```text
//! Mirror  M(t)                       the present
//! Memory  H(t) = {M(t-k) .. M(t)}    the past
//! ```
//!
//! # Why this is a separate crate rather than fields on the snapshot
//!
//! A mirror reflects the present. The moment a rolling average, a trend, a
//! percentile or a "previous value" is stored inside `M(t)`, the snapshot stops
//! being a function of the machine at time `t` and becomes a function of the
//! machine *and of what the observer happened to see before*. Two consumers
//! sampling at different rates would then disagree about the present.
//!
//! This is also why the mirror publishes cumulative counters and never rates.
//! The counter is a present fact: the current value of a register or of a
//! kernel variable, readable in one observation. A rate is a statement about an
//! interval, requires two observations, and is therefore memory. Turning
//! counters into rates is [`transitions`]' job, and it happens here.
//!
//! # Three things, deliberately distinct
//!
//! | module | what it is |
//! |---|---|
//! | [`history`] | a bounded ring of recent reflections, in memory |
//! | [`recording`] | a durable file of reflections, and replay from it |
//! | [`experience`] | one *observer's* private record of what it saw |
//!
//! The last is not the same as the first. "What was the machine doing 500 ms
//! ago" is a fact about the machine; "what have I seen" is a fact about the
//! observer, and two observers that started at different times have different
//! answers while the machine has only one history.
//!
//! # Live and recorded are the same thing
//!
//! [`Source`] is the abstraction that makes `mirror-observer` and
//! `mirror-learn` unable to tell whether they are watching a live plane or
//! replaying a file. That is not a convenience: an analysis that behaves
//! differently on recorded data cannot be checked, and an experiment that
//! cannot be re-run on the same input is not an experiment.

pub mod experience;
pub mod history;
pub mod recording;
pub mod source;
pub mod transitions;

pub use experience::{Experience, Observation};
pub use history::History;
pub use recording::{Recorder, Recording};
pub use source::{ReflectionSource, Source};
pub use transitions::{Transition, TransitionSet};
