//! A reproducible procedure for making the machine become something.
//!
//! # What distinguishes this from a recipe
//!
//! [`corescout_concept::resource::VirtualResource`] is a concept plus a recipe
//! plus a reliability. A [`Capability`] adds the three things that make it
//! executable knowledge rather than a description:
//!
//! | part | why it is needed |
//! |---|---|
//! | precondition | a procedure that only works from somewhere is not "a thing you can do" until you know where from |
//! | failure conditions | learned from attempts that did not work, so the procedure can be declined rather than blindly retried |
//! | composability | a capability that can be the first half of another is worth more than one that cannot |
//!
//! # What it is *not* until proven
//!
//! A capability is a claim: *from here, doing this, I end up there, this often*.
//! Every part of that is measured. [`Capability::reliability`] is `None` until
//! there have been enough attempts, and a procedure whose reliability falls is
//! retired.
//!
//! Being able to describe a procedure is not possessing a capability. The
//! difference is the attempt record, and it is the only thing here that cannot
//! be copied between machines.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// The machine's own name for a capability it mined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CapabilityId(pub u32);

impl fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "capability_{}", self.0)
    }
}

/// One step: an action family, and what it acts on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub family: String,
    pub target_row: Option<usize>,
}

impl Step {
    pub fn new(family: impl Into<String>, target_row: Option<usize>) -> Step {
        Step {
            family: family.into(),
            target_row,
        }
    }
}

/// Why an attempt failed.
///
/// Recorded rather than counted, so a procedure can learn the circumstances in
/// which it does not work. A capability that fails 30% of the time for a known
/// reason is far more useful than one that fails 30% of the time for no reason
/// anyone can state: the first can be declined in advance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    /// The machine was not in the precondition when the attempt began.
    PreconditionAbsent,
    /// A step could not be performed at all.
    StepRefused { family: String },
    /// Every step ran and the machine did not end up in the target.
    DidNotArrive,
    /// It arrived and left again immediately.
    DidNotHold,
}

impl Failure {
    pub fn label(&self) -> String {
        match self {
            Failure::PreconditionAbsent => "precondition absent".into(),
            Failure::StepRefused { family } => format!("{family} was refused"),
            Failure::DidNotArrive => "did not arrive".into(),
            Failure::DidNotHold => "arrived but did not hold".into(),
        }
    }
}

/// What one attempt cost and whether it worked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attempt {
    pub at_ns: u64,
    /// `None` when it worked.
    pub failure: Option<Failure>,
    /// What performing the steps cost, whatever the outcome.
    pub cost: f64,
}

impl Attempt {
    pub fn succeeded(at_ns: u64, cost: f64) -> Attempt {
        Attempt {
            at_ns,
            failure: None,
            cost,
        }
    }

    pub fn failed(at_ns: u64, failure: Failure, cost: f64) -> Attempt {
        Attempt {
            at_ns,
            failure: Some(failure),
            cost,
        }
    }

    pub fn worked(&self) -> bool {
        self.failure.is_none()
    }
}

/// Attempts before a reliability is reported.
///
/// A procedure that worked three times has no reliability. Reporting 100% from
/// three successes is how a lucky sequence becomes a capability.
pub const ENOUGH_ATTEMPTS: u32 = 12;

/// A procedure the machine discovered for making itself become something.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capability {
    pub id: CapabilityId,
    /// The concept this procedure produces, in the machine's own vocabulary.
    pub target: String,
    /// The concept it must start from. `None` means it works from anywhere,
    /// which is a strong claim and is measured like any other.
    pub precondition: Option<String>,
    pub steps: Vec<Step>,
    /// The capabilities this was composed from, when it was.
    pub derived_from: Vec<CapabilityId>,
    attempts: u32,
    successes: u32,
    total_cost: f64,
    success_cost: f64,
    #[serde(with = "corescout_core::serde_util::pairs")]
    failures: BTreeMap<Failure, u32>,
    pub discovered_ns: u64,
    pub retired: Option<String>,
}

