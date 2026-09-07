//! Learning what actions do, from having done them.

use std::collections::BTreeMap;

use corescout_intent::{Achievement, Intent, Objective};
use corescout_mirror::MirrorSnapshot;
use serde::{Deserialize, Serialize};

use crate::outcome::{score, Confidence, Outcome, Prediction, Support};

/// What an effect is indexed by.
///
/// `(family, target entity, channel)`. Not the exact action: pinning to CPU 3
/// and pinning to CPU 5 are the same *kind* of intervention, and treating them
/// as unrelated would mean never accumulating enough trials to learn anything.
/// The target entity is what distinguishes them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EffectKey {
    pub family: String,
    /// Entity row the action was aimed at, where it named one.
    pub target_row: Option<usize>,
    pub channel: usize,
}

/// The distribution of changes seen after one kind of action.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ObservedEffect {
    pub trials: u32,
    /// Mean change in the channel's own unit.
    pub mean_delta: f64,
    /// Mean squared change, for the spread.
    mean_square: f64,
}

impl ObservedEffect {
    fn observe(&mut self, delta: f64) {
        if !delta.is_finite() {
            return;
        }
        let n = self.trials as f64;
        self.trials += 1;
        self.mean_delta = (self.mean_delta * n + delta) / (n + 1.0);
        self.mean_square = (self.mean_square * n + delta * delta) / (n + 1.0);
    }

    pub fn spread(&self) -> f64 {
        (self.mean_square - self.mean_delta * self.mean_delta)
            .max(0.0)
            .sqrt()
    }

    /// How reproducible the effect is: 1.0 when every trial agreed, falling to
    /// 0.0 as the spread comes to dominate the effect.
    ///
    /// An effect whose spread exceeds its magnitude is indistinguishable from
    /// noise however many trials produced it, and this is what says so.
    ///
    /// The denominator includes the spread so that "this action reliably does
    /// nothing" scores 1.0. Dividing by the magnitude alone made a
    /// perfectly reproducible zero effect look maximally inconsistent, which
    /// then dragged down the confidence of every action that left some channel
    /// untouched.
    pub fn consistency(&self) -> f64 {
        if self.trials < 2 {
            return 0.0;
        }
        let spread = self.spread();
        let denominator = self.mean_delta.abs() + spread;
        if denominator <= 1e-12 {
            // No effect and no variation: perfectly reproducible.
            return 1.0;
        }
        (1.0 - (spread / denominator)).clamp(0.0, 1.0)
    }
}

/// How a reflection maps to the objectives an intent names.
///
/// A function rather than a table, and supplied by the caller rather than
/// built in, because which channel stands for "latency" on a given machine is a
/// judgement about that machine's sensors. Baking it in here would reintroduce
/// exactly the hardcoded ontology the project exists to avoid.
///
/// The slice carries hypothetical cell deltas, so the same function scores both
/// what is happening and what would happen.
pub type Measure<'a> = &'a dyn Fn(&MirrorSnapshot, &[(usize, usize, f64)]) -> Achievement;

/// One action worth considering: its family, the entity it acts on, and a
/// description for the audit trail.
pub type Candidate = (String, Option<usize>, String);

/// What this system has learned about the consequences of its own actions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EffectModel {
    #[serde(with = "corescout_core::serde_util::pairs")]
    effects: BTreeMap<EffectKey, ObservedEffect>,
    /// Trials per `(family, target)`, for deciding support.
    #[serde(with = "corescout_core::serde_util::pairs")]
    trials: BTreeMap<(String, Option<usize>), u32>,
    /// Observations folded in.
    updates: u64,
}

impl EffectModel {
    pub fn new() -> EffectModel {
        EffectModel::default()
    }

    pub fn updates(&self) -> u64 {
        self.updates
    }

