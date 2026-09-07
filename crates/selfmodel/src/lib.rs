//! `Z(t)`: how the machine behaves.
//!
//! ```text
//! prediction    M(t-k:t)        ->  M_hat(t + dt)     with uncertainty
//! anomaly       M(t)            ->  how surprising is this
//! ```
//!
//! # Prediction before optimisation
//!
//! This crate improves nothing and chooses nothing. It watches, forms claims,
//! and is scored on how wrong it was. A system that cannot predict its own
//! next state has no business acting on itself, and prediction error is the
//! only honest measure of whether the mirror carries enough signal to be worth
//! having.
//!
//! The central metric is deliberately **skill against a hard baseline**, not
//! absolute error. Most cells of a mirror barely move between consecutive
//! reflections, so copying the last value scores extremely well; any model
//! that does not clearly beat that has learned nothing, however small its
//! error looks.
//!
//! # Uncertainty is part of the output
//!
//! A prediction without a confidence is unusable by a controller: it cannot
//! tell the difference between "move the thread, I am sure" and "move the
//! thread, I am guessing". Every [`predictor::Prediction`] carries an interval
//! and a confidence derived from how well the model has been doing lately on
//! that specific cell, not from a global average.
//!
//! # What this crate cannot see
//!
//! It depends on `corescout-mirror`, `corescout-memory` and
//! `corescout-represent`, and on nothing that touches hardware. Everything it
//! knows came through the reflection.

pub mod anomaly;
pub mod predict;
pub mod predictor;
pub mod uncertainty;

pub use anomaly::{Anomaly, AnomalyDetector, Surprise};
pub use predict::{evaluate, CellScore, PredictionReport};
pub use predictor::{Prediction, SelfModel, StatePrediction};
pub use uncertainty::{Confidence, Interval};
