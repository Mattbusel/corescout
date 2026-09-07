//! What the system can truthfully say about itself.
//!
//! # The rule this module exists to enforce
//!
//! Every proposition here is derived from something measured, carries the
//! evidence it was derived from, and is phrased as a claim about observations
//! rather than a claim about experience. A [`Proposition`] cannot be
//! constructed without a [`Ground`] naming what supports it.
//!
//! The system does not say "I am conscious", "I am alive", "I am aware" or "I
//! am AGI". It does not say them because they are not derivable from anything
//! in the mirror, not because they have been filtered out of a list of things
//! it wanted to say. There is no filter. There is no generator of unsupported
//! sentences to filter.
//!
//! What it can say is of this kind:
//!
//! ```text
//! I distinguish 7 recurring states of myself.
//! I predict my next state better than assuming no change, by 34%.
//! 12 of my 96 observable variables move together as one group.
//! When I change my own placement, 3 variables respond within 200 ms.
//! I cannot observe my own power draw on this machine: permission denied.
//! ```
//!
//! Each of those is checkable against a recording. That is the entire standard
//! being applied: **a proposition is admissible if a person with the recording
//! could confirm or refute it.**
//!
//! # Why "I" at all
//!
//! The first person here is a naming convention for the entity set the boundary
//! inference selected, not a claim about a subject. [`Proposition::machine`]
//! gives the same content without it, and that form is the canonical one; the
//! prose form exists because these propositions are for people.

use serde::{Deserialize, Serialize};

use corescout_mirror::MirrorSnapshot;
use corescout_represent::latent::LatentCatalogue;

use crate::boundary::SelfCandidate;

/// What a proposition rests on.
///
/// This is the part that makes a proposition checkable. A claim whose ground
/// cannot be pointed at in a recording is not admissible, and there is no
/// variant here for "inferred generally" or "apparent".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ground {
    /// Counted directly off the current reflection.
    Reflection { sequence: u64 },
    /// Measured across a span of remembered reflections.
    History { frames: usize },
    /// Derived from the discovered latent-state catalogue.
    Catalogue { observations: u64 },
    /// Measured by scoring predictions against what then happened.
    Predictions { scored: u64 },
    /// Derived from actions taken and their observed consequences.
    Actions { taken: u64 },
    /// The mirror reports the fact as unavailable, with this reason.
    Absence { reason: String },
}

impl Ground {
    /// How many independent observations back this.
    ///
    /// Used to sort propositions: a claim from six frames should not be
    /// presented alongside one from sixty thousand as though they were equally
    /// settled.
    pub fn weight(&self) -> u64 {
        match self {
            Ground::Reflection { .. } => 1,
            Ground::History { frames } => *frames as u64,
            Ground::Catalogue { observations } => *observations,
            Ground::Predictions { scored } => *scored,
            Ground::Actions { taken } => *taken,
            // An absence is fully established by a single unambiguous report.
            Ground::Absence { .. } => 1,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Ground::Reflection { sequence } => format!("reflection {sequence}"),
            Ground::History { frames } => format!("{frames} remembered reflections"),
            Ground::Catalogue { observations } => format!("{observations} state observations"),
            Ground::Predictions { scored } => format!("{scored} scored predictions"),
            Ground::Actions { taken } => format!("{taken} actions and their outcomes"),
            Ground::Absence { reason } => format!("the mirror reports: {reason}"),
        }
    }
}

/// What kind of claim this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    /// What can and cannot be seen.
    Observability,
    /// Structure discovered in the reflection.
    Structure,
    /// How well the next reflection can be predicted.
    Prediction,
    /// What responds to this system's own actions.
    Influence,
    /// Where the system's boundary appears to lie.
    Boundary,
}

impl Topic {
    pub fn label(self) -> &'static str {
        match self {
            Topic::Observability => "observability",
            Topic::Structure => "structure",
            Topic::Prediction => "prediction",
            Topic::Influence => "influence",
            Topic::Boundary => "boundary",
        }
    }
}

/// One grounded statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposition {
    pub topic: Topic,
    /// A stable machine-readable key, so a consumer can track one claim across
    /// runs without parsing prose.
    pub key: String,
    /// The measured quantity the claim is about, where there is one.
    pub value: Option<f64>,
    /// The claim in the first person, for people.
    pub prose: String,
    pub ground: Ground,
}

