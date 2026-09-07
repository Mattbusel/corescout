//! Concepts the machine coined, and the rule that makes coining them mean
//! something.
//!
//! # The difference between a cluster and a concept
//!
//! `corescout-represent` produces `latent_state_13` for any region of state
//! space the machine visits more than once. That is cheap, and most of them are
//! worthless: an artifact of the novelty threshold, a transient, a coincidence.
//!
//! A [`Concept`] is a latent state that has **earned promotion**, by paying its
//! way against a stated bar:
//!
//! | requirement | why |
//! |---|---|
//! | recurs | a thing seen once is an event, not a category |
//! | predicts | knowing you are in it must beat not knowing |
//! | survives testing | its hypotheses must not have been refuted |
//!
//! Only the second is really load-bearing, and it is the one that makes this
//! more than renaming. A concept that does not improve prediction is a label,
//! and labelling is what we were trying to stop doing.
//!
//! # Concepts can be de-coined
//!
//! [`Registry::audit`] retires concepts whose predictive utility has decayed.
//! That is not tidying: a system that can only ever add concepts will
//! accumulate them until every reflection is in a category of its own, and the
//! ontology will have explained nothing while appearing to explain everything.
//!
//! # The naming rule
//!
//! A concept is `concept_7`. It stays `concept_7`. If it turns out to coincide
//! with what we call thermal throttling, that coincidence is a **finding** and
//! is recorded as [`Concept::resembles`], which is a note for humans and is
//! never used by any decision. The concept is not renamed, because renaming it
//! would destroy the finding and replace it with our assumption.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use corescout_represent::latent::{LatentCatalogue, LatentState, LatentStateId};

use crate::signature::Signature;

/// A machine-coined concept's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ConceptId(pub u32);

impl fmt::Display for ConceptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "concept_{}", self.0)
    }
}

/// Where a concept came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Promoted from a recurring state the machine found.
    LatentState(LatentStateId),
    /// Coined from a group of variables that move together.
    VariableGroup { members: Vec<usize> },
    /// Built by composing concepts already held.
    Composition { parts: Vec<ConceptId> },
    /// Adopted after recognising a foreign machine's concept in itself.
    ///
    /// The concept is still this machine's own: what was adopted is the
    /// *question* "do I have one of these", and the answer came from its own
    /// reflections.
    Analogy {
        foreign_name: String,
        similarity: f64,
    },
}

impl Origin {
    pub fn describe(&self) -> String {
        match self {
            Origin::LatentState(state) => format!("promoted from {state}"),
            Origin::VariableGroup { members } => {
                format!("coined from {} variables that move together", members.len())
            }
            Origin::Composition { parts } => format!(
                "composed from {}",
                parts
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(" and ")
            ),
            Origin::Analogy {
                foreign_name,
                similarity,
            } => format!(
                "found in itself after another machine described {foreign_name} \
                 (similarity {similarity:.2})"
            ),
        }
    }
}

/// A first-class computational primitive the machine invented.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Concept {
    pub id: ConceptId,
    pub origin: Origin,
    /// The defining configuration, in normalised channel space.
    pub definition: Vec<f64>,
    /// How tight the definition is.
    pub radius: f64,
    /// The portable, machine-independent description.
    pub signature: Signature,
    /// Times the machine has been in it since coinage.
    pub occurrences: u64,
    /// How much knowing the machine is in it improves prediction.
    pub predictive_utility: f64,
    /// Utility when it was coined, so decay is visible.
    pub utility_at_coinage: f64,
    /// Hypotheses about it that survived, and ones that were killed.
    pub supported_claims: u32,
    pub refuted_claims: u32,
    pub coined_ns: u64,
    pub last_seen_ns: u64,
    /// A human's note that this appears to coincide with something we have a
    /// word for. **Never read by any decision.** It exists so a person can
    /// follow along, and so the coincidence can be reported as a finding.
    pub resembles: Option<String>,
    pub retired: Option<String>,
}

