//! A body of surviving hypotheses, and the loop that keeps it honest.
//!
//! # What a theory is here
//!
//! Not a grand unified account of the machine. A [`Theory`] is the set of
//! statements that have been made, tested, and not yet killed, together with the
//! record of the ones that were. The graveyard is kept deliberately: a theory
//! that only remembers its successes cannot tell you how much its successes are
//! worth.
//!
//! # The loop
//!
//! ```text
//! observe  ->  conjecture  ->  predict  ->  test  ->  refute or corroborate
//!    ^                                                        |
//!    +--------------------------------------------------------+
//! ```
//!
//! [`Theory::conjecture`] generates candidate hypotheses from what the
//! representation layer has discovered. [`Theory::confront`] tests every open
//! hypothesis against a reflection. Statements that keep failing are retired.
//!
//! # Conjecture is cheap and should be
//!
//! Most conjectures are wrong, and that is the intended operating point. The
//! expensive thing is testing, so the generator is allowed to be prolific and
//! the [`TheoryConfig::max_open`] cap does the rationing, dropping the
//! worst-supported open hypotheses first.
//!
//! # What this crate refuses to do
//!
//! It never renames a discovered state to whatever it appears to correspond to,
//! and it never converts a supported hypothesis into an assumption. A supported
//! hypothesis remains testable forever, and keeps being tested; the machinery
//! for "we are sure enough to stop checking" does not exist here on purpose.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use corescout_mirror::MirrorSnapshot;
use corescout_represent::latent::{LatentCatalogue, LatentStateId};

use crate::hypothesis::{Claim, Expectation, Hypothesis, Verdict};

/// How the theory is run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TheoryConfig {
    /// Trials before a verdict is anything but `Untested`.
    pub min_trials: u32,
    /// Open hypotheses to carry at once.
    pub max_open: usize,
    /// Refutations after which a hypothesis is retired outright.
    pub give_up_after: u32,
    /// Occurrences of a state before it is worth theorising about.
    pub min_state_evidence: u64,
}

impl Default for TheoryConfig {
    fn default() -> Self {
        TheoryConfig {
            min_trials: 20,
            max_open: 256,
            // Ten clear failures is enough. Carrying a dead hypothesis costs
            // testing budget that an untried one could use.
            give_up_after: 10,
            min_state_evidence: 12,
        }
    }
}

/// The record of one dead hypothesis.
///
/// Kept because the ratio of refuted to supported is the only honest measure of
/// how much a surviving hypothesis is worth. A generator that produces only
/// safe statements will show a suspiciously low refutation rate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Refutation {
    pub claim: Claim,
    pub trials: u32,
    pub support: f64,
    pub mean_error: f64,
    pub retired_ns: u64,
}

/// Everything currently believed, everything disproved, and the tally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theory {
    config: TheoryConfig,
    /// Open and settled hypotheses, by claim key.
    #[serde(with = "corescout_core::serde_util::pairs")]
    hypotheses: BTreeMap<String, Hypothesis>,
    /// What has been killed.
    graveyard: Vec<Refutation>,
    conjectured: u64,
    tested: u64,
}

impl Default for Theory {
    fn default() -> Self {
        Theory::new(TheoryConfig::default())
    }
}

impl Theory {
    pub fn new(config: TheoryConfig) -> Theory {
        Theory {
            config,
            hypotheses: BTreeMap::new(),
            graveyard: Vec::new(),
            conjectured: 0,
            tested: 0,
        }
    }

    pub fn config(&self) -> &TheoryConfig {
        &self.config
    }

    pub fn len(&self) -> usize {
        self.hypotheses.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hypotheses.is_empty()
    }

    /// Hypotheses stated, including ones since killed.
    pub fn conjectured(&self) -> u64 {
        self.conjectured
    }

    /// Individual trials performed.
    pub fn tested(&self) -> u64 {
        self.tested
    }

    pub fn graveyard(&self) -> &[Refutation] {
        &self.graveyard
    }

    pub fn get(&self, key: &str) -> Option<&Hypothesis> {
        self.hypotheses.get(key)
    }