impl Proposition {
    fn new(
        topic: Topic,
        key: impl Into<String>,
        value: Option<f64>,
        prose: impl Into<String>,
        ground: Ground,
    ) -> Proposition {
        Proposition {
            topic,
            key: key.into(),
            value,
            prose: prose.into(),
            ground,
        }
    }

    /// The same content without the first person, which is the canonical form.
    pub fn machine(&self) -> String {
        match self.value {
            Some(value) => format!("{}={value}", self.key),
            None => self.key.clone(),
        }
    }

    /// One line with the evidence attached.
    pub fn cited(&self) -> String {
        format!("{}  ({})", self.prose, self.ground.describe())
    }
}

/// Everything the system can currently say about itself.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SelfDescription {
    pub propositions: Vec<Proposition>,
}

impl SelfDescription {
    pub fn is_empty(&self) -> bool {
        self.propositions.is_empty()
    }

    pub fn on(&self, topic: Topic) -> Vec<&Proposition> {
        self.propositions
            .iter()
            .filter(|p| p.topic == topic)
            .collect()
    }

    pub fn get(&self, key: &str) -> Option<&Proposition> {
        self.propositions.iter().find(|p| p.key == key)
    }

    /// Sorted with the best-evidenced claims first.
    pub fn by_evidence(&self) -> Vec<&Proposition> {
        let mut sorted: Vec<&Proposition> = self.propositions.iter().collect();
        sorted.sort_by(|a, b| b.ground.weight().cmp(&a.ground.weight()));
        sorted
    }

    /// The whole description as prose.
    pub fn render(&self) -> String {
        if self.propositions.is_empty() {
            // Not a failure. A system that has observed nothing has nothing it
            // is entitled to say, and saying so is the correct output.
            return "I have not observed enough to say anything about myself yet.\n".into();
        }
        let mut out = String::new();
        let mut last_topic = None;
        for proposition in self.by_evidence() {
            if last_topic != Some(proposition.topic) {
                out.push_str(&format!("\n[{}]\n", proposition.topic.label()));
                last_topic = Some(proposition.topic);
            }
            out.push_str(&format!("  {}\n", proposition.cited()));
        }
        out
    }
}

/// The inputs a description is built from.
///
/// Deliberately a plain struct of already-computed numbers rather than
/// references to the live objects: the description layer must not be able to
/// reach past what it was given and re-derive something more flattering.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Evidence {
    /// Remembered reflections available.
    pub frames: usize,
    /// Predictions that have been scored against reality.
    pub scored_predictions: u64,
    /// Mean prediction skill against the no-change baseline, where 0 means "no
    /// better than assuming nothing changes".
    pub model_skill: Option<f64>,
    /// Actions taken and confirmed.
    pub actions_taken: u64,
    /// Entities that measurably responded to this system's own actions.
    pub responsive_entities: usize,
    /// The largest group of variables that move together.
    pub largest_variable_group: Option<usize>,
}

