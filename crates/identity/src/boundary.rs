//! Constructing candidate self-boundaries from evidence.

use std::collections::BTreeMap;

use corescout_mirror::{EntityId, MirrorSnapshot};
use corescout_represent::discover;
use serde::{Deserialize, Serialize};

/// Identifier of a candidate boundary.
///
/// Renders as `self_candidate_a`. Lettered rather than named, for the same
/// reason latent states are numbered: a name would smuggle in a conclusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CandidateId(pub u8);

impl std::fmt::Display for CandidateId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let letter = (b'a' + (self.0 % 26)) as char;
        write!(f, "self_candidate_{letter}")
    }
}

/// What is known about one entity's claim to be part of the machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityEvidence {
    pub row: u32,
    pub id: EntityId,
    /// Behavioural correlation with the rest of the machine, once the
    /// machine-wide common mode is removed. `0.0` when incomparable.
    pub coupling: f64,
    /// How reliably this entity changed when an action was taken.
    /// `None` when no action has ever been tried that could have affected it,
    /// which is a different thing from "it did not respond".
    pub controllability: Option<f64>,
    /// Fraction of observed reflections in which this entity was present.
    pub persistence: f64,
    /// Fraction of this entity's cells that carry values at all.
    pub observability: f64,
}

impl EntityEvidence {
    /// A single support score, for ranking. Deliberately not the basis of
    /// membership: a boundary is defined by a criterion, not by a threshold on
    /// a blended number.
    pub fn support(&self) -> f64 {
        let controllability = self.controllability.unwrap_or(0.0);
        0.35 * self.coupling
            + 0.35 * controllability
            + 0.2 * self.persistence
            + 0.1 * self.observability
    }

    /// Whether anything has ever been tried that would reveal controllability.
    pub fn controllability_tested(&self) -> bool {
        self.controllability.is_some()
    }
}

/// The rule that defined a candidate boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryCriterion {
    /// Everything the mirror can see at all. The widest possible boundary, and
    /// the one that needs no inference.
    Observable,
    /// Entities that move together with the machine as a whole.
    Coupled,
    /// Entities that responded to this system's own actions.
    Controllable,
    /// Both coupled and controllable: the strictest.
    CoupledAndControllable,
    /// Entities present continuously across every epoch seen.
    Persistent,
}

impl BoundaryCriterion {
    pub fn label(self) -> &'static str {
        match self {
            BoundaryCriterion::Observable => "observable",
            BoundaryCriterion::Coupled => "coupled",
            BoundaryCriterion::Controllable => "controllable",
            BoundaryCriterion::CoupledAndControllable => "coupled and controllable",
            BoundaryCriterion::Persistent => "persistent",
        }
    }

    /// The question this criterion is an answer to.
    pub fn question(self) -> &'static str {
        match self {
            BoundaryCriterion::Observable => "what can I see?",
            BoundaryCriterion::Coupled => "what moves with me?",
            BoundaryCriterion::Controllable => "what responds when I act?",
            BoundaryCriterion::CoupledAndControllable => {
                "what both moves with me and responds when I act?"
            }
            BoundaryCriterion::Persistent => "what is always there?",
        }
    }
}

/// One hypothesis about where the machine ends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelfCandidate {
    pub id: CandidateId,
    pub criterion: BoundaryCriterion,
    /// Entity rows inside the boundary.
    pub members: Vec<u32>,
    /// Stable ids of the members, so a candidate survives an epoch change.
    pub member_ids: Vec<EntityId>,
    /// Mean support across members.
    pub support: f64,
    /// How much of the observable machine this includes.
    pub coverage: f64,
}

impl SelfCandidate {
    pub fn contains(&self, row: u32) -> bool {
        self.members.contains(&row)
    }

    pub fn size(&self) -> usize {
        self.members.len()
    }
}

/// Gathers evidence and constructs candidates.
#[derive(Debug, Clone, Default)]
pub struct BoundaryInference {
    /// Per entity: how often it has been present.
    presence: BTreeMap<EntityId, u64>,
    /// Per entity: observed cells, and cells possible.
    observed: BTreeMap<EntityId, (u64, u64)>,
    /// Per entity: (times an action could have affected it, times it changed).
    responses: BTreeMap<EntityId, (u32, u32)>,
    reflections: u64,
}

impl BoundaryInference {
    pub fn new() -> BoundaryInference {
        BoundaryInference::default()
    }

    pub fn reflections(&self) -> u64 {
        self.reflections
    }

    /// Fold in a reflection: who was here, and how much of them was visible.
    pub fn observe(&mut self, snapshot: &MirrorSnapshot) {
        self.reflections += 1;
        let cols = snapshot.state.cols();
        for (row, entity) in snapshot.entities.iter().enumerate() {
            *self.presence.entry(entity.id).or_insert(0) += 1;
            let filled = (0..cols)
                .filter(|col| snapshot.state.get(row, *col).is_finite())
                .count() as u64;
            let entry = self.observed.entry(entity.id).or_insert((0, 0));
            entry.0 += filled;
            entry.1 += cols as u64;
        }
    }