    /// Every hypothesis that currently has the given verdict shape.
    pub fn supported(&self) -> Vec<&Hypothesis> {
        self.with(|verdict| verdict.is_supported())
    }

    pub fn open(&self) -> Vec<&Hypothesis> {
        self.with(|verdict| verdict.is_open())
    }

    fn with(&self, predicate: impl Fn(&Verdict) -> bool) -> Vec<&Hypothesis> {
        let mut found: Vec<&Hypothesis> = self
            .hypotheses
            .values()
            .filter(|h| predicate(&h.verdict(self.config.min_trials)))
            .collect();
        // Best-evidenced first, so a caller taking the top few gets the ones
        // worth acting on.
        found.sort_by(|a, b| {
            b.trials()
                .cmp(&a.trials())
                .then(b.support().total_cmp(&a.support()))
        });
        found
    }

    /// How often a stated hypothesis turns out to be wrong.
    ///
    /// Worth watching. A rate near zero means the generator is only making safe
    /// statements, which is a failure mode that looks like success: the theory
    /// fills with true, useless claims and nothing is ever learned.
    pub fn refutation_rate(&self) -> Option<f64> {
        let settled = self.graveyard.len()
            + self
                .hypotheses
                .values()
                .filter(|h| !h.verdict(self.config.min_trials).is_open())
                .count();
        (settled > 0).then(|| self.graveyard.len() as f64 / settled as f64)
    }

    /// Propose hypotheses from what has been discovered so far.
    ///
    /// Returns how many new ones were admitted. Claims already present are not
    /// restated: a hypothesis under test keeps its accumulated evidence rather
    /// than being reset each time the generator runs.
    pub fn conjecture(
        &mut self,
        catalogue: &LatentCatalogue,
        snapshot: &MirrorSnapshot,
        now_ns: u64,
    ) -> usize {
        let mut admitted = 0;

        for state in catalogue.states() {
            if state.occupancy < self.config.min_state_evidence {
                continue;
            }

            // A state's centroid *is* a set of predictions: it says what the
            // machine looks like when it is in that state. Turning each
            // component into a testable claim is the cheapest useful conjecture
            // available, and it is exactly the claim a discovered state has to
            // earn its name with.
            for (col, centre) in state.centroid.iter().enumerate() {
                if !centre.is_finite() || col >= snapshot.channels.len() {
                    continue;
                }
                // The state's radius is the natural null spread: it is how much
                // reflections in this state vary. Predicting to within a
                // fraction of that is a real commitment.
                let tolerance = state.radius * 0.5;
                let expectation = Expectation::new(*centre, tolerance, state.radius);
                for row in 0..snapshot.entities.len() {
                    let claim = Claim::StatePredicts {
                        state: state.id,
                        row,
                        col,
                    };
                    if self.admit(claim, expectation.clone(), now_ns) {
                        admitted += 1;
                    }
                }
            }
        }

        // Transitions the catalogue has seen often enough to bet on.
        for ((from, to), count) in catalogue.transition_counts() {
            if count < self.config.min_state_evidence {
                continue;
            }
            let total: u64 = catalogue
                .transition_counts()
                .iter()
                .filter(|((f, _), _)| *f == from)
                .map(|(_, c)| *c)
                .sum();
            if total == 0 {
                continue;
            }
            let probability = count as f64 / total as f64;
            // Commit to the transition happening at this rate, sharper than the
            // spread of a coin flip.
            let expectation = Expectation::new(probability, 0.15, 0.5);
            if self.admit(Claim::StateTransitions { from, to }, expectation, now_ns) {
                admitted += 1;
            }
        }

        self.trim();
        admitted
    }

    /// Add a hypothesis if it is falsifiable and not already stated.
    pub fn admit(&mut self, claim: Claim, expectation: Expectation, now_ns: u64) -> bool {
        let key = claim.key();
        if self.hypotheses.contains_key(&key) {
            return false;
        }
        let Some(hypothesis) = Hypothesis::new(claim, expectation, now_ns) else {
            return false;
        };
        self.hypotheses.insert(key, hypothesis);
        self.conjectured += 1;
        true
    }

