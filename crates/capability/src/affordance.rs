//! *From here, doing this tends to take me there.*
//!
//! # The layer between knowing and being able
//!
//! A concept is knowledge: *this state of me exists*. A capability is
//! executable ability: *I can invoke this reliably enough to treat it as a
//! primitive*. Between them sits controllability, and this is it.
//!
//! An [`Affordance`] is a measured regularity in the machine's own transitions.
//! It is weaker than a capability in two specific ways, and the difference is
//! worth being exact about because collapsing them is how a system comes to
//! believe it can do things it cannot:
//!
//! | | affordance | capability |
//! |---|---|---|
//! | evidence | observed transitions | attempts *made in order to arrive* |
//! | claim | this tends to happen | I can make this happen |
//! | invocable | no | yes |
//!
//! The second row is the important one. An affordance can be measured entirely
//! passively: watch the machine, notice that `S14` under `a7` is usually
//! followed by `S31`. That is a real fact and it is **not** the same as being
//! able to bring `S31` about, because the times `a7` was taken from `S14` may
//! all have been times when something else was also true.
//!
//! Promotion to a capability therefore requires more than a high probability.
//! See [`Affordance::promotable`].
//!
//! # Failure modes are part of the edge
//!
//! An edge that fails 30% of the time for a reason the machine can state is far
//! more useful than one that fails 30% of the time for no reason: the first can
//! be declined in advance. The reasons are counted here, not just the failures.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::capability::Failure;

/// The machine's own name for an affordance it found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AffordanceId(pub u32);

impl fmt::Display for AffordanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "affordance_{}", self.0)
    }
}

/// One observed transition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub at_ns: u64,
    /// Where the machine actually ended up.
    pub arrived: String,
    /// What taking the action cost.
    pub cost: f64,
    /// How long it took to get there.
    pub latency_ns: u64,
    /// Whether the action was taken by coin flip rather than by choice.
    ///
    /// Carried because an edge measured only from chosen actions is confounded
    /// in exactly the way [`corescout_science::causal`] exists to refuse, and an
    /// affordance that wants to become a capability has to say which kind of
    /// evidence it rests on.
    pub randomised: bool,
    /// Why it did not arrive, when it did not.
    pub failure: Option<Failure>,
}

/// Observations before a probability is reported.
pub const ENOUGH_OBSERVATIONS: u32 = 12;
/// Randomised observations before an affordance may become a capability.
///
/// Higher than [`ENOUGH_OBSERVATIONS`], and required to be *randomised*,
/// because promotion is the point at which the machine starts acting on the
/// belief rather than merely holding it.
pub const ENOUGH_FOR_PROMOTION: u32 = 12;

/// A measured edge of the atlas: `from --action--> to`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Affordance {
    pub id: AffordanceId,
    /// The state the machine was in.
    pub from: String,
    /// The action family taken.
    pub action: String,
    /// The state it tends to arrive in.
    pub to: String,
    observations: u32,
    arrivals: u32,
    randomised_observations: u32,
    randomised_arrivals: u32,
    total_cost: f64,
    arrival_cost: f64,
    total_latency_ns: u64,
    arrival_latency_ns: u64,
    #[serde(with = "corescout_core::serde_util::pairs")]
    failures: BTreeMap<Failure, u32>,
    pub first_seen_ns: u64,
    pub last_seen_ns: u64,
}

impl Affordance {
    pub fn new(
        id: AffordanceId,
        from: impl Into<String>,
        action: impl Into<String>,
        to: impl Into<String>,
        now_ns: u64,
    ) -> Affordance {
        Affordance {
            id,
            from: from.into(),
            action: action.into(),
            to: to.into(),
            observations: 0,
            arrivals: 0,
            randomised_observations: 0,
            randomised_arrivals: 0,
            total_cost: 0.0,
            arrival_cost: 0.0,
            total_latency_ns: 0,
            arrival_latency_ns: 0,
            failures: BTreeMap::new(),
            first_seen_ns: now_ns,
            last_seen_ns: now_ns,
        }
    }

