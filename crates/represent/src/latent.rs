//! Latent states: categories the machine derives for itself.
//!
//! # The research target
//!
//! > Can a machine construct useful categories of its own computational
//! > existence?
//!
//! A [`LatentState`] is a recurring configuration of the whole machine that was
//! not named by anyone. It might involve a particular thermal distribution, a
//! certain idle pattern, a frequency trajectory and an interrupt skew all at
//! once. If such a configuration recurs, persists, and improves prediction, it
//! is a *concept*, and this crate lets it become one: it gets an identifier,
//! an occurrence history, and a record of how it responds to actions.
//!
//! Crucially it does **not** get translated back into human vocabulary.
//! `latent_state_13` stays `latent_state_13`. Naming it "the throttled state"
//! would be a hypothesis dressed as an observation, and would quietly discard
//! whatever part of it does not fit the name.
//!
//! # Online, not batch
//!
//! States are discovered incrementally: each reflection is assigned to the
//! nearest known state, or founds a new one if it is far enough from all of
//! them. Batch clustering would require deciding in advance how many kinds of
//! state a machine has, which is exactly the assumption worth avoiding, and
//! would make a state discovered on Tuesday unrecognisable on Wednesday.
//!
//! The cost is order-dependence: the states found depend on what the machine
//! did first. [`LatentCatalogue::consolidate`] mitigates it by merging states
//! that have drifted together, and the order dependence is documented rather
//! than papered over.
//!
//! # Distance
//!
//! Root-mean-square distance in normalised space, so the threshold means the
//! same thing regardless of how many channels a machine exposes. A raw
//! Euclidean threshold would have to be retuned for every machine, and a state
//! found on an 8-CPU laptop would not transfer to a 128-CPU server.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Identifier of a machine-derived state.
///
/// Deliberately opaque and numeric. It renders as `latent_state_13`, which is
/// a name the machine can use without it implying a meaning it has not earned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LatentStateId(pub u32);

impl std::fmt::Display for LatentStateId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "latent_state_{}", self.0)
    }
}

/// How a latent state responded to a family of actions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionResponse {
    /// The action family, e.g. `affinity`.
    pub family: String,
    /// Times an action of this family was taken while in this state.
    pub trials: u32,
    /// Times the machine left this state within the observation window after.
    pub departures: u32,
    /// Mean change in the observed objective, where one was supplied.
    /// Sign convention is the caller's; the catalogue only aggregates.
    pub mean_effect: f64,
}

impl ActionResponse {
    /// Fraction of trials after which the machine left this state.
    ///
    /// A state that reliably ends when acted upon is a controllable one, which
    /// matters for self-boundary inference.
    pub fn departure_rate(&self) -> f64 {
        if self.trials == 0 {
            0.0
        } else {
            self.departures as f64 / self.trials as f64
        }
    }
}

/// One state the machine discovered in itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatentState {
    pub id: LatentStateId,
    /// The state's definition: a point in normalised channel space.
    pub centroid: Vec<f64>,
    /// Mean distance of member reflections from the centroid. How tight it is.
    pub radius: f64,
    /// Times the machine entered this state.
    pub entries: u64,
    /// Reflections assigned to it in total.
    pub occupancy: u64,
    /// Total time spent in it.
    pub dwell_ns: u64,
    pub first_seen_ns: u64,
    pub last_seen_ns: u64,
    /// How much knowing the machine is in this state improves prediction,
    /// relative to not knowing. Filled in by the self-model; zero until then.
    pub predictive_utility: f64,
    /// Confidence that this is a real, persistent state rather than a handful
    /// of coincidental reflections.
    pub confidence: f64,
    /// How it responds to each family of action tried while in it.
    pub action_responses: Vec<ActionResponse>,
}

impl LatentState {
    /// Mean dwell per entry: how long the machine stays once it arrives.
    ///
    /// A state entered constantly and left immediately is a threshold artefact,
    /// not a regime.
    pub fn mean_dwell_ns(&self) -> f64 {
        if self.entries == 0 {
            0.0
        } else {
            self.dwell_ns as f64 / self.entries as f64
        }
    }

    /// Root-mean-square distance from this state's centroid.
    pub fn distance(&self, point: &[f64]) -> f64 {
        rms_distance(&self.centroid, point)
    }