    pub fn len(&self) -> usize {
        self.effects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    /// Learn from one action and what followed it.
    ///
    /// `before` and `after` must be from the same epoch, or the rows describe
    /// different hardware and the difference is meaningless.
    pub fn learn(
        &mut self,
        family: &str,
        target_row: Option<usize>,
        before: &MirrorSnapshot,
        after: &MirrorSnapshot,
    ) -> bool {
        if before.epoch != after.epoch {
            return false;
        }
        let rows = after.state.rows().min(before.state.rows());
        let cols = after.state.cols().min(before.state.cols());
        if rows == 0 || cols == 0 {
            return false;
        }

        *self
            .trials
            .entry((family.to_string(), target_row))
            .or_insert(0) += 1;

        for row in 0..rows {
            for col in 0..cols {
                let a = before.state.get(row, col);
                let b = after.state.get(row, col);
                if !a.is_finite() || !b.is_finite() {
                    continue;
                }
                // Effects are indexed by the *acted-on* entity, not by every
                // entity that happened to move. Attributing a change on an
                // unrelated core to this action is how a model learns
                // superstitions.
                if target_row.is_some_and(|target| target != row) {
                    continue;
                }
                self.effects
                    .entry(EffectKey {
                        family: family.to_string(),
                        target_row,
                        channel: col,
                    })
                    .or_default()
                    .observe(b - a);
            }
        }
        self.updates += 1;
        true
    }

    /// What is known about one exact effect.
    pub fn effect(
        &self,
        family: &str,
        target_row: Option<usize>,
        channel: usize,
    ) -> Option<&ObservedEffect> {
        self.effects.get(&EffectKey {
            family: family.to_string(),
            target_row,
            channel,
        })
    }

    /// Trials for a family and target.
    pub fn trials(&self, family: &str, target_row: Option<usize>) -> u32 {
        self.trials
            .get(&(family.to_string(), target_row))
            .copied()
            .unwrap_or(0)
    }

    /// How well supported a prediction about this action would be.
    pub fn support_for(&self, family: &str, target_row: Option<usize>) -> Support {
        if self.trials(family, target_row) > 0 {
            return Support::Observed;
        }
        // Has anything in this family ever been tried, on any target? If so the
        // effect can be transferred, with less confidence.
        let transferable = self
            .trials
            .iter()
            .any(|((f, _), count)| f == family && *count > 0);
        if transferable {
            Support::Transferred
        } else {
            Support::Extrapolated
        }
    }

    /// Mean effect of a family across every target it has been tried on.
    ///
    /// The basis of a transferred prediction: "moving a thread usually does
    /// roughly this, though never yet to that entity".
    fn transferred_effect(&self, family: &str, channel: usize) -> Option<ObservedEffect> {
        let matching: Vec<&ObservedEffect> = self
            .effects
            .iter()
            .filter(|(key, _)| key.family == family && key.channel == channel)
            .map(|(_, effect)| effect)
            .collect();
        if matching.is_empty() {
            return None;
        }
        let trials: u32 = matching.iter().map(|e| e.trials).sum();
        let mean = matching.iter().map(|e| e.mean_delta).sum::<f64>() / matching.len() as f64;
        let mean_square =
            matching.iter().map(|e| e.mean_square).sum::<f64>() / matching.len() as f64;
        Some(ObservedEffect {
            trials,
            mean_delta: mean,
            mean_square,
        })
    }

    /// Imagine the consequences of an action.
    pub fn imagine(
        &self,
        family: &str,
        target_row: Option<usize>,
        current: &MirrorSnapshot,
        horizon_ns: u64,
    ) -> Outcome {
        let support = self.support_for(family, target_row);
        if support == Support::Extrapolated {
            // Nothing in this family has ever been tried. The honest answer is
            // that nothing is known, rather than a prediction of no change.
            return Outcome {
                deltas: Vec::new(),
                horizon_ns,
                confidence: Confidence::from(Support::Extrapolated, 0, 0.0),
            };
        }

        let cols = current.state.cols();
        let mut deltas = Vec::new();
        let mut trials = 0u32;
        // Consistency is summarised across channels weighted by how much is
        // happening in each, where "happening" is effect size *plus* spread.
        //
        // An unweighted mean gets this wrong in both directions: a channel the
        // action never touches is perfectly reproducible and would inflate the
        // score, while a channel that swings wildly around a mean of zero looks
        // like "no effect" and would be ignored. Weighting by magnitude plus
        // spread makes the busy channels decide, which is the right answer for
        // "how much do I trust this prediction".
        let mut weighted_consistency = 0.0;
        let mut total_weight = 0.0;
        let mut counted = 0usize;

        for col in 0..cols {
            let effect = match support {
                Support::Observed => self.effect(family, target_row, col).copied(),
                Support::Transferred => self.transferred_effect(family, col),
                _ => None,
            };
            let Some(effect) = effect else { continue };
            if effect.trials < 2 {
                continue;
            }
            if let Some(row) = target_row {
                deltas.push((row, col, effect.mean_delta));
            }
            trials = trials.max(effect.trials);
            let weight = effect.mean_delta.abs() + effect.spread();
            weighted_consistency += weight * effect.consistency();
            total_weight += weight;
            counted += 1;
        }

        let consistency = if counted == 0 {
            0.0
        } else if total_weight <= 1e-12 {
            // Every channel is flat and steady: the action reliably does
            // nothing, which is a confident prediction.
            1.0
        } else {
            weighted_consistency / total_weight
        };
        Outcome {
            deltas,
            horizon_ns,
            confidence: Confidence::from(support, trials, consistency),
        }
    }

    /// A full counterfactual, scored against an intent.
    ///
    /// `measure` maps a snapshot to the objectives an intent cares about. It is
    /// supplied by the caller because only the caller knows which channels
    /// stand for latency or throughput on this machine, and baking that mapping
    /// in here would reintroduce exactly the hardcoded ontology the project is
    /// avoiding.
    pub fn predict(
        &self,
        candidate: &Candidate,
        current: &MirrorSnapshot,
        horizon_ns: u64,
        intent: &Intent,
        measure: Measure<'_>,
    ) -> Prediction {
        let (family, target_row, description) = candidate;
        let outcome = self.imagine(family, *target_row, current, horizon_ns);
        let now = measure(current, &[]);
        let predicted = measure(current, &outcome.deltas);
        let improvement = if outcome.is_informative() {
            score(intent, &predicted, &now)
        } else {
            None
        };
        Prediction {
            family: family.to_string(),
            description: description.clone(),
            outcome,
            achievement: Some(predicted),
            improvement,
        }
    }

    /// Rank candidate actions for an intent, best first.
    pub fn rank(
        &self,
        candidates: &[Candidate],
        current: &MirrorSnapshot,
        horizon_ns: u64,
        intent: &Intent,
        measure: Measure<'_>,
    ) -> Vec<Prediction> {
        let mut out: Vec<Prediction> = candidates
            .iter()
            .map(|candidate| self.predict(candidate, current, horizon_ns, intent, measure))
            .collect();
        out.sort_by(|a, b| {
            b.improvement
                .unwrap_or(f64::NEG_INFINITY)
                .partial_cmp(&a.improvement.unwrap_or(f64::NEG_INFINITY))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }
}

/// A measurement function for the common case: read one channel as one
/// objective, applying any predicted deltas.
pub fn channel_measure(
    mapping: Vec<(Objective, usize, usize)>,
) -> impl Fn(&MirrorSnapshot, &[(usize, usize, f64)]) -> Achievement {
    move |snapshot, deltas| {
        let mut achievement = Achievement::new();
        for (objective, row, col) in &mapping {
            let base = snapshot.state.get(*row, *col);
            if !base.is_finite() {
                continue;
            }
            let delta: f64 = deltas
                .iter()
                .filter(|(r, c, _)| r == row && c == col)
                .map(|(_, _, d)| *d)
                .sum();
            achievement = achievement.with(*objective, base + delta);
        }
        achievement
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_intent::Constraint;
    use corescout_mirror::test_support::fixture;

    fn pair(before_value: f64, after_value: f64) -> (MirrorSnapshot, MirrorSnapshot) {
        let mut before = fixture();
        before.state.set(2, 0, before_value);
        let mut after = fixture();
        after.state.set(2, 0, after_value);
        (before, after)
    }

    #[test]
    fn an_untried_action_yields_no_prediction() {
        // The most important negative in the crate. A model that has never
        // acted must not claim to know what acting would do.
        let model = EffectModel::new();
        let outcome = model.imagine("affinity", Some(2), &fixture(), 100_000_000);
        assert_eq!(outcome.confidence.support, Support::Extrapolated);
        assert!(!outcome.is_informative());
        assert!(outcome.deltas.is_empty());
    }

    #[test]
    fn a_repeated_effect_is_learned_and_predicted() {
        let mut model = EffectModel::new();
        for _ in 0..20 {
            let (before, after) = pair(1000.0, 900.0);
            assert!(model.learn("affinity", Some(2), &before, &after));
        }
        assert_eq!(model.trials("affinity", Some(2)), 20);
        assert_eq!(model.support_for("affinity", Some(2)), Support::Observed);

        let outcome = model.imagine("affinity", Some(2), &fixture(), 100_000_000);
        assert!(outcome.is_informative());
        assert_eq!(outcome.delta(2, 0), Some(-100.0));
        assert!(outcome.confidence.score > 0.5, "{:?}", outcome.confidence);
    }

    #[test]
    fn reliably_doing_nothing_is_a_consistent_effect() {
        // A channel an action never touches must not drag the whole
        // prediction's confidence down: "this does not change that" is a
        // perfectly good, perfectly reproducible claim.
        let mut effect = ObservedEffect::default();
        for _ in 0..10 {
            effect.observe(0.0);
        }
        assert_eq!(effect.consistency(), 1.0);
    }

    #[test]
    fn an_inconsistent_effect_earns_little_confidence() {
        // Twenty trials that disagree are not twenty trials' worth of
        // knowledge.
        let mut model = EffectModel::new();
        for i in 0..20 {
            let delta = if i % 2 == 0 { -500.0 } else { 500.0 };
            let (before, after) = pair(1000.0, 1000.0 + delta);
            model.learn("affinity", Some(2), &before, &after);
        }
        let effect = model.effect("affinity", Some(2), 0).unwrap();
        assert!(effect.spread() > effect.mean_delta.abs());
        assert_eq!(effect.consistency(), 0.0);

        let outcome = model.imagine("affinity", Some(2), &fixture(), 100_000_000);
        assert!(outcome.confidence.score < 0.1);
    }

    #[test]
    fn knowledge_transfers_to_an_untried_target_with_less_confidence() {
        let mut model = EffectModel::new();
        for _ in 0..20 {
            let (before, after) = pair(1000.0, 900.0);
            model.learn("affinity", Some(2), &before, &after);
        }
        // A target never acted on.
        assert_eq!(model.support_for("affinity", Some(3)), Support::Transferred);
        let known = model.imagine("affinity", Some(2), &fixture(), 100_000_000);
        let transferred = model.imagine("affinity", Some(3), &fixture(), 100_000_000);
        assert!(transferred.is_informative());
        assert!(
            transferred.confidence.score < known.confidence.score,
            "transferred knowledge must be worth less than direct evidence"
        );
    }

    #[test]
    fn effects_are_attributed_to_the_acted_on_entity_only() {
        // Otherwise the model learns that moving CPU 2 cools CPU 3, because
        // they happened to change together once.
        let mut model = EffectModel::new();
        let mut before = fixture();
        let mut after = fixture();
        before.state.set(2, 0, 1000.0);
        after.state.set(2, 0, 900.0);
        before.state.set(3, 0, 500.0);
        after.state.set(3, 0, 100.0);
        model.learn("affinity", Some(2), &before, &after);

        assert!(model.effect("affinity", Some(2), 0).is_some());
        // Row 3 changed too, and is not attributed to an action aimed at row 2.
        let outcome = model.imagine("affinity", Some(2), &fixture(), 1000);
        assert!(outcome.deltas.iter().all(|(row, _, _)| *row == 2));
    }

    #[test]
    fn learning_across_an_epoch_change_is_refused() {
        let mut model = EffectModel::new();
        let before = fixture();
        let mut after = fixture();
        after.epoch += 1;
        assert!(!model.learn("affinity", Some(2), &before, &after));
        assert_eq!(model.updates(), 0);
    }

    #[test]
    fn candidates_are_ranked_by_predicted_improvement() {
        let mut model = EffectModel::new();
        // Acting on row 2 reduces the objective a lot; row 3 a little.
        for _ in 0..20 {
            let mut before = fixture();
            let mut after = fixture();
            before.state.set(2, 0, 1000.0);
            after.state.set(2, 0, 500.0);
            model.learn("affinity", Some(2), &before, &after);

            let mut before3 = fixture();
            let mut after3 = fixture();
            before3.state.set(3, 0, 1000.0);
            after3.state.set(3, 0, 950.0);
            model.learn("affinity", Some(3), &before3, &after3);
        }

        let intent = Intent::new("latency")
            .with_constraint(Constraint::latency_p99_us(10_000_000.0))
            .preferring(Objective::Latency, 1.0);
        let measure_row2 = channel_measure(vec![(Objective::Latency, 2, 0)]);

        let candidates = vec![
            (
                "affinity".to_string(),
                Some(2usize),
                "move to 2".to_string(),
            ),
            (
                "affinity".to_string(),
                Some(3usize),
                "move to 3".to_string(),
            ),
        ];
        let ranked = model.rank(&candidates, &fixture(), 100_000_000, &intent, &measure_row2);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].description, "move to 2");
        assert!(ranked[0].improvement.unwrap() > 0.0);
    }

    #[test]
    fn a_measure_applies_predicted_deltas() {
        let measure = channel_measure(vec![(Objective::Latency, 2, 0)]);
        let now = measure(&fixture(), &[]);
        let after = measure(&fixture(), &[(2, 0, -600_000.0)]);
        assert_eq!(now.get(Objective::Latency), Some(3_600_000.0));
        assert_eq!(after.get(Objective::Latency), Some(3_000_000.0));
    }

    #[test]
    fn an_effect_model_survives_serialisation() {
        let mut model = EffectModel::new();
        for _ in 0..5 {
            let (before, after) = pair(1000.0, 900.0);
            model.learn("affinity", Some(2), &before, &after);
        }
        let json = serde_json::to_string(&model).unwrap();
        let back: EffectModel = serde_json::from_str(&json).unwrap();
        assert_eq!(model, back);
    }
}
