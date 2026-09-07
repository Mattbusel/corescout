//! Runs the scenario x policy matrix and reports it honestly.
//!
//! # The comparison this project has to survive
//!
//! The interesting claim is "a machine that models itself places work better
//! than the alternatives". That claim is only worth making if it is made in a
//! way that could have come out the other way, so the harness is built to let
//! it fail:
//!
//! **Interleaved, not batched.** Policies are visited in a rotating order,
//! round by round, exactly as the core benchmark visits cores. Running every
//! repetition of policy A and then every repetition of policy B would confound
//! the policy with the machine's thermal state, and the policy that happened to
//! go first would win.
//!
//! **Repetitions, and the spread is reported.** A single run of each policy
//! measures the machine's mood. Each policy is run several times and compared
//! on the median, with the spread carried into the verdict.
//!
//! **A difference smaller than the noise is not a difference.** [`Verdict`] has
//! an [`Verdict::Inconclusive`] arm and the harness uses it freely. Most
//! placement decisions on most machines genuinely do not matter, and a harness
//! that cannot say so is a machine for generating false claims.
//!
//! **The control is included.** [`crate::Baseline::Random`] runs alongside the
//! rest. If the self-model cannot beat random placement, the harness will say
//! that in the same words it would have used to report a success.
//!
//! # What it does not do
//!
//! It does not run the autonomous controller. [`crate::Baseline::SelfModel`] is
//! measured by handing the harness a placement the controller chose; keeping
//! the controller out of this crate is what stops the comparison from being
//! written by the thing being compared.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use corescout_core::clock;
use corescout_core::error::Result;
use corescout_core::CpuSet;
use corescout_substrate::platform::{self, Platform, SwitchCounters};

use crate::baselines::{Baseline, Placement};
use crate::benchmarks::stats::{self, Summary};
use crate::workloads::{Metric, Scenario, ScenarioKind};

/// How the harness is run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarnessConfig {
    /// Times each policy is measured. Rounds are interleaved.
    pub repetitions: u32,
    /// Fraction by which one policy's median must beat another's before the
    /// difference is called real, on top of the observed spread.
    pub minimum_effect: f64,
    /// Seed for the random baseline, so the control is reproducible.
    pub seed: u64,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        HarnessConfig {
            repetitions: 5,
            // Five per cent. Smaller differences than this are real on paper and
            // meaningless in practice on a machine anyone actually uses.
            minimum_effect: 0.05,
            seed: 0x5EED_1234,
        }
    }
}

/// One measured run of one scenario under one placement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub scenario: ScenarioKind,
    pub baseline: Baseline,
    /// Nanoseconds per measured iteration, cleaned of known-contaminated
    /// samples.
    pub latency: Summary,
    /// Iterations per second, derived from the median.
    pub throughput: f64,
    /// Relative spread: the tail as a multiple of the median.
    pub jitter: f64,
    /// Wall time from first iteration to last.
    pub makespan_ns: u64,
    /// Involuntary context switches during the measured window. A migration
    /// shows up here, which is the closest thing to a migration count that is
    /// available without perf.
    pub involuntary_switches: Option<u64>,
    /// Iterations excluded because the scheduler preempted us during them.
    pub contaminated: usize,
}

impl Measurement {
    /// Read one metric off this measurement.
    pub fn metric(&self, metric: Metric) -> Option<f64> {
        Some(match metric {
            Metric::MeanLatency => self.latency.mean,
            Metric::P95Latency => self.latency.p95,
            Metric::P99Latency => self.latency.p99,
            Metric::Jitter => self.jitter,
            Metric::Throughput => self.throughput,
            Metric::MakespanNs => self.makespan_ns as f64,
            Metric::Migrations => self.involuntary_switches? as f64,
        })
    }
}

/// What happened to one policy across its repetitions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyResult {
    pub placement: Placement,
    pub runs: Vec<Measurement>,
    /// Median of the scenario's primary metric across repetitions.
    pub median: f64,
    /// Half the interquartile range, used as the noise floor.
    pub spread: f64,
}