impl Concept {
    /// Whether this concept is still worth having.
    ///
    /// The bar is that it predicts. A concept that has stopped predicting has
    /// stopped being a concept and become a label.
    pub fn pays_its_way(&self, min_utility: f64) -> bool {
        self.retired.is_none() && self.predictive_utility >= min_utility
    }

    /// How much its usefulness has decayed since it was coined.
    pub fn decay(&self) -> f64 {
        self.utility_at_coinage - self.predictive_utility
    }

    /// Evidence for it, net of evidence against.
    pub fn standing(&self) -> f64 {
        let total = self.supported_claims + self.refuted_claims;
        if total == 0 {
            return 0.0;
        }
        self.supported_claims as f64 / total as f64
    }

    pub fn is_retired(&self) -> bool {
        self.retired.is_some()
    }

    /// A description that uses the machine's own name and no others.
    pub fn describe(&self) -> String {
        let mut text = format!(
            "{}: {}, seen {} times, predictive utility {:.3}",
            self.id,
            self.origin.describe(),
            self.occurrences,
            self.predictive_utility
        );
        if let Some(resembles) = &self.resembles {
            // Reported as an observation about us, not about the concept.
            text.push_str(&format!("  (a human noted this looks like {resembles})"));
        }
        if let Some(reason) = &self.retired {
            text.push_str(&format!("  [retired: {reason}]"));
        }
        text
    }
}

/// Conditions a state must meet to be promoted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoinageRules {
    /// Occurrences before a state may be promoted.
    pub min_occurrences: u64,
    /// Predictive utility required at coinage.
    ///
    /// The load-bearing rule. Without it, coinage is renaming.
    pub min_utility: f64,
    /// Confidence that the state is real rather than coincidental.
    pub min_confidence: f64,
    /// Utility below which an existing concept is retired.
    ///
    /// Lower than `min_utility`, so a concept hovering at the bar does not
    /// flicker in and out of existence.
    pub retire_below: f64,
    /// Concepts to hold at once.
    pub max_concepts: usize,
}

impl Default for CoinageRules {
    fn default() -> Self {
        CoinageRules {
            min_occurrences: 30,
            // Must beat not knowing by a clear margin, not merely tie it.
            min_utility: 0.05,
            min_confidence: 0.6,
            retire_below: 0.01,
            max_concepts: 64,
        }
    }
}

/// Why a state was not promoted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    TooRare { occurrences: u64, needed: u64 },
    DoesNotPredict { utility: f64, needed: f64 },
    NotConfident { confidence: f64, needed: f64 },
    AlreadyCoined(ConceptId),
    NoRoom,
}

impl Refusal {
    pub fn describe(&self) -> String {
        match self {
            Refusal::TooRare {
                occurrences,
                needed,
            } => format!("seen {occurrences} times, needs {needed}"),
            Refusal::DoesNotPredict { utility, needed } => format!(
                "predictive utility {utility:.3} does not reach {needed:.3}; \
                 coining it would be renaming"
            ),
            Refusal::NotConfident { confidence, needed } => {
                format!("confidence {confidence:.2} does not reach {needed:.2}")
            }
            Refusal::AlreadyCoined(id) => format!("already coined as {id}"),
            Refusal::NoRoom => "the registry is full of better concepts".into(),
        }
    }
}

/// Everything the machine has coined.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Registry {
    rules: CoinageRules,
    concepts: Vec<Concept>,
    /// Which latent state produced which concept, so a state is not coined
    /// twice under different names.
    #[serde(with = "corescout_core::serde_util::pairs")]
    from_state: BTreeMap<LatentStateId, ConceptId>,
    next_id: u32,
    coined: u64,
    retired: u64,
    refused: u64,
}

impl Default for Registry {
    fn default() -> Self {
        Registry::new(CoinageRules::default())
    }
}