    /// Test every open hypothesis against a reflection.
    ///
    /// `current_state` is where the machine is now, so state-conditional claims
    /// are tested only when their condition holds. Testing "when in state 3, X"
    /// against a reflection in state 7 would refute it for the wrong reason.
    pub fn confront(
        &mut self,
        snapshot: &MirrorSnapshot,
        current_state: Option<LatentStateId>,
        normalised: &[f64],
        now_ns: u64,
    ) -> Confrontation {
        let cols = snapshot.channels.len();
        let mut tested = 0;
        let mut agreed = 0;
        let mut newly_refuted = Vec::new();

        for hypothesis in self.hypotheses.values_mut() {
            if hypothesis.is_retired() {
                continue;
            }
            let observed = match &hypothesis.claim {
                Claim::StatePredicts { state, row, col } => {
                    // Only applicable while the machine is in that state.
                    if current_state != Some(*state) {
                        continue;
                    }
                    let index = row * cols + col;
                    normalised.get(index).copied()
                }
                // The remaining claim kinds are tested by the layers that own
                // the relevant evidence: transitions by the catalogue, causes by
                // the effect model. `confront` handles what a single reflection
                // can settle.
                _ => None,
            };
            let Some(observed) = observed else {
                continue;
            };
            tested += 1;
            if hypothesis.test(observed) {
                agreed += 1;
            }
            if hypothesis.refutations >= self.config.give_up_after
                && hypothesis.verdict(self.config.min_trials).is_refuted()
            {
                hypothesis.retire("refuted repeatedly");
                newly_refuted.push(hypothesis.claim.clone());
            }
        }

        self.tested += tested as u64;
        self.bury(now_ns);

        Confrontation {
            tested,
            agreed,
            refuted: newly_refuted,
        }
    }

    /// Report a result the theory could not gather itself.
    ///
    /// Transition and causal claims are settled by the catalogue and the effect
    /// model respectively, which hold evidence a single reflection does not.
    /// They report here so that every claim, whatever tested it, ends up in one
    /// record with one standard of evidence.
    pub fn report(&mut self, claim: &Claim, observed: f64, now_ns: u64) -> bool {
        let Some(hypothesis) = self.hypotheses.get_mut(&claim.key()) else {
            return false;
        };
        if hypothesis.is_retired() {
            return false;
        }
        self.tested += 1;
        let agreed = hypothesis.test(observed);
        if hypothesis.refutations >= self.config.give_up_after
            && hypothesis.verdict(self.config.min_trials).is_refuted()
        {
            hypothesis.retire("refuted repeatedly");
        }
        self.bury(now_ns);
        agreed
    }

    /// Retire every hypothesis that referred to a state that no longer exists.
    ///
    /// The catalogue consolidates and prunes states. A claim about a state that
    /// has been merged away is not false, it is meaningless, and the difference
    /// matters: it goes to the graveyard as retired rather than as refuted, so
    /// it does not distort the refutation rate.
    pub fn forget_state(&mut self, state: LatentStateId) -> usize {
        let mut retired = 0;
        for hypothesis in self.hypotheses.values_mut() {
            let mentions = match &hypothesis.claim {
                Claim::StatePredicts { state: s, .. } | Claim::StateFavours { state: s, .. } => {
                    *s == state
                }
                Claim::StateTransitions { from, to } => *from == state || *to == state,
                _ => false,
            };
            if mentions && !hypothesis.is_retired() {
                hypothesis.retire(format!("{state} no longer exists"));
                retired += 1;
            }
        }
        retired
    }