    /// A stable key for `(from, action, to)`.
    pub fn key(&self) -> String {
        format!("{}--{}-->{}", self.from, self.action, self.to)
    }

    pub fn observations(&self) -> u32 {
        self.observations
    }

    pub fn arrivals(&self) -> u32 {
        self.arrivals
    }

    pub fn randomised_observations(&self) -> u32 {
        self.randomised_observations
    }

    /// Credit this edge with attempts of its `(from, action)` that happened
    /// before it was discovered.
    ///
    /// All of them were non-arrivals for this outcome, by definition: had any
    /// arrived here, the edge would already exist. Without this, an outcome
    /// first seen on the hundredth attempt reports a hundred per cent, because
    /// it has one observation and one arrival.
    pub(crate) fn backfill(&mut self, prior_attempts: u32) {
        self.observations += prior_attempts;
    }

    pub fn record(&mut self, transition: &Transition) {
        if !transition.cost.is_finite() {
            return;
        }
        self.observations += 1;
        self.total_cost += transition.cost;
        self.total_latency_ns += transition.latency_ns;
        self.last_seen_ns = transition.at_ns;
        if transition.randomised {
            self.randomised_observations += 1;
        }
        if transition.arrived == self.to {
            self.arrivals += 1;
            self.arrival_cost += transition.cost;
            self.arrival_latency_ns += transition.latency_ns;
            if transition.randomised {
                self.randomised_arrivals += 1;
            }
        } else if let Some(failure) = &transition.failure {
            *self.failures.entry(failure.clone()).or_insert(0) += 1;
        }
    }

    /// How often this action from this state lands in that state.
    ///
    /// `None` until there is enough evidence. "Unknown" and "unlikely" are
    /// different facts and a caller must be able to tell them apart.
    pub fn probability(&self) -> Option<f64> {
        (self.observations >= ENOUGH_OBSERVATIONS)
            .then(|| self.arrivals as f64 / self.observations as f64)
    }

    /// The same, from randomised evidence only.
    ///
    /// The number that supports a claim about being *able* to do something,
    /// rather than a claim about what tends to happen.
    pub fn causal_probability(&self) -> Option<f64> {
        (self.randomised_observations >= ENOUGH_FOR_PROMOTION)
            .then(|| self.randomised_arrivals as f64 / self.randomised_observations as f64)
    }

    /// Mean cost of the times it worked.
    pub fn cost_on_arrival(&self) -> Option<f64> {
        (self.arrivals > 0).then(|| self.arrival_cost / self.arrivals as f64)
    }

    /// Expected cost of actually arriving, retries included.
    pub fn expected_cost(&self) -> Option<f64> {
        let probability = self.probability()?;
        if probability <= 0.0 {
            return None;
        }
        Some((self.total_cost / self.observations as f64) / probability)
    }

    pub fn mean_latency_ns(&self) -> Option<f64> {
        (self.arrivals > 0).then(|| self.arrival_latency_ns as f64 / self.arrivals as f64)
    }

    /// Whether this counts as an edge of the atlas at all.
    pub fn is_edge(&self, threshold: f64) -> bool {
        self.probability().is_some_and(|p| p >= threshold)
    }

    /// Whether this affordance has earned the right to become a capability.
    ///
    /// Three conditions, and the middle one is what makes this more than a
    /// rename:
    ///
    /// 1. it happens often enough to be worth invoking;
    /// 2. the evidence is **randomised**, so the claim is about what the machine
    ///    can cause rather than about what tends to accompany what;
    /// 3. it is not dominated by a single failure mode, which would mean the
    ///    precondition is wrong rather than the procedure unreliable.
    pub fn promotable(&self, threshold: f64) -> Result<f64, NotPromotable> {
        let Some(causal) = self.causal_probability() else {
            return Err(NotPromotable::NotIntervened {
                randomised: self.randomised_observations,
                needed: ENOUGH_FOR_PROMOTION,
            });
        };
        if causal < threshold {
            return Err(NotPromotable::TooUnreliable {
                probability: causal,
                needed: threshold,
            });
        }
        if let Some((failure, share)) = self.dominant_failure() {
            return Err(NotPromotable::Conditional {
                failure: failure.label(),
                share,
            });
        }
        Ok(causal)
    }