    pub fn response_to(&self, family: &str) -> Option<&ActionResponse> {
        self.action_responses.iter().find(|r| r.family == family)
    }
}

/// Everything the machine has learned to recognise about itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatentCatalogue {
    states: Vec<LatentState>,
    /// `transitions[a][b]`: times the machine went from state `a` to `b`.
    transitions: Vec<Vec<u64>>,
    /// Distance beyond which a reflection founds a new state.
    novelty_threshold: f64,
    /// Ceiling on how many states may exist, so a noisy machine cannot mint a
    /// new concept every tick.
    max_states: usize,
    /// Reflections seen.
    observations: u64,
    /// Reflections that founded a new state.
    novelties: u64,
    /// Reflections rejected as novel because the catalogue was full.
    suppressed: u64,
    next_id: u32,
    current: Option<LatentStateId>,
    current_since_ns: u64,
}

impl Default for LatentCatalogue {
    fn default() -> Self {
        LatentCatalogue::new(0.75, 32)
    }
}

impl LatentCatalogue {
    /// A new, empty catalogue.
    ///
    /// `novelty_threshold` is in units of normalised standard deviations per
    /// channel. 0.75 means "this reflection differs from every known state by
    /// about three quarters of a typical channel's spread, averaged over all
    /// channels", which is a large difference and yields few, broad states.
    pub fn new(novelty_threshold: f64, max_states: usize) -> LatentCatalogue {
        LatentCatalogue {
            states: Vec::new(),
            transitions: Vec::new(),
            novelty_threshold: novelty_threshold.max(1e-6),
            max_states: max_states.max(1),
            observations: 0,
            novelties: 0,
            suppressed: 0,
            next_id: 0,
            current: None,
            current_since_ns: 0,
        }
    }

    pub fn states(&self) -> &[LatentState] {
        &self.states
    }

