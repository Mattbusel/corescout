//! Turning compute into computational capital, and checking whether it worked.
//!
//! # The claim this crate exists to test
//!
//! > Spend compute learning a machine, compile that experience into a more
//! > productive descendant, and the descendant does the same useful work with
//! > fewer scarce physical inputs.
//!
//! If that holds and compounds, the process manufactures effective compute out
//! of knowledge. If it does not, it is an expensive way to keep a machine busy.
//! Both are findable here, and the crate is arranged so the second is as easy
//! to report as the first.
//!
//! # Why the accounting comes before the mechanism
//!
//! `Q = verified work / scarce inputs` is the number the whole project would be
//! judged by, which makes it the number most worth faking, and a system that
//! measures its own success will find that it is succeeding.
//!
//! So [`productivity`] is built adversarially: unverified work is worth nothing
//! and still costs, search is charged to whoever inherits the gain, held-out
//! work is accounted separately, and the weights that price scarce inputs come
//! from outside. [`lineage::Verdict`] can return
//! [`lineage::Verdict::LearnedTheBenchmark`],
//! [`lineage::Verdict::SearchCostTooHigh`] and
//! [`lineage::Verdict::Untrustworthy`], and a harness unable to return those is
//! not measuring anything.
//!
//! # The control
//!
//! Machines drift. A lineage that improves proves nothing without a control
//! lineage spending the same inputs and carrying nothing forward.
//! [`lineage::Comparison::attributable_gain`] is the difference, and it is the
//! only figure here worth quoting to somebody sceptical.

pub mod lineage;
pub mod productivity;

pub use lineage::{Comparison, Generation, Lineage, Verdict};
pub use productivity::{Inputs, Ledger, Pool, Weights, Work};