    /// Record that an action was taken and whether an entity's state changed
    /// materially afterwards.
    ///
    /// This is the only input that can distinguish self from environment, and
    /// it can only come from having acted. A system that never acts cannot
    /// learn its own boundary, which is a claim worth taking seriously rather
    /// than a limitation of this implementation.
    pub fn record_response(&mut self, entity: EntityId, changed: bool) {
        let entry = self.responses.entry(entity).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += u32::from(changed);
    }

    /// Assemble the evidence, given a behavioural similarity matrix.
    ///
    /// `similarity` is expected to be the common-mode-removed entity matrix
    /// from `corescout-represent`, so "coupled" means genuinely coupled rather
    /// than "both busy at the same time".
    pub fn evidence(
        &self,
        snapshot: &MirrorSnapshot,
        similarity: &[Vec<f64>],
    ) -> Vec<EntityEvidence> {
        snapshot
            .entities
            .iter()
            .enumerate()
            .map(|(row, entity)| {
                let coupling = mean_finite(
                    similarity
                        .get(row)
                        .map(|r| {
                            r.iter()
                                .enumerate()
                                .filter(|(other, _)| *other != row)
                                .map(|(_, v)| *v)
                                .collect::<Vec<f64>>()
                        })
                        .unwrap_or_default(),
                );
                let controllability =
                    self.responses.get(&entity.id).and_then(|(tried, changed)| {
                        (*tried > 0).then(|| *changed as f64 / *tried as f64)
                    });
                let persistence = self
                    .presence
                    .get(&entity.id)
                    .map(|seen| *seen as f64 / self.reflections.max(1) as f64)
                    .unwrap_or(0.0);
                let observability = self
                    .observed
                    .get(&entity.id)
                    .map(|(filled, possible)| {
                        if *possible == 0 {
                            0.0
                        } else {
                            *filled as f64 / *possible as f64
                        }
                    })
                    .unwrap_or(0.0);
                EntityEvidence {
                    row: row as u32,
                    id: entity.id,
                    coupling,
                    controllability,
                    persistence,
                    observability,
                }
            })
            .collect()
    }

