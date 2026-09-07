//! Autonomy: choosing among permitted actions.
//!
//! ```text
//! A(t) = policy(Mirror(t), Memory(t), Z(t), F, Intent, AvailableActions)
//! ```
//!
//! # The loop
//!
//! ```text
//! SEE        the current reflection
//! REMEMBER   consult history
//! INTERPRET  which latent state is this
//! IMAGINE    predict candidate futures, with support and confidence
//! EVALUATE   score them against the intent
//! CHOOSE     pick one, or choose to hold
//! ACT        through an audited actuator
//! SEE        observe the consequence and learn from it
//! ```
//!
//! # Holding is a real choice
//!
//! The most common correct output of this layer is to do nothing. A controller
//! that acts whenever it can find any predicted improvement will thrash, and
//! thrashing costs cache warmth on every move while the predicted gains were
//! inside the noise. [`policy::Decision::Hold`] is a first-class outcome and is
//! audited like any other.
//!
//! # Exploration is separated from exploitation
//!
//! [`curiosity`] proposes experiments whose purpose is *reducing uncertainty*,
//! not improving the objective. Mixing the two would make it impossible to say
//! whether a regression came from a bad optimisation or a deliberate probe, and
//! would let a controller justify any action as "exploration".
//!
//! # Safety is not advisory
//!
//! [`safeguards::Watchdog`] runs independently of the policy and can freeze all
//! actuation. The controller cannot disable it, because it does not hold a
//! mutable reference to it. If the loop stalls, violates its budget, or makes
//! decisions the watchdog cannot verify, everything reverts and the machine is
//! left to the operating system.

pub mod conditioned;
pub mod curiosity;
pub mod loop_;
pub mod policy;
pub mod safeguards;

pub use curiosity::{Curiosity, Experiment, ExperimentOutcome};
pub use loop_::{ControlLoop, LoopConfig, Tick};
pub use policy::{Decision, Policy, PolicyConfig};
pub use safeguards::{Verdict, Watchdog, WatchdogConfig};
