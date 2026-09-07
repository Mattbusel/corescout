//! Virtual hardware the machine discovered rather than the vendor shipped.
//!
//! # What a virtual resource is
//!
//! A [`VirtualResource`] is a concept plus a **recipe** for producing it plus a
//! measured record of how often that recipe works.
//!
//! ```text
//! concept_31   "a configuration I recognise"
//!    + recipe  "these actions put me into it"
//!    + record  "which worked 84% of the time, over 51 attempts"
//!    = V1      "a thing an application can ask to run on"
//! ```
//!
//! Nobody manufactured `V1`. There is no such component on the die. It is a
//! recurring physical configuration that the machine found, learned to
//! reproduce, and can now offer as a place to put work.
//!
//! # Why this is not just a named CPU set
//!
//! A CPU set is a set of CPUs. A virtual resource is a set of *conditions*,
//! which may involve what other components are doing, and which the runtime has
//! to actively bring about and hold. Two requests for `V1` at the same time may
//! be impossible even though the machine has plenty of idle CPUs, because `V1`
//! is partly defined by other things being quiet.
//!
//! # Reliability is the whole product
//!
//! An application asking to run on `V1` is asking for a property, and the only
//! honest answer involves how often the property is actually delivered. Every
//! attempt is recorded, successes and failures alike, and
//! [`VirtualResource::reliability`] is the number a caller should be shown.
//!
//! A resource whose reliability falls below its promise is **withdrawn**. This
//! is the part most likely to be got wrong by wishful implementation: a virtual
//! resource that is offered but rarely achieved is worse than no resource at
//! all, because a caller has arranged its work around a guarantee that is not
//! being kept.

use serde::{Deserialize, Serialize};

use crate::concept::ConceptId;

/// One step of a recipe: an action family and the target it acts on.
///
/// Deliberately not an `ActionKind`. This crate does not depend on `agency`,
/// because a recipe is a *claim about what would work*, and turning it into
/// syscalls is the job of the layer that is allowed to touch the machine. The
/// separation also means a recipe can be recorded, replayed and reasoned about
/// on a machine that cannot execute it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// The action family, e.g. `affinity`.
    pub family: String,
    /// The mirror row the action targets, where it has one.
    pub target_row: Option<usize>,
    /// A description the audit trail can use.
    pub description: String,
}

/// How to bring a concept about.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Recipe {
    pub steps: Vec<Step>,
}

impl Recipe {
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn len(&self) -> usize {
        self.steps.len()
    }

    pub fn describe(&self) -> String {
        if self.steps.is_empty() {
            return "wait for it to occur".into();
        }
        self.steps
            .iter()
            .map(|s| s.description.clone())
            .collect::<Vec<_>>()
            .join(", then ")
    }
}

/// An attempt to produce a resource, and what came of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attempt {
    pub at_ns: u64,
    /// Whether the machine ended up in the concept.
    pub achieved: bool,
    /// How long it took, when it worked.
    pub latency_ns: Option<u64>,
}

/// A computational unit the machine invented.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VirtualResource {
    /// The machine's own name for it, `v1`, `v2`.
    pub name: String,
    /// The concept that defines what it *is*.
    pub concept: ConceptId,
    /// The actions believed to bring it about.
    pub recipe: Recipe,
    /// What it is for, in the machine's own terms: the objective it was found
    /// to be good for.
    pub good_for: String,
    attempts: u32,
    achieved: u32,
    total_latency_ns: u64,
    /// Reliability promised to callers when it was published.
    pub promised: f64,
    pub published_ns: u64,
    pub withdrawn: Option<String>,
}

