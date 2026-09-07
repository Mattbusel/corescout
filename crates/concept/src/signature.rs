//! A concept described without reference to the machine that holds it.
//!
//! # The problem this solves
//!
//! Machine A discovers something it calls `concept_31`. Internally that is a
//! centroid over A's channels, indexed by A's entity numbering. Sending it to
//! machine B is useless: B has different channels, a different number of
//! entities, different units, possibly a different instruction set.
//!
//! Sending `"use CPU 7"` is worse than useless. It is sharing *configuration*,
//! which is only meaningful between identical machines.
//!
//! A [`Signature`] is the concept expressed as **dimensionless relational
//! facts**: how many things participate, how concentrated it is, how long it
//! lasts relative to that machine's own timescale, how strongly it responds to
//! being acted on, how reliably it recurs. None of those carry a unit, a channel
//! name, or an entity index.
//!
//! Machine B can then ask: *do I have anything shaped like this?* and search its
//! own concepts, which are made of its own physics. If it finds one, the two
//! machines have not exchanged a setting. They have exchanged a concept, and
//! each holds it in its own body.
//!
//! # Why every feature is normalised against the machine itself
//!
//! `dwell` is not nanoseconds, it is a multiple of that machine's median state
//! dwell. `participation` is not a count, it is a fraction of that machine's
//! entities. This is what makes the comparison meaningful across machines whose
//! clocks, sizes and units have nothing in common, and it is the only reason a
//! similarity score between an x86 concept and an ARM concept could mean
//! anything at all.
//!
//! # What a match is and is not
//!
//! A high similarity is a **hypothesis**, not an identification. It says two
//! machines each have something playing a structurally similar role. Whether
//! those are "the same concept" in any deeper sense is exactly the open question
//! in `RESEARCH.md`, and this crate is the apparatus, not the answer.

use serde::{Deserialize, Serialize};

/// A concept expressed so another machine could look for its own version.
///
/// Every field is dimensionless and normalised against the originating
/// machine's own scale. There are deliberately no channel names, no entity
/// indices, and no units anywhere in this struct.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signature {
    /// Fraction of the machine's entities that participate.
    ///
    /// A concept involving 2 of 4 entities and one involving 40 of 80 are the
    /// same shape at this level of description, which is the point.
    pub participation: f64,
    /// Fraction of observable variables that are distinctively away from their
    /// usual value while the concept holds.
    pub distinctiveness: f64,
    /// How tightly the participating variables agree. 1.0 is a single sharp
    /// configuration; 0.0 is a diffuse cloud.
    pub coherence: f64,
    /// Mean dwell as a multiple of this machine's median state dwell.
    ///
    /// Dimensionless on purpose: a state lasting 40 ms on a machine whose states
    /// typically last 10 ms is the same *kind* of thing as one lasting 4 s on a
    /// machine whose states typically last a second.
    pub relative_dwell: f64,
    /// How often it recurs, as a fraction of all observations.
    pub recurrence: f64,
    /// How strongly the machine's own actions move it, from 0 (nothing this
    /// machine can do touches it) to 1 (reliably produced on demand).
    ///
    /// The most important field for section-10 purposes: two machines with
    /// nothing physical in common may still both have a state they can enter at
    /// will, and that is a real correspondence.
    pub controllability: f64,
    /// How much knowing the machine is in this state improves prediction of its
    /// next state, relative to not knowing.
    pub predictive_utility: f64,
}

impl Signature {
    /// The features as a vector, in a fixed order.
    ///
    /// Order is part of the wire format: two machines comparing signatures must
    /// agree on which slot is which, and nothing here should be reordered
    /// without a version bump.
    pub fn features(&self) -> [f64; 7] {
        [
            self.participation,
            self.distinctiveness,
            self.coherence,
            self.relative_dwell,
            self.recurrence,
            self.controllability,
            self.predictive_utility,
        ]
    }

    /// Whether this signature is well formed enough to compare.
    ///
    /// A signature with a non-finite feature is not "slightly wrong", it is
    /// uncomparable, and matching against it would produce a number with no
    /// meaning.
    pub fn is_comparable(&self) -> bool {
        self.features().iter().all(|f| f.is_finite())
    }

