//! The substrate: hardware discovery and passive observation.
//!
//! **This is the only crate in the workspace that touches hardware.** It reads
//! sysfs, procfs, `CPUID` and `perf_event_open`, and it produces
//! [`MirrorSnapshot`](corescout_mirror::MirrorSnapshot)s. Every other crate
//! either produces snapshots for it to publish, or consumes them.
//!
//! That is the load-bearing fact about the dependency graph. `corescout-model`,
//! `corescout-observer` and their neighbours do not depend on this crate, so an
//! observer cannot read `/sys` even by mistake: the code that would do it is not
//! linked into its binary.
//!
//! # Layers here
//!
//! | module | responsibility |
//! |---|---|
//! | [`platform`] | the only `cfg(target_os)` in the project |
//! | [`topology`] | platform-neutral model of what the CPU is made of |
//! | [`discovery`] | turning topology into stable entities and relations |
//! | [`observation`] | passive sensors, each declaring what looking costs |
//! | [`reflector`] | the loop that binds sensors and produces reflections |
//!
//! # The rule this crate enforces
//!
//! > Observation belongs to the mirror. Intentional perturbation belongs to
//! > experimentation.
//!
//! Every sensor here is passive: it reads state the machine maintains anyway,
//! for its own reasons, whether or not anyone is looking. Nothing here runs a
//! workload, moves a thread, changes a frequency or requests an idle state.
//! Code that wants to do those things lives in `corescout-experiment` or
//! `corescout-agency`.

pub mod discovery;
pub mod observation;
pub mod platform;
pub mod reflector;
pub mod topology;

#[doc(hidden)]
pub mod test_support;

pub use discovery::{Roots, Substrate};
pub use reflector::Reflector;
pub use topology::Topology;