    pub fn len(&self) -> usize {
        self.states.len()
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    pub fn observations(&self) -> u64 {
        self.observations
    }

    pub fn novelties(&self) -> u64 {
        self.novelties
    }

    /// Reflections that would have founded a state but could not, because the
    /// catalogue was full. A large number means the machine is more varied than
    /// the budget allows and the threshold or ceiling should be revisited.
    pub fn suppressed(&self) -> u64 {
        self.suppressed
    }

    pub fn current(&self) -> Option<LatentStateId> {
        self.current
    }

    pub fn get(&self, id: LatentStateId) -> Option<&LatentState> {
        self.states.iter().find(|s| s.id == id)
    }

    fn index_of(&self, id: LatentStateId) -> Option<usize> {
        self.states.iter().position(|s| s.id == id)
    }

    /// Assign a reflection, creating a state if it is unlike anything known.
    ///
    /// Returns the state it landed in and whether that state is new.
    pub fn observe(&mut self, point: &[f64], monotonic_ns: u64) -> (LatentStateId, bool) {
        self.observations += 1;

        let nearest = self
            .states
            .iter()
            .enumerate()
            .map(|(index, state)| (index, state.distance(point)))
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let (index, novel) = match nearest {
            Some((index, distance)) if distance <= self.novelty_threshold => (index, false),
            _ => {
                if self.states.len() >= self.max_states {
                    // Full. Fall back to the nearest state rather than
                    // discarding the observation: the machine is somewhere, and
                    // the least-wrong answer is the closest thing it knows.
                    self.suppressed += 1;
                    match nearest {
                        Some((index, _)) => (index, false),
                        None => return (LatentStateId(0), false),
                    }
                } else {
                    self.novelties += 1;
                    let id = LatentStateId(self.next_id);
                    self.next_id += 1;
                    self.states.push(LatentState {
                        id,
                        centroid: point.to_vec(),
                        radius: 0.0,
                        entries: 0,
                        occupancy: 0,
                        dwell_ns: 0,
                        first_seen_ns: monotonic_ns,
                        last_seen_ns: monotonic_ns,
                        predictive_utility: 0.0,
                        confidence: 0.0,
                        action_responses: Vec::new(),
                    });
                    self.grow_transitions();
                    (self.states.len() - 1, true)
                }
            }
        };

        let id = self.states[index].id;

        // Update the definition towards this reflection. A decaying rate, so an
        // established state is not yanked around by one sample while a new one
        // still moves freely.
        {
            let occupancy = self.states[index].occupancy;
            let rate = (1.0 / (occupancy as f64 + 1.0)).max(0.01);
            let distance = self.states[index].distance(point);
            let state = &mut self.states[index];
            for (slot, value) in state.centroid.iter_mut().zip(point) {
                if value.is_finite() {
                    *slot += rate * (value - *slot);
                }
            }
            state.radius += rate * (distance - state.radius);
            state.occupancy += 1;
            state.last_seen_ns = monotonic_ns;
        }

        // Transition and dwell accounting.
        match self.current {
            Some(previous) if previous == id => {
                let dwell = monotonic_ns.saturating_sub(self.current_since_ns);
                if let Some(index) = self.index_of(id) {
                    self.states[index].dwell_ns += dwell;
                }
                self.current_since_ns = monotonic_ns;
            }
            Some(previous) => {
                if let (Some(from), Some(to)) = (self.index_of(previous), self.index_of(id)) {
                    self.transitions[from][to] += 1;
                }
                self.states[index].entries += 1;
                self.current = Some(id);
                self.current_since_ns = monotonic_ns;
            }
            None => {
                self.states[index].entries += 1;
                self.current = Some(id);
                self.current_since_ns = monotonic_ns;
            }
        }

        self.recompute_confidence();
        (id, novel)
    }

    fn grow_transitions(&mut self) {
        let n = self.states.len();
        for row in self.transitions.iter_mut() {
            row.resize(n, 0);
        }
        self.transitions.resize(n, vec![0; n]);
    }

    /// Confidence that each state is real rather than coincidence.
    ///
    /// Rises with how often it recurs and how long the machine stays, falls
    /// when it is diffuse. A state seen once for one tick has no confidence
    /// however far it sits from everything else.
    fn recompute_confidence(&mut self) {
        let total = self.observations.max(1) as f64;
        for state in self.states.iter_mut() {
            let share = state.occupancy as f64 / total;
            // Recurrence matters more than raw occupancy: a state entered
            // repeatedly is a regime, a state occupied once for a long time is
            // an episode.
            let recurrence = (state.entries as f64 / 5.0).min(1.0);
            let tightness = if state.radius <= 0.0 {
                1.0
            } else {
                (1.0 / (1.0 + state.radius)).clamp(0.0, 1.0)
            };
            state.confidence =
                (share.min(1.0) * 0.3 + recurrence * 0.5 + tightness * 0.2).clamp(0.0, 1.0);
        }
    }

    /// Merge states whose centroids have drifted within half the novelty
    /// threshold of each other.
    ///
    /// Online clustering creates states in the order the machine happens to
    /// visit them, and two that were distinct when founded can converge as
    /// their definitions move. Returns how many merges happened.
    pub fn consolidate(&mut self) -> usize {
        let mut merged = 0;
        let mut index = 0;
        while index < self.states.len() {
            let mut other = index + 1;
            while other < self.states.len() {
                let distance =
                    rms_distance(&self.states[index].centroid, &self.states[other].centroid);
                if distance <= self.novelty_threshold * 0.5 {
                    let absorbed = self.states.remove(other);
                    let keeper = &mut self.states[index];
                    let total = (keeper.occupancy + absorbed.occupancy).max(1) as f64;
                    let weight = absorbed.occupancy as f64 / total;
                    for (slot, value) in keeper.centroid.iter_mut().zip(&absorbed.centroid) {
                        *slot += weight * (value - *slot);
                    }
                    keeper.occupancy += absorbed.occupancy;
                    keeper.entries += absorbed.entries;
                    keeper.dwell_ns += absorbed.dwell_ns;
                    keeper.radius = keeper.radius.max(absorbed.radius);
                    keeper.first_seen_ns = keeper.first_seen_ns.min(absorbed.first_seen_ns);
                    keeper.last_seen_ns = keeper.last_seen_ns.max(absorbed.last_seen_ns);
                    keeper.predictive_utility =
                        keeper.predictive_utility.max(absorbed.predictive_utility);
                    if self.current == Some(absorbed.id) {
                        self.current = Some(keeper.id);
                    }
                    merged += 1;
                } else {
                    other += 1;
                }
            }
            index += 1;
        }
        if merged > 0 {
            // The transition matrix indexes by position, so it cannot survive a
            // removal. Losing the counts is the honest outcome: they described
            // a set of states that no longer exists.
            let n = self.states.len();
            self.transitions = vec![vec![0; n]; n];
            self.recompute_confidence();
        }
        merged
    }

    /// Drop states that never recurred and hold almost no occupancy.
    ///
    /// Returns the discarded ids, so a caller can note that a concept the
    /// machine briefly entertained has been abandoned.
    pub fn prune(&mut self, min_entries: u64, min_confidence: f64) -> Vec<LatentStateId> {
        let discarded: Vec<LatentStateId> = self
            .states
            .iter()
            .filter(|s| s.entries < min_entries && s.confidence < min_confidence)
            .map(|s| s.id)
            .collect();
        if discarded.is_empty() {
            return discarded;
        }
        self.states.retain(|s| !discarded.contains(&s.id));
        if self.current.is_some_and(|c| discarded.contains(&c)) {
            self.current = None;
        }
        let n = self.states.len();
        self.transitions = vec![vec![0; n]; n];
        discarded
    }

    /// Record that an action was taken while in a state, and what followed.
    pub fn record_action(
        &mut self,
        state: LatentStateId,
        family: &str,
        departed: bool,
        effect: f64,
    ) {
        let Some(index) = self.index_of(state) else {
            return;
        };
        let responses = &mut self.states[index].action_responses;
        match responses.iter_mut().find(|r| r.family == family) {
            Some(response) => {
                let n = response.trials as f64;
                response.trials += 1;
                response.departures += u32::from(departed);
                if effect.is_finite() {
                    response.mean_effect = (response.mean_effect * n + effect) / (n + 1.0);
                }
            }
            None => responses.push(ActionResponse {
                family: family.to_string(),
                trials: 1,
                departures: u32::from(departed),
                mean_effect: if effect.is_finite() { effect } else { 0.0 },
            }),
        }
    }

    /// Attach a predictive utility measured by the self-model.
    pub fn set_predictive_utility(&mut self, state: LatentStateId, utility: f64) {
        if let Some(index) = self.index_of(state) {
            self.states[index].predictive_utility = utility;
        }
    }

    /// Which state most often follows another.
    pub fn most_likely_successor(&self, state: LatentStateId) -> Option<(LatentStateId, f64)> {
        let index = self.index_of(state)?;
        let row = self.transitions.get(index)?;
        let total: u64 = row.iter().sum();
        if total == 0 {
            return None;
        }
        row.iter()
            .enumerate()
            .max_by_key(|(_, count)| **count)
            .filter(|(_, count)| **count > 0)
            .map(|(to, count)| (self.states[to].id, *count as f64 / total as f64))
    }

    /// Transition counts, by state id.
    pub fn transition_counts(&self) -> BTreeMap<(LatentStateId, LatentStateId), u64> {
        let mut out = BTreeMap::new();
        for (from, row) in self.transitions.iter().enumerate() {
            for (to, count) in row.iter().enumerate() {
                if *count > 0 {
                    out.insert((self.states[from].id, self.states[to].id), *count);
                }
            }
        }
        out
    }

    /// States worth keeping, ordered by how useful they have proved.
    ///
    /// Predictive utility first, because a state that improves prediction has
    /// earned its place regardless of how often it occurs; then confidence.
    pub fn ranked(&self) -> Vec<&LatentState> {
        let mut states: Vec<&LatentState> = self.states.iter().collect();
        states.sort_by(|a, b| {
            b.predictive_utility
                .partial_cmp(&a.predictive_utility)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(
                    b.confidence
                        .partial_cmp(&a.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(a.id.cmp(&b.id))
        });
        states
    }
}

/// Root-mean-square distance, so a threshold means the same thing whatever the
/// dimensionality.
pub fn rms_distance(a: &[f64], b: &[f64]) -> f64 {
    let mut sum = 0.0;
    let mut count = 0usize;
    for (x, y) in a.iter().zip(b) {
        if x.is_finite() && y.is_finite() {
            sum += (x - y).powi(2);
            count += 1;
        }
    }
    if count == 0 {
        return f64::INFINITY;
    }
    (sum / count as f64).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(values: &[f64]) -> Vec<f64> {
        values.to_vec()
    }

    #[test]
    fn the_first_reflection_founds_a_state() {
        let mut catalogue = LatentCatalogue::new(0.75, 8);
        let (id, novel) = catalogue.observe(&point(&[0.0, 0.0]), 0);
        assert!(novel);
        assert_eq!(id, LatentStateId(0));
        assert_eq!(catalogue.len(), 1);
        assert_eq!(catalogue.current(), Some(id));
    }

    #[test]
    fn similar_reflections_join_the_same_state() {
        let mut catalogue = LatentCatalogue::new(0.75, 8);
        catalogue.observe(&point(&[0.0, 0.0]), 0);
        let (id, novel) = catalogue.observe(&point(&[0.1, 0.1]), 100);
        assert!(!novel);
        assert_eq!(id, LatentStateId(0));
        assert_eq!(catalogue.len(), 1);
    }

    #[test]
    fn a_genuinely_different_reflection_founds_a_new_state() {
        let mut catalogue = LatentCatalogue::new(0.75, 8);
        catalogue.observe(&point(&[0.0, 0.0]), 0);
        let (id, novel) = catalogue.observe(&point(&[5.0, 5.0]), 100);
        assert!(novel);
        assert_eq!(id, LatentStateId(1));
        assert_eq!(catalogue.len(), 2);
    }

    #[test]
    fn the_catalogue_cannot_grow_without_bound() {
        // A noisy machine must not mint a concept every tick.
        let mut catalogue = LatentCatalogue::new(0.1, 3);
        for i in 0..50 {
            catalogue.observe(&point(&[i as f64 * 10.0, 0.0]), i as u64 * 100);
        }
        assert_eq!(catalogue.len(), 3);
        assert!(catalogue.suppressed() > 0, "suppression must be reported");
    }

    #[test]
    fn recurrence_and_dwell_are_tracked() {
        let mut catalogue = LatentCatalogue::new(0.75, 8);
        // Alternate between two regimes.
        for i in 0..10u64 {
            let value = if i % 2 == 0 { 0.0 } else { 5.0 };
            catalogue.observe(&point(&[value, value]), i * 1_000_000);
        }
        assert_eq!(catalogue.len(), 2);
        for state in catalogue.states() {
            assert!(state.entries > 1, "a regime should be re-entered");
            assert!(state.confidence > 0.0);
        }
    }

    #[test]
    fn transitions_between_states_are_counted() {
        let mut catalogue = LatentCatalogue::new(0.75, 8);
        for i in 0..10u64 {
            let value = if i % 2 == 0 { 0.0 } else { 5.0 };
            catalogue.observe(&point(&[value, value]), i * 1_000_000);
        }
        let counts = catalogue.transition_counts();
        assert!(!counts.is_empty());
        let (successor, probability) = catalogue
            .most_likely_successor(LatentStateId(0))
            .expect("state 0 leads somewhere");
        assert_eq!(successor, LatentStateId(1));
        assert!(probability > 0.9, "it always alternates");
    }

    #[test]
    fn a_state_seen_once_has_low_confidence() {
        let mut catalogue = LatentCatalogue::new(0.75, 8);
        for i in 0..30u64 {
            catalogue.observe(&point(&[0.0, 0.0]), i * 1000);
        }
        catalogue.observe(&point(&[9.0, 9.0]), 30_000);
        let rare = catalogue.get(LatentStateId(1)).unwrap();
        let common = catalogue.get(LatentStateId(0)).unwrap();
        assert!(
            rare.confidence < common.confidence,
            "a one-off should not be as trusted as a regime: {} vs {}",
            rare.confidence,
            common.confidence
        );
    }

    #[test]
    fn drifted_states_can_be_consolidated() {
        // Leader clustering founds states at least `threshold` apart, so two
        // states can only converge by their *definitions* drifting. Feed each
        // one a stream just inside its own side of the midpoint: both centroids
        // walk toward the middle until they are the same thing wearing two
        // names, which is exactly what consolidation is for.
        let mut catalogue = LatentCatalogue::new(2.0, 8);
        catalogue.observe(&point(&[0.0, 0.0]), 0);
        catalogue.observe(&point(&[4.0, 4.0]), 100);
        assert_eq!(
            catalogue.len(),
            2,
            "founded 4.0 apart, beyond the threshold"
        );

        // Two streams, each staying on its own side of the midpoint so it keeps
        // joining its own state, both closing in. This is a machine whose two
        // regimes gradually become the same regime.
        for i in 0..200u64 {
            let offset = 2.0 * 0.90_f64.powi((i / 2) as i32);
            let value = if i % 2 == 0 {
                2.0 - offset
            } else {
                2.0 + offset
            };
            catalogue.observe(&point(&[value, value]), 200 + i * 100);
        }
        assert_eq!(catalogue.len(), 2, "no third state should appear");

        let separation = rms_distance(
            &catalogue.states()[0].centroid,
            &catalogue.states()[1].centroid,
        );
        assert!(
            separation < 2.0,
            "the definitions should have converged, still {separation} apart"
        );
        assert_eq!(catalogue.consolidate(), 1);
        assert_eq!(catalogue.len(), 1);
    }

    #[test]
    fn consolidation_leaves_distinct_states_alone() {
        // The counterpart: merging must not be so eager that it destroys real
        // structure.
        let mut catalogue = LatentCatalogue::new(0.75, 8);
        for i in 0..20u64 {
            let value = if i % 2 == 0 { 0.0 } else { 6.0 };
            catalogue.observe(&point(&[value, value]), i * 1000);
        }
        assert_eq!(catalogue.len(), 2);
        assert_eq!(catalogue.consolidate(), 0);
        assert_eq!(catalogue.len(), 2);
    }

    #[test]
    fn abandoned_concepts_can_be_pruned() {
        let mut catalogue = LatentCatalogue::new(0.75, 8);
        for i in 0..30u64 {
            catalogue.observe(&point(&[0.0, 0.0]), i * 1000);
        }
        catalogue.observe(&point(&[9.0, 9.0]), 40_000);
        let discarded = catalogue.prune(2, 0.5);
        assert_eq!(discarded, vec![LatentStateId(1)]);
        assert_eq!(catalogue.len(), 1);
    }

    #[test]
    fn action_responses_accumulate_per_family() {
        let mut catalogue = LatentCatalogue::new(0.75, 8);
        let (id, _) = catalogue.observe(&point(&[0.0, 0.0]), 0);
        catalogue.record_action(id, "affinity", true, -0.1);
        catalogue.record_action(id, "affinity", false, -0.3);
        catalogue.record_action(id, "priority", false, 0.0);

        let state = catalogue.get(id).unwrap();
        let affinity = state.response_to("affinity").unwrap();
        assert_eq!(affinity.trials, 2);
        assert_eq!(affinity.departures, 1);
        assert!((affinity.mean_effect - -0.2).abs() < 1e-9);
        assert!((affinity.departure_rate() - 0.5).abs() < 1e-9);
        assert!(state.response_to("numa").is_none());
    }

    #[test]
    fn ranking_puts_predictive_states_first() {
        let mut catalogue = LatentCatalogue::new(0.75, 8);
        catalogue.observe(&point(&[0.0, 0.0]), 0);
        catalogue.observe(&point(&[5.0, 5.0]), 100);
        catalogue.set_predictive_utility(LatentStateId(1), 0.8);
        assert_eq!(catalogue.ranked()[0].id, LatentStateId(1));
    }

    #[test]
    fn a_catalogue_survives_serialisation() {
        // A discovered ontology has to outlive the process that found it, or
        // the machine starts from nothing every boot.
        let mut catalogue = LatentCatalogue::new(0.75, 8);
        for i in 0..10u64 {
            let value = if i % 2 == 0 { 0.0 } else { 5.0 };
            catalogue.observe(&point(&[value, value]), i * 1000);
        }
        catalogue.record_action(LatentStateId(0), "affinity", true, -0.2);
        let json = serde_json::to_string(&catalogue).unwrap();
        let back: LatentCatalogue = serde_json::from_str(&json).unwrap();
        assert_eq!(catalogue, back);
        assert_eq!(
            back.get(LatentStateId(0)).unwrap().action_responses.len(),
            1
        );
    }

    #[test]
    fn distance_is_dimension_independent() {
        // The property that lets a threshold transfer between machines with
        // different channel counts.
        let two = rms_distance(&[0.0, 0.0], &[1.0, 1.0]);
        let ten = rms_distance(&[0.0; 10], &[1.0; 10]);
        assert!((two - ten).abs() < 1e-9);
    }

    #[test]
    fn incomparable_points_are_infinitely_far_apart() {
        assert!(rms_distance(&[f64::NAN], &[f64::NAN]).is_infinite());
    }

    #[test]
    fn the_id_renders_as_a_machine_derived_name() {
        assert_eq!(LatentStateId(13).to_string(), "latent_state_13");
    }
}
