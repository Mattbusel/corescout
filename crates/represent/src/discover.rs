//! Finding structure in a series of reflections.
//!
//! Nothing here knows what a core, a cache or a temperature is. Every function
//! takes numbers indexed by `(entity row, variable column, time)` plus an edge
//! list, and returns claims about which of those move together.
//!
//! # The four questions
//!
//! 1. Which variables accumulate rather than fluctuate? (`accumulators`)
//! 2. Which variables move together? (`variable_groups`)
//! 3. Which entities behave alike? (`entity_similarity`, `cluster`)
//! 4. Do the mirror's own edges predict behavioural similarity? (`relation_lift`)
//!
//! The fourth is the interesting one. The mirror publishes edges of several
//! kinds without saying what any of them mean. If entities joined by edge type
//! 2 covary far more than random pairs, an observer has discovered that this
//! kind of connection is behaviourally real, without ever being told that
//! humans call it SMT siblinghood.
//!
//! # Method notes
//!
//! Correlation is Pearson on **first differences**, not on levels. Levels are
//! dominated by scale and by slow drift, and for an accumulator two unrelated
//! counters both going up correlate at nearly 1.0, which is true and useless.
//! Differencing asks the question that matters: when this moves, does that move
//! with it?

use std::collections::BTreeMap;

/// Pearson correlation of two equal-length series, ignoring pairs where either
/// side is unobserved.
///
/// Returns `None` when fewer than three usable pairs remain, or when either
/// side is constant. A constant series has no correlation with anything, and
/// reporting 0.0 would be a claim rather than an absence.
pub fn correlation(a: &[f64], b: &[f64]) -> Option<f64> {
    let pairs: Vec<(f64, f64)> = a
        .iter()
        .zip(b)
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .map(|(x, y)| (*x, *y))
        .collect();
    if pairs.len() < 3 {
        return None;
    }
    let n = pairs.len() as f64;
    let mean_a = pairs.iter().map(|(x, _)| *x).sum::<f64>() / n;
    let mean_b = pairs.iter().map(|(_, y)| *y).sum::<f64>() / n;

    let mut cov = 0.0;
    let mut var_a = 0.0;
    let mut var_b = 0.0;
    for (x, y) in &pairs {
        let dx = x - mean_a;
        let dy = y - mean_b;
        cov += dx * dy;
        var_a += dx * dx;
        var_b += dy * dy;
    }
    if var_a <= f64::EPSILON || var_b <= f64::EPSILON {
        return None;
    }
    Some((cov / (var_a.sqrt() * var_b.sqrt())).clamp(-1.0, 1.0))
}

/// First differences of a series. `NaN` propagates across a gap, because the
/// difference across a hole is not a measurement.
pub fn differences(series: &[f64]) -> Vec<f64> {
    series
        .windows(2)
        .map(|w| {
            if w[0].is_finite() && w[1].is_finite() {
                w[1] - w[0]
            } else {
                f64::NAN
            }
        })
        .collect()
}

/// Evidence that a variable is a running total rather than a reading.
#[derive(Debug, Clone, PartialEq)]
pub struct Accumulator {
    pub column: usize,
    /// Fraction of observed steps that did not decrease.
    pub monotone_fraction: f64,
    /// Fraction of steps that strictly increased. A variable that never moves
    /// is monotone too, and is not an accumulator.
    pub increase_fraction: f64,
}

