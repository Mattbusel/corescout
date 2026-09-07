//! What the machine can reliably make itself do, and what it took to get there.
//!
//! # The rule that decides what counts
//!
//! ```text
//! R(A) = the set of states this machine can reliably reach using A
//!
//! a discovered procedure c is capital when
//!     R(A + c) > R(A)                    it reaches something new
//! or  cost(target | A + c) < cost(target | A)    it reaches it cheaper
//! ```
//!
//! Everything else is a redundant procedure. It may be correct, reproducible
//! and elegantly discovered, and it is not an addition to the machine's
//! capability, because the machine could already do that thing at least as well.
//!
//! Enforcing this is the whole point of [`Frontier::admit`]. Without it, a
//! system that mines procedures will accumulate hundreds of them and report a
//! growing "capability set" that does not correspond to a growing set of things
//! it can actually do. The count would rise and the machine would be no more
//! capable, which is the failure mode that would make this whole idea a fiction.
//!
//! # Why cost reduction counts as capital
//!
//! A state reachable only by an expensive, unreliable route is barely reachable.
//! Halving the cost of getting somewhere is a real expansion of what the machine
//! can afford to do, in the same way that a cheaper machine tool expands a
//! workshop even when it makes nothing the workshop could not already make.
//!
//! # What is not modelled
//!
//! Contention. Two capabilities may each be reliable alone and impossible
//! together, because one holds a resource the other needs. Nothing here notices,
//! and a frontier is therefore an upper bound on what the machine can do *at
//! once*.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::capability::{Capability, CapabilityId, Step};

/// How a target is currently reached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    /// A single action the machine was given.
    Primitive(String),
    /// A procedure it discovered.
    Discovered(CapabilityId),
}

impl Via {
    pub fn describe(&self) -> String {
        match self {
            Via::Primitive(family) => format!("the primitive action {family}"),
            Via::Discovered(id) => id.to_string(),
        }
    }

    pub fn is_discovered(&self) -> bool {
        matches!(self, Via::Discovered(_))
    }
}

/// The best known way to reach one target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reach {
    pub via: Via,
    pub reliability: f64,
    /// Expected cost to actually arrive, retries included.
    pub expected_cost: f64,
}

/// What admitting a procedure did to the frontier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Admission {
    /// The machine can now reach something it could not reach before.
    ///
    /// `R(A + c) > R(A)`. The strong form of capital.
    NewlyReachable { target: String },
    /// It could already get there, and now it can get there for less.
    Cheaper { target: String, was: f64, now: f64 },
    /// It could already get there just as cheaply. Not an addition.
    Redundant { target: String, existing: String },
    /// Not enough attempts to say.
    Unproven { attempts: u32 },
    /// Proven, and not reliable enough to count as reaching anything.
    Unreliable { reliability: f64, needed: f64 },
}

impl Admission {
    /// Whether this procedure expanded what the machine can do.
    pub fn is_capital(&self) -> bool {
        matches!(
            self,
            Admission::NewlyReachable { .. } | Admission::Cheaper { .. }
        )
    }

    pub fn describe(&self) -> String {
        match self {
            Admission::NewlyReachable { target } => {
                format!("{target} is now reachable, and was not before")
            }
            Admission::Cheaper { target, was, now } => {
                format!("{target} now costs {now:.2} to reach, down from {was:.2}")
            }
            Admission::Redundant { target, existing } => {
                format!("{target} was already reachable via {existing}, at least as cheaply")
            }
            Admission::Unproven { attempts } => {
                format!("only {attempts} attempts; not enough to claim anything")
            }
            Admission::Unreliable {
                reliability,
                needed,
            } => format!(
                "delivers {:.0}% of the time, below the {:.0}% needed to count as reaching it",
                reliability * 100.0,
                needed * 100.0
            ),
        }
    }
}

/// The set of states the machine can reliably reach, and how.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frontier {
    /// Reliability below which a route does not count as reaching anything.
    threshold: f64,
    /// A cost must fall by at least this fraction to count as an improvement.
    ///
    /// Without it, noise in the cost estimate would let the same procedure be
    /// re-admitted forever as marginally cheaper.
    material_saving: f64,
    #[serde(with = "corescout_core::serde_util::pairs")]
    reachable: BTreeMap<String, Reach>,
}

impl Default for Frontier {
    fn default() -> Self {
        Frontier::new(0.7, 0.15)
    }
}

impl Frontier {
    pub fn new(threshold: f64, material_saving: f64) -> Frontier {
        Frontier {
            threshold: threshold.clamp(0.0, 1.0),
            material_saving: material_saving.max(0.0),
            reachable: BTreeMap::new(),
        }
    }

    pub fn threshold(&self) -> f64 {
        self.threshold
    }

    /// `|R(A)|`: how many distinct states the machine can reliably reach.
    pub fn len(&self) -> usize {
        self.reachable.len()
    }