impl PolicyResult {
    fn summarise(placement: Placement, runs: Vec<Measurement>, metric: Metric) -> PolicyResult {
        let mut values: Vec<f64> = runs.iter().filter_map(|run| run.metric(metric)).collect();
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median = if values.is_empty() {
            f64::NAN
        } else {
            stats::percentile_sorted(&values, 50.0)
        };
        let spread = if values.len() < 4 {
            // Too few points for a quartile; the full range is the honest
            // answer, and it is deliberately pessimistic.
            values
                .last()
                .zip(values.first())
                .map(|(hi, lo)| (hi - lo) / 2.0)
                .unwrap_or(f64::NAN)
        } else {
            (stats::percentile_sorted(&values, 75.0) - stats::percentile_sorted(&values, 25.0))
                / 2.0
        };
        PolicyResult {
            placement,
            runs,
            median,
            spread,
        }
    }
}

/// Why a scenario did not run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Skipped {
    pub scenario: ScenarioKind,
    pub reason: String,
}

/// The comparison between two policies on one scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// One policy beat the other by more than the noise.
    Better {
        winner: Baseline,
        loser: Baseline,
        /// Fractional improvement, always positive.
        margin: f64,
    },
    /// The difference is smaller than the noise, or smaller than the minimum
    /// effect worth claiming. This is the most common honest answer.
    Inconclusive { reason: String },
}

impl Verdict {
    pub fn summary(&self) -> String {
        match self {
            Verdict::Better {
                winner,
                loser,
                margin,
            } => format!(
                "{} beat {} by {:.1}%",
                winner.label(),
                loser.label(),
                margin * 100.0
            ),
            Verdict::Inconclusive { reason } => format!("inconclusive: {reason}"),
        }
    }

    pub fn is_conclusive(&self) -> bool {
        matches!(self, Verdict::Better { .. })
    }
}

/// Everything measured for one scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioResult {
    pub scenario: ScenarioKind,
    pub metric: Metric,
    #[serde(with = "corescout_core::serde_util::pairs")]
    pub policies: BTreeMap<Baseline, PolicyResult>,
    /// The best policy, when one is distinguishable from the rest.
    pub best: Option<Baseline>,
}

impl ScenarioResult {
    /// Compare two policies on this scenario.
    ///
    /// The comparison is deliberately conservative: the difference must exceed
    /// both policies' spread *and* the configured minimum effect.
    pub fn compare(&self, a: Baseline, b: Baseline, minimum_effect: f64) -> Verdict {
        let (Some(left), Some(right)) = (self.policies.get(&a), self.policies.get(&b)) else {
            return Verdict::Inconclusive {
                reason: "one of the policies was not measured".into(),
            };
        };
        if !left.median.is_finite() || !right.median.is_finite() {
            return Verdict::Inconclusive {
                reason: "the metric was not measurable".into(),
            };
        }

        let higher_wins = self.metric.higher_is_better();
        let (better, worse, better_id, worse_id) = if (left.median > right.median) == higher_wins {
            (left, right, a, b)
        } else {
            (right, left, b, a)
        };

        let denominator = worse.median.abs().max(f64::MIN_POSITIVE);
        let margin = (better.median - worse.median).abs() / denominator;
        let noise = (better.spread.max(0.0) + worse.spread.max(0.0)) / denominator;

        if margin < minimum_effect {
            return Verdict::Inconclusive {
                reason: format!(
                    "{:.1}% apart, below the {:.0}% worth claiming",
                    margin * 100.0,
                    minimum_effect * 100.0
                ),
            };
        }
        if margin <= noise {
            return Verdict::Inconclusive {
                reason: format!(
                    "{:.1}% apart but the runs vary by {:.1}%",
                    margin * 100.0,
                    noise * 100.0
                ),
            };
        }
        Verdict::Better {
            winner: better_id,
            loser: worse_id,
            margin,
        }
    }

    /// Every pairwise verdict against a reference policy.
    pub fn against(&self, reference: Baseline, minimum_effect: f64) -> Vec<(Baseline, Verdict)> {
        self.policies
            .keys()
            .filter(|policy| **policy != reference)
            .map(|policy| (*policy, self.compare(*policy, reference, minimum_effect)))
            .collect()
    }
}

/// A whole comparison run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    pub config: HarnessConfig,
    pub machine: String,
    pub results: Vec<ScenarioResult>,
    pub skipped: Vec<Skipped>,
}

