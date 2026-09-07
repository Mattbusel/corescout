//! Score normalisation.
//!
//! # Why relative-to-best rather than a curve
//!
//! Scores are `100 * value / best_value` (inverted for lower-is-better
//! metrics). That has one property that matters more than statistical
//! elegance: **the numbers keep their physical meaning**. A core scoring 94.0
//! for compute really does deliver 94% of the throughput of the best core in
//! this machine. Z-scores or percentile ranks would spread the field out
//! prettily and, in doing so, make a machine whose cores are all within 1% of
//! each other look like it has dramatic winners and losers. On most hardware,
//! most of the time, the cores *are* nearly identical, and the report should
//! say so plainly.
//!
//! The consequence to keep in mind when reading a report: the top core always
//! scores 100. That is a definition, not a discovery.

use std::collections::BTreeMap;

use corescout_substrate::topology::PhysicalId;

/// Which end of the scale is good.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Better {
    Higher,
    Lower,
}

/// Normalise `(core, value)` pairs onto 0-100, where 100 is the best core.
///
/// Non-finite and non-positive values are dropped rather than scored: they mean
/// a measurement failed, and a zero would misrepresent that as "this core is
/// infinitely bad".
pub fn normalise(values: &[(PhysicalId, f64)], better: Better) -> BTreeMap<PhysicalId, f64> {
    let usable: Vec<(PhysicalId, f64)> = values
        .iter()
        .copied()
        .filter(|(_, v)| v.is_finite() && *v > 0.0)
        .collect();
    if usable.is_empty() {
        return BTreeMap::new();
    }

    let best = match better {
        Better::Higher => usable.iter().map(|(_, v)| *v).fold(f64::MIN, f64::max),
        Better::Lower => usable.iter().map(|(_, v)| *v).fold(f64::MAX, f64::min),
    };
    if best <= 0.0 {
        return BTreeMap::new();
    }

    usable
        .into_iter()
        .map(|(core, v)| {
            let score = match better {
                Better::Higher => 100.0 * v / best,
                Better::Lower => 100.0 * best / v,
            };
            (core, score.clamp(0.0, 100.0))
        })
        .collect()
}

/// Combine several 0-100 component scores with weights.
///
/// Missing components are skipped and the weights renormalised, so a machine
/// where one workload family could not be measured still produces a usable
/// composite instead of a silently deflated one.
pub fn weighted(components: &[(Option<f64>, f64)]) -> Option<f64> {
    let mut total = 0.0;
    let mut weight_sum = 0.0;
    for (value, weight) in components {
        if let Some(v) = value {
            total += v * weight;
            weight_sum += weight;
        }
    }
    if weight_sum <= 0.0 {
        None
    } else {
        Some(total / weight_sum)
    }
}

/// Order cores by score, best first, breaking ties by core id so the output is
/// stable across runs.
pub fn ranked(scores: &BTreeMap<PhysicalId, f64>) -> Vec<(PhysicalId, f64)> {
    let mut v: Vec<(PhysicalId, f64)> = scores.iter().map(|(k, s)| (*k, *s)).collect();
    v.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .expect("scores are never NaN")
            .then(a.0.cmp(&b.0))
    });
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "{a} != {b}");
    }

    #[test]
    fn higher_is_better_puts_the_maximum_at_100() {
        let s = normalise(&[(0, 50.0), (1, 100.0), (2, 75.0)], Better::Higher);
        approx(s[&1], 100.0);
        approx(s[&0], 50.0);
        approx(s[&2], 75.0);
    }

    #[test]
    fn lower_is_better_puts_the_minimum_at_100() {
        let s = normalise(&[(0, 100.0), (1, 200.0)], Better::Lower);
        approx(s[&0], 100.0);
        approx(s[&1], 50.0);
    }

    #[test]
    fn near_identical_cores_score_near_identically() {
        // The property the module docs promise: a uniform machine must not be
        // dramatised into winners and losers.
        let s = normalise(&[(0, 1000.0), (1, 995.0), (2, 990.0)], Better::Higher);
        assert!(s[&2] > 98.0, "spread was exaggerated: {}", s[&2]);
    }

    #[test]
    fn failed_measurements_are_dropped_not_zeroed() {
        let s = normalise(
            &[(0, 100.0), (1, f64::NAN), (2, 0.0), (3, -1.0)],
            Better::Higher,
        );
        assert_eq!(s.len(), 1);
        assert!(s.contains_key(&0));
    }

    #[test]
    fn empty_input_yields_empty_scores() {
        assert!(normalise(&[], Better::Higher).is_empty());
    }

    #[test]
    fn weights_renormalise_around_missing_components() {
        // 80 and 60 with equal weights is 70, whether or not a third
        // unmeasurable component is nominally in the formula.
        approx(
            weighted(&[(Some(80.0), 1.0), (Some(60.0), 1.0), (None, 5.0)]).unwrap(),
            70.0,
        );
        assert!(weighted(&[(None, 1.0)]).is_none());
    }

    #[test]
    fn ranking_is_stable_for_ties() {
        let mut scores = BTreeMap::new();
        scores.insert(5, 90.0);
        scores.insert(2, 90.0);
        scores.insert(9, 100.0);
        assert_eq!(ranked(&scores), vec![(9, 100.0), (2, 90.0), (5, 90.0)]);
    }
}
