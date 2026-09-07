//! Turning measurements into rankings, recommendations and an [`Analysis`].
//!
//! # What a "best core" means here
//!
//! There is no single best core, which is why this module produces several
//! rankings rather than one. The core with the highest sustained integer
//! throughput is frequently *not* the core with the lowest tail latency,
//! because the properties that produce each are different and sometimes
//! opposed: the core the firmware bins highest boosts furthest, and a core that
//! boosts furthest changes frequency more often, and frequency transitions are
//! themselves a source of jitter. Meanwhile the lowest-numbered CPUs typically
//! carry the bulk of the machine's interrupt load, which does not show up in a
//! throughput average at all but dominates a p99.
//!
//! So CoreScout ranks cores separately for compute, memory, latency and jitter,
//! and only then combines those into per-profile recommendations with
//! explicitly stated weights.

pub mod placement;
pub mod profile_cache;
pub mod score;

pub use placement::{explain, select, Profile, Selection};

#[doc(hidden)]
pub mod test_support;

use std::collections::BTreeMap;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use corescout_core::error::{Error, Result};
use corescout_experiment::benchmarks::workloads::WorkloadKind;
use corescout_experiment::benchmarks::{BenchmarkResults, Measurement};
use corescout_substrate::topology::{CoreType, LogicalId, PhysicalId, Topology};
use score::{normalise, ranked, weighted, Better};

/// Bumped when the on-disk analysis format changes incompatibly, so a cached
/// profile from an older CoreScout is rejected rather than misread.
pub const SCHEMA_VERSION: u32 = 1;

/// Weights used to combine component scores into a profile recommendation.
///
/// These are judgement calls, stated here rather than buried in the code so
/// they can be argued with. The latency profile weights jitter above raw
/// latency deliberately: for a thread with a deadline, the worst case is the
/// budget, and a core that is 3% slower but has half the tail is the better
/// choice every time.
pub mod weights {
    pub const LATENCY_JITTER: f64 = 0.45;
    pub const LATENCY_LATENCY: f64 = 0.30;
    pub const LATENCY_STABILITY: f64 = 0.15;
    pub const LATENCY_COMPUTE: f64 = 0.10;

    pub const COMPUTE_COMPUTE: f64 = 0.85;
    pub const COMPUTE_STABILITY: f64 = 0.15;

    pub const MEMORY_MEMORY: f64 = 0.85;
    pub const MEMORY_STABILITY: f64 = 0.15;
}

/// Raw per-core metrics behind the scores, in physical units.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct CoreMetrics {
    /// Geometric mean of compute workload rates, in operations per second.
    pub compute_ops_per_second: Option<f64>,
    /// Geometric mean of memory workload rates, in hops per second.
    pub memory_ops_per_second: Option<f64>,
    /// Clean median time for one short operation, nanoseconds.
    pub latency_ns_per_op: Option<f64>,
    /// `p99 (including disturbed samples) - clean median`, nanoseconds per
    /// iteration. This is the tail a thread on this core has to budget for.
    pub jitter_p99_excess_ns: Option<f64>,
    /// Fraction of samples that were disturbed, averaged over workloads.
    pub contamination: f64,
}

/// Scores for one physical core, all on 0-100 where 100 is the best core in
/// this machine for that category.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreScores {
    pub core: PhysicalId,
    pub cpu: LogicalId,
    pub core_type: CoreType,
    pub numa_node: Option<u32>,
    pub smt_siblings: Vec<LogicalId>,
    /// Firmware preferred-core rank, 1 = the manufacturer's favourite.
    pub firmware_rank: Option<u32>,
    pub compute: Option<f64>,
    pub memory: Option<f64>,
    pub latency: Option<f64>,
    pub jitter: Option<f64>,
    /// How measurable the core was: 100 means every sample was clean.
    pub stability: f64,
    /// Equal-weighted blend of the four categories, for a single-column view.
    pub overall: Option<f64>,
    pub metrics: CoreMetrics,
}

/// One ranked list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ranking {
    /// Machine-readable id, e.g. `single_thread_compute`.
    pub id: String,
    /// Title for human output, e.g. `Single-thread compute`.
    pub title: String,
    /// What the `value` column contains, e.g. `us p99 excess`.
    pub unit: String,
    pub entries: Vec<RankEntry>,
}