impl Comparison {
    /// A plain-language report.
    ///
    /// Written to be readable by someone deciding whether to believe the
    /// project, which means the inconclusive results are printed as prominently
    /// as the wins.
    pub fn report(&self, reference: Baseline) -> String {
        let mut out = format!(
            "comparison on {} ({} repetitions, vs {})\n",
            self.machine,
            self.config.repetitions,
            reference.label()
        );
        for result in &self.results {
            out.push_str(&format!(
                "\n{}  [{}]\n",
                result.scenario.label(),
                result.metric.label()
            ));
            for (policy, verdict) in result.against(reference, self.config.minimum_effect) {
                out.push_str(&format!("  {:<20} {}\n", policy.label(), verdict.summary()));
            }
        }
        for skipped in &self.skipped {
            out.push_str(&format!(
                "\n{}  skipped: {}\n",
                skipped.scenario.label(),
                skipped.reason
            ));
        }
        if self.results.is_empty() {
            out.push_str("\nnothing was measured\n");
        }
        out
    }

    /// Scenarios where the reference was beaten, and by whom.
    pub fn wins_against(&self, reference: Baseline) -> Vec<(ScenarioKind, Baseline, f64)> {
        let mut wins = Vec::new();
        for result in &self.results {
            for (_, verdict) in result.against(reference, self.config.minimum_effect) {
                if let Verdict::Better {
                    winner,
                    loser,
                    margin,
                } = verdict
                {
                    if loser == reference {
                        wins.push((result.scenario, winner, margin));
                    }
                }
            }
        }
        wins
    }
}

/// Runs scenarios under placements.
pub struct Harness {
    config: HarnessConfig,
    platform: Box<dyn Platform>,
    permitted: CpuSet,
    smt_available: bool,
    numa_nodes: usize,
}

impl std::fmt::Debug for Harness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Harness")
            .field("platform", &self.platform.name())
            .field("permitted", &self.permitted.len())
            .field("smt", &self.smt_available)
            .field("numa_nodes", &self.numa_nodes)
            .finish()
    }
}

impl Harness {
    /// Build a harness for this machine.
    pub fn detect(config: HarnessConfig) -> Result<Harness> {
        let platform = platform::detect();
        let permitted = platform.process_affinity()?;
        let topology = platform.discover_topology()?;
        let smt_available = topology.smt_enabled();
        let numa_nodes = topology.numa_nodes.len().max(1);
        Ok(Harness {
            config,
            platform,
            permitted,
            smt_available,
            numa_nodes,
        })
    }

    pub fn permitted(&self) -> &CpuSet {
        &self.permitted
    }

    pub fn config(&self) -> &HarnessConfig {
        &self.config
    }

    /// Why this machine cannot host a scenario, if it cannot.
    pub fn cannot_run(&self, scenario: &Scenario) -> Option<String> {
        let needs = scenario.requirements();
        if self.permitted.len() < needs.min_cpus {
            return Some(format!(
                "needs {} permitted CPUs, this process has {}",
                needs.min_cpus,
                self.permitted.len()
            ));
        }
        if needs.needs_smt && !self.smt_available {
            return Some("no SMT siblings on this machine".into());
        }
        if needs.needs_multiple_numa_nodes && self.numa_nodes < 2 {
            return Some("only one NUMA node on this machine".into());
        }
        None
    }