    /// Construct every candidate boundary the evidence supports.
    ///
    /// Returns them widest first. A caller comparing them is asking the
    /// question the crate exists for; the crate does not answer it.
    pub fn candidates(&self, evidence: &[EntityEvidence]) -> Vec<SelfCandidate> {
        let observable: Vec<&EntityEvidence> =
            evidence.iter().filter(|e| e.observability > 0.0).collect();
        let total = observable.len().max(1);

        let build = |id: u8,
                     criterion: BoundaryCriterion,
                     members: Vec<&EntityEvidence>|
         -> Option<SelfCandidate> {
            if members.is_empty() {
                return None;
            }
            let support = members.iter().map(|e| e.support()).sum::<f64>() / members.len() as f64;
            Some(SelfCandidate {
                id: CandidateId(id),
                criterion,
                members: members.iter().map(|e| e.row).collect(),
                member_ids: members.iter().map(|e| e.id).collect(),
                support,
                coverage: members.len() as f64 / total as f64,
            })
        };

        let coupled: Vec<&EntityEvidence> = observable
            .iter()
            .copied()
            .filter(|e| e.coupling > 0.3)
            .collect();
        let controllable: Vec<&EntityEvidence> = observable
            .iter()
            .copied()
            .filter(|e| e.controllability.is_some_and(|c| c > 0.5))
            .collect();
        let both: Vec<&EntityEvidence> = observable
            .iter()
            .copied()
            .filter(|e| e.coupling > 0.3 && e.controllability.is_some_and(|c| c > 0.5))
            .collect();
        let persistent: Vec<&EntityEvidence> = observable
            .iter()
            .copied()
            .filter(|e| e.persistence > 0.99)
            .collect();

        [
            build(0, BoundaryCriterion::Observable, observable.clone()),
            build(1, BoundaryCriterion::Persistent, persistent),
            build(2, BoundaryCriterion::Coupled, coupled),
            build(3, BoundaryCriterion::Controllable, controllable),
            build(4, BoundaryCriterion::CoupledAndControllable, both),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// Whether controllability has been tested at all.
    ///
    /// Until it has, every candidate is built from correlation alone, and the
    /// two most interesting criteria are empty. A report should say so rather
    /// than presenting a coupling-only boundary as the system's view of itself.
    pub fn has_acted(&self) -> bool {
        self.responses.values().any(|(tried, _)| *tried > 0)
    }
}

fn mean_finite(values: Vec<f64>) -> f64 {
    let usable: Vec<f64> = values.into_iter().filter(|v| v.is_finite()).collect();
    if usable.is_empty() {
        return 0.0;
    }
    discover::mean(&usable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::test_support::fixture;

    fn evidence_for(coupling: f64, controllability: Option<f64>) -> EntityEvidence {
        EntityEvidence {
            row: 0,
            id: EntityId(1),
            coupling,
            controllability,
            persistence: 1.0,
            observability: 1.0,
        }
    }

    #[test]
    fn a_candidate_renders_as_a_lettered_hypothesis() {
        assert_eq!(CandidateId(0).to_string(), "self_candidate_a");
        assert_eq!(CandidateId(3).to_string(), "self_candidate_d");
    }

    #[test]
    fn presence_and_observability_accumulate() {
        let mut inference = BoundaryInference::new();
        for _ in 0..10 {
            inference.observe(&fixture());
        }
        assert_eq!(inference.reflections(), 10);

        let snapshot = fixture();
        let similarity = vec![vec![f64::NAN; 4]; 4];
        let evidence = inference.evidence(&snapshot, &similarity);
        assert_eq!(evidence.len(), 4);
        // Every fixture entity was present in every reflection.
        assert!(evidence.iter().all(|e| e.persistence > 0.99));
        // CPU 0 has two of two cells; the machine row has none.
        let cpu0 = evidence.iter().find(|e| e.row == 2).unwrap();
        let machine = evidence.iter().find(|e| e.row == 0).unwrap();
        assert!(cpu0.observability > machine.observability);
    }

    #[test]
    fn controllability_is_none_until_something_has_been_tried() {
        // "I have not tested it" and "it did not respond" are different facts
        // and must not collapse to the same number.
        let mut inference = BoundaryInference::new();
        inference.observe(&fixture());
        let evidence = inference.evidence(&fixture(), &vec![vec![0.5; 4]; 4]);
        assert!(evidence.iter().all(|e| !e.controllability_tested()));
        assert!(!inference.has_acted());
    }

    #[test]
    fn acting_produces_controllability_evidence() {
        let mut inference = BoundaryInference::new();
        inference.observe(&fixture());
        let cpu0 = fixture().entities[2].id;
        for changed in [true, true, true, false] {
            inference.record_response(cpu0, changed);
        }
        let evidence = inference.evidence(&fixture(), &vec![vec![0.5; 4]; 4]);
        let cpu0_evidence = evidence.iter().find(|e| e.id == cpu0).unwrap();
        assert_eq!(cpu0_evidence.controllability, Some(0.75));
        assert!(inference.has_acted());
    }

    #[test]
    fn candidates_are_built_from_criteria_not_from_a_blended_threshold() {
        let evidence = vec![
            // Coupled and controllable: inside every candidate.
            EntityEvidence {
                row: 0,
                id: EntityId(1),
                coupling: 0.9,
                controllability: Some(0.9),
                persistence: 1.0,
                observability: 1.0,
            },
            // Coupled but never responds: correlated environment.
            EntityEvidence {
                row: 1,
                id: EntityId(2),
                coupling: 0.9,
                controllability: Some(0.0),
                persistence: 1.0,
                observability: 1.0,
            },
            // Responds but uncorrelated.
            EntityEvidence {
                row: 2,
                id: EntityId(3),
                coupling: 0.0,
                controllability: Some(0.9),
                persistence: 1.0,
                observability: 1.0,
            },
        ];
        let candidates = BoundaryInference::new().candidates(&evidence);

        let coupled = candidates
            .iter()
            .find(|c| c.criterion == BoundaryCriterion::Coupled)
            .unwrap();
        assert_eq!(coupled.members, vec![0, 1]);

        let controllable = candidates
            .iter()
            .find(|c| c.criterion == BoundaryCriterion::Controllable)
            .unwrap();
        assert_eq!(controllable.members, vec![0, 2]);

        let both = candidates
            .iter()
            .find(|c| c.criterion == BoundaryCriterion::CoupledAndControllable)
            .unwrap();
        assert_eq!(
            both.members,
            vec![0],
            "the strictest boundary excludes correlated-but-unresponsive entities"
        );
        assert!(both.coverage < 1.0);
    }

    #[test]
    fn without_action_the_controllable_candidates_are_absent() {
        // The important negative: a system that has never acted has no evidence
        // for the boundary that matters, and must not manufacture one.
        let evidence = vec![evidence_for(0.9, None), evidence_for(0.8, None)];
        let candidates = BoundaryInference::new().candidates(&evidence);
        assert!(candidates
            .iter()
            .all(|c| c.criterion != BoundaryCriterion::Controllable));
        assert!(candidates
            .iter()
            .any(|c| c.criterion == BoundaryCriterion::Coupled));
    }

    #[test]
    fn every_criterion_states_the_question_it_answers() {
        for criterion in [
            BoundaryCriterion::Observable,
            BoundaryCriterion::Coupled,
            BoundaryCriterion::Controllable,
            BoundaryCriterion::CoupledAndControllable,
            BoundaryCriterion::Persistent,
        ] {
            assert!(criterion.question().ends_with('?'));
            assert!(!criterion.label().is_empty());
        }
    }

    #[test]
    fn an_entity_the_mirror_cannot_see_is_in_no_candidate() {
        let evidence = vec![EntityEvidence {
            row: 0,
            id: EntityId(1),
            coupling: 1.0,
            controllability: Some(1.0),
            persistence: 1.0,
            observability: 0.0,
        }];
        assert!(BoundaryInference::new().candidates(&evidence).is_empty());
    }
}