/// One line of a ranking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankEntry {
    pub core: PhysicalId,
    pub cpu: LogicalId,
    pub score: f64,
    /// The underlying measurement in physical units.
    pub value: f64,
}

/// A single-core recommendation for a workload profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recommendation {
    pub core: PhysicalId,
    /// The logical CPU to pin to.
    pub cpu: LogicalId,
    /// SMT siblings that should be left idle (or excluded from the mask) for
    /// the recommendation to hold.
    pub keep_idle: Vec<LogicalId>,
    pub score: f64,
    /// Why this core, in plain language.
    pub reasons: Vec<String>,
}

/// A recommendation of two cores that should work well together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairRecommendation {
    pub cores: [PhysicalId; 2],
    pub cpus: [LogicalId; 2],
    pub score: f64,
    pub reasons: Vec<String>,
}

/// The recommendations CoreScout derived.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Recommendations {
    pub latency_critical: Option<Recommendation>,
    pub compute_heavy: Option<Recommendation>,
    pub memory_heavy: Option<Recommendation>,
    pub worker_pair: Option<PairRecommendation>,
}

/// The complete, serializable result of `corescout analyze`.
///
/// This is also the cached machine profile format, which is why it carries a
/// schema version and the full topology rather than just the scores.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    pub schema_version: u32,
    pub generated_unix: u64,
    pub topology: Topology,
    pub benchmark: BenchmarkResults,
    pub cores: Vec<CoreScores>,
    pub rankings: Vec<Ranking>,
    pub recommendations: Recommendations,
    /// Notable things CoreScout found, including disagreements with the
    /// firmware's preferred-core ordering.
    pub observations: Vec<String>,
}

impl Analysis {
    /// Build an analysis from a topology and its measurements.
    pub fn build(topology: Topology, benchmark: BenchmarkResults) -> Result<Analysis> {
        if benchmark.measurements.is_empty() {
            return Err(Error::invalid("no measurements to analyse"));
        }

        let cores = build_core_scores(&topology, &benchmark);
        let rankings = build_rankings(&cores);
        let recommendations = build_recommendations(&topology, &cores);
        let observations = build_observations(&topology, &cores, &recommendations);

        Ok(Analysis {
            schema_version: SCHEMA_VERSION,
            generated_unix: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            topology,
            benchmark,
            cores,
            rankings,
            recommendations,
            observations,
        })
    }

    /// Scores for a specific core.
    pub fn core(&self, id: PhysicalId) -> Option<&CoreScores> {
        self.cores.iter().find(|c| c.core == id)
    }

    /// Look up a ranking by id.
    pub fn ranking(&self, id: &str) -> Option<&Ranking> {
        self.rankings.iter().find(|r| r.id == id)
    }
}