    /// Run one scenario once under one placement.
    pub fn measure(&self, scenario: &Scenario, placement: &Placement) -> Result<Measurement> {
        if let Some(cpus) = &placement.cpus {
            self.platform.set_current_thread_affinity(cpus)?;
        }

        let mut run = scenario.instantiate();
        let mut sink = 0u64;
        for _ in 0..scenario.warmup {
            sink = sink.wrapping_add(run.iterate());
        }

        let mut samples = Vec::with_capacity(scenario.iterations as usize);
        let mut dirty = Vec::with_capacity(scenario.iterations as usize);
        let before_switches = self.platform.thread_switch_counters();
        let started = clock::now_ns();

        for _ in 0..scenario.iterations {
            let switches_before = self.platform.thread_switch_counters();
            let start = clock::now_ns();
            sink = sink.wrapping_add(run.iterate());
            let elapsed = clock::now_ns().saturating_sub(start) as f64;
            let switches_after = self.platform.thread_switch_counters();

            // An iteration the scheduler interrupted is *known* bad, not merely
            // slow. It is excluded from the clean statistics and counted, since
            // the contamination rate is itself a result.
            let preempted = match (switches_before, switches_after) {
                (Some(before), Some(after)) => after.involuntary > before.involuntary,
                _ => false,
            };
            if preempted {
                dirty.push(elapsed);
            } else {
                samples.push(elapsed);
            }
        }

        let makespan_ns = clock::now_ns().saturating_sub(started);
        let after_switches = self.platform.thread_switch_counters();
        // Consume the checksum so the optimiser cannot delete the work.
        std::hint::black_box(sink);

        // If the machine was so busy that everything was contaminated, fall
        // back to the dirty samples rather than reporting nothing: a fully
        // contaminated measurement is still the truth about that machine.
        let usable = if samples.len() >= 4 { &samples } else { &dirty };
        let latency = Summary::from_samples(usable).unwrap_or(Summary {
            count: 0,
            min: f64::NAN,
            max: f64::NAN,
            mean: f64::NAN,
            median: f64::NAN,
            p95: f64::NAN,
            p99: f64::NAN,
            mad: f64::NAN,
            stddev: f64::NAN,
            tail_excess: f64::NAN,
        });

        let throughput = if latency.median > 0.0 {
            1e9 / latency.median
        } else {
            f64::NAN
        };

        Ok(Measurement {
            scenario: scenario.kind,
            baseline: placement.baseline,
            jitter: latency.relative_jitter(),
            throughput,
            latency,
            makespan_ns,
            involuntary_switches: switch_delta(before_switches, after_switches),
            contaminated: dirty.len(),
        })
    }

    /// Run every policy on one scenario, interleaved, and summarise.
    ///
    /// `extra` lets a caller add placements the baselines cannot produce,
    /// notably the autonomous controller's own choice.
    pub fn run_scenario(
        &self,
        scenario: &Scenario,
        ranking: Option<u32>,
        extra: &[Placement],
    ) -> Result<ScenarioResult> {
        let mut placements: Vec<Placement> = Baseline::all_static()
            .into_iter()
            .map(|baseline| {
                crate::baselines::decide(baseline, &self.permitted, None, ranking, self.config.seed)
            })
            .collect();
        placements.extend_from_slice(extra);

        let mut runs: BTreeMap<Baseline, Vec<Measurement>> = BTreeMap::new();
        for round in 0..self.config.repetitions {
            // Rotate the visiting order each round so no policy systematically
            // inherits another's thermal wake.
            let offset = round as usize % placements.len().max(1);
            for index in 0..placements.len() {
                let placement = &placements[(index + offset) % placements.len()];
                let measurement = self.measure(scenario, placement)?;
                runs.entry(placement.baseline)
                    .or_default()
                    .push(measurement);
            }
        }

        // Leave the thread as we found it, so the harness does not silently
        // outlive its own experiment.
        let _ = self.platform.set_current_thread_affinity(&self.permitted);

        let metric = scenario.kind.primary_metric();
        let policies: BTreeMap<Baseline, PolicyResult> = placements
            .into_iter()
            .filter_map(|placement| {
                let baseline = placement.baseline;
                let measurements = runs.remove(&baseline)?;
                Some((
                    baseline,
                    PolicyResult::summarise(placement, measurements, metric),
                ))
            })
            .collect();

        let best = best_policy(&policies, metric, self.config.minimum_effect);
        Ok(ScenarioResult {
            scenario: scenario.kind,
            metric,
            policies,
            best,
        })
    }

    /// Run the whole matrix.
    pub fn run(&self, scenarios: &[Scenario], ranking: Option<u32>) -> Result<Comparison> {
        let mut results = Vec::new();
        let mut skipped = Vec::new();
        for scenario in scenarios {
            match self.cannot_run(scenario) {
                Some(reason) => skipped.push(Skipped {
                    scenario: scenario.kind,
                    reason,
                }),
                None => results.push(self.run_scenario(scenario, ranking, &[])?),
            }
        }
        Ok(Comparison {
            config: self.config.clone(),
            machine: self.platform.name().to_string(),
            results,
            skipped,
        })
    }
}

