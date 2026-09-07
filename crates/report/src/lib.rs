//! Human-facing reports of things that required touching the machine.
//!
//! # Why this is not part of `corescout-human`
//!
//! Rendering a topology needs the topology crate; rendering a benchmark needs
//! the benchmark crate. Both of those reach hardware, so a crate that renders
//! them inherits a path to `/sys` and passes it on to everything that links it.
//!
//! That is exactly what happened: the programs that are supposed to know the
//! machine only through its mirror were linking the human-output crate, which
//! linked the analysis crate, which linked the substrate. The boundary held in
//! the source and leaked through the dependency graph.
//!
//! Splitting the reports out fixes it at the level where it is checkable. The
//! mirror consumers link `corescout-human` and get formatting helpers and the
//! mirror projections; nothing they link can reach the machine.

pub mod human_debug;
pub mod json;