/// Per-core metrics and scores.
fn build_core_scores(topology: &Topology, benchmark: &BenchmarkResults) -> Vec<CoreScores> {
    let mut metrics: BTreeMap<PhysicalId, CoreMetrics> = BTreeMap::new();
    let mut cpu_of: BTreeMap<PhysicalId, LogicalId> = BTreeMap::new();

    for core in benchmark.cores() {
        let ms: Vec<&Measurement> = benchmark
            .measurements
            .iter()
            .filter(|m| m.core == core)
            .collect();
        if ms.is_empty() {
            continue;
        }
        cpu_of.insert(core, ms[0].cpu);

        let rate_of = |kind: WorkloadKind| -> Option<f64> {
            // Geometric mean, because these are rates: an arithmetic mean of
            // an L1 chase at 300M hops/s and a DRAM chase at 12M hops/s is
            // just the L1 number wearing a hat.
            let values: Vec<f64> = ms
                .iter()
                .filter(|m| m.kind == kind && m.ops_per_second > 0.0)
                .map(|m| m.ops_per_second)
                .collect();
            geometric_mean(&values)
        };

        let latency_measurement = ms.iter().find(|m| m.kind == WorkloadKind::Latency);

        metrics.insert(
            core,
            CoreMetrics {
                compute_ops_per_second: rate_of(WorkloadKind::Compute),
                memory_ops_per_second: rate_of(WorkloadKind::Memory),
                latency_ns_per_op: latency_measurement.map(|m| m.ns_per_op),
                // The tail is taken from the *raw* distribution on purpose.
                // Interrupts and preemption are not measurement error here;
                // they are what this core does to a thread living on it, and
                // the interrupt load is genuinely unevenly distributed across
                // CPUs. Cleaning them out would hide the finding.
                jitter_p99_excess_ns: latency_measurement
                    .map(|m| (m.raw.p99 - m.clean.median).max(0.0)),
                contamination: ms.iter().map(|m| m.contamination()).sum::<f64>() / ms.len() as f64,
            },
        );
    }

    let collect = |f: fn(&CoreMetrics) -> Option<f64>| -> Vec<(PhysicalId, f64)> {
        metrics
            .iter()
            .filter_map(|(core, m)| f(m).map(|v| (*core, v)))
            .collect()
    };

    let compute_scores = normalise(&collect(|m| m.compute_ops_per_second), Better::Higher);
    let memory_scores = normalise(&collect(|m| m.memory_ops_per_second), Better::Higher);
    let latency_scores = normalise(&collect(|m| m.latency_ns_per_op), Better::Lower);
    // A core with a *zero* p99 excess would be perfect and would make every
    // other core score 0 by comparison; `normalise` drops non-positive values,
    // so we floor the excess at one nanosecond to keep the comparison sane.
    let jitter_input: Vec<(PhysicalId, f64)> = collect(|m| m.jitter_p99_excess_ns)
        .into_iter()
        .map(|(c, v)| (c, v.max(1.0)))
        .collect();
    let jitter_scores = normalise(&jitter_input, Better::Lower);

    let mut out = Vec::new();
    for (core, metric) in metrics {
        let cpu = cpu_of[&core];
        let topo_cpu = topology.cpu(cpu);
        let compute = compute_scores.get(&core).copied();
        let memory = memory_scores.get(&core).copied();
        let latency = latency_scores.get(&core).copied();
        let jitter = jitter_scores.get(&core).copied();
        let stability = (100.0 * (1.0 - metric.contamination)).clamp(0.0, 100.0);

        out.push(CoreScores {
            core,
            cpu,
            core_type: topology
                .core(core)
                .map(|c| c.core_type)
                .unwrap_or(CoreType::Unknown),
            numa_node: topo_cpu.and_then(|c| c.numa_node),
            smt_siblings: topo_cpu.map(|c| c.smt_siblings.clone()).unwrap_or_default(),
            firmware_rank: topo_cpu.and_then(|c| c.favored.as_ref().map(|f| f.rank)),
            compute,
            memory,
            latency,
            jitter,
            stability,
            overall: weighted(&[(compute, 1.0), (memory, 1.0), (latency, 1.0), (jitter, 1.0)]),
            metrics: metric,
        });
    }
    out.sort_by_key(|c| c.core);
    out
}

fn build_rankings(cores: &[CoreScores]) -> Vec<Ranking> {
    let mut rankings = Vec::new();

    let mut add = |id: &str,
                   title: &str,
                   unit: &str,
                   score_of: &dyn Fn(&CoreScores) -> Option<f64>,
                   value_of: &dyn Fn(&CoreScores) -> Option<f64>| {
        let scores: BTreeMap<PhysicalId, f64> = cores
            .iter()
            .filter_map(|c| score_of(c).map(|s| (c.core, s)))
            .collect();
        if scores.is_empty() {
            return;
        }
        let entries: Vec<RankEntry> = ranked(&scores)
            .into_iter()
            .map(|(core, score)| {
                let c = cores.iter().find(|c| c.core == core).expect("core exists");
                RankEntry {
                    core,
                    cpu: c.cpu,
                    score,
                    value: value_of(c).unwrap_or(0.0),
                }
            })
            .collect();
        rankings.push(Ranking {
            id: id.to_string(),
            title: title.to_string(),
            unit: unit.to_string(),
            entries,
        });
    };

    add(
        "single_thread_compute",
        "Single-thread compute",
        "Mops/s",
        &|c| c.compute,
        &|c| c.metrics.compute_ops_per_second.map(|v| v / 1e6),
    );
    add(
        "lowest_jitter",
        "Lowest jitter",
        "us p99 excess",
        &|c| c.jitter,
        &|c| c.metrics.jitter_p99_excess_ns.map(|v| v / 1000.0),
    );
    add(
        "lowest_latency",
        "Lowest latency",
        "ns/op",
        &|c| c.latency,
        &|c| c.metrics.latency_ns_per_op,
    );
    add(
        "memory_heavy",
        "Memory-heavy work",
        "Mhops/s",
        &|c| c.memory,
        &|c| c.metrics.memory_ops_per_second.map(|v| v / 1e6),
    );
    add("overall", "Overall", "score", &|c| c.overall, &|c| {
        c.overall
    });

    rankings
}

