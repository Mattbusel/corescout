//! The commands, grouped by which layer of the architecture they touch.
//!
//! The grouping is the point. [`observe`] reads hardware and changes nothing.
//! [`consume`] reads only reflections and cannot reach hardware. [`perturb`]
//! deliberately changes the machine to find something out. [`agent`] acts on it
//! under an intent. [`demo`] runs the whole cycle once.
//!
//! A command in the wrong module is a design error, not a filing error.

pub mod agent;
pub mod consume;
pub mod demo;
pub mod observe;
pub mod perturb;
pub mod science;