/// Detect which variables behave like accumulators, from behaviour alone.
///
/// This is the observer rediscovering, unaided, a distinction the mirror
/// declares explicitly as `Semantics::Cumulative`. Comparing what this finds
/// against what the mirror declares measures exactly what that label was worth.
///
/// A variable counts as an accumulator when it essentially never decreases and
/// does actually rise. `min_increase` keeps configuration variables, which are
/// constant and therefore trivially non-decreasing, out of the result.
pub fn accumulators(
    series_by_cell: &BTreeMap<(usize, usize), Vec<f64>>,
    cols: usize,
    monotone_threshold: f64,
    min_increase: f64,
) -> Vec<Accumulator> {
    let mut out = Vec::new();
    for col in 0..cols {
        let mut steps = 0usize;
        let mut non_decreasing = 0usize;
        let mut increasing = 0usize;
        for ((_, c), series) in series_by_cell.iter() {
            if *c != col {
                continue;
            }
            for delta in differences(series) {
                if !delta.is_finite() {
                    continue;
                }
                steps += 1;
                if delta >= 0.0 {
                    non_decreasing += 1;
                }
                if delta > 0.0 {
                    increasing += 1;
                }
            }
        }
        if steps < 3 {
            continue;
        }
        let monotone_fraction = non_decreasing as f64 / steps as f64;
        let increase_fraction = increasing as f64 / steps as f64;
        if monotone_fraction >= monotone_threshold && increase_fraction >= min_increase {
            out.push(Accumulator {
                column: col,
                monotone_fraction,
                increase_fraction,
            });
        }
    }
    out
}

/// A group of variables that move together across the machine.
#[derive(Debug, Clone, PartialEq)]
pub struct VariableGroup {
    pub columns: Vec<usize>,
    /// Mean pairwise correlation within the group.
    pub cohesion: f64,
}

/// Group variables by how strongly they covary, pooled over all entities.
///
/// "Pooled" means the correlation between column `i` and column `j` is computed
/// over every entity's series concatenated, so the claim is about the
/// variables themselves rather than about one entity.
pub fn variable_groups(
    differenced: &BTreeMap<(usize, usize), Vec<f64>>,
    cols: usize,
    threshold: f64,
) -> Vec<VariableGroup> {
    let mut pooled: Vec<Vec<f64>> = vec![Vec::new(); cols];
    // Pool per entity, standardising within entity so that one high-variance
    // entity does not dominate the pooled correlation.
    let mut rows: Vec<usize> = differenced.keys().map(|(r, _)| *r).collect();
    rows.sort_unstable();
    rows.dedup();

    for row in rows {
        for (col, target) in pooled.iter_mut().enumerate() {
            if let Some(series) = differenced.get(&(row, col)) {
                target.extend(standardise(series));
            }
        }
    }

    let mut similarity = vec![vec![f64::NAN; cols]; cols];
    for i in 0..cols {
        for j in 0..cols {
            if i == j {
                similarity[i][j] = 1.0;
            } else if pooled[i].len() == pooled[j].len() {
                if let Some(r) = correlation(&pooled[i], &pooled[j]) {
                    similarity[i][j] = r;
                }
            }
        }
    }

    let groups = cluster_by_similarity(cols, &similarity, threshold);
    groups
        .into_iter()
        .filter(|g| g.len() > 1)
        .map(|columns| {
            let cohesion = mean_pairwise(&columns, &similarity);
            VariableGroup { columns, cohesion }
        })
        .collect()
}