fn build_recommendations(topology: &Topology, cores: &[CoreScores]) -> Recommendations {
    Recommendations {
        latency_critical: pick(
            topology,
            cores,
            |c| {
                weighted(&[
                    (c.jitter, weights::LATENCY_JITTER),
                    (c.latency, weights::LATENCY_LATENCY),
                    (Some(c.stability), weights::LATENCY_STABILITY),
                    (c.compute, weights::LATENCY_COMPUTE),
                ])
            },
            |c| {
                let mut reasons = Vec::new();
                if let Some(us) = c.metrics.jitter_p99_excess_ns {
                    reasons.push(format!(
                        "tail latency {:.1} us above its own median, the lowest measured",
                        us / 1000.0
                    ));
                }
                if let Some(ns) = c.metrics.latency_ns_per_op {
                    reasons.push(format!("{ns:.2} ns per short operation"));
                }
                reasons.push(format!(
                    "{:.0}% of samples on this core were undisturbed",
                    c.stability
                ));
                reasons
            },
        ),
        compute_heavy: pick(
            topology,
            cores,
            |c| {
                weighted(&[
                    (c.compute, weights::COMPUTE_COMPUTE),
                    (Some(c.stability), weights::COMPUTE_STABILITY),
                ])
            },
            |c| {
                let mut reasons = Vec::new();
                if let Some(ops) = c.metrics.compute_ops_per_second {
                    reasons.push(format!(
                        "highest sustained compute rate measured ({:.0} Mops/s)",
                        ops / 1e6
                    ));
                }
                if let Some(rank) = c.firmware_rank {
                    reasons.push(format!(
                        "firmware ranks this core {rank} by preferred-core order"
                    ));
                }
                reasons
            },
        ),
        memory_heavy: pick(
            topology,
            cores,
            |c| {
                weighted(&[
                    (c.memory, weights::MEMORY_MEMORY),
                    (Some(c.stability), weights::MEMORY_STABILITY),
                ])
            },
            |c| {
                let mut reasons = Vec::new();
                if let Some(ops) = c.metrics.memory_ops_per_second {
                    reasons.push(format!(
                        "fastest dependent-load traversal ({:.1} Mhops/s)",
                        ops / 1e6
                    ));
                }
                if let Some(node) = c.numa_node {
                    reasons.push(format!("NUMA node {node}"));
                }
                reasons
            },
        ),
        worker_pair: pick_pair(topology, cores),
    }
}

/// Choose the best core by a scoring function, and explain the choice.
fn pick(
    topology: &Topology,
    cores: &[CoreScores],
    score_of: impl Fn(&CoreScores) -> Option<f64>,
    reasons_of: impl Fn(&CoreScores) -> Vec<String>,
) -> Option<Recommendation> {
    let best = cores
        .iter()
        .filter_map(|c| score_of(c).map(|s| (c, s)))
        .max_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .expect("scores are never NaN")
                // Lower core id wins ties, for a stable answer across runs.
                .then(b.0.core.cmp(&a.0.core))
        })?;

    let (core, score) = best;
    let mut reasons = reasons_of(core);
    if !core.smt_siblings.is_empty() {
        reasons.push(format!(
            "leave CPU {} idle: it is the SMT sibling sharing this core's execution units",
            corescout_core::cpuset::format_list(&core.smt_siblings)
        ));
    }
    if core.cpu == 0 {
        reasons.push(
            "note: CPU 0 is the default target for most interrupts on Linux; consider \
             moving IRQs elsewhere before relying on it"
                .to_string(),
        );
    }
    if topology.hybrid && core.core_type != CoreType::Unknown {
        reasons.push(format!("this is a {}-core", core.core_type.short()));
    }

    Some(Recommendation {
        core: core.core,
        cpu: core.cpu,
        keep_idle: core.smt_siblings.clone(),
        score,
        reasons,
    })
}