    pub fn is_empty(&self) -> bool {
        self.reachable.is_empty()
    }

    pub fn targets(&self) -> impl Iterator<Item = &String> {
        self.reachable.keys()
    }

    pub fn reach(&self, target: &str) -> Option<&Reach> {
        self.reachable.get(target)
    }

    pub fn can_reach(&self, target: &str) -> bool {
        self.reachable.contains_key(target)
    }

    /// Targets reachable only because of something the machine discovered.
    ///
    /// The interesting subset: what this machine can do that a fresh one with
    /// the same silicon cannot.
    pub fn discovered_targets(&self) -> Vec<&String> {
        self.reachable
            .iter()
            .filter(|(_, reach)| reach.via.is_discovered())
            .map(|(target, _)| target)
            .collect()
    }

    /// Record what a primitive action can do, measured.
    ///
    /// Seeds the frontier with the machine's given abilities, so that a
    /// discovered procedure is measured against them rather than against
    /// nothing. A system that compared its discoveries to an empty frontier
    /// would call everything an expansion.
    pub fn observe_primitive(
        &mut self,
        target: impl Into<String>,
        family: impl Into<String>,
        reliability: f64,
        expected_cost: f64,
    ) -> Admission {
        let target = target.into();
        if reliability < self.threshold {
            return Admission::Unreliable {
                reliability,
                needed: self.threshold,
            };
        }
        self.consider(
            target,
            Via::Primitive(family.into()),
            reliability,
            expected_cost,
        )
    }

    /// Offer a discovered procedure to the frontier.
    pub fn admit(&mut self, capability: &Capability) -> Admission {
        let Some(reliability) = capability.reliability() else {
            return Admission::Unproven {
                attempts: capability.attempts(),
            };
        };
        if capability.is_retired() || reliability < self.threshold {
            return Admission::Unreliable {
                reliability,
                needed: self.threshold,
            };
        }
        let Some(expected_cost) = capability.expected_cost() else {
            return Admission::Unreliable {
                reliability,
                needed: self.threshold,
            };
        };
        self.consider(
            capability.target.clone(),
            Via::Discovered(capability.id),
            reliability,
            expected_cost,
        )
    }

    fn consider(
        &mut self,
        target: String,
        via: Via,
        reliability: f64,
        expected_cost: f64,
    ) -> Admission {
        match self.reachable.get(&target) {
            None => {
                self.reachable.insert(
                    target.clone(),
                    Reach {
                        via,
                        reliability,
                        expected_cost,
                    },
                );
                Admission::NewlyReachable { target }
            }
            Some(existing) => {
                let saving = (existing.expected_cost - expected_cost) / existing.expected_cost;
                if saving > self.material_saving {
                    let was = existing.expected_cost;
                    self.reachable.insert(
                        target.clone(),
                        Reach {
                            via,
                            reliability,
                            expected_cost,
                        },
                    );
                    Admission::Cheaper {
                        target,
                        was,
                        now: expected_cost,
                    }
                } else {
                    Admission::Redundant {
                        existing: existing.via.describe(),
                        target,
                    }
                }
            }
        }
    }

    /// Drop a route that is no longer available.
    ///
    /// The frontier shrinks. A machine whose capability set can only grow is
    /// reporting a history rather than a capability.
    pub fn withdraw(&mut self, id: CapabilityId) -> Vec<String> {
        let lost: Vec<String> = self
            .reachable
            .iter()
            .filter(|(_, reach)| reach.via == Via::Discovered(id))
            .map(|(target, _)| target.clone())
            .collect();
        for target in &lost {
            self.reachable.remove(target);
        }
        lost
    }

    pub fn render(&self) -> String {
        if self.reachable.is_empty() {
            return "nothing is reliably reachable\n".into();
        }
        let discovered = self.discovered_targets().len();
        let mut out = format!(
            "{} states reliably reachable, {} of them only because of something \
             the machine discovered\n",
            self.reachable.len(),
            discovered
        );
        for (target, reach) in &self.reachable {
            out.push_str(&format!(
                "  {target} via {} ({:.0}%, expected cost {:.2})\n",
                reach.via.describe(),
                reach.reliability * 100.0,
                reach.expected_cost
            ));
        }
        out
    }
}