    /// The commonest reason it fails, when one dominates.
    pub fn dominant_failure(&self) -> Option<(&Failure, f64)> {
        let total: u32 = self.failures.values().sum();
        if total < 4 {
            return None;
        }
        let (failure, count) = self.failures.iter().max_by_key(|(_, n)| **n)?;
        let share = *count as f64 / total as f64;
        (share >= 0.75).then_some((failure, share))
    }

    pub fn describe(&self) -> String {
        let probability = match self.probability() {
            Some(value) => format!("{:.0}%", value * 100.0),
            None => format!("unknown after {} observations", self.observations),
        };
        let mut text = format!(
            "{}: from {} do {} to reach {} ({probability} of {} observations)",
            self.id, self.from, self.action, self.to, self.observations
        );
        match self.causal_probability() {
            Some(causal) => text.push_str(&format!(", {:.0}% under intervention", causal * 100.0)),
            None => text.push_str(", never tested by intervention"),
        }
        if let Some((failure, share)) = self.dominant_failure() {
            text.push_str(&format!(
                "; {:.0}% of failures are {}",
                share * 100.0,
                failure.label()
            ));
        }
        text
    }
}

/// Why an affordance is not yet executable ability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotPromotable {
    /// It has been watched but never deliberately tried.
    NotIntervened { randomised: u32, needed: u32 },
    /// Tried, and it does not happen often enough.
    TooUnreliable { probability: f64, needed: f64 },
    /// It fails for one reason most of the time, so the precondition is wrong
    /// rather than the procedure unreliable.
    Conditional { failure: String, share: f64 },
}

