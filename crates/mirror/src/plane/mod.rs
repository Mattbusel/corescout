//! The Self-State Plane: the machine's reflection, exposed as memory.
//!
//! ```text
//! physical machine
//!        |
//!        v
//! observation layer          reads sysfs, procfs, perf, once
//!        |
//!        v
//! self-state plane           shared memory, fixed layout, read-only
//!        |
//!        v
//! consumer                   mmap, then memcpy
//! ```
//!
//! # What this is for
//!
//! Every consumer that wants to know something about the machine currently
//! reopens and reparses the same text files that every other consumer is
//! reparsing: `/proc/stat` on each poll, `/sys/.../scaling_cur_freq` per CPU per
//! tick, `/proc/interrupts` in full to read one row. The cost scales with the
//! number of observers, and each observer normalises the data slightly
//! differently, so they do not even agree with each other.
//!
//! CoreScout does that work once and publishes the result as a numeric matrix in
//! shared memory. A consumer maps it and reads. The normalisation, the identity
//! assignment, the unit conversion and the parsing have already happened.
//!
//! # Design constraints, and how each is met
//!
//! | constraint | mechanism |
//! |---|---|
//! | low latency | mmap; a read is a `memcpy`, no syscalls |
//! | minimal parsing | fixed binary layout; static tables decoded once per epoch |
//! | numeric representation | `f64` matrix, row-major, cache-line aligned |
//! | stable layout | offsets in the header, versioned, validated before use |
//! | read-only consumers | `O_RDONLY` + `PROT_READ`; enforced by the MMU |
//! | cheap frequent reads | seqlock, so readers never block the writer |
//!
//! # Modules
//!
//! - [`layout`] is the binary format: offsets, records, and the encode and
//!   decode primitives. Pure functions over byte slices, so the format is
//!   testable without a machine, an mmap, or Linux.
//! - [`memory`] owns the region, and is where the read-only boundary is
//!   enforced.
//! - [`writer`] publishes snapshots. There is exactly one.
//! - [`reader`] consumes them. There may be any number.

pub mod layout;
pub mod memory;
pub mod reader;
pub mod writer;

pub use memory::{default_path, PlaneMemory};
pub use reader::{PlaneReader, PlaneState};
pub use writer::PlaneWriter;
