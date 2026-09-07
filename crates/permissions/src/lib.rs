//! What CoreScout is allowed to do, and who decided.
//!
//! # Four words the user has to understand, and no more
//!
//! [`Autonomy`] is the whole model a person needs: *Observe*, *Suggest*,
//! *Assist*, *Autopilot*. Everything else in this crate exists so those four
//! words mean something precise enough to be enforced rather than merely
//! displayed.
//!
//! # A ruling is never a boolean
//!
//! [`Ruling`] is *allowed*, *needs approval*, or *refused with a reason*. A
//! product that answers permission questions with `true`/`false` cannot tell a
//! user why something did not happen, and the reason is the part they need. So
//! [`Permissions::rule`] always carries one, and the Explanations UI reads it
//! rather than inventing prose.
//!
//! # The order the checks run in matters
//!
//! Emergency pause, then authority, then autonomy, then rate limit. Authority
//! comes before autonomy deliberately: turning Autopilot on must not be able
//! to grant reach that Suggest did not have. Autonomy decides *whether a human
//! is asked*, not *what may be touched*.
//!
//! ```text
//! paused?          -> Refused
//! outside authority-> Refused        (mode cannot override this)
//! autonomy mode    -> Refused | NeedsApproval | continue
//! rate limit       -> Refused | Allowed
//! ```
//!
//! # This crate touches nothing
//!
//! It has no hardware access, spawns no process, and opens no file. It decides.
//! Whatever carries a decision out lives elsewhere, which means a bug here can
//! only ever be too strict or too permissive on paper, and the enforcement
//! point is a single call that is easy to find.

#![deny(missing_docs)]

pub mod authority;
pub mod autonomy;
pub mod limits;
pub mod ruling;

pub use authority::{Authority, Target};
pub use autonomy::Autonomy;
pub use limits::{Limits, RateLimiter};
pub use ruling::{Grant, Permissions, Request, Risk, Ruling};