    /// Structural similarity to another machine's concept, from 0 to 1.
    ///
    /// `None` when either signature is not comparable, rather than a low score:
    /// "we could not compare these" and "these are dissimilar" are different
    /// facts and collapsing them would let a broken signature read as a
    /// confident non-match.
    pub fn similarity(&self, other: &Signature) -> Option<f64> {
        if !self.is_comparable() || !other.is_comparable() {
            return None;
        }
        let mine = self.features();
        let theirs = other.features();

        // Weighted, because the features are not equally diagnostic.
        // Controllability and predictive utility say what a concept *does*;
        // participation and recurrence say how big and how common it is, which
        // is more likely to differ between machines of different sizes for
        // uninteresting reasons.
        const WEIGHTS: [f64; 7] = [0.8, 1.0, 1.0, 1.0, 0.8, 1.6, 1.6];

        let mut total = 0.0;
        let mut weight_sum = 0.0;
        for i in 0..7 {
            // Each feature is a ratio or fraction, so a plain absolute
            // difference is already on a comparable scale. `relative_dwell` is
            // unbounded above, so it is squashed first.
            let (a, b) = if i == 3 {
                (squash(mine[i]), squash(theirs[i]))
            } else {
                (mine[i].clamp(0.0, 1.0), theirs[i].clamp(0.0, 1.0))
            };
            total += WEIGHTS[i] * (1.0 - (a - b).abs());
            weight_sum += WEIGHTS[i];
        }
        Some((total / weight_sum).clamp(0.0, 1.0))
    }

    /// A description a person can read, with no machine-specific terms in it.
    pub fn describe(&self) -> String {
        format!(
            "involves {:.0}% of parts, {:.0}% of variables distinctive, \
             coherence {:.2}, lasts {:.1}x a typical state, recurs in {:.0}% of \
             observations, controllability {:.2}, predictive utility {:.2}",
            self.participation * 100.0,
            self.distinctiveness * 100.0,
            self.coherence,
            self.relative_dwell,
            self.recurrence * 100.0,
            self.controllability,
            self.predictive_utility
        )
    }
}

/// Squash an unbounded positive ratio into 0..1 around 1.0.
///
/// A concept lasting twice as long as typical and one lasting half as long are
/// equally far from typical, so the mapping is symmetric in log space.
fn squash(ratio: f64) -> f64 {
    if ratio <= 0.0 {
        return 0.0;
    }
    let log = ratio.ln();
    // logistic on the log-ratio: 1.0 -> 0.5, 0 -> 0, infinity -> 1
    1.0 / (1.0 + (-log).exp())
}

/// A match between one machine's concept and another's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Analogue {
    /// The foreign concept's name, as its own machine calls it.
    pub foreign_name: String,
    /// The local concept's name.
    pub local_name: String,
    pub similarity: f64,
    /// Where the two signatures differ most, so the match can be argued with.
    pub largest_difference: String,
}

impl Analogue {
    /// Whether this is strong enough to be worth acting on.
    ///
    /// Deliberately high. A weak structural match between two machines is the
    /// easiest false positive in this entire project: with seven features and
    /// enough concepts, something will always look similar to something.
    pub fn is_strong(&self) -> bool {
        self.similarity >= 0.85
    }
}

/// Find the local concept most like a foreign one.
///
/// `local` is `(name, signature)` for each concept this machine holds. Returns
/// `None` when nothing is comparable, rather than the least bad match.
pub fn find_analogue(
    foreign_name: &str,
    foreign: &Signature,
    local: &[(String, Signature)],
) -> Option<Analogue> {
    let mut best: Option<(f64, &String, &Signature)> = None;
    for (name, signature) in local {
        let Some(similarity) = foreign.similarity(signature) else {
            continue;
        };
        if best.map_or(true, |(score, _, _)| similarity > score) {
            best = Some((similarity, name, signature));
        }
    }
    let (similarity, name, signature) = best?;
    Some(Analogue {
        foreign_name: foreign_name.to_string(),
        local_name: name.clone(),
        similarity,
        largest_difference: largest_difference(foreign, signature),
    })
}