/// Compose two capabilities into one.
///
/// Valid when the first ends where the second must begin. The result is a
/// *candidate*: its steps are the concatenation and its reliability is unknown,
/// because it has never been run.
///
/// # The reliability of a composition must be measured
///
/// The obvious thing is to multiply: `r1 * r2`. That assumes the two are
/// independent, and the interesting compositions are exactly the ones where
/// they are not. The first procedure may leave the machine in a corner of the
/// target concept from which the second works better than usual, or worse.
///
/// So [`predicted_reliability`] exists, is clearly named a prediction, and is
/// never written into the composite. It is there to be compared against what
/// actually happens, which makes every composition a testable claim rather than
/// an arithmetic assertion.
pub fn compose(
    id: CapabilityId,
    first: &Capability,
    second: &Capability,
    now_ns: u64,
) -> Option<Capability> {
    // The second must start where the first arrives. A second that works from
    // anywhere composes with anything.
    match &second.precondition {
        Some(required) if *required != first.target => return None,
        _ => {}
    }
    if first.is_retired() || second.is_retired() {
        return None;
    }
    // A procedure that ends where it began is not a composition, it is a loop.
    if first.target == second.target {
        return None;
    }

    let mut steps: Vec<Step> = first.steps.clone();
    steps.extend(second.steps.iter().cloned());
    let mut composed = Capability::new(
        id,
        second.target.clone(),
        first.precondition.clone(),
        steps,
        now_ns,
    );
    composed.derived_from = vec![first.id, second.id];
    Some(composed)
}

/// What the composition would achieve if its parts were independent.
///
/// A prediction to be tested, never a value to be recorded. See [`compose`].
pub fn predicted_reliability(first: &Capability, second: &Capability) -> Option<f64> {
    Some(first.reliability()? * second.reliability()?)
}