/// The single best policy, or `None` when the top two are indistinguishable.
///
/// Refusing to name a winner is the point: a leaderboard that always has a
/// first place manufactures a result on every machine where placement does not
/// matter, which is most of them.
fn best_policy(
    policies: &BTreeMap<Baseline, PolicyResult>,
    metric: Metric,
    minimum_effect: f64,
) -> Option<Baseline> {
    let mut ranked: Vec<(&Baseline, &PolicyResult)> = policies
        .iter()
        .filter(|(_, result)| result.median.is_finite())
        .collect();
    if ranked.len() < 2 {
        return ranked.first().map(|(baseline, _)| **baseline);
    }
    ranked.sort_by(|a, b| {
        let ordering =
            a.1.median
                .partial_cmp(&b.1.median)
                .unwrap_or(std::cmp::Ordering::Equal);
        if metric.higher_is_better() {
            ordering.reverse()
        } else {
            ordering
        }
    });

    let first = ranked[0].1.median;
    let second = ranked[1].1.median;
    let denominator = second.abs().max(f64::MIN_POSITIVE);
    let margin = (first - second).abs() / denominator;
    let noise = (ranked[0].1.spread.max(0.0) + ranked[1].1.spread.max(0.0)) / denominator;
    if margin >= minimum_effect && margin > noise {
        Some(*ranked[0].0)
    } else {
        None
    }
}