impl VirtualResource {
    /// How often the recipe actually delivers the concept.
    ///
    /// `None` until there is enough evidence to say. A resource with three
    /// attempts has no reliability, and reporting 100% from three successes is
    /// the exact wishful implementation this type exists to prevent.
    pub fn reliability(&self) -> Option<f64> {
        const ENOUGH: u32 = 10;
        (self.attempts >= ENOUGH).then(|| self.achieved as f64 / self.attempts as f64)
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Mean time to bring the resource about, over successful attempts.
    pub fn mean_latency_ns(&self) -> Option<f64> {
        (self.achieved > 0).then(|| self.total_latency_ns as f64 / self.achieved as f64)
    }

    pub fn is_available(&self) -> bool {
        self.withdrawn.is_none()
    }

    /// Record an attempt.
    pub fn record(&mut self, attempt: Attempt) {
        self.attempts += 1;
        if attempt.achieved {
            self.achieved += 1;
            self.total_latency_ns += attempt.latency_ns.unwrap_or(0);
        }
    }

    /// Withdraw the resource if it is no longer keeping its promise.
    ///
    /// Returns the reason when it withdraws. A resource that is offered but
    /// rarely achieved is worse than no resource: the caller has arranged its
    /// work around a guarantee nobody is keeping.
    pub fn review(&mut self) -> Option<&str> {
        if self.withdrawn.is_some() {
            return self.withdrawn.as_deref();
        }
        let reliability = self.reliability()?;
        if reliability < self.promised {
            self.withdrawn = Some(format!(
                "delivered {:.0}% of {} attempts, below the {:.0}% promised",
                reliability * 100.0,
                self.attempts,
                self.promised * 100.0
            ));
        }
        self.withdrawn.as_deref()
    }

    /// What a caller should be told before asking to run here.
    pub fn offer(&self) -> String {
        let reliability = match self.reliability() {
            Some(value) => format!(
                "{:.0}% reliable over {} attempts",
                value * 100.0,
                self.attempts
            ),
            // The honest answer for a new resource.
            None => format!(
                "reliability unknown: only {} attempts so far, not enough to say",
                self.attempts
            ),
        };
        let latency = match self.mean_latency_ns() {
            Some(ns) => format!(", takes {:.1} ms to enter", ns / 1e6),
            None => String::new(),
        };
        format!(
            "{} ({}): good for {}, {reliability}{latency}. Recipe: {}",
            self.name,
            self.concept,
            self.good_for,
            self.recipe.describe()
        )
    }
}

/// The resources this machine offers.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Catalogue {
    resources: Vec<VirtualResource>,
    next: u32,
}

impl Catalogue {
    pub fn new() -> Catalogue {
        Catalogue::default()
    }