impl Registry {
    pub fn new(rules: CoinageRules) -> Registry {
        Registry {
            rules,
            concepts: Vec::new(),
            from_state: BTreeMap::new(),
            next_id: 1,
            coined: 0,
            retired: 0,
            refused: 0,
        }
    }

    pub fn rules(&self) -> &CoinageRules {
        &self.rules
    }

    pub fn concepts(&self) -> &[Concept] {
        &self.concepts
    }

    /// Concepts that are still earning their place.
    pub fn live(&self) -> Vec<&Concept> {
        self.concepts
            .iter()
            .filter(|c| c.pays_its_way(self.rules.retire_below))
            .collect()
    }

    pub fn get(&self, id: ConceptId) -> Option<&Concept> {
        self.concepts.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: ConceptId) -> Option<&mut Concept> {
        self.concepts.iter_mut().find(|c| c.id == id)
    }

    pub fn len(&self) -> usize {
        self.concepts.iter().filter(|c| !c.is_retired()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn coined(&self) -> u64 {
        self.coined
    }

    pub fn retired_count(&self) -> u64 {
        self.retired
    }

    pub fn refused_count(&self) -> u64 {
        self.refused
    }

    /// Try to promote a discovered state into a concept.
    ///
    /// `utility` is how much knowing the machine is in this state improves
    /// prediction, measured by the self-model. The registry does not compute it
    /// and must not guess at it: a registry that could invent its own
    /// justification for coining would coin everything.
    pub fn coin(
        &mut self,
        state: &LatentState,
        utility: f64,
        signature: Signature,
        now_ns: u64,
    ) -> Result<ConceptId, Refusal> {
        if let Some(existing) = self.from_state.get(&state.id) {
            return Err(Refusal::AlreadyCoined(*existing));
        }
        if state.occupancy < self.rules.min_occurrences {
            self.refused += 1;
            return Err(Refusal::TooRare {
                occurrences: state.occupancy,
                needed: self.rules.min_occurrences,
            });
        }
        if state.confidence < self.rules.min_confidence {
            self.refused += 1;
            return Err(Refusal::NotConfident {
                confidence: state.confidence,
                needed: self.rules.min_confidence,
            });
        }
        // The rule that makes coinage mean something. NaN is rejected
        // explicitly: it fails every comparison, so a plain `<` would let it
        // through as "not below the bar".
        if !utility.is_finite() || utility < self.rules.min_utility {
            self.refused += 1;
            return Err(Refusal::DoesNotPredict {
                utility,
                needed: self.rules.min_utility,
            });
        }
        if self.len() >= self.rules.max_concepts && !self.evict_worst_than(utility) {
            self.refused += 1;
            return Err(Refusal::NoRoom);
        }

        let id = ConceptId(self.next_id);
        self.next_id += 1;
        self.concepts.push(Concept {
            id,
            origin: Origin::LatentState(state.id),
            definition: state.centroid.clone(),
            radius: state.radius,
            signature,
            occurrences: state.occupancy,
            predictive_utility: utility,
            utility_at_coinage: utility,
            supported_claims: 0,
            refuted_claims: 0,
            coined_ns: now_ns,
            last_seen_ns: now_ns,
            resembles: None,
            retired: None,
        });
        self.from_state.insert(state.id, id);
        self.coined += 1;
        Ok(id)
    }

    /// Coin a concept this machine found in itself after another described one.
    ///
    /// The same bar applies. Being told a concept exists elsewhere is not
    /// evidence that it exists here, and adopting one on a foreign machine's
    /// word is exactly the mistake this whole design is arranged to avoid.
    pub fn coin_by_analogy(
        &mut self,
        state: &LatentState,
        utility: f64,
        signature: Signature,
        foreign_name: &str,
        similarity: f64,
        now_ns: u64,
    ) -> Result<ConceptId, Refusal> {
        let id = self.coin(state, utility, signature, now_ns)?;
        if let Some(concept) = self.get_mut(id) {
            concept.origin = Origin::Analogy {
                foreign_name: foreign_name.to_string(),
                similarity,
            };
        }
        Ok(id)
    }

    /// Record that the machine is in a concept right now.
    pub fn observe(&mut self, id: ConceptId, now_ns: u64) {
        if let Some(concept) = self.get_mut(id) {
            concept.occurrences += 1;
            concept.last_seen_ns = now_ns;
        }
    }

    /// Update a concept's measured usefulness.
    pub fn set_utility(&mut self, id: ConceptId, utility: f64) {
        if let Some(concept) = self.get_mut(id) {
            if utility.is_finite() {
                concept.predictive_utility = utility;
            }
        }
    }

    /// Record how a hypothesis about a concept turned out.
    pub fn record_claim(&mut self, id: ConceptId, supported: bool) {
        if let Some(concept) = self.get_mut(id) {
            if supported {
                concept.supported_claims += 1;
            } else {
                concept.refuted_claims += 1;
            }
        }
    }

    /// Attach a human's note that a concept looks like something we have a word
    /// for.
    ///
    /// A finding, recorded next to the concept. The concept keeps its own name
    /// and nothing downstream reads this.
    pub fn note_resemblance(&mut self, id: ConceptId, resembles: impl Into<String>) {
        if let Some(concept) = self.get_mut(id) {
            concept.resembles = Some(resembles.into());
        }
    }

    /// Reduce the standing of concepts that have not occurred lately.
    ///
    /// # Why this is necessary
    ///
    /// Predictive utility is measured over evidence, and evidence accumulates.
    /// A concept whose conditions have stopped occurring keeps every
    /// observation that ever supported it, so its measured utility never falls
    /// and [`Registry::audit`] never retires it. The ontology would then be able
    /// to grow but never to respond to a machine that has changed.
    ///
    /// A concept is a claim about the machine *as it now is*. Absence is
    /// evidence against that claim, so standing halves for every `half_life` of
    /// not being seen, and the ordinary audit does the retiring.
    ///
    /// This will eventually retire a genuinely rare concept that is real. That
    /// is the intended trade: a rare concept can be re-coined the next time it
    /// occurs, whereas a stale one that is never questioned drives behaviour
    /// forever.
    pub fn decay_unseen(&mut self, now_ns: u64, half_life_ns: u64) -> usize {
        if half_life_ns == 0 {
            return 0;
        }
        let mut decayed = 0;
        for concept in &mut self.concepts {
            if concept.is_retired() {
                continue;
            }
            let absent = now_ns.saturating_sub(concept.last_seen_ns);
            if absent < half_life_ns {
                continue;
            }
            let halvings = (absent / half_life_ns) as f64;
            concept.predictive_utility *= 0.5f64.powf(halvings);
            decayed += 1;
        }
        decayed
    }

    /// Retire concepts that have stopped paying their way.
    ///
    /// Returns the ones retired. Without this the ontology grows without bound
    /// and explains less the larger it gets.
    pub fn audit(&mut self) -> Vec<ConceptId> {
        let bar = self.rules.retire_below;
        let mut retired = Vec::new();
        for concept in &mut self.concepts {
            if concept.is_retired() {
                continue;
            }
            if concept.predictive_utility < bar {
                concept.retired = Some(format!(
                    "predictive utility fell to {:.3}, below {bar:.3}",
                    concept.predictive_utility
                ));
                retired.push(concept.id);
            } else if concept.refuted_claims > 0 && concept.standing() < 0.4 {
                concept.retired = Some(format!(
                    "most claims about it were refuted ({} of {})",
                    concept.refuted_claims,
                    concept.supported_claims + concept.refuted_claims
                ));
                retired.push(concept.id);
            }
        }
        self.retired += retired.len() as u64;
        retired
    }

    /// The portable signatures of every live concept, for exchange.
    pub fn exportable(&self) -> Vec<(String, Signature)> {
        self.live()
            .into_iter()
            .map(|c| (c.id.to_string(), c.signature.clone()))
            .collect()
    }

    /// Make room by retiring a concept worse than the one being coined.
    fn evict_worst_than(&mut self, utility: f64) -> bool {
        let worst = self
            .concepts
            .iter_mut()
            .filter(|c| !c.is_retired())
            .min_by(|a, b| a.predictive_utility.total_cmp(&b.predictive_utility));
        match worst {
            Some(concept) if concept.predictive_utility < utility => {
                concept.retired = Some("displaced by a more useful concept".into());
                self.retired += 1;
                true
            }
            _ => false,
        }
    }

    /// A report a person can read.
    pub fn render(&self) -> String {
        let live = self.live();
        let mut out = format!(
            "{} concepts coined, {} retired, {} refused; {} currently held\n",
            self.coined,
            self.retired,
            self.refused,
            live.len()
        );
        if live.is_empty() {
            out.push_str(
                "nothing has earned promotion yet: a recurring state is not a concept until \
                 knowing about it improves prediction\n",
            );
            return out;
        }
        let mut ranked = live;
        ranked.sort_by(|a, b| b.predictive_utility.total_cmp(&a.predictive_utility));
        for concept in ranked {
            out.push_str(&format!("  {}\n", concept.describe()));
        }
        out
    }
}

/// Build a portable signature for a discovered state.
///
/// Everything here is normalised against the machine's own scale, which is what
/// makes the result comparable to another machine's. `median_dwell_ns` and
/// `total_observations` are that scale.
pub fn signature_for(
    state: &LatentState,
    catalogue: &LatentCatalogue,
    entities: usize,
    median_dwell_ns: f64,
    controllability: f64,
) -> Signature {
    let dimensions = state.centroid.len().max(1);
    // A component is distinctive if it sits well away from zero, which in
    // normalised space means away from that variable's typical value.
    let distinctive = state
        .centroid
        .iter()
        .filter(|v| v.is_finite() && v.abs() > 1.0)
        .count();
    let observations = catalogue.observations().max(1) as f64;

    Signature {
        // Participation is over entities, and the centroid is over cells, so
        // this is the fraction of the machine the state says something about.
        participation: if entities == 0 {
            0.0
        } else {
            (distinctive as f64 / dimensions as f64).clamp(0.0, 1.0)
        },
        distinctiveness: distinctive as f64 / dimensions as f64,
        // A tight state is a coherent one. Radius is in normalised units, so a
        // radius of 1 is "as spread out as the data".
        coherence: 1.0 / (1.0 + state.radius.max(0.0)),
        relative_dwell: if median_dwell_ns > 0.0 {
            state.mean_dwell_ns() / median_dwell_ns
        } else {
            1.0
        },
        recurrence: (state.occupancy as f64 / observations).clamp(0.0, 1.0),
        controllability: controllability.clamp(0.0, 1.0),
        predictive_utility: state.predictive_utility.clamp(0.0, 1.0),
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    pub(crate) fn state(id: u32, occupancy: u64, confidence: f64) -> LatentState {
        LatentState {
            id: LatentStateId(id),
            centroid: vec![2.0, 0.1, 3.0, 0.0],
            radius: 0.4,
            entries: 12,
            occupancy,
            dwell_ns: 1_000_000_000,
            first_seen_ns: 0,
            last_seen_ns: 1_000_000_000,
            predictive_utility: 0.3,
            confidence,
            action_responses: Vec::new(),
        }
    }

    pub(crate) fn signature() -> Signature {
        Signature {
            participation: 0.5,
            distinctiveness: 0.5,
            coherence: 0.7,
            relative_dwell: 1.0,
            recurrence: 0.2,
            controllability: 0.5,
            predictive_utility: 0.3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::*;
    use super::*;

    #[test]
    fn a_state_that_does_not_predict_is_not_coined() {
        // The rule that makes coinage more than renaming, and the one worth
        // testing hardest.
        let mut registry = Registry::default();
        let refusal = registry
            .coin(&state(1, 1000, 0.9), 0.0, signature(), 0)
            .unwrap_err();
        assert!(matches!(refusal, Refusal::DoesNotPredict { .. }));
        assert!(refusal.describe().contains("renaming"));
        assert!(registry.is_empty());
    }

    #[test]
    fn a_state_with_nan_utility_is_not_coined() {
        // NaN fails every comparison, including `>=`, so it must not slip
        // through as "not less than the bar".
        let mut registry = Registry::default();
        assert!(registry
            .coin(&state(1, 1000, 0.9), f64::NAN, signature(), 0)
            .is_err());
        assert!(registry.is_empty());
    }

    #[test]
    fn a_rare_state_is_not_coined_however_useful_it_looks() {
        let mut registry = Registry::default();
        let refusal = registry
            .coin(&state(1, 2, 0.9), 0.9, signature(), 0)
            .unwrap_err();
        assert!(matches!(refusal, Refusal::TooRare { .. }));
    }

    #[test]
    fn a_state_the_machine_is_unsure_of_is_not_coined() {
        let mut registry = Registry::default();
        let refusal = registry
            .coin(&state(1, 1000, 0.1), 0.9, signature(), 0)
            .unwrap_err();
        assert!(matches!(refusal, Refusal::NotConfident { .. }));
    }

    #[test]
    fn a_state_that_meets_the_bar_is_coined_and_keeps_a_machine_name() {
        let mut registry = Registry::default();
        let id = registry
            .coin(&state(1, 1000, 0.9), 0.4, signature(), 100)
            .expect("coined");
        assert_eq!(id.to_string(), "concept_1");
        assert_eq!(registry.len(), 1);
        let concept = registry.get(id).expect("present");
        assert_eq!(concept.origin, Origin::LatentState(LatentStateId(1)));
        assert!((concept.utility_at_coinage - 0.4).abs() < 1e-9);
    }

    #[test]
    fn the_same_state_is_not_coined_twice() {
        let mut registry = Registry::default();
        registry
            .coin(&state(1, 1000, 0.9), 0.4, signature(), 0)
            .expect("coined");
        let refusal = registry
            .coin(&state(1, 1000, 0.9), 0.4, signature(), 0)
            .unwrap_err();
        assert!(matches!(refusal, Refusal::AlreadyCoined(_)));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn a_concept_that_stops_predicting_is_retired() {
        // Without this the ontology grows without bound and explains less the
        // larger it gets.
        let mut registry = Registry::default();
        let id = registry
            .coin(&state(1, 1000, 0.9), 0.4, signature(), 0)
            .expect("coined");
        assert!(registry.audit().is_empty(), "nothing to retire yet");

        registry.set_utility(id, 0.0);
        let retired = registry.audit();
        assert_eq!(retired, vec![id]);
        assert!(registry.get(id).unwrap().is_retired());
        assert!(registry.is_empty());
        assert!(registry.get(id).unwrap().decay() > 0.0);
    }

    #[test]
    fn a_concept_whose_claims_were_refuted_is_retired() {
        let mut registry = Registry::default();
        let id = registry
            .coin(&state(1, 1000, 0.9), 0.4, signature(), 0)
            .expect("coined");
        for _ in 0..8 {
            registry.record_claim(id, false);
        }
        registry.record_claim(id, true);
        let retired = registry.audit();
        assert_eq!(retired, vec![id]);
        assert!(registry
            .get(id)
            .unwrap()
            .retired
            .as_deref()
            .unwrap()
            .contains("refuted"));
    }

    #[test]
    fn a_concept_with_only_supporting_claims_survives_the_audit() {
        let mut registry = Registry::default();
        let id = registry
            .coin(&state(1, 1000, 0.9), 0.4, signature(), 0)
            .expect("coined");
        for _ in 0..8 {
            registry.record_claim(id, true);
        }
        assert!(registry.audit().is_empty());
        assert!((registry.get(id).unwrap().standing() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn retiring_uses_a_lower_bar_than_coining_so_concepts_do_not_flicker() {
        let rules = CoinageRules::default();
        assert!(rules.retire_below < rules.min_utility);
        let mut registry = Registry::new(rules.clone());
        let id = registry
            .coin(&state(1, 1000, 0.9), rules.min_utility, signature(), 0)
            .expect("coined");
        // Just below the coinage bar but above the retirement bar: it stays.
        registry.set_utility(id, (rules.min_utility + rules.retire_below) / 2.0);
        assert!(registry.audit().is_empty());
    }

    #[test]
    fn a_full_registry_displaces_a_worse_concept_but_not_a_better_one() {
        let mut registry = Registry::new(CoinageRules {
            max_concepts: 2,
            ..CoinageRules::default()
        });
        registry
            .coin(&state(1, 1000, 0.9), 0.9, signature(), 0)
            .expect("coined");
        registry
            .coin(&state(2, 1000, 0.9), 0.8, signature(), 0)
            .expect("coined");
        // Worse than everything held: refused.
        let refusal = registry
            .coin(&state(3, 1000, 0.9), 0.1, signature(), 0)
            .unwrap_err();
        assert!(matches!(refusal, Refusal::NoRoom));
        // Better than the worst held: displaces it.
        assert!(registry
            .coin(&state(4, 1000, 0.9), 0.95, signature(), 0)
            .is_ok());
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn a_resemblance_note_never_changes_the_concepts_name() {
        // The naming rule. If a coincidence with a human word renamed the
        // concept, the coincidence would stop being a finding.
        let mut registry = Registry::default();
        let id = registry
            .coin(&state(1, 1000, 0.9), 0.4, signature(), 0)
            .expect("coined");
        registry.note_resemblance(id, "thermal throttling");
        let concept = registry.get(id).expect("present");
        assert_eq!(concept.id.to_string(), "concept_1");
        assert!(concept.describe().starts_with("concept_1"));
        assert!(concept.describe().contains("a human noted"));
    }

    #[test]
    fn a_concept_adopted_by_analogy_still_had_to_earn_it_here() {
        // Being told a concept exists elsewhere is not evidence it exists here.
        let mut registry = Registry::default();
        assert!(registry
            .coin_by_analogy(&state(1, 2, 0.9), 0.9, signature(), "Z31", 0.95, 0)
            .is_err());

        let id = registry
            .coin_by_analogy(&state(2, 1000, 0.9), 0.4, signature(), "Z31", 0.95, 0)
            .expect("coined");
        assert!(matches!(
            registry.get(id).unwrap().origin,
            Origin::Analogy { .. }
        ));
        assert!(registry.get(id).unwrap().describe().contains("concept_"));
    }

    #[test]
    fn only_live_concepts_are_exportable() {
        let mut registry = Registry::default();
        let a = registry
            .coin(&state(1, 1000, 0.9), 0.4, signature(), 0)
            .expect("coined");
        registry
            .coin(&state(2, 1000, 0.9), 0.4, signature(), 0)
            .expect("coined");
        registry.set_utility(a, 0.0);
        registry.audit();
        let exported = registry.exportable();
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].0, "concept_2");
    }

    #[test]
    fn an_empty_registry_says_why_rather_than_looking_broken() {
        let registry = Registry::default();
        assert!(registry.render().contains("improves prediction"));
    }

    #[test]
    fn a_signature_built_from_a_state_is_dimensionless() {
        let catalogue = LatentCatalogue::new(1.0, 8);
        let signature = signature_for(&state(1, 100, 0.9), &catalogue, 8, 1_000_000.0, 0.5);
        assert!(signature.is_comparable());
        for feature in signature.features() {
            assert!(feature.is_finite());
            assert!(feature >= 0.0, "{feature} should not be negative");
        }
        // Two of four components are distinctive.
        assert!((signature.distinctiveness - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_signature_survives_a_machine_with_no_dwell_history() {
        let catalogue = LatentCatalogue::new(1.0, 8);
        let signature = signature_for(&state(1, 100, 0.9), &catalogue, 8, 0.0, 0.5);
        assert!(signature.is_comparable());
    }

    #[test]
    fn a_registry_round_trips_through_json() {
        let mut registry = Registry::default();
        registry
            .coin(&state(1, 1000, 0.9), 0.4, signature(), 0)
            .expect("coined");
        let text = serde_json::to_string(&registry).expect("serialises");
        let back: Registry = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back, registry);
    }
}

#[cfg(test)]
mod decay_tests {
    use super::tests_support::*;
    use super::*;

    #[test]
    fn a_concept_that_stops_occurring_loses_standing_and_is_retired() {
        // The gap a four-arm experiment exposed: utility is measured over
        // accumulated evidence, so without this a concept whose regime vanished
        // would keep its old score forever and keep driving behaviour.
        let mut registry = Registry::default();
        let id = registry
            .coin(&state(1, 1000, 0.9), 0.4, signature(), 0)
            .expect("coined");
        assert!(registry.audit().is_empty());

        // Four half-lives: 0.4 -> 0.025. Decayed, but the default retirement
        // bar is 0.01, so it survives. Absence weakens a concept gradually
        // rather than killing it the moment it stops occurring.
        registry.decay_unseen(4_000, 1_000);
        let after_four = registry.get(id).unwrap().predictive_utility;
        assert!((after_four - 0.025).abs() < 1e-9, "got {after_four}");
        assert!(
            registry.audit().is_empty(),
            "four absences is not yet death"
        );

        // Six: 0.4 -> 0.00625, below the bar, and the ordinary audit does the
        // retiring rather than the decay doing it directly.
        registry.decay_unseen(6_000, 1_000);
        assert!(registry.get(id).unwrap().predictive_utility < 0.01);
        assert_eq!(registry.audit(), vec![id]);
    }

    #[test]
    fn a_concept_still_occurring_does_not_decay() {
        let mut registry = Registry::default();
        let id = registry
            .coin(&state(1, 1000, 0.9), 0.4, signature(), 0)
            .expect("coined");
        registry.observe(id, 3_900);
        registry.decay_unseen(4_000, 1_000);
        assert!((registry.get(id).unwrap().predictive_utility - 0.4).abs() < 1e-9);
        assert!(registry.audit().is_empty());
    }

    #[test]
    fn decay_is_gradual_rather_than_a_cliff() {
        let mut registry = Registry::default();
        let id = registry
            .coin(&state(1, 1000, 0.9), 0.4, signature(), 0)
            .expect("coined");
        registry.decay_unseen(1_000, 1_000);
        let after_one = registry.get(id).unwrap().predictive_utility;
        assert!((after_one - 0.2).abs() < 1e-9, "got {after_one}");
        // Still above the retirement bar: one absence is not a death sentence.
        assert!(registry.audit().is_empty());
    }

    #[test]
    fn a_zero_half_life_decays_nothing() {
        let mut registry = Registry::default();
        registry
            .coin(&state(1, 1000, 0.9), 0.4, signature(), 0)
            .expect("coined");
        assert_eq!(registry.decay_unseen(10_000, 0), 0);
    }

    #[test]
    fn a_retired_concept_is_not_decayed_further() {
        let mut registry = Registry::default();
        let id = registry
            .coin(&state(1, 1000, 0.9), 0.4, signature(), 0)
            .expect("coined");
        registry.set_utility(id, 0.0);
        registry.audit();
        assert_eq!(registry.decay_unseen(100_000, 1_000), 0);
    }
}