fn switch_delta(before: Option<SwitchCounters>, after: Option<SwitchCounters>) -> Option<u64> {
    Some(after?.delta(before?).involuntary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(metric: Metric, entries: &[(Baseline, f64, f64)]) -> ScenarioResult {
        let policies = entries
            .iter()
            .map(|(baseline, median, spread)| {
                (
                    *baseline,
                    PolicyResult {
                        placement: Placement::none(*baseline, "test"),
                        runs: Vec::new(),
                        median: *median,
                        spread: *spread,
                    },
                )
            })
            .collect();
        ScenarioResult {
            scenario: ScenarioKind::Mixed,
            metric,
            policies,
            best: None,
        }
    }

    #[test]
    fn a_clear_latency_win_is_reported_as_one() {
        let scenario = result(
            Metric::P99Latency,
            &[
                (Baseline::SelfModel, 100.0, 1.0),
                (Baseline::Random, 200.0, 1.0),
            ],
        );
        let verdict = scenario.compare(Baseline::SelfModel, Baseline::Random, 0.05);
        match verdict {
            Verdict::Better { winner, margin, .. } => {
                assert_eq!(winner, Baseline::SelfModel);
                assert!((margin - 0.5).abs() < 1e-9);
            }
            other => panic!("expected a win, got {other:?}"),
        }
    }

    #[test]
    fn for_throughput_the_larger_number_wins() {
        let scenario = result(
            Metric::Throughput,
            &[
                (Baseline::SelfModel, 200.0, 1.0),
                (Baseline::Random, 100.0, 1.0),
            ],
        );
        let verdict = scenario.compare(Baseline::SelfModel, Baseline::Random, 0.05);
        assert!(matches!(
            verdict,
            Verdict::Better {
                winner: Baseline::SelfModel,
                ..
            }
        ));
    }

    #[test]
    fn a_difference_smaller_than_the_noise_is_inconclusive() {
        // The central honesty requirement: two policies that differ by less
        // than the machine's own variation have not been distinguished.
        let scenario = result(
            Metric::P99Latency,
            &[
                (Baseline::SelfModel, 100.0, 30.0),
                (Baseline::Scheduler, 130.0, 30.0),
            ],
        );
        let verdict = scenario.compare(Baseline::SelfModel, Baseline::Scheduler, 0.05);
        assert!(!verdict.is_conclusive(), "{}", verdict.summary());
        assert!(verdict.summary().contains("vary by"));
    }

    #[test]
    fn a_difference_too_small_to_matter_is_inconclusive_even_without_noise() {
        let scenario = result(
            Metric::P99Latency,
            &[
                (Baseline::SelfModel, 100.0, 0.0),
                (Baseline::Scheduler, 101.0, 0.0),
            ],
        );
        let verdict = scenario.compare(Baseline::SelfModel, Baseline::Scheduler, 0.05);
        assert!(!verdict.is_conclusive());
        assert!(verdict.summary().contains("worth claiming"));
    }

    #[test]
    fn losing_to_the_control_is_reported_as_plainly_as_winning() {
        // If the self-model loses to random placement the harness must say so.
        let scenario = result(
            Metric::P99Latency,
            &[
                (Baseline::SelfModel, 200.0, 1.0),
                (Baseline::Random, 100.0, 1.0),
            ],
        );
        let verdict = scenario.compare(Baseline::SelfModel, Baseline::Random, 0.05);
        match verdict {
            Verdict::Better { winner, loser, .. } => {
                assert_eq!(winner, Baseline::Random);
                assert_eq!(loser, Baseline::SelfModel);
            }
            other => panic!("expected the control to win, got {other:?}"),
        }
    }

    #[test]
    fn an_unmeasured_policy_is_not_a_win() {
        let scenario = result(Metric::P99Latency, &[(Baseline::SelfModel, 100.0, 1.0)]);
        let verdict = scenario.compare(Baseline::SelfModel, Baseline::Random, 0.05);
        assert!(!verdict.is_conclusive());
    }

    #[test]
    fn an_unmeasurable_metric_is_not_a_win() {
        let scenario = result(
            Metric::P99Latency,
            &[
                (Baseline::SelfModel, f64::NAN, f64::NAN),
                (Baseline::Random, 100.0, 1.0),
            ],
        );
        assert!(!scenario
            .compare(Baseline::SelfModel, Baseline::Random, 0.05)
            .is_conclusive());
    }

    #[test]
    fn no_best_policy_is_named_when_the_top_two_are_indistinguishable() {
        let scenario = result(
            Metric::P99Latency,
            &[
                (Baseline::SelfModel, 100.0, 20.0),
                (Baseline::Scheduler, 102.0, 20.0),
                (Baseline::Random, 400.0, 5.0),
            ],
        );
        assert_eq!(best_policy(&scenario.policies, scenario.metric, 0.05), None);
    }

    #[test]
    fn a_clear_best_policy_is_named() {
        let scenario = result(
            Metric::P99Latency,
            &[
                (Baseline::SelfModel, 100.0, 1.0),
                (Baseline::Scheduler, 300.0, 1.0),
                (Baseline::Random, 400.0, 1.0),
            ],
        );
        assert_eq!(
            best_policy(&scenario.policies, scenario.metric, 0.05),
            Some(Baseline::SelfModel)
        );
    }

    #[test]
    fn the_report_prints_skipped_scenarios_with_their_reason() {
        let comparison = Comparison {
            config: HarnessConfig::default(),
            machine: "test".into(),
            results: Vec::new(),
            skipped: vec![Skipped {
                scenario: ScenarioKind::NumaSensitive,
                reason: "only one NUMA node on this machine".into(),
            }],
        };
        let report = comparison.report(Baseline::Scheduler);
        assert!(report.contains("numa-sensitive"));
        assert!(report.contains("only one NUMA node"));
        assert!(report.contains("nothing was measured"));
    }

    #[test]
    fn wins_are_only_counted_against_the_named_reference() {
        let comparison = Comparison {
            config: HarnessConfig::default(),
            machine: "test".into(),
            results: vec![result(
                Metric::P99Latency,
                &[
                    (Baseline::SelfModel, 100.0, 1.0),
                    (Baseline::Scheduler, 300.0, 1.0),
                ],
            )],
            skipped: Vec::new(),
        };
        assert_eq!(comparison.wins_against(Baseline::Scheduler).len(), 1);
        assert!(comparison.wins_against(Baseline::SelfModel).is_empty());
    }

    #[test]
    fn a_comparison_round_trips_through_json() {
        // Baseline is a map key, which is exactly the shape that has broken
        // serialisation elsewhere in this workspace.
        let comparison = Comparison {
            config: HarnessConfig::default(),
            machine: "test".into(),
            results: vec![result(
                Metric::Throughput,
                &[(Baseline::SelfModel, 1.0, 0.1)],
            )],
            skipped: Vec::new(),
        };
        let text = serde_json::to_string(&comparison).expect("serialises");
        let back: Comparison = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back.machine, "test");
        assert_eq!(back.results.len(), 1);
    }
}