/// Choose two cores that should work well *together*.
///
/// The score is the weaker of the two cores' compute scores, not their average:
/// a parallel pair runs at the speed of its slower half, so pairing the fastest
/// core with a mediocre one is worse than pairing two good ones. Locality is
/// then a tie-break, because two threads that co-operate pay for every cache
/// line that has to cross an L3 or a NUMA boundary.
fn pick_pair(topology: &Topology, cores: &[CoreScores]) -> Option<PairRecommendation> {
    /// Bonus, in score points, for a pair sharing a last-level cache.
    const SHARED_L3_BONUS: f64 = 3.0;
    /// Penalty for a pair split across NUMA nodes.
    const CROSS_NUMA_PENALTY: f64 = 5.0;

    let candidates: Vec<&CoreScores> = cores.iter().filter(|c| c.compute.is_some()).collect();
    if candidates.len() < 2 {
        return None;
    }

    let mut best: Option<(f64, &CoreScores, &CoreScores, bool, bool)> = None;
    for (i, a) in candidates.iter().enumerate() {
        for b in candidates.iter().skip(i + 1) {
            let weaker = a.compute.unwrap().min(b.compute.unwrap());
            let shared_l3 = topology.shares_cache(a.cpu, b.cpu, 3);
            let cross_numa = match (a.numa_node, b.numa_node) {
                (Some(x), Some(y)) => x != y,
                _ => false,
            };
            let mut score = weaker;
            if shared_l3 {
                score += SHARED_L3_BONUS;
            }
            if cross_numa {
                score -= CROSS_NUMA_PENALTY;
            }
            if best.is_none() || score > best.as_ref().unwrap().0 {
                best = Some((score, a, b, shared_l3, cross_numa));
            }
        }
    }

    let (score, a, b, shared_l3, cross_numa) = best?;
    let mut reasons = vec![format!(
        "both cores are within {:.1}% of the fastest core measured",
        100.0 - a.compute.unwrap().min(b.compute.unwrap())
    )];
    if shared_l3 {
        reasons.push(
            "they share a last-level cache, so data passed between them stays on-die".to_string(),
        );
    } else if cross_numa {
        reasons.push(
            "no same-node pair scored better; this pair spans NUMA nodes, so keep their \
             working sets separate"
                .to_string(),
        );
    }
    if !a.smt_siblings.is_empty() || !b.smt_siblings.is_empty() {
        let mut idle: Vec<LogicalId> = a.smt_siblings.clone();
        idle.extend(b.smt_siblings.iter().copied());
        reasons.push(format!(
            "these are two distinct physical cores, not SMT siblings; leave CPU {} idle",
            corescout_core::cpuset::format_list(&idle)
        ));
    }

    Some(PairRecommendation {
        cores: [a.core, b.core],
        cpus: [a.cpu, b.cpu],
        score: score.min(100.0),
        reasons,
    })
}