/// Subtract the machine-wide common mode from every series.
///
/// # Why this is necessary rather than a refinement
///
/// A computer is driven by things that move all of it at once: a workload
/// starting, a phase of a benchmark, a thermal envelope tightening. Under such a
/// driver *every* entity covaries with every other, often above 0.8, and the
/// local couplings that distinguish one pair of entities from another are
/// buried under it.
///
/// The first version of this observer had no common-mode removal and reported
/// exactly that: one cluster containing every CPU, cohesion 0.84, and an SMT
/// edge lift of +0.16 against a baseline of +0.84. Every one of those numbers
/// was true, and together they said nothing, because "all these things go up
/// and down together" is a fact about the workload rather than about the
/// machine's structure.
///
/// So for each variable at each instant, the mean across all entities is
/// subtracted, leaving what made each entity differ from the machine as a whole.
/// SMT siblings still track each other in the residual, because their coupling
/// is local. The global phase does not survive, because it was global.
///
/// This is common-average referencing, borrowed from electrophysiology, where
/// the same problem has the same shape: many sensors, one large shared signal,
/// and the interesting structure underneath it.
pub fn remove_common_mode(
    differenced: &BTreeMap<(usize, usize), Vec<f64>>,
    rows: usize,
    cols: usize,
) -> BTreeMap<(usize, usize), Vec<f64>> {
    let length = differenced.values().map(|v| v.len()).max().unwrap_or(0);
    let mut residuals: BTreeMap<(usize, usize), Vec<f64>> = BTreeMap::new();

    for col in 0..cols {
        // The common mode of this variable at each instant: the mean across
        // every entity that reported it. Standardised per entity first, so an
        // entity with a large scale does not define the mean by itself.
        let members: Vec<(usize, Vec<f64>)> = (0..rows)
            .filter_map(|row| {
                differenced
                    .get(&(row, col))
                    .map(|series| (row, standardise(series)))
            })
            .collect();
        if members.len() < 2 {
            // Nothing to reference against: pass the series through unchanged
            // rather than zeroing it.
            for (row, series) in members {
                residuals.insert((row, col), series);
            }
            continue;
        }

        let mut common = vec![f64::NAN; length];
        for (index, slot) in common.iter_mut().enumerate() {
            let values: Vec<f64> = members
                .iter()
                .filter_map(|(_, series)| series.get(index).copied())
                .filter(|v| v.is_finite())
                .collect();
            if !values.is_empty() {
                *slot = mean(&values);
            }
        }

        for (row, series) in members {
            let residual: Vec<f64> = series
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    let reference = common.get(index).copied().unwrap_or(f64::NAN);
                    if value.is_finite() && reference.is_finite() {
                        value - reference
                    } else {
                        f64::NAN
                    }
                })
                .collect();
            residuals.insert((row, col), residual);
        }
    }
    residuals
}

/// Every comparable pair of a symmetric similarity matrix, once each.
pub fn upper_triangle(similarity: &[Vec<f64>]) -> Vec<f64> {
    let mut values = Vec::new();
    for (a, row) in similarity.iter().enumerate() {
        for (b, value) in row.iter().enumerate() {
            if a < b && value.is_finite() {
                values.push(*value);
            }
        }
    }
    values
}

/// Mean similarity across every comparable pair.
///
/// Computed on raw differences, this measures how much of the machine moves as
/// one. A high value is not noise: it is a real fact about the workload, and it
/// is the reason [`remove_common_mode`] exists.
pub fn mean_similarity(similarity: &[Vec<f64>]) -> f64 {
    let values = upper_triangle(similarity);
    if values.is_empty() {
        0.0
    } else {
        mean(&values)
    }
}

/// Behavioural similarity between two entities.
///
/// Computed over the variables both of them report, on first differences, and
/// averaged. Entities sharing no observed variable are incomparable, which is
/// `None` rather than 0.0.
pub fn entity_similarity(
    differenced: &BTreeMap<(usize, usize), Vec<f64>>,
    a: usize,
    b: usize,
    cols: usize,
) -> Option<f64> {
    let mut total = 0.0;
    let mut count = 0usize;
    for col in 0..cols {
        let (Some(sa), Some(sb)) = (differenced.get(&(a, col)), differenced.get(&(b, col))) else {
            continue;
        };
        if let Some(r) = correlation(sa, sb) {
            total += r;
            count += 1;
        }
    }
    if count == 0 {
        None
    } else {
        Some(total / count as f64)
    }
}

/// The full entity-by-entity similarity matrix, `NaN` where incomparable.
#[allow(clippy::needless_range_loop)] // triangular fill: each step writes [a][b] and [b][a]
pub fn similarity_matrix(
    differenced: &BTreeMap<(usize, usize), Vec<f64>>,
    rows: usize,
    cols: usize,
) -> Vec<Vec<f64>> {
    let mut matrix = vec![vec![f64::NAN; rows]; rows];
    for a in 0..rows {
        matrix[a][a] = 1.0;
        for b in (a + 1)..rows {
            if let Some(r) = entity_similarity(differenced, a, b, cols) {
                matrix[a][b] = r;
                matrix[b][a] = r;
            }
        }
    }
    matrix
}

/// A set of entities that behave alike.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityCluster {
    pub rows: Vec<usize>,
    pub cohesion: f64,
}