impl NotPromotable {
    pub fn describe(&self) -> String {
        match self {
            NotPromotable::NotIntervened { randomised, needed } => format!(
                "observed but not intervened on: {randomised} randomised trials of {needed} \
                 needed. Watching a transition happen is not evidence of being able to \
                 cause it"
            ),
            NotPromotable::TooUnreliable {
                probability,
                needed,
            } => format!(
                "happens {:.0}% of the time under intervention, below the {:.0}% needed",
                probability * 100.0,
                needed * 100.0
            ),
            NotPromotable::Conditional { failure, share } => format!(
                "{:.0}% of failures are '{failure}': the precondition is wrong rather than \
                 the procedure unreliable, so narrow it rather than promoting this",
                share * 100.0
            ),
        }
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub(crate) fn affordance() -> Affordance {
        Affordance::new(AffordanceId(7), "concept_3", "affinity", "concept_9", 0)
    }

    pub(crate) fn watch(affordance: &mut Affordance, arrivals: u32, misses: u32, randomised: bool) {
        for i in 0..arrivals {
            affordance.record(&Transition {
                at_ns: i as u64,
                arrived: affordance.to.clone(),
                cost: 1.0,
                latency_ns: 1_000_000,
                randomised,
                failure: None,
            });
        }
        for i in 0..misses {
            affordance.record(&Transition {
                at_ns: i as u64,
                arrived: "elsewhere".into(),
                cost: 1.0,
                latency_ns: 1_000_000,
                randomised,
                failure: Some(Failure::DidNotArrive),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn a_transition_seen_three_times_has_no_probability() {
        let mut affordance = affordance();
        watch(&mut affordance, 3, 0, false);
        assert_eq!(affordance.probability(), None);
        assert!(affordance.describe().contains("unknown"));
    }

    #[test]
    fn enough_observations_produce_a_probability() {
        let mut affordance = affordance();
        watch(&mut affordance, 18, 2, false);
        assert!((affordance.probability().unwrap() - 0.9).abs() < 1e-9);
        assert!(affordance.is_edge(0.8));
        assert!(!affordance.is_edge(0.95));
    }

    #[test]
    fn watching_a_transition_is_not_evidence_of_being_able_to_cause_it() {
        // The rule separating controllability from correlation, at the layer
        // where the distinction first bites.
        let mut affordance = affordance();
        watch(&mut affordance, 200, 0, false);
        assert_eq!(affordance.probability(), Some(1.0));
        assert_eq!(affordance.causal_probability(), None);
        let error = affordance.promotable(0.7).unwrap_err();
        assert!(matches!(error, NotPromotable::NotIntervened { .. }));
        assert!(error.describe().contains("not evidence of being able"));
    }

    #[test]
    fn randomised_evidence_permits_promotion() {
        let mut affordance = affordance();
        watch(&mut affordance, 18, 2, true);
        let probability = affordance.promotable(0.7).expect("promotable");
        assert!((probability - 0.9).abs() < 1e-9);
        assert!(affordance.describe().contains("under intervention"));
    }

    #[test]
    fn an_unreliable_affordance_is_not_promotable() {
        let mut affordance = affordance();
        watch(&mut affordance, 6, 14, true);
        let error = affordance.promotable(0.7).unwrap_err();
        assert!(matches!(error, NotPromotable::TooUnreliable { .. }));
    }

    #[test]
    fn an_affordance_with_one_dominant_failure_is_conditional_not_unreliable() {
        // The honest response is to narrow the precondition, not to promote a
        // procedure that fails for a stateable reason.
        let mut affordance = affordance();
        watch(&mut affordance, 20, 0, true);
        for i in 0..8 {
            affordance.record(&Transition {
                at_ns: i,
                arrived: "elsewhere".into(),
                cost: 1.0,
                latency_ns: 0,
                randomised: true,
                failure: Some(Failure::PreconditionAbsent),
            });
        }
        let error = affordance.promotable(0.5).unwrap_err();
        assert!(matches!(error, NotPromotable::Conditional { .. }));
        assert!(error.describe().contains("narrow it"));
    }

    #[test]
    fn expected_cost_includes_the_misses() {
        let mut affordance = affordance();
        watch(&mut affordance, 10, 10, false);
        assert_eq!(affordance.cost_on_arrival(), Some(1.0));
        assert!((affordance.expected_cost().unwrap() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn an_affordance_that_never_arrives_has_no_expected_cost() {
        let mut affordance = affordance();
        watch(&mut affordance, 0, 20, false);
        assert_eq!(affordance.probability(), Some(0.0));
        assert_eq!(affordance.expected_cost(), None);
    }

    #[test]
    fn a_non_finite_cost_is_dropped() {
        let mut affordance = affordance();
        affordance.record(&Transition {
            at_ns: 0,
            arrived: "concept_9".into(),
            cost: f64::NAN,
            latency_ns: 0,
            randomised: false,
            failure: None,
        });
        assert_eq!(affordance.observations(), 0);
    }

    #[test]
    fn the_key_identifies_the_edge_and_not_the_endpoints_alone() {
        let a = Affordance::new(AffordanceId(1), "s1", "affinity", "s2", 0);
        let b = Affordance::new(AffordanceId(2), "s1", "priority", "s2", 0);
        assert_ne!(
            a.key(),
            b.key(),
            "two actions between the same states are two edges"
        );
    }

    #[test]
    fn an_affordance_round_trips_through_json() {
        let mut affordance = affordance();
        watch(&mut affordance, 10, 5, true);
        let text = serde_json::to_string(&affordance).expect("serialises");
        let back: Affordance = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back, affordance);
    }
}

#[cfg(test)]
mod backfill_tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn an_outcome_discovered_late_does_not_report_certainty() {
        // Without backfilling, one observation and one arrival reads as 100%,
        // and a rare outcome first seen on the hundredth attempt looks like a
        // sure thing.
        let mut rare = Affordance::new(AffordanceId(2), "s1", "affinity", "rare", 0);
        rare.backfill(99);
        watch(&mut rare, 1, 0, false);
        assert_eq!(rare.observations(), 100);
        assert!((rare.probability().unwrap() - 0.01).abs() < 1e-9);
        assert!(!rare.is_edge(0.7));
    }

    #[test]
    fn backfilling_nothing_changes_nothing() {
        let mut affordance = affordance();
        watch(&mut affordance, 20, 0, false);
        let before = affordance.probability();
        affordance.backfill(0);
        assert_eq!(affordance.probability(), before);
    }
}