impl Capability {
    pub fn new(
        id: CapabilityId,
        target: impl Into<String>,
        precondition: Option<String>,
        steps: Vec<Step>,
        discovered_ns: u64,
    ) -> Capability {
        Capability {
            id,
            target: target.into(),
            precondition,
            steps,
            derived_from: Vec::new(),
            attempts: 0,
            successes: 0,
            total_cost: 0.0,
            success_cost: 0.0,
            failures: BTreeMap::new(),
            discovered_ns,
            retired: None,
        }
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    pub fn successes(&self) -> u32 {
        self.successes
    }

    /// How often the procedure delivers its target.
    ///
    /// `None` until there is enough evidence. The distinction between "unknown"
    /// and "unreliable" is load-bearing: a caller can wait for the first and
    /// must not rely on the second.
    pub fn reliability(&self) -> Option<f64> {
        (self.attempts >= ENOUGH_ATTEMPTS).then(|| self.successes as f64 / self.attempts as f64)
    }

    /// Mean cost of a successful use.
    ///
    /// Successes only, because that is what a caller planning to use it will
    /// pay when it works. [`Capability::expected_cost`] is the number to plan
    /// with, since it includes the failures.
    pub fn cost_when_it_works(&self) -> Option<f64> {
        (self.successes > 0).then(|| self.success_cost / self.successes as f64)
    }

    /// What it costs in expectation to actually arrive, retries included.
    ///
    /// A procedure that is cheap but works a third of the time costs three
    /// times its face value. Planning with the face value is the standard way
    /// to be disappointed by an unreliable capability.
    pub fn expected_cost(&self) -> Option<f64> {
        let reliability = self.reliability()?;
        if reliability <= 0.0 {
            return None;
        }
        let per_attempt = self.total_cost / self.attempts as f64;
        Some(per_attempt / reliability)
    }

    /// Whether it may be relied on.
    pub fn is_usable(&self, min_reliability: f64) -> bool {
        self.retired.is_none()
            && self
                .reliability()
                .is_some_and(|value| value >= min_reliability)
    }

    pub fn is_retired(&self) -> bool {
        self.retired.is_some()
    }

    /// Whether this was built from other capabilities.
    pub fn is_composed(&self) -> bool {
        !self.derived_from.is_empty()
    }

    pub fn record(&mut self, attempt: &Attempt) {
        if !attempt.cost.is_finite() {
            return;
        }
        self.attempts += 1;
        self.total_cost += attempt.cost;
        match &attempt.failure {
            None => {
                self.successes += 1;
                self.success_cost += attempt.cost;
            }
            Some(failure) => {
                *self.failures.entry(failure.clone()).or_insert(0) += 1;
            }
        }
    }

    /// The ways this procedure is known to fail, commonest first.
    pub fn failure_modes(&self) -> Vec<(&Failure, u32)> {
        let mut modes: Vec<(&Failure, u32)> = self.failures.iter().map(|(f, n)| (f, *n)).collect();
        modes.sort_by(|a, b| b.1.cmp(&a.1));
        modes
    }

    /// Whether a known failure mode dominates.
    ///
    /// When one reason accounts for most failures, the procedure is not
    /// unreliable so much as conditional, and the honest response is to narrow
    /// its precondition rather than to keep retrying.
    pub fn dominant_failure(&self) -> Option<(&Failure, f64)> {
        let total: u32 = self.failures.values().sum();
        if total == 0 {
            return None;
        }
        let (failure, count) = self.failure_modes().into_iter().next()?;
        let share = count as f64 / total as f64;
        (share >= 0.6).then_some((failure, share))
    }

    /// Withdraw the capability.
    pub fn retire(&mut self, reason: impl Into<String>) {
        self.retired = Some(reason.into());
    }

    /// Review it, retiring if it has stopped delivering.
    pub fn review(&mut self, min_reliability: f64) -> Option<&str> {
        if self.retired.is_some() {
            return self.retired.as_deref();
        }
        let reliability = self.reliability()?;
        if reliability < min_reliability {
            self.retired = Some(format!(
                "delivered {:.0}% of {} attempts, below the {:.0}% required",
                reliability * 100.0,
                self.attempts,
                min_reliability * 100.0
            ));
        }
        self.retired.as_deref()
    }

    /// What a caller needs to know before relying on it.
    pub fn describe(&self) -> String {
        let steps = self
            .steps
            .iter()
            .map(|s| s.family.as_str())
            .collect::<Vec<_>>()
            .join(" then ");
        let from = match &self.precondition {
            Some(concept) => format!("from {concept}"),
            None => "from anywhere".to_string(),
        };
        let reliability = match self.reliability() {
            Some(value) => format!("{:.0}% of {} attempts", value * 100.0, self.attempts),
            None => format!("unproven, {} attempts so far", self.attempts),
        };
        let mut text = format!(
            "{}: {from}, do [{steps}] to reach {} ({reliability})",
            self.id, self.target
        );
        if let Some(cost) = self.expected_cost() {
            text.push_str(&format!(", expected cost {cost:.2}"));
        }
        if self.is_composed() {
            text.push_str(&format!(
                ", composed from {}",
                self.derived_from
                    .iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(" and ")
            ));
        }
        if let Some((failure, share)) = self.dominant_failure() {
            text.push_str(&format!(
                "; {:.0}% of failures are {}",
                share * 100.0,
                failure.label()
            ));
        }
        if let Some(reason) = &self.retired {
            text.push_str(&format!("  [retired: {reason}]"));
        }
        text
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub(crate) fn capability(id: u32, target: &str, precondition: Option<&str>) -> Capability {
        Capability::new(
            CapabilityId(id),
            target,
            precondition.map(|s| s.to_string()),
            vec![Step::new("affinity", Some(3)), Step::new("priority", None)],
            0,
        )
    }

    pub(crate) fn prove(capability: &mut Capability, successes: u32, failures: u32) {
        for i in 0..successes {
            capability.record(&Attempt::succeeded(i as u64, 1.0));
        }
        for i in 0..failures {
            capability.record(&Attempt::failed(i as u64, Failure::DidNotArrive, 1.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn a_procedure_that_worked_three_times_has_no_reliability() {
        // Reporting 100% from three successes is how a lucky sequence becomes a
        // capability.
        let mut capability = capability(1, "concept_7", None);
        prove(&mut capability, 3, 0);
        assert_eq!(capability.reliability(), None);
        assert!(!capability.is_usable(0.5));
        assert!(capability.describe().contains("unproven"));
    }

    #[test]
    fn enough_attempts_produce_a_reliability() {
        let mut capability = capability(1, "concept_7", None);
        prove(&mut capability, 18, 2);
        assert!((capability.reliability().unwrap() - 0.9).abs() < 1e-9);
        assert!(capability.is_usable(0.8));
        assert!(!capability.is_usable(0.95));
    }

    #[test]
    fn expected_cost_accounts_for_the_retries() {
        // A procedure that is cheap and works half the time costs twice its
        // face value to actually arrive.
        let mut capability = capability(1, "concept_7", None);
        prove(&mut capability, 10, 10);
        assert_eq!(capability.cost_when_it_works(), Some(1.0));
        let expected = capability.expected_cost().expect("measured");
        assert!((expected - 2.0).abs() < 1e-9, "expected cost {expected}");
    }

    #[test]
    fn a_procedure_that_never_works_has_no_expected_cost() {
        let mut capability = capability(1, "concept_7", None);
        prove(&mut capability, 0, 20);
        assert_eq!(capability.reliability(), Some(0.0));
        assert_eq!(capability.expected_cost(), None);
        assert_eq!(capability.cost_when_it_works(), None);
    }

    #[test]
    fn a_dominant_failure_mode_is_surfaced() {
        // Not unreliable so much as conditional: the honest response is to
        // narrow the precondition rather than keep retrying.
        let mut capability = capability(1, "concept_7", None);
        prove(&mut capability, 10, 0);
        for i in 0..9 {
            capability.record(&Attempt::failed(i, Failure::PreconditionAbsent, 1.0));
        }
        capability.record(&Attempt::failed(99, Failure::DidNotHold, 1.0));
        let (failure, share) = capability.dominant_failure().expect("a dominant mode");
        assert_eq!(*failure, Failure::PreconditionAbsent);
        assert!(share > 0.8);
        assert!(capability.describe().contains("precondition absent"));
    }

    #[test]
    fn scattered_failures_have_no_dominant_mode() {
        let mut capability = capability(1, "concept_7", None);
        capability.record(&Attempt::failed(0, Failure::DidNotArrive, 1.0));
        capability.record(&Attempt::failed(1, Failure::DidNotHold, 1.0));
        capability.record(&Attempt::failed(
            2,
            Failure::StepRefused {
                family: "affinity".into(),
            },
            1.0,
        ));
        assert_eq!(capability.dominant_failure(), None);
    }

    #[test]
    fn a_capability_that_stops_delivering_is_retired() {
        let mut capability = capability(1, "concept_7", None);
        prove(&mut capability, 4, 16);
        assert!(capability.review(0.7).is_some());
        assert!(capability.is_retired());
        assert!(
            !capability.is_usable(0.1),
            "a retired capability is not usable"
        );
    }

    #[test]
    fn a_capability_is_not_retired_before_there_is_evidence() {
        let mut capability = capability(1, "concept_7", None);
        prove(&mut capability, 0, 2);
        assert_eq!(capability.review(0.7), None);
        assert!(!capability.is_retired());
    }

    #[test]
    fn a_non_finite_cost_is_dropped_rather_than_poisoning_the_record() {
        let mut capability = capability(1, "concept_7", None);
        capability.record(&Attempt::succeeded(0, f64::NAN));
        assert_eq!(capability.attempts(), 0);
    }

    #[test]
    fn a_description_states_where_it_works_from() {
        // A procedure that only works from somewhere is not "a thing you can
        // do" until the caller knows where from.
        let anywhere = capability(1, "concept_7", None);
        assert!(anywhere.describe().contains("from anywhere"));
        let conditional = capability(2, "concept_7", Some("concept_3"));
        assert!(conditional.describe().contains("from concept_3"));
    }

    #[test]
    fn a_capability_round_trips_through_json() {
        let mut capability = capability(1, "concept_7", Some("concept_3"));
        prove(&mut capability, 10, 3);
        capability.record(&Attempt::failed(1, Failure::PreconditionAbsent, 2.0));
        let text = serde_json::to_string(&capability).expect("serialises");
        let back: Capability = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back, capability);
    }
}
