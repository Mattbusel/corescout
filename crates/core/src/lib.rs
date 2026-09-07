//! Shared primitives.
//!
//! Errors, the monotonic clock, and CPU sets. Everything here is used by more
//! than one layer and belongs to none of them.
//!
//! This crate depends on nothing but `serde`. That is deliberate: it sits at
//! the bottom of the graph, so anything it pulled in would be pulled in by the
//! whole system, including the crates whose entire purpose is to have no
//! hardware access.

pub mod clock;
pub mod cpuset;
pub mod error;
pub mod serde_util;

pub use cpuset::CpuSet;
pub use error::{Error, Result};

/// Identifier of a *logical* CPU as the OS numbers it (Linux: the `N` in
/// `/sys/devices/system/cpu/cpuN`).
///
/// A primitive rather than a topology concept: CPU sets, affinity masks and
/// actuators all speak in these, and none of them should have to depend on the
/// crate that discovers topology in order to name one.
pub type LogicalId = u32;

/// Identifier of a *physical* core, unique across the whole machine.
///
/// Synthesised by the substrate layer from `(package, core_id)`, because Linux
/// only guarantees `core_id` is unique within a package. Opaque to consumers.
pub type PhysicalId = u32;