fn build_observations(
    topology: &Topology,
    cores: &[CoreScores],
    recommendations: &Recommendations,
) -> Vec<String> {
    let mut notes = Vec::new();

    // Spread: is choosing a core worth the trouble on this machine at all?
    let compute: Vec<f64> = cores.iter().filter_map(|c| c.compute).collect();
    if let (Some(min), Some(_max)) = (
        compute.iter().copied().reduce(f64::min),
        compute.iter().copied().reduce(f64::max),
    ) {
        let spread = 100.0 - min;
        if spread < 3.0 {
            notes.push(format!(
                "compute performance varies by only {spread:.1}% across cores; on this machine \
                 pinning buys consistency, not throughput"
            ));
        } else {
            notes.push(format!(
                "the slowest core is {spread:.1}% behind the fastest on compute"
            ));
        }
    }

    // The headline question: does the firmware's bin sort match reality?
    if let Some(firmware_favourite) = topology.firmware_favored_cpu() {
        if let Some(measured_best) = cores.iter().filter(|c| c.compute.is_some()).max_by(|a, b| {
            a.compute
                .unwrap()
                .partial_cmp(&b.compute.unwrap())
                .expect("scores are never NaN")
        }) {
            let favoured_core = topology.core_of_cpu(firmware_favourite.id).map(|c| c.id);
            if favoured_core == Some(measured_best.core) {
                notes.push(format!(
                    "the firmware's preferred core (CPU {}) is also the fastest measured core",
                    firmware_favourite.id
                ));
            } else {
                let favoured_score = favoured_core
                    .and_then(|c| cores.iter().find(|s| s.core == c))
                    .and_then(|s| s.compute);
                match favoured_score {
                    Some(s) => notes.push(format!(
                        "the firmware's preferred core is CPU {}, but CPU {} measured faster \
                         ({:.1} vs {:.1}); factory binning does not account for this chassis, \
                         its cooling, or where the interrupts land",
                        firmware_favourite.id,
                        measured_best.cpu,
                        measured_best.compute.unwrap(),
                        s
                    )),
                    None => notes.push(format!(
                        "the firmware's preferred core is CPU {}, which was not measured",
                        firmware_favourite.id
                    )),
                }
            }
        }
    }

    // Does the best compute core differ from the best latency core?
    if let (Some(latency), Some(compute_rec)) = (
        recommendations.latency_critical.as_ref(),
        recommendations.compute_heavy.as_ref(),
    ) {
        if latency.core != compute_rec.core {
            notes.push(format!(
                "the lowest-jitter core (CPU {}) is not the fastest core (CPU {}); this is \
                 normal, and it is the reason CoreScout ranks them separately",
                latency.cpu, compute_rec.cpu
            ));
        }
    }

    // Interrupt load usually concentrates on the low-numbered CPUs.
    if let Some(worst) = cores
        .iter()
        .max_by(|a, b| a.metrics.contamination.total_cmp(&b.metrics.contamination))
    {
        if worst.metrics.contamination > 0.1 && worst.cpu <= 1 {
            notes.push(format!(
                "CPU {} saw the most disturbed samples ({:.0}%), which is the usual signature \
                 of a CPU carrying the machine's interrupt load",
                worst.cpu,
                worst.metrics.contamination * 100.0
            ));
        }
    }

    if topology.hybrid {
        notes.push(
            "this is a hybrid processor: P-cores and E-cores are not interchangeable, and a \
             ranking that mixes them will always put the P-cores on top"
                .to_string(),
        );
    }

    notes
}