/// Cluster entities by behavioural similarity.
pub fn cluster_entities(similarity: &[Vec<f64>], threshold: f64) -> Vec<EntityCluster> {
    let rows = similarity.len();
    cluster_by_similarity(rows, similarity, threshold)
        .into_iter()
        .filter(|group| group.len() > 1)
        .map(|rows| {
            let cohesion = mean_pairwise(&rows, similarity);
            EntityCluster { rows, cohesion }
        })
        .collect()
}

/// What one kind of edge is worth as a predictor of shared behaviour.
#[derive(Debug, Clone, PartialEq)]
pub struct RelationLift {
    /// The edge type, as an opaque code.
    pub kind: u16,
    pub pairs: usize,
    /// Mean similarity of entities this edge joins.
    pub connected_similarity: f64,
    /// Mean similarity of all comparable pairs.
    pub baseline_similarity: f64,
    /// Connected minus baseline. The headline number.
    pub lift: f64,
    /// Standardised separation between the two distributions, in the manner of
    /// Cohen's d. Above about 0.8 is a large effect.
    pub separation: f64,
}

/// Ask whether the mirror's own edges predict behavioural similarity.
///
/// This is the discovery that matters most. The observer is handed a graph with
/// unnamed edge types and asks, of each type: do the entities you connect
/// actually behave more alike than random pairs? A large positive lift means
/// this kind of connection is physically real, whatever it is called.
pub fn relation_lift(similarity: &[Vec<f64>], edges: &[(usize, usize, u16)]) -> Vec<RelationLift> {
    let rows = similarity.len();

    // The baseline is every comparable pair, computed once.
    let all = upper_triangle(similarity);
    if all.is_empty() {
        return Vec::new();
    }
    let baseline = mean(&all);
    let baseline_sd = std_dev(&all, baseline);

    let mut by_kind: BTreeMap<u16, Vec<f64>> = BTreeMap::new();
    for (a, b, kind) in edges {
        if *a == *b || *a >= rows || *b >= rows {
            continue;
        }
        let value = similarity[*a][*b];
        if value.is_finite() {
            by_kind.entry(*kind).or_default().push(value);
        }
    }

    let mut out: Vec<RelationLift> = by_kind
        .into_iter()
        .filter(|(_, values)| values.len() >= 2)
        .map(|(kind, values)| {
            let connected = mean(&values);
            let connected_sd = std_dev(&values, connected);
            // Pooled standard deviation, guarded against the degenerate case
            // where both sides are constant.
            let pooled = ((connected_sd.powi(2) + baseline_sd.powi(2)) / 2.0).sqrt();
            let separation = if pooled > 1e-12 {
                (connected - baseline) / pooled
            } else {
                0.0
            };
            RelationLift {
                kind,
                pairs: values.len(),
                connected_similarity: connected,
                baseline_similarity: baseline,
                lift: connected - baseline,
                separation,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.lift
            .partial_cmp(&a.lift)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// A recurring configuration of the whole machine.
#[derive(Debug, Clone, PartialEq)]
pub struct Regime {
    pub id: usize,
    /// How many observed reflections fell into it.
    pub occupancy: usize,
    /// Mean distance from the regime's centre, as a measure of how tight it is.
    pub spread: f64,
}

/// Find recurring whole-machine states by k-means over the standardised
/// reflection vectors.
///
/// Deterministic in two senses, both of which matter. Two runs over the same
/// experience produce identical concepts, because a learner that thought
/// something different every time it considered the same data would not be
/// discovering anything. And the seeding is *farthest-point*: start from the
/// first frame, then repeatedly take the frame furthest from everything chosen
/// so far.
///
/// Evenly spaced seeding was tried first and is wrong. On a machine that
/// alternates between two states, every even-numbered frame is identical, so
/// seeds at `0` and `len/2` land on the same point and one cluster is born
/// empty. Periodic behaviour is not a corner case here; it is what a machine
/// under a regular workload does.
///
/// Returns fewer regimes than asked for when the experience contains fewer
/// distinct states, rather than padding the answer with empty clusters.
#[allow(clippy::needless_range_loop)] // centres and dimensions are indexed in lockstep
pub fn regimes(frames: &[Vec<f64>], k: usize, iterations: usize) -> (Vec<Regime>, Vec<usize>) {
    if frames.is_empty() || k == 0 {
        return (Vec::new(), Vec::new());
    }
    let dims = frames[0].len();

    let mut centres: Vec<Vec<f64>> = vec![frames[0].clone()];
    while centres.len() < k.min(frames.len()) {
        let mut best_index = 0usize;
        let mut best_distance = -1.0;
        for (index, frame) in frames.iter().enumerate() {
            let nearest = centres
                .iter()
                .map(|centre| distance(frame, centre))
                .fold(f64::INFINITY, f64::min);
            if nearest > best_distance {
                best_distance = nearest;
                best_index = index;
            }
        }
        // Every remaining frame coincides with a centre we already have: the
        // experience holds fewer distinct states than were asked for.
        if best_distance <= 1e-12 {
            break;
        }
        centres.push(frames[best_index].clone());
    }
    let k = centres.len();
    let mut assignment = vec![0usize; frames.len()];

    for _ in 0..iterations {
        let mut changed = false;
        for (index, frame) in frames.iter().enumerate() {
            let mut best = 0;
            let mut best_distance = f64::INFINITY;
            for (centre_index, centre) in centres.iter().enumerate() {
                let d = distance(frame, centre);
                if d < best_distance {
                    best_distance = d;
                    best = centre_index;
                }
            }
            if assignment[index] != best {
                assignment[index] = best;
                changed = true;
            }
        }

        for centre_index in 0..k {
            let members: Vec<&Vec<f64>> = frames
                .iter()
                .enumerate()
                .filter(|(i, _)| assignment[*i] == centre_index)
                .map(|(_, f)| f)
                .collect();
            if members.is_empty() {
                continue;
            }
            for dim in 0..dims {
                let mut total = 0.0;
                let mut count = 0.0;
                for member in &members {
                    if let Some(v) = member.get(dim) {
                        if v.is_finite() {
                            total += v;
                            count += 1.0;
                        }
                    }
                }
                centres[centre_index][dim] = if count > 0.0 { total / count } else { 0.0 };
            }
        }
        if !changed {
            break;
        }
    }

    let summary = (0..centres.len())
        .map(|id| {
            let members: Vec<usize> = assignment
                .iter()
                .enumerate()
                .filter(|(_, a)| **a == id)
                .map(|(i, _)| i)
                .collect();
            let spread = if members.is_empty() {
                0.0
            } else {
                members
                    .iter()
                    .map(|i| distance(&frames[*i], &centres[id]))
                    .sum::<f64>()
                    / members.len() as f64
            };
            Regime {
                id,
                occupancy: members.len(),
                spread,
            }
        })
        .collect();
    (summary, assignment)
}

/// Transition counts between regimes, `[from][to]`.
pub fn regime_transitions(assignment: &[usize], k: usize) -> Vec<Vec<usize>> {
    let mut counts = vec![vec![0usize; k]; k];
    for pair in assignment.windows(2) {
        if pair[0] < k && pair[1] < k {
            counts[pair[0]][pair[1]] += 1;
        }
    }
    counts
}

// ---------------------------------------------------------------------------
// Shared helpers.
// ---------------------------------------------------------------------------

/// Single-linkage agglomeration: anything similar enough to any member joins
/// the group.
///
/// Chosen over average linkage because the structures being looked for are
/// chains and tight cliques (two SMT threads, four cores under one cache) and
/// single linkage finds those without needing a cluster count in advance.
fn cluster_by_similarity(n: usize, similarity: &[Vec<f64>], threshold: f64) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..n).collect();

    fn find(parent: &mut Vec<usize>, x: usize) -> usize {
        if parent[x] != x {
            let root = find(parent, parent[x]);
            parent[x] = root;
        }
        parent[x]
    }

    for a in 0..n {
        for b in (a + 1)..n {
            let value = similarity.get(a).and_then(|r| r.get(b)).copied();
            if value.is_some_and(|v| v.is_finite() && v >= threshold) {
                let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                if ra != rb {
                    parent[ra] = rb;
                }
            }
        }
    }

    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for item in 0..n {
        let root = find(&mut parent, item);
        groups.entry(root).or_default().push(item);
    }
    let mut out: Vec<Vec<usize>> = groups.into_values().collect();
    out.sort_by_key(|g| g[0]);
    out
}

fn mean_pairwise(members: &[usize], similarity: &[Vec<f64>]) -> f64 {
    let mut total = 0.0;
    let mut count = 0usize;
    for i in 0..members.len() {
        for j in (i + 1)..members.len() {
            let v = similarity[members[i]][members[j]];
            if v.is_finite() {
                total += v;
                count += 1;
            }
        }
    }
    if count == 0 {
        1.0
    } else {
        total / count as f64
    }
}

/// Standardise a series to zero mean and unit variance, preserving gaps.
pub fn standardise(series: &[f64]) -> Vec<f64> {
    let usable: Vec<f64> = series.iter().copied().filter(|v| v.is_finite()).collect();
    if usable.len() < 2 {
        return vec![f64::NAN; series.len()];
    }
    let m = mean(&usable);
    let sd = std_dev(&usable, m);
    if sd <= 1e-12 {
        return vec![0.0; series.len()];
    }
    series
        .iter()
        .map(|v| {
            if v.is_finite() {
                (v - m) / sd
            } else {
                f64::NAN
            }
        })
        .collect()
}

pub fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

pub fn std_dev(values: &[f64], mean: f64) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64).sqrt()
}