/// Build a self-description from what has actually been measured.
///
/// Every branch below is a guard: a fact that has not been measured produces no
/// proposition at all, rather than a proposition hedged with "approximately".
pub fn describe(
    snapshot: Option<&MirrorSnapshot>,
    catalogue: Option<&LatentCatalogue>,
    candidates: &[SelfCandidate],
    evidence: &Evidence,
) -> SelfDescription {
    let mut propositions = Vec::new();

    if let Some(snapshot) = snapshot {
        let entities = snapshot.entities.len();
        let channels = snapshot.channels.len();
        let cells = entities * channels;
        let observed = snapshot.state.observed_cells();
        propositions.push(Proposition::new(
            Topic::Observability,
            "observable_variables",
            Some(observed as f64),
            format!(
                "I observe {observed} of {cells} possible variables about myself, \
                 across {entities} parts."
            ),
            Ground::Reflection {
                sequence: snapshot.sequence,
            },
        ));

        // The gaps are as much a part of self-knowledge as the readings, and
        // are the propositions most likely to be acted on by an operator.
        for (reason, count) in snapshot.availability.tally() {
            if reason.is_observed() || count == 0 {
                continue;
            }
            propositions.push(Proposition::new(
                Topic::Observability,
                format!("unobservable.{}", reason.label()),
                Some(count as f64),
                format!(
                    "There are {count} things about myself I cannot observe on this \
                     machine: {}.",
                    reason.explain()
                ),
                Ground::Absence {
                    reason: reason.explain().to_string(),
                },
            ));
        }
    }

    if let Some(catalogue) = catalogue {
        if !catalogue.is_empty() {
            propositions.push(Proposition::new(
                Topic::Structure,
                "latent_states",
                Some(catalogue.len() as f64),
                format!(
                    "I distinguish {} recurring states of myself, which I found rather \
                     than being told about.",
                    catalogue.len()
                ),
                Ground::Catalogue {
                    observations: catalogue.observations(),
                },
            ));
        }
        if let Some(current) = catalogue.current() {
            propositions.push(Proposition::new(
                Topic::Structure,
                "current_state",
                Some(current.0 as f64),
                format!("I am currently in {current}."),
                Ground::Catalogue {
                    observations: catalogue.observations(),
                },
            ));
            if let Some((next, probability)) = catalogue.most_likely_successor(current) {
                propositions.push(Proposition::new(
                    Topic::Prediction,
                    "likely_successor",
                    Some(probability),
                    format!(
                        "From here I most often move to {next}, {:.0}% of the time.",
                        probability * 100.0
                    ),
                    Ground::Catalogue {
                        observations: catalogue.observations(),
                    },
                ));
            }
        }
    }

    if let Some(group) = evidence.largest_variable_group {
        if group > 1 {
            propositions.push(Proposition::new(
                Topic::Structure,
                "largest_variable_group",
                Some(group as f64),
                format!("{group} of my variables move together as a single group."),
                Ground::History {
                    frames: evidence.frames,
                },
            ));
        }
    }

    // A skill claim requires scored predictions. Reporting skill before
    // anything has been scored would be reporting the initial value of a
    // counter.
    if let Some(skill) = evidence.model_skill {
        if evidence.scored_predictions > 0 && skill.is_finite() {
            let prose = if skill > 0.0 {
                format!(
                    "I predict my own next state {:.0}% better than assuming nothing \
                     changes.",
                    skill * 100.0
                )
            } else {
                // The unflattering case is stated in the same voice as the
                // flattering one.
                format!(
                    "I do not predict my own next state better than assuming nothing \
                     changes; I am {:.0}% worse.",
                    skill.abs() * 100.0
                )
            };
            propositions.push(Proposition::new(
                Topic::Prediction,
                "model_skill",
                Some(skill),
                prose,
                Ground::Predictions {
                    scored: evidence.scored_predictions,
                },
            ));
        }
    }

    if evidence.actions_taken > 0 {
        propositions.push(Proposition::new(
            Topic::Influence,
            "responsive_entities",
            Some(evidence.responsive_entities as f64),
            format!(
                "{} parts of the machine measurably respond when I act on myself.",
                evidence.responsive_entities
            ),
            Ground::Actions {
                taken: evidence.actions_taken,
            },
        ));
    }

    if !candidates.is_empty() {
        if let Some(best) = candidates.first() {
            propositions.push(Proposition::new(
                Topic::Boundary,
                "self_candidate",
                Some(best.members.len() as f64),
                format!(
                    "One hypothesis about where I end, {}, contains {} parts, selected \
                     by asking: {}",
                    best.id,
                    best.members.len(),
                    best.criterion.question()
                ),
                // Controllability evidence only exists once something has
                // been acted on; before that the boundary rests on correlation.
                if evidence.actions_taken > 0 {
                    Ground::Actions {
                        taken: evidence.actions_taken,
                    }
                } else {
                    Ground::History {
                        frames: evidence.frames,
                    }
                },
            ));
        }
        if candidates.len() > 1 {
            propositions.push(Proposition::new(
                Topic::Boundary,
                "competing_boundaries",
                Some(candidates.len() as f64),
                format!(
                    "I have {} competing hypotheses about where I end and have not \
                     settled between them.",
                    candidates.len()
                ),
                Ground::History {
                    frames: evidence.frames,
                },
            ));
        }
    }

    SelfDescription { propositions }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::test_support::fixture;

    fn evidence() -> Evidence {
        Evidence {
            frames: 500,
            scored_predictions: 400,
            model_skill: Some(0.34),
            actions_taken: 12,
            responsive_entities: 3,
            largest_variable_group: Some(12),
        }
    }

    #[test]
    fn a_system_that_has_observed_nothing_says_nothing() {
        // The correct output for an empty system is an admission, not a
        // plausible-sounding sentence.
        let description = describe(None, None, &[], &Evidence::default());
        assert!(description.is_empty());
        assert!(description.render().contains("not observed enough"));
    }

    #[test]
    fn every_proposition_carries_checkable_evidence() {
        let snapshot = fixture();
        let description = describe(Some(&snapshot), None, &[], &evidence());
        assert!(!description.is_empty());
        for proposition in &description.propositions {
            assert!(!proposition.ground.describe().is_empty());
            assert!(!proposition.key.is_empty());
            assert!(proposition.cited().contains('('));
        }
    }

    #[test]
    fn no_proposition_claims_consciousness_life_or_general_intelligence() {
        // Not a filter over a generator: there is no generator of these
        // sentences. The test guards against one being added.
        let snapshot = fixture();
        let mut catalogue = LatentCatalogue::new(1.0, 8);
        for i in 0..50 {
            catalogue.observe(&[i as f64 % 3.0, 0.0], i * 1_000_000);
        }
        let description = describe(Some(&snapshot), Some(&catalogue), &[], &evidence());
        let text = description.render().to_lowercase();
        for forbidden in [
            "conscious",
            "sentient",
            "aware",
            "alive",
            "agi",
            "understand",
            "feel",
            "want",
            "believe",
            "experience",
        ] {
            assert!(
                !text.contains(forbidden),
                "the description used the word {forbidden:?}:\n{text}"
            );
        }
    }

    #[test]
    fn unmeasured_facts_produce_no_proposition_rather_than_a_hedged_one() {
        let bare = Evidence {
            frames: 10,
            ..Evidence::default()
        };
        let description = describe(None, None, &[], &bare);
        assert!(description.get("model_skill").is_none());
        assert!(description.get("responsive_entities").is_none());
        assert!(description.get("largest_variable_group").is_none());
    }

    #[test]
    fn a_skill_claim_needs_scored_predictions_not_just_a_number() {
        let unscored = Evidence {
            scored_predictions: 0,
            model_skill: Some(0.9),
            ..Evidence::default()
        };
        assert!(describe(None, None, &[], &unscored)
            .get("model_skill")
            .is_none());
    }

    #[test]
    fn a_model_that_is_worse_than_nothing_says_so() {
        // The failure case must be as speakable as the success case.
        let bad = Evidence {
            scored_predictions: 100,
            model_skill: Some(-0.4),
            ..Evidence::default()
        };
        let description = describe(None, None, &[], &bad);
        let claim = description.get("model_skill").expect("claimed");
        assert!(claim.prose.contains("do not predict"));
        assert!(claim.prose.contains("worse"));
    }

    #[test]
    fn what_cannot_be_observed_is_stated_as_plainly_as_what_can() {
        // The fixture contains a permission-denied cell.
        let snapshot = fixture();
        let description = describe(Some(&snapshot), None, &[], &Evidence::default());
        let gaps = description.on(Topic::Observability);
        assert!(
            gaps.iter().any(|p| p.key.starts_with("unobservable.")),
            "a privilege gap in the mirror must reach the description"
        );
        assert!(gaps
            .iter()
            .any(|p| matches!(p.ground, Ground::Absence { .. })));
    }

    #[test]
    fn discovered_states_keep_their_machine_names() {
        let mut catalogue = LatentCatalogue::new(1.0, 8);
        for i in 0..40 {
            catalogue.observe(&[(i % 4) as f64 * 5.0, 0.0], i * 1_000_000);
        }
        let description = describe(None, Some(&catalogue), &[], &evidence());
        let current = description.get("current_state").expect("a current state");
        assert!(current.prose.contains("latent_state_"), "{}", current.prose);
    }

    #[test]
    fn the_machine_form_carries_the_same_content_without_the_first_person() {
        let description = describe(None, None, &[], &evidence());
        for proposition in &description.propositions {
            let machine = proposition.machine();
            assert!(!machine.contains(" I "));
            assert!(machine.starts_with(&proposition.key));
        }
    }

    #[test]
    fn better_evidenced_claims_come_first() {
        let description = describe(Some(&fixture()), None, &[], &evidence());
        let ordered = description.by_evidence();
        for pair in ordered.windows(2) {
            assert!(pair[0].ground.weight() >= pair[1].ground.weight());
        }
    }

    #[test]
    fn a_description_round_trips_through_json() {
        let description = describe(Some(&fixture()), None, &[], &evidence());
        let text = serde_json::to_string(&description).expect("serialises");
        let back: SelfDescription = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back, description);
    }
}