/// Geometric mean, or `None` for an empty or non-positive input.
fn geometric_mean(values: &[f64]) -> Option<f64> {
    let usable: Vec<f64> = values
        .iter()
        .copied()
        .filter(|v| v.is_finite() && *v > 0.0)
        .collect();
    if usable.is_empty() {
        return None;
    }
    // Summing logarithms rather than multiplying: the product of six rates in
    // the hundreds of millions overflows nothing in f64, but the log form keeps
    // precision uniform and is the standard formulation.
    let log_sum: f64 = usable.iter().map(|v| v.ln()).sum();
    Some((log_sum / usable.len() as f64).exp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{analysis_fixture, measurement};
    use corescout_experiment::benchmarks::RunConfig;

    #[test]
    fn geometric_mean_behaves() {
        assert_eq!(geometric_mean(&[]), None);
        assert_eq!(geometric_mean(&[0.0, -1.0]), None);
        let g = geometric_mean(&[4.0, 9.0]).unwrap();
        assert!((g - 6.0).abs() < 1e-9);
    }

    #[test]
    fn empty_measurements_are_rejected() {
        let topology = corescout_substrate::test_support::fake_topology();
        let benchmark = BenchmarkResults {
            config: RunConfig::default(),
            clock_overhead_ns: 0.0,
            clock_resolution_ns: 0,
            started_unix: 0,
            duration_seconds: 0.0,
            measurements: vec![],
            warnings: vec![],
        };
        assert!(Analysis::build(topology, benchmark).is_err());
    }

    #[test]
    fn fastest_core_wins_compute_and_scores_100() {
        let a = analysis_fixture();
        let compute = a.ranking("single_thread_compute").unwrap();
        assert_eq!(compute.entries[0].core, 0);
        assert!((compute.entries[0].score - 100.0).abs() < 1e-6);
        // Core 1 is 10% slower in time, so about 9% slower in rate.
        assert!(compute.entries[1].score < 92.0);
    }

    #[test]
    fn quietest_core_wins_jitter_even_though_it_is_slower() {
        // The headline claim of the README, asserted.
        let a = analysis_fixture();
        let jitter = a.ranking("lowest_jitter").unwrap();
        assert_eq!(jitter.entries[0].core, 1);
        let rec = a.recommendations.latency_critical.as_ref().unwrap();
        assert_eq!(rec.core, 1, "latency recommendation must follow the tail");
        let compute_rec = a.recommendations.compute_heavy.as_ref().unwrap();
        assert_eq!(compute_rec.core, 0);
        assert!(
            a.observations
                .iter()
                .any(|o| o.contains("is not the fastest core")),
            "the compute/latency split should be called out: {:?}",
            a.observations
        );
    }

    #[test]
    fn memory_ranking_is_independent_of_compute() {
        let a = analysis_fixture();
        let mem = a.ranking("memory_heavy").unwrap();
        assert_eq!(mem.entries[0].core, 1, "core 1 has the faster chase");
    }

    #[test]
    fn recommendations_name_the_smt_sibling_to_keep_idle() {
        let a = analysis_fixture();
        let rec = a.recommendations.latency_critical.as_ref().unwrap();
        assert!(!rec.keep_idle.is_empty());
        assert!(rec.reasons.iter().any(|r| r.contains("SMT sibling")));
    }

    #[test]
    fn worker_pair_uses_two_distinct_physical_cores() {
        let a = analysis_fixture();
        let pair = a.recommendations.worker_pair.as_ref().unwrap();
        assert_ne!(pair.cores[0], pair.cores[1]);
        assert_ne!(pair.cpus[0], pair.cpus[1]);
        // Both fixture cores share an L3, so the bonus reason should appear.
        assert!(pair.reasons.iter().any(|r| r.contains("last-level cache")));
    }

    #[test]
    fn analysis_round_trips_through_json() {
        // Note: serde_json's float parser can lose a single ULP on some f64
        // values, so byte-identical struct equality is not achievable through
        // one encode/decode. What must hold, and what a cached profile relies
        // on, is that decoding is *stable* (a second round trip changes
        // nothing) and that every decision-carrying field survives.
        let a = analysis_fixture();
        let json = serde_json::to_string(&a).unwrap();
        let back: Analysis = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string(&back).unwrap();
        let back2: Analysis = serde_json::from_str(&json2).unwrap();
        assert_eq!(json2, serde_json::to_string(&back2).unwrap());

        assert_eq!(a.topology, back.topology);
        assert_eq!(a.observations, back.observations);

        // Recommendations carry f64 scores, so compare the parts that drive a
        // decision: which CPU, what to keep idle, and the stated reasoning.
        let rec_before = a.recommendations.latency_critical.as_ref().unwrap();
        let rec_after = back.recommendations.latency_critical.as_ref().unwrap();
        assert_eq!(rec_before.cpu, rec_after.cpu);
        assert_eq!(rec_before.keep_idle, rec_after.keep_idle);
        assert_eq!(rec_before.reasons, rec_after.reasons);
        assert!((rec_before.score - rec_after.score).abs() < 1e-9);
        assert_eq!(
            a.recommendations.worker_pair.as_ref().unwrap().cpus,
            back.recommendations.worker_pair.as_ref().unwrap().cpus
        );
        for (before, after) in a.cores.iter().zip(&back.cores) {
            assert_eq!(before.core, after.core);
            assert_eq!(before.cpu, after.cpu);
            assert!((before.compute.unwrap() - after.compute.unwrap()).abs() < 1e-9);
        }
        for (before, after) in a.rankings.iter().zip(&back.rankings) {
            assert_eq!(before.id, after.id);
            let ids_before: Vec<PhysicalId> = before.entries.iter().map(|e| e.core).collect();
            let ids_after: Vec<PhysicalId> = after.entries.iter().map(|e| e.core).collect();
            assert_eq!(ids_before, ids_after, "ranking order must survive caching");
        }
    }

    #[test]
    fn stability_reflects_contamination() {
        let topology = corescout_substrate::test_support::fake_topology();
        let mut m = measurement(0, 0, "int-alu", WorkloadKind::Compute, 100.0, 100.0, 10.0);
        m.samples_collected = 100;
        m.samples_used = 75;
        let benchmark = BenchmarkResults {
            config: RunConfig::default(),
            clock_overhead_ns: 0.0,
            clock_resolution_ns: 0,
            started_unix: 0,
            duration_seconds: 0.0,
            measurements: vec![m],
            warnings: vec![],
        };
        let a = Analysis::build(topology, benchmark).unwrap();
        assert!((a.cores[0].stability - 75.0).abs() < 1e-6);
    }
}