    /// Move newly retired-as-refuted hypotheses into the graveyard.
    fn bury(&mut self, now_ns: u64) {
        let min_trials = self.config.min_trials;
        let dead: Vec<String> = self
            .hypotheses
            .iter()
            .filter(|(_, h)| {
                h.retired.as_deref() == Some("refuted repeatedly")
                    || h.verdict(min_trials).is_refuted() && h.is_retired()
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in dead {
            if let Some(hypothesis) = self.hypotheses.remove(&key) {
                self.graveyard.push(Refutation {
                    trials: hypothesis.trials(),
                    support: hypothesis.support(),
                    mean_error: hypothesis.mean_error().unwrap_or(f64::NAN),
                    claim: hypothesis.claim,
                    retired_ns: now_ns,
                });
            }
        }
    }

    /// Drop the least useful open hypotheses when over the cap.
    fn trim(&mut self) {
        if self.hypotheses.len() <= self.config.max_open {
            return;
        }
        let min_trials = self.config.min_trials;
        let mut ranked: Vec<(String, u32, f64)> = self
            .hypotheses
            .iter()
            .filter(|(_, h)| h.verdict(min_trials).is_open())
            .map(|(key, h)| (key.clone(), h.trials(), h.support()))
            .collect();
        // Untried and unsupported first: a hypothesis with evidence, even
        // mixed, is worth more than one with none.
        ranked.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.1.cmp(&b.1)));
        let excess = self.hypotheses.len() - self.config.max_open;
        for (key, _, _) in ranked.into_iter().take(excess) {
            self.hypotheses.remove(&key);
        }
    }

    /// A report a person can read.
    pub fn render(&self) -> String {
        let mut out = format!(
            "{} hypotheses stated, {} trials, {} refuted and buried\n",
            self.conjectured,
            self.tested,
            self.graveyard.len()
        );
        if let Some(rate) = self.refutation_rate() {
            out.push_str(&format!(
                "{:.0}% of settled claims were wrong",
                rate * 100.0
            ));
            if rate < 0.05 {
                // The failure mode that looks like success.
                out.push_str("  (suspiciously low: is the generator only making safe claims?)");
            }
            out.push('\n');
        }
        let supported = self.supported();
        if supported.is_empty() {
            out.push_str("\nnothing is supported yet\n");
        } else {
            out.push_str("\nsurviving:\n");
            for hypothesis in supported.iter().take(10) {
                out.push_str(&format!(
                    "  {}\n",
                    hypothesis.summary(self.config.min_trials)
                ));
            }
        }
        if !self.graveyard.is_empty() {
            out.push_str("\nkilled:\n");
            for refutation in self.graveyard.iter().rev().take(5) {
                out.push_str(&format!(
                    "  {}  ({:.0}% of {} trials)\n",
                    refutation.claim.describe(),
                    refutation.support * 100.0,
                    refutation.trials
                ));
            }
        }
        out
    }
}

/// What one round of testing did.
#[derive(Debug, Clone, PartialEq)]
pub struct Confrontation {
    pub tested: u32,
    pub agreed: u32,
    /// Claims killed by this reflection.
    pub refuted: Vec<Claim>,
}