/// How far a composition's measured reliability departed from the prediction.
///
/// The interesting number. A large positive value means the parts help each
/// other, which is a discovery about the machine rather than about the
/// procedures.
pub fn composition_surprise(composed: &Capability, predicted: f64) -> Option<f64> {
    Some(composed.reliability()? - predicted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::fixtures::{capability, prove};

    fn proven(id: u32, target: &str, precondition: Option<&str>, ok: u32, bad: u32) -> Capability {
        let mut capability = capability(id, target, precondition);
        prove(&mut capability, ok, bad);
        capability
    }

    #[test]
    fn an_empty_frontier_can_reach_nothing() {
        let frontier = Frontier::default();
        assert_eq!(frontier.len(), 0);
        assert!(frontier.render().contains("nothing is reliably reachable"));
    }

    #[test]
    fn a_procedure_reaching_something_new_is_capital() {
        // R(A + c) > R(A). The strong form.
        let mut frontier = Frontier::default();
        let capability = proven(1, "concept_7", None, 18, 2);
        let admission = frontier.admit(&capability);
        assert!(admission.is_capital());
        assert!(matches!(admission, Admission::NewlyReachable { .. }));
        assert_eq!(frontier.len(), 1);
        assert_eq!(frontier.discovered_targets().len(), 1);
    }

    #[test]
    fn a_procedure_duplicating_a_primitive_is_not_capital() {
        // The rule that stops a growing count of procedures being reported as a
        // growing set of things the machine can do.
        let mut frontier = Frontier::default();
        frontier.observe_primitive("concept_7", "affinity", 0.95, 1.0);
        let capability = proven(1, "concept_7", None, 18, 2);
        let admission = frontier.admit(&capability);
        assert!(!admission.is_capital(), "{}", admission.describe());
        assert!(matches!(admission, Admission::Redundant { .. }));
        assert_eq!(frontier.len(), 1);
        assert!(frontier.discovered_targets().is_empty());
    }

    #[test]
    fn reaching_the_same_place_materially_cheaper_is_capital() {
        // A cheaper route to somewhere expands what the machine can afford to
        // do, in the way a cheaper machine tool expands a workshop.
        let mut frontier = Frontier::default();
        frontier.observe_primitive("concept_7", "affinity", 0.95, 10.0);
        // 20 successes at cost 1: expected cost 1.0 against the primitive's 10.
        let capability = proven(1, "concept_7", None, 20, 0);
        let admission = frontier.admit(&capability);
        assert!(admission.is_capital());
        match admission {
            Admission::Cheaper { was, now, .. } => {
                assert!((was - 10.0).abs() < 1e-9);
                assert!(now < 2.0);
            }
            other => panic!("expected a saving, got {other:?}"),
        }
    }

    #[test]
    fn a_marginal_saving_is_not_capital() {
        // Otherwise noise in the cost estimate re-admits the same procedure
        // forever as slightly cheaper.
        let mut frontier = Frontier::default();
        frontier.observe_primitive("concept_7", "affinity", 0.95, 1.02);
        let capability = proven(1, "concept_7", None, 20, 0);
        assert!(!frontier.admit(&capability).is_capital());
    }

    #[test]
    fn an_unproven_procedure_is_not_admitted() {
        let mut frontier = Frontier::default();
        let capability = proven(1, "concept_7", None, 3, 0);
        assert!(matches!(
            frontier.admit(&capability),
            Admission::Unproven { .. }
        ));
        assert_eq!(frontier.len(), 0);
    }

    #[test]
    fn an_unreliable_procedure_does_not_count_as_reaching_anything() {
        let mut frontier = Frontier::default();
        let capability = proven(1, "concept_7", None, 8, 12);
        let admission = frontier.admit(&capability);
        assert!(matches!(admission, Admission::Unreliable { .. }));
        assert!(admission.describe().contains("below"));
        assert_eq!(frontier.len(), 0);
    }

    #[test]
    fn a_retired_procedure_is_not_admitted() {
        let mut frontier = Frontier::default();
        let mut capability = proven(1, "concept_7", None, 20, 0);
        capability.retire("superseded");
        assert!(!frontier.admit(&capability).is_capital());
    }

    #[test]
    fn withdrawing_a_capability_shrinks_the_frontier() {
        // A machine whose capability set can only grow is reporting a history
        // rather than a capability.
        let mut frontier = Frontier::default();
        let capability = proven(1, "concept_7", None, 18, 2);
        frontier.admit(&capability);
        assert_eq!(frontier.len(), 1);
        let lost = frontier.withdraw(CapabilityId(1));
        assert_eq!(lost, vec!["concept_7".to_string()]);
        assert_eq!(frontier.len(), 0);
        assert!(!frontier.can_reach("concept_7"));
    }

    #[test]
    fn withdrawing_does_not_remove_a_primitive_route() {
        let mut frontier = Frontier::default();
        frontier.observe_primitive("concept_7", "affinity", 0.95, 1.0);
        assert!(frontier.withdraw(CapabilityId(1)).is_empty());
        assert!(frontier.can_reach("concept_7"));
    }

    #[test]
    fn two_capabilities_compose_when_one_ends_where_the_other_starts() {
        let first = proven(1, "concept_3", None, 18, 2);
        let second = proven(2, "concept_9", Some("concept_3"), 18, 2);
        let composed = compose(CapabilityId(3), &first, &second, 0).expect("composable");
        assert_eq!(composed.target, "concept_9");
        assert_eq!(composed.precondition, None);
        assert_eq!(composed.steps.len(), first.steps.len() + second.steps.len());
        assert_eq!(
            composed.derived_from,
            vec![CapabilityId(1), CapabilityId(2)]
        );
        assert!(composed.is_composed());
    }

    #[test]
    fn capabilities_that_do_not_meet_do_not_compose() {
        let first = proven(1, "concept_3", None, 18, 2);
        let second = proven(2, "concept_9", Some("concept_5"), 18, 2);
        assert!(compose(CapabilityId(3), &first, &second, 0).is_none());
    }

    #[test]
    fn a_capability_that_works_from_anywhere_composes_with_anything() {
        let first = proven(1, "concept_3", None, 18, 2);
        let second = proven(2, "concept_9", None, 18, 2);
        assert!(compose(CapabilityId(3), &first, &second, 0).is_some());
    }

    #[test]
    fn a_composition_that_ends_where_it_began_is_a_loop_not_a_capability() {
        let first = proven(1, "concept_3", None, 18, 2);
        let second = proven(2, "concept_3", Some("concept_3"), 18, 2);
        assert!(compose(CapabilityId(3), &first, &second, 0).is_none());
    }

    #[test]
    fn a_composition_starts_with_no_reliability_of_its_own() {
        // Multiplying the parts would assume independence, and the interesting
        // compositions are the ones where that is false.
        let first = proven(1, "concept_3", None, 20, 0);
        let second = proven(2, "concept_9", Some("concept_3"), 20, 0);
        let composed = compose(CapabilityId(3), &first, &second, 0).expect("composable");
        assert_eq!(
            composed.reliability(),
            None,
            "a composition that has never been run cannot have a reliability"
        );
        assert_eq!(predicted_reliability(&first, &second), Some(1.0));
    }

    #[test]
    fn the_surprise_of_a_composition_is_measurable() {
        // The number worth having: how far the parts departed from independence
        // is a discovery about the machine.
        let first = proven(1, "concept_3", None, 10, 10);
        let second = proven(2, "concept_9", Some("concept_3"), 10, 10);
        let predicted = predicted_reliability(&first, &second).expect("both proven");
        assert!((predicted - 0.25).abs() < 1e-9);

        let mut composed = compose(CapabilityId(3), &first, &second, 0).expect("composable");
        // In fact it works far better than independence would suggest.
        prove(&mut composed, 18, 2);
        let surprise = composition_surprise(&composed, predicted).expect("measured");
        assert!(surprise > 0.5, "surprise was {surprise}");
    }

    #[test]
    fn a_frontier_round_trips_through_json() {
        let mut frontier = Frontier::default();
        frontier.observe_primitive("concept_1", "affinity", 0.9, 2.0);
        frontier.admit(&proven(1, "concept_7", None, 18, 2));
        let text = serde_json::to_string(&frontier).expect("serialises");
        let back: Frontier = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back, frontier);
    }
}