    pub fn len(&self) -> usize {
        self.available().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn all(&self) -> &[VirtualResource] {
        &self.resources
    }

    pub fn available(&self) -> Vec<&VirtualResource> {
        self.resources.iter().filter(|r| r.is_available()).collect()
    }

    pub fn get(&self, name: &str) -> Option<&VirtualResource> {
        self.resources.iter().find(|r| r.name == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut VirtualResource> {
        self.resources.iter_mut().find(|r| r.name == name)
    }

    /// Declare a new virtual resource.
    ///
    /// `promised` is the reliability the resource commits to. Committing to
    /// something is what makes it withdrawable later, and a resource that
    /// promises nothing cannot fail to keep its promise, which would make the
    /// whole arrangement decorative.
    pub fn declare(
        &mut self,
        concept: ConceptId,
        recipe: Recipe,
        good_for: impl Into<String>,
        promised: f64,
        now_ns: u64,
    ) -> Option<String> {
        if !(0.0..=1.0).contains(&promised) || !promised.is_finite() {
            return None;
        }
        // A resource nobody knows how to produce is not a resource. It is a
        // concept, and it already has a name.
        if recipe.is_empty() {
            return None;
        }
        self.next += 1;
        let name = format!("v{}", self.next);
        self.resources.push(VirtualResource {
            name: name.clone(),
            concept,
            recipe,
            good_for: good_for.into(),
            attempts: 0,
            achieved: 0,
            total_latency_ns: 0,
            promised,
            published_ns: now_ns,
            withdrawn: None,
        });
        Some(name)
    }

    /// Record an attempt against a resource.
    pub fn record(&mut self, name: &str, attempt: Attempt) -> bool {
        match self.get_mut(name) {
            Some(resource) => {
                resource.record(attempt);
                true
            }
            None => false,
        }
    }

    /// Review every resource, withdrawing those that stopped delivering.
    pub fn review(&mut self) -> Vec<String> {
        let mut withdrawn = Vec::new();
        for resource in &mut self.resources {
            if resource.is_available() && resource.review().is_some() {
                withdrawn.push(resource.name.clone());
            }
        }
        withdrawn
    }

    /// The best available resource for an objective.
    ///
    /// Ranked by reliability. A resource with unknown reliability is never
    /// preferred to one with a measured record, because "we have not checked" is
    /// not the same as "it works".
    pub fn best_for(&self, objective: &str) -> Option<&VirtualResource> {
        self.available()
            .into_iter()
            .filter(|r| r.good_for == objective)
            .max_by(|a, b| {
                a.reliability()
                    .unwrap_or(-1.0)
                    .total_cmp(&b.reliability().unwrap_or(-1.0))
            })
    }

    /// What this machine offers, in words.
    pub fn render(&self) -> String {
        let available = self.available();
        let withdrawn: Vec<&VirtualResource> = self
            .resources
            .iter()
            .filter(|r| !r.is_available())
            .collect();

        // Never return early on an empty available list. A catalogue whose
        // resources have all been withdrawn must say so: "none available" and
        // "all of them stopped working" are different facts, and the second is
        // the one a caller planning around a guarantee needs to see.
        let mut out = if available.is_empty() {
            "no virtual resources are available: the machine has not found a configuration \
             it can both recognise and reproduce\n"
                .to_string()
        } else {
            let mut out = format!("{} virtual resources:\n", available.len());
            for resource in available {
                out.push_str(&format!("  {}\n", resource.offer()));
            }
            out
        };
        if !withdrawn.is_empty() {
            out.push_str("\nwithdrawn:\n");
            for resource in withdrawn {
                out.push_str(&format!(
                    "  {}: {}\n",
                    resource.name,
                    resource.withdrawn.as_deref().unwrap_or("")
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe() -> Recipe {
        Recipe {
            steps: vec![Step {
                family: "affinity".into(),
                target_row: Some(3),
                description: "move to the entity at row 3".into(),
            }],
        }
    }

    fn catalogue() -> Catalogue {
        let mut catalogue = Catalogue::new();
        catalogue
            .declare(ConceptId(31), recipe(), "jitter", 0.8, 0)
            .expect("declared");
        catalogue
    }

    #[test]
    fn a_resource_nobody_knows_how_to_produce_is_not_declared() {
        // That is a concept, and it already has a name.
        let mut catalogue = Catalogue::new();
        assert_eq!(
            catalogue.declare(ConceptId(1), Recipe::default(), "jitter", 0.8, 0),
            None
        );
        assert!(catalogue.is_empty());
    }

    #[test]
    fn a_resource_must_promise_something_meaningful() {
        let mut catalogue = Catalogue::new();
        for bad in [-0.1, 1.5, f64::NAN] {
            assert_eq!(
                catalogue.declare(ConceptId(1), recipe(), "jitter", bad, 0),
                None
            );
        }
    }

    #[test]
    fn a_declared_resource_gets_the_machines_own_name() {
        let catalogue = catalogue();
        assert_eq!(catalogue.available()[0].name, "v1");
        assert_eq!(catalogue.available()[0].concept, ConceptId(31));
    }

    #[test]
    fn reliability_is_unknown_until_there_is_enough_evidence() {
        // Three successes is not 100% reliability, and reporting it as such is
        // the wishful implementation this type exists to prevent.
        let mut catalogue = catalogue();
        for _ in 0..3 {
            catalogue.record(
                "v1",
                Attempt {
                    at_ns: 0,
                    achieved: true,
                    latency_ns: Some(1_000_000),
                },
            );
        }
        let resource = catalogue.get("v1").expect("present");
        assert_eq!(resource.reliability(), None);
        assert!(resource.offer().contains("not enough to say"));
    }

    #[test]
    fn reliability_is_reported_once_there_is_evidence() {
        let mut catalogue = catalogue();
        for i in 0..20 {
            catalogue.record(
                "v1",
                Attempt {
                    at_ns: i,
                    achieved: i % 10 != 0,
                    latency_ns: Some(2_000_000),
                },
            );
        }
        let resource = catalogue.get("v1").expect("present");
        assert!((resource.reliability().unwrap() - 0.9).abs() < 1e-9);
        assert!(resource.offer().contains("90% reliable"));
        assert!(resource.mean_latency_ns().is_some());
    }

    #[test]
    fn a_resource_that_stops_keeping_its_promise_is_withdrawn() {
        // The most important behaviour here: an unkept guarantee is worse than
        // no guarantee, because callers have arranged work around it.
        let mut catalogue = catalogue();
        for i in 0..20 {
            catalogue.record(
                "v1",
                Attempt {
                    at_ns: i,
                    achieved: i % 2 == 0,
                    latency_ns: None,
                },
            );
        }
        let withdrawn = catalogue.review();
        assert_eq!(withdrawn, vec!["v1".to_string()]);
        assert!(!catalogue.get("v1").unwrap().is_available());
        assert!(catalogue.is_empty());
        assert!(catalogue.render().contains("withdrawn:"));
    }

    #[test]
    fn a_resource_keeping_its_promise_is_not_withdrawn() {
        let mut catalogue = catalogue();
        for i in 0..20 {
            catalogue.record(
                "v1",
                Attempt {
                    at_ns: i,
                    achieved: i % 10 != 0,
                    latency_ns: None,
                },
            );
        }
        assert!(catalogue.review().is_empty());
        assert!(catalogue.get("v1").unwrap().is_available());
    }

    #[test]
    fn a_resource_is_not_withdrawn_before_there_is_evidence() {
        // Two early failures must not kill a resource that has not been tried
        // enough to have a reliability at all.
        let mut catalogue = catalogue();
        for i in 0..2 {
            catalogue.record(
                "v1",
                Attempt {
                    at_ns: i,
                    achieved: false,
                    latency_ns: None,
                },
            );
        }
        assert!(catalogue.review().is_empty());
        assert!(catalogue.get("v1").unwrap().is_available());
    }

    #[test]
    fn an_unmeasured_resource_is_never_preferred_to_a_measured_one() {
        // "We have not checked" is not the same as "it works".
        let mut catalogue = catalogue();
        catalogue
            .declare(ConceptId(32), recipe(), "jitter", 0.5, 0)
            .expect("declared");
        for i in 0..20 {
            catalogue.record(
                "v1",
                Attempt {
                    at_ns: i,
                    achieved: true,
                    latency_ns: None,
                },
            );
        }
        assert_eq!(catalogue.best_for("jitter").unwrap().name, "v1");
    }

    #[test]
    fn a_withdrawn_resource_is_never_offered() {
        let mut catalogue = catalogue();
        for i in 0..20 {
            catalogue.record(
                "v1",
                Attempt {
                    at_ns: i,
                    achieved: false,
                    latency_ns: None,
                },
            );
        }
        catalogue.review();
        assert!(catalogue.best_for("jitter").is_none());
    }

    #[test]
    fn nothing_is_offered_for_an_objective_no_resource_serves() {
        assert!(catalogue().best_for("energy").is_none());
    }

    #[test]
    fn an_empty_catalogue_says_why() {
        let catalogue = Catalogue::new();
        assert!(catalogue.render().contains("both recognise and reproduce"));
    }

    #[test]
    fn recording_against_an_unknown_resource_does_nothing() {
        let mut catalogue = catalogue();
        assert!(!catalogue.record(
            "v99",
            Attempt {
                at_ns: 0,
                achieved: true,
                latency_ns: None
            }
        ));
    }

    #[test]
    fn an_offer_names_the_recipe_so_a_caller_can_judge_it() {
        let catalogue = catalogue();
        let offer = catalogue.get("v1").unwrap().offer();
        assert!(offer.contains("Recipe:"));
        assert!(offer.contains("row 3"));
        assert!(offer.contains("concept_31"));
    }

    #[test]
    fn a_catalogue_round_trips_through_json() {
        let mut catalogue = catalogue();
        catalogue.record(
            "v1",
            Attempt {
                at_ns: 1,
                achieved: true,
                latency_ns: Some(5),
            },
        );
        let text = serde_json::to_string(&catalogue).expect("serialises");
        let back: Catalogue = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back, catalogue);
    }
}