/// Which feature the two signatures disagree on most.
fn largest_difference(a: &Signature, b: &Signature) -> String {
    const NAMES: [&str; 7] = [
        "participation",
        "distinctiveness",
        "coherence",
        "relative dwell",
        "recurrence",
        "controllability",
        "predictive utility",
    ];
    let mine = a.features();
    let theirs = b.features();
    let mut worst = (0usize, -1.0f64);
    for i in 0..7 {
        let difference = (mine[i] - theirs[i]).abs();
        if difference.is_finite() && difference > worst.1 {
            worst = (i, difference);
        }
    }
    format!("{} differs by {:.2}", NAMES[worst.0], worst.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature() -> Signature {
        Signature {
            participation: 0.25,
            distinctiveness: 0.40,
            coherence: 0.80,
            relative_dwell: 2.0,
            recurrence: 0.15,
            controllability: 0.70,
            predictive_utility: 0.35,
        }
    }

    #[test]
    fn a_signature_carries_no_machine_specific_terms() {
        // The property that makes exchange possible at all. If this ever grows
        // a channel name or an entity index, the concept has stopped being
        // portable and has become configuration.
        let text = serde_json::to_string(&signature()).expect("serialises");
        for forbidden in [
            "cpu", "core", "cache", "channel", "entity", "row", "col", "ns",
        ] {
            assert!(
                !text.to_lowercase().contains(forbidden),
                "signature leaked {forbidden}: {text}"
            );
        }
    }

    #[test]
    fn a_signature_is_most_similar_to_itself() {
        let s = signature();
        assert_eq!(s.similarity(&s), Some(1.0));
    }

    #[test]
    fn two_machines_of_different_sizes_can_share_a_concept() {
        // The whole point of normalising: 2-of-8 and 20-of-80 are the same
        // shape, and a machine ten times larger should still recognise it.
        let small = Signature {
            participation: 2.0 / 8.0,
            ..signature()
        };
        let large = Signature {
            participation: 20.0 / 80.0,
            ..signature()
        };
        assert_eq!(small.similarity(&large), Some(1.0));
    }

    #[test]
    fn two_machines_with_different_clocks_can_share_a_concept() {
        // relative_dwell is a multiple of that machine's own median, so a state
        // lasting 40 ms where states last 10 ms matches one lasting 4 s where
        // states last a second.
        let fast = Signature {
            relative_dwell: 4.0,
            ..signature()
        };
        let slow = Signature {
            relative_dwell: 4.0,
            ..signature()
        };
        assert_eq!(fast.similarity(&slow), Some(1.0));
    }

    #[test]
    fn structurally_different_concepts_score_low() {
        let a = signature();
        let b = Signature {
            participation: 0.95,
            distinctiveness: 0.02,
            coherence: 0.05,
            relative_dwell: 0.05,
            recurrence: 0.99,
            controllability: 0.0,
            predictive_utility: 0.0,
        };
        let score = a.similarity(&b).expect("comparable");
        assert!(score < 0.5, "unrelated concepts scored {score}");
    }

    #[test]
    fn what_a_concept_does_weighs_more_than_how_big_it_is() {
        // Two concepts differing only in size should stay closer than two
        // differing only in controllability.
        let base = signature();
        let bigger = Signature {
            participation: 0.9,
            ..base.clone()
        };
        let uncontrollable = Signature {
            controllability: 0.0,
            ..base.clone()
        };
        let size_score = base.similarity(&bigger).unwrap();
        let role_score = base.similarity(&uncontrollable).unwrap();
        assert!(
            size_score > role_score,
            "size {size_score} should matter less than role {role_score}"
        );
    }

    #[test]
    fn an_uncomparable_signature_yields_none_not_a_low_score() {
        // "Could not compare" and "dissimilar" are different facts.
        let broken = Signature {
            coherence: f64::NAN,
            ..signature()
        };
        assert!(!broken.is_comparable());
        assert_eq!(signature().similarity(&broken), None);
        assert_eq!(broken.similarity(&signature()), None);
    }

    #[test]
    fn the_dwell_squash_is_symmetric_in_log_space() {
        // Twice as long and half as long are equally unusual.
        let double = squash(2.0);
        let half = squash(0.5);
        assert!((double - 0.5).abs() - (0.5 - half).abs() < 1e-9);
        assert!((squash(1.0) - 0.5).abs() < 1e-9);
        assert_eq!(squash(0.0), 0.0);
    }

    #[test]
    fn an_analogue_is_found_and_names_where_it_disagrees() {
        let foreign = signature();
        let local = vec![
            (
                "concept_4".to_string(),
                Signature {
                    controllability: 0.0,
                    ..signature()
                },
            ),
            ("concept_9".to_string(), signature()),
        ];
        let analogue = find_analogue("Z31", &foreign, &local).expect("a match");
        assert_eq!(analogue.local_name, "concept_9");
        assert_eq!(analogue.foreign_name, "Z31");
        assert!(analogue.is_strong());
        assert!(!analogue.largest_difference.is_empty());
    }

    #[test]
    fn a_weak_match_is_reported_but_not_called_strong() {
        // With seven features and enough concepts, something always looks a bit
        // similar. The bar for acting on it is deliberately high.
        let foreign = signature();
        let local = vec![(
            "concept_1".to_string(),
            Signature {
                participation: 0.99,
                distinctiveness: 0.01,
                coherence: 0.01,
                relative_dwell: 0.01,
                recurrence: 0.99,
                controllability: 0.0,
                predictive_utility: 0.0,
            },
        )];
        let analogue = find_analogue("Z31", &foreign, &local).expect("a match");
        assert!(!analogue.is_strong(), "scored {}", analogue.similarity);
    }

    #[test]
    fn a_machine_with_no_comparable_concepts_finds_nothing() {
        assert!(find_analogue("Z31", &signature(), &[]).is_none());
        let broken = vec![(
            "concept_1".to_string(),
            Signature {
                coherence: f64::NAN,
                ..signature()
            },
        )];
        assert!(find_analogue("Z31", &signature(), &broken).is_none());
    }

    #[test]
    fn a_signature_round_trips_through_json() {
        let text = serde_json::to_string(&signature()).expect("serialises");
        let back: Signature = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back, signature());
    }
}