fn distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b)
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f64>()
        .sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell_map(entries: Vec<((usize, usize), Vec<f64>)>) -> BTreeMap<(usize, usize), Vec<f64>> {
        entries.into_iter().collect()
    }

    #[test]
    fn correlation_finds_agreement_and_disagreement() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0];
        let rising = [2.0, 4.0, 6.0, 8.0, 10.0];
        let falling = [5.0, 4.0, 3.0, 2.0, 1.0];
        assert!((correlation(&a, &rising).unwrap() - 1.0).abs() < 1e-9);
        assert!((correlation(&a, &falling).unwrap() + 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_constant_series_correlates_with_nothing() {
        // Reporting 0.0 would be a claim; there is simply no answer.
        let constant = [3.0, 3.0, 3.0, 3.0];
        assert_eq!(correlation(&constant, &[1.0, 2.0, 3.0, 4.0]), None);
    }

    #[test]
    fn correlation_ignores_gaps_and_needs_enough_data() {
        let a = [1.0, f64::NAN, 3.0, 4.0, 5.0];
        let b = [2.0, 9.0, 6.0, 8.0, 10.0];
        assert!(correlation(&a, &b).unwrap() > 0.99);
        assert_eq!(correlation(&[1.0, 2.0], &[1.0, 2.0]), None);
    }

    #[test]
    fn differences_break_across_a_gap() {
        let d = differences(&[1.0, 2.0, f64::NAN, 5.0]);
        assert_eq!(d[0], 1.0);
        assert!(d[1].is_nan());
        assert!(d[2].is_nan());
    }

    #[test]
    fn accumulators_are_detected_from_behaviour_alone() {
        let series = cell_map(vec![
            // A counter.
            ((0, 0), vec![10.0, 20.0, 30.0, 45.0, 60.0]),
            ((1, 0), vec![5.0, 9.0, 14.0, 22.0, 30.0]),
            // A reading that goes up and down.
            ((0, 1), vec![3.0, 5.0, 4.0, 6.0, 2.0]),
            ((1, 1), vec![7.0, 6.0, 8.0, 5.0, 9.0]),
            // A constant configuration value: monotone, but not accumulating.
            ((0, 2), vec![100.0, 100.0, 100.0, 100.0, 100.0]),
        ]);
        let found = accumulators(&series, 3, 0.95, 0.5);
        let columns: Vec<usize> = found.iter().map(|a| a.column).collect();
        assert_eq!(columns, vec![0], "only the counter accumulates");
        assert!((found[0].monotone_fraction - 1.0).abs() < 1e-9);
    }

    #[test]
    fn entities_driven_by_one_signal_are_found_to_be_similar() {
        // Two entities whose variables track a shared driver, and a third that
        // does its own thing. No labels involved.
        let driver: Vec<f64> = (0..40).map(|i| ((i as f64) * 0.7).sin()).collect();
        let other: Vec<f64> = (0..40).map(|i| ((i as f64) * 0.31).cos()).collect();
        let jitter = |seed: f64| -> Vec<f64> {
            driver
                .iter()
                .enumerate()
                .map(|(i, v)| v + 0.01 * ((i as f64 + seed) * 1.7).sin())
                .collect()
        };

        let series = cell_map(vec![
            ((0, 0), jitter(0.0)),
            ((1, 0), jitter(5.0)),
            ((2, 0), other),
        ]);
        let differenced: BTreeMap<(usize, usize), Vec<f64>> =
            series.iter().map(|(k, v)| (*k, differences(v))).collect();

        let matrix = similarity_matrix(&differenced, 3, 1);
        assert!(matrix[0][1] > 0.9, "shared driver: {}", matrix[0][1]);
        assert!(matrix[0][2].abs() < 0.5, "unrelated: {}", matrix[0][2]);

        let clusters = cluster_entities(&matrix, 0.8);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].rows, vec![0, 1]);
    }

    #[test]
    fn common_mode_removal_uncovers_local_structure_under_a_global_driver() {
        // Four entities. All of them follow one machine-wide phase; on top of
        // that, 0 and 1 share a local driver, as do 2 and 3.
        let global: Vec<f64> = (0..80).map(|i| ((i as f64) * 0.2).sin() * 10.0).collect();
        let local_a: Vec<f64> = (0..80).map(|i| ((i as f64) * 1.1).sin()).collect();
        let local_b: Vec<f64> = (0..80).map(|i| ((i as f64) * 0.9).cos()).collect();

        let combine = |local: &[f64], phase: f64| -> Vec<f64> {
            global
                .iter()
                .zip(local)
                .enumerate()
                .map(|(i, (g, l))| g + l + 0.01 * ((i as f64 + phase) * 2.3).sin())
                .collect()
        };

        let series: BTreeMap<(usize, usize), Vec<f64>> = [
            ((0usize, 0usize), combine(&local_a, 0.0)),
            ((1, 0), combine(&local_a, 3.0)),
            ((2, 0), combine(&local_b, 0.0)),
            ((3, 0), combine(&local_b, 3.0)),
        ]
        .into_iter()
        .collect();
        let differenced: BTreeMap<(usize, usize), Vec<f64>> =
            series.iter().map(|(k, v)| (*k, differences(v))).collect();

        // Before: the global driver makes everything look alike, and the true
        // pairing is invisible.
        let raw = similarity_matrix(&differenced, 4, 1);
        assert!(raw[0][2] > 0.6, "the common mode dominates: {}", raw[0][2]);
        assert_eq!(
            cluster_entities(&raw, 0.7).len(),
            1,
            "without removal, everything is one cluster"
        );

        // After: the pairs separate.
        let residual = remove_common_mode(&differenced, 4, 1);
        let cleaned = similarity_matrix(&residual, 4, 1);
        assert!(
            cleaned[0][1] > 0.7,
            "pair (0,1) survives: {}",
            cleaned[0][1]
        );
        assert!(
            cleaned[2][3] > 0.7,
            "pair (2,3) survives: {}",
            cleaned[2][3]
        );
        assert!(
            cleaned[0][2] < 0.3,
            "unrelated entities separate: {}",
            cleaned[0][2]
        );

        let clusters = cluster_entities(&cleaned, 0.7);
        assert_eq!(clusters.len(), 2, "the two true pairs");
    }

    #[test]
    fn mean_similarity_measures_how_much_moves_as_one() {
        let similarity = vec![
            vec![1.0, 0.8, 0.8],
            vec![0.8, 1.0, 0.8],
            vec![0.8, 0.8, 1.0],
        ];
        assert!((mean_similarity(&similarity) - 0.8).abs() < 1e-9);
    }

    #[test]
    fn relation_lift_separates_a_real_edge_type_from_a_meaningless_one() {
        // Entities 0-1 and 2-3 are behaviourally paired. Edge type 2 joins the
        // real pairs; edge type 9 joins arbitrary ones.
        let mut similarity = vec![vec![0.1; 4]; 4];
        for (a, b) in [(0, 1), (2, 3)] {
            similarity[a][b] = 0.95;
            similarity[b][a] = 0.95;
        }
        for (i, row) in similarity.iter_mut().enumerate() {
            row[i] = 1.0;
        }

        let edges = vec![(0, 1, 2u16), (2, 3, 2), (0, 2, 9), (1, 3, 9)];
        let lifts = relation_lift(&similarity, &edges);

        assert_eq!(lifts[0].kind, 2, "the real edge type must rank first");
        assert!(lifts[0].lift > 0.4, "lift was {}", lifts[0].lift);
        assert!(
            lifts[0].separation > 0.8,
            "effect size {}",
            lifts[0].separation
        );

        let meaningless = lifts.iter().find(|l| l.kind == 9).unwrap();
        assert!(
            meaningless.lift < 0.1,
            "an edge type with no behavioural meaning must not show lift: {}",
            meaningless.lift
        );
    }

    #[test]
    fn variable_groups_find_covarying_columns() {
        let base: Vec<f64> = (0..30).map(|i| ((i as f64) * 0.5).sin()).collect();
        let mut entries = Vec::new();
        for row in 0..3usize {
            entries.push(((row, 0), base.clone()));
            // Column 1 is column 0 scaled: same movement, different units.
            entries.push(((row, 1), base.iter().map(|v| v * 1000.0).collect()));
            // Column 2 is unrelated.
            entries.push((
                (row, 2),
                (0..30).map(|i| ((i as f64) * 0.23).cos()).collect(),
            ));
        }
        let series = cell_map(entries);
        let differenced: BTreeMap<(usize, usize), Vec<f64>> =
            series.iter().map(|(k, v)| (*k, differences(v))).collect();

        let groups = variable_groups(&differenced, 3, 0.9);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].columns, vec![0, 1]);
        assert!(groups[0].cohesion > 0.95);
    }

    #[test]
    fn regimes_are_deterministic_and_recur() {
        // Two alternating whole-machine configurations.
        let frames: Vec<Vec<f64>> = (0..40)
            .map(|i| {
                if i % 2 == 0 {
                    vec![1.0, 1.0, 0.0]
                } else {
                    vec![0.0, 0.0, 1.0]
                }
            })
            .collect();
        let (summary, assignment) = regimes(&frames, 2, 20);
        assert_eq!(summary.len(), 2);
        assert!(
            summary.iter().all(|r| r.occupancy > 0),
            "seeding must not produce an empty cluster on periodic data"
        );
        assert_eq!(summary.iter().map(|r| r.occupancy).sum::<usize>(), 40);
        assert!(summary.iter().all(|r| r.spread < 1e-6), "tight clusters");

        // Determinism: the same experience produces the same concepts.
        let (again, _) = regimes(&frames, 2, 20);
        assert_eq!(summary, again);

        // And the machine alternates, which the transition matrix shows.
        let transitions = regime_transitions(&assignment, 2);
        assert_eq!(transitions[0][0], 0);
        assert_eq!(transitions[1][1], 0);
        assert!(transitions[0][1] > 0 && transitions[1][0] > 0);
    }

    #[test]
    fn fewer_distinct_states_than_requested_yields_fewer_regimes() {
        // Do not invent structure: three requested, two present.
        let frames: Vec<Vec<f64>> = (0..30)
            .map(|i| {
                if i % 2 == 0 {
                    vec![1.0, 0.0]
                } else {
                    vec![0.0, 1.0]
                }
            })
            .collect();
        let (summary, _) = regimes(&frames, 3, 20);
        assert_eq!(summary.len(), 2);
    }

    #[test]
    fn clustering_is_stable_against_incomparable_entities() {
        // An entity that shares no observed variable with anything must not
        // crash the clustering or be silently merged.
        let similarity = vec![
            vec![1.0, 0.95, f64::NAN],
            vec![0.95, 1.0, f64::NAN],
            vec![f64::NAN, f64::NAN, 1.0],
        ];
        let clusters = cluster_entities(&similarity, 0.8);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].rows, vec![0, 1]);
    }
}