impl Confrontation {
    pub fn agreement(&self) -> Option<f64> {
        (self.tested > 0).then(|| self.agreed as f64 / self.tested as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::test_support::fixture;

    fn theory() -> Theory {
        Theory::new(TheoryConfig {
            min_trials: 5,
            give_up_after: 4,
            min_state_evidence: 2,
            ..TheoryConfig::default()
        })
    }

    fn claim() -> Claim {
        Claim::StatePredicts {
            state: LatentStateId(1),
            row: 0,
            col: 0,
        }
    }

    #[test]
    fn a_new_theory_believes_nothing() {
        let theory = theory();
        assert!(theory.is_empty());
        assert!(theory.supported().is_empty());
        assert_eq!(theory.refutation_rate(), None);
        assert!(theory.render().contains("nothing is supported"));
    }

    #[test]
    fn the_same_claim_is_not_restated_and_keeps_its_evidence() {
        let mut theory = theory();
        assert!(theory.admit(claim(), Expectation::new(1.0, 0.1, 1.0), 0));
        theory.report(&claim(), 1.0, 0);
        // A second attempt must not wipe the trial already recorded.
        assert!(!theory.admit(claim(), Expectation::new(9.0, 0.1, 1.0), 0));
        assert_eq!(theory.get(&claim().key()).unwrap().trials(), 1);
    }

    #[test]
    fn an_unfalsifiable_conjecture_is_not_admitted() {
        let mut theory = theory();
        assert!(!theory.admit(claim(), Expectation::new(1.0, 5.0, 1.0), 0));
        assert!(theory.is_empty());
        assert_eq!(theory.conjectured(), 0);
    }

    #[test]
    fn a_claim_that_keeps_failing_is_killed_and_buried() {
        let mut theory = theory();
        theory.admit(claim(), Expectation::new(1.0, 0.1, 1.0), 0);
        for _ in 0..10 {
            theory.report(&claim(), 99.0, 100);
        }
        assert!(theory.get(&claim().key()).is_none(), "it should be buried");
        assert_eq!(theory.graveyard().len(), 1);
        assert!(theory.graveyard()[0].support < 0.5);
        assert!(theory.render().contains("killed:"));
    }

    #[test]
    fn a_claim_that_holds_survives_and_is_still_tested() {
        // Support is never converted into an assumption: it keeps accumulating
        // trials, so it can still be killed later.
        let mut theory = theory();
        theory.admit(claim(), Expectation::new(1.0, 0.1, 1.0), 0);
        for _ in 0..10 {
            theory.report(&claim(), 1.02, 0);
        }
        assert_eq!(theory.supported().len(), 1);
        let before = theory.get(&claim().key()).unwrap().trials();
        theory.report(&claim(), 1.02, 0);
        assert_eq!(theory.get(&claim().key()).unwrap().trials(), before + 1);
    }

    #[test]
    fn a_state_conditional_claim_is_only_tested_while_that_state_holds() {
        // Testing "when in state 1, X" during state 7 would refute it for the
        // wrong reason, and the theory would lose a true statement.
        let mut theory = theory();
        theory.admit(claim(), Expectation::new(1.0, 0.1, 1.0), 0);
        let snapshot = fixture();
        let wrong_state = theory.confront(&snapshot, Some(LatentStateId(7)), &vec![99.0; 64], 0);
        assert_eq!(wrong_state.tested, 0, "it must not be tested out of state");
        assert_eq!(theory.get(&claim().key()).unwrap().trials(), 0);

        let right_state = theory.confront(&snapshot, Some(LatentStateId(1)), &vec![1.0; 64], 0);
        assert_eq!(right_state.tested, 1);
    }

    #[test]
    fn a_reflection_with_no_current_state_tests_no_state_claims() {
        let mut theory = theory();
        theory.admit(claim(), Expectation::new(1.0, 0.1, 1.0), 0);
        let result = theory.confront(&fixture(), None, &vec![1.0; 64], 0);
        assert_eq!(result.tested, 0);
    }

    #[test]
    fn claims_about_a_vanished_state_are_retired_not_refuted() {
        // The distinction matters: a meaningless claim must not count against
        // the refutation rate, which measures how often the theory was wrong.
        let mut theory = theory();
        theory.admit(claim(), Expectation::new(1.0, 0.1, 1.0), 0);
        assert_eq!(theory.forget_state(LatentStateId(1)), 1);
        let hypothesis = theory.get(&claim().key()).expect("still present");
        assert!(hypothesis.is_retired());
        assert!(matches!(
            hypothesis.verdict(5),
            Verdict::Retired(reason) if reason.contains("no longer exists")
        ));
        assert!(theory.graveyard().is_empty(), "not a refutation");
    }

    #[test]
    fn a_retired_claim_accepts_no_further_evidence() {
        let mut theory = theory();
        theory.admit(claim(), Expectation::new(1.0, 0.1, 1.0), 0);
        theory.forget_state(LatentStateId(1));
        assert!(!theory.report(&claim(), 1.0, 0));
        assert_eq!(theory.get(&claim().key()).unwrap().trials(), 0);
    }

    #[test]
    fn reporting_on_a_claim_that_was_never_stated_does_nothing() {
        let mut theory = theory();
        assert!(!theory.report(&claim(), 1.0, 0));
        assert!(theory.is_empty());
    }

    #[test]
    fn conjecture_generates_testable_claims_from_discovered_states() {
        let mut theory = theory();
        let mut catalogue = LatentCatalogue::new(1.0, 8);
        for i in 0..40u64 {
            catalogue.observe(&[(i % 3) as f64 * 4.0, 1.0], i * 1_000_000);
        }
        let admitted = theory.conjecture(&catalogue, &fixture(), 0);
        assert!(
            admitted > 0,
            "no conjectures from {} states",
            catalogue.len()
        );
        assert_eq!(theory.conjectured() as usize, admitted);
        // Every admitted claim must be falsifiable by construction.
        for hypothesis in theory.open() {
            assert!(hypothesis.expectation.is_falsifiable());
        }
    }

    #[test]
    fn conjecture_ignores_states_with_too_little_evidence() {
        let mut theory = Theory::new(TheoryConfig {
            min_state_evidence: 1000,
            ..TheoryConfig::default()
        });
        let mut catalogue = LatentCatalogue::new(1.0, 8);
        for i in 0..40u64 {
            catalogue.observe(&[(i % 3) as f64 * 4.0, 1.0], i * 1_000_000);
        }
        assert_eq!(theory.conjecture(&catalogue, &fixture(), 0), 0);
    }

    #[test]
    fn the_open_set_is_capped_and_drops_the_least_evidenced_first() {
        let mut theory = Theory::new(TheoryConfig {
            max_open: 3,
            min_trials: 2,
            ..TheoryConfig::default()
        });
        for i in 0..10 {
            theory.admit(
                Claim::StatePredicts {
                    state: LatentStateId(1),
                    row: i,
                    col: 0,
                },
                Expectation::new(1.0, 0.1, 1.0),
                0,
            );
        }
        // Give one of them evidence, so it should survive the trim.
        let keeper = Claim::StatePredicts {
            state: LatentStateId(1),
            row: 0,
            col: 0,
        };
        for _ in 0..5 {
            theory.report(&keeper, 1.0, 0);
        }
        theory.trim();
        assert!(theory.len() <= 3);
        assert!(theory.get(&keeper.key()).is_some(), "evidence should win");
    }

    #[test]
    fn a_suspiciously_low_refutation_rate_is_called_out() {
        // The failure mode that looks like success: a generator making only
        // safe claims fills the theory with true, useless statements.
        let mut theory = theory();
        for i in 0..10 {
            let c = Claim::StatePredicts {
                state: LatentStateId(1),
                row: i,
                col: 0,
            };
            theory.admit(c.clone(), Expectation::new(1.0, 0.1, 1.0), 0);
            for _ in 0..6 {
                theory.report(&c, 1.0, 0);
            }
        }
        assert_eq!(theory.refutation_rate(), Some(0.0));
        assert!(theory.render().contains("suspiciously low"));
    }

    #[test]
    fn the_refutation_rate_counts_both_sides() {
        let mut theory = theory();
        let good = Claim::StatePredicts {
            state: LatentStateId(1),
            row: 0,
            col: 0,
        };
        let bad = Claim::StatePredicts {
            state: LatentStateId(1),
            row: 1,
            col: 0,
        };
        theory.admit(good.clone(), Expectation::new(1.0, 0.1, 1.0), 0);
        theory.admit(bad.clone(), Expectation::new(1.0, 0.1, 1.0), 0);
        for _ in 0..8 {
            theory.report(&good, 1.0, 0);
            theory.report(&bad, 99.0, 0);
        }
        let rate = theory.refutation_rate().expect("both settled");
        assert!((rate - 0.5).abs() < 1e-9, "got {rate}");
    }

    #[test]
    fn a_theory_round_trips_through_json() {
        let mut theory = theory();
        theory.admit(claim(), Expectation::new(1.0, 0.1, 1.0), 0);
        theory.report(&claim(), 1.0, 0);
        let text = serde_json::to_string(&theory).expect("serialises");
        let back: Theory = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back, theory);
    }
}
