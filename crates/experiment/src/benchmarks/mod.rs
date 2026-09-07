//! The benchmark engine.
//!
//! # Methodology
//!
//! The engine exists to answer one question honestly: *given this machine as it
//! is right now, how does each physical core actually perform?* Everything here
//! is shaped by the ways that question is normally answered badly.
//!
//! **Interleaved rounds, not batches.** The naive approach measures core 0
//! completely, then core 1, and so on. That silently confounds core identity
//! with time: turbo budget drains, the package heats up, a background job
//! starts. Whichever core was measured first gets an unearned advantage.
//! CoreScout instead runs *rounds*; each round visits every core once, in an
//! order reshuffled per round, and the samples from all rounds are pooled. Drift
//! and background load then hit every core roughly equally, and the shuffling
//! stops any core from systematically inheriting the previous core's thermal
//! wake.
//!
//! **Fresh state per visit.** Workload buffers are allocated on the pinned
//! thread at the start of every visit, so first-touch NUMA placement is local to
//! the core being measured, and no core is measured against another core's warm
//! cache.
//!
//! **Warm before measuring.** Each visit runs unmeasured warmup iterations
//! first. A core that has been idle is at its lowest P-state with cold caches,
//! cold TLBs and an empty branch predictor; measuring that would rank cores by
//! how recently they happened to run something.
//!
//! **Interference is detected, not assumed away.** Every timed iteration is
//! bracketed by a read of the thread's involuntary context-switch counter. An
//! iteration during which the scheduler preempted us is *known* bad, not merely
//! statistically suspicious, and is excluded from the clean statistics while
//! still being counted and reported. On top of that, a robust one-sided outlier
//! test catches disturbances that do not involve a context switch, such as
//! interrupts and SMI.
//!
//! **Nothing is thrown away silently.** Both the cleaned and the raw
//! distributions are kept, along with the contamination rate. A core whose
//! measurements had to be heavily cleaned is a core you should not trust with
//! latency-critical work, and that fact is itself a result.

// The monotonic clock is a shared primitive, not an experiment concept: the
// mirror times its own observation passes with it. It lives at the crate root
// and is re-exported here so existing paths keep working.
pub use corescout_core::clock;
pub mod stats;
pub mod workloads;

use std::collections::BTreeMap;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use corescout_core::error::{Error, Result};
use corescout_substrate::platform::Platform;
use corescout_substrate::topology::{LogicalId, PhysicalId, Topology};
use stats::{SampleClass, Summary};
use workloads::{Direction, Rng, Workload, WorkloadKind, WorkloadSpec};

/// How thoroughly to measure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunConfig {
    /// Interleaved passes over the whole core set.
    pub rounds: u32,
    /// Timed iterations collected per core per round.
    pub samples_per_visit: u32,
    /// Unmeasured iterations run at the start of every visit.
    pub warmup_iterations: u32,
    /// Robust z-score above which a sample counts as an outlier.
    pub outlier_threshold: f64,
    /// Seed for the per-round core ordering. Fixed by default so a repeated
    /// run visits cores in the same sequence.
    pub seed: u64,
    /// Measure every logical CPU rather than one per physical core. Off by
    /// default: measuring both SMT threads of a core sequentially tells you
    /// very little, since a lone thread on a core has the whole core.
    pub all_logical_cpus: bool,
}

impl Default for RunConfig {
    fn default() -> Self {
        RunConfig {
            // 5 rounds x 6 samples = 30 samples per core per workload, and the
            // jitter workload multiplies that to 180. Enough for a stable
            // median and a usable p99 without a multi-minute run.
            rounds: 5,
            samples_per_visit: 6,
            warmup_iterations: 3,
            // 5 sigma-equivalents above the median. Loose enough that ordinary
            // frequency wobble is not called an outlier, tight enough to catch
            // a stolen timeslice.
            outlier_threshold: 5.0,
            seed: 0x0C0F_FEE0_5C00_7ADE,
            all_logical_cpus: false,
        }
    }
}

impl RunConfig {
    /// A fast, less precise configuration, for `--quick`.
    pub fn quick() -> Self {
        RunConfig {
            rounds: 3,
            samples_per_visit: 4,
            warmup_iterations: 2,
            ..Self::default()
        }
    }

    /// Total samples collected per core for a workload with the given spec.
    pub fn samples_for(&self, spec: &WorkloadSpec) -> u32 {
        self.rounds * self.samples_per_visit * spec.sample_multiplier
    }
}

/// One workload measured on one core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measurement {
    pub core: PhysicalId,
    /// The logical CPU that was actually pinned.
    pub cpu: LogicalId,
    pub workload: String,
    pub kind: WorkloadKind,
    pub direction: Direction,
    /// Iteration times in nanoseconds, outliers and preempted samples removed.
    pub clean: Summary,
    /// Iteration times including every sample collected.
    pub raw: Summary,
    /// Derived rate, using the clean median. Meaningless for latency
    /// workloads, where `ns_per_op` is the number to read.
    pub ops_per_second: f64,
    /// Clean median divided by the workload's declared operation count.
    pub ns_per_op: f64,
    pub samples_collected: usize,
    pub samples_used: usize,
    /// Samples discarded by the statistical outlier test.
    pub outliers: usize,
    /// Samples during which the kernel reported an involuntary context switch.
    /// These are known-bad rather than merely suspicious.
    pub preempted: usize,
}

impl Measurement {
    /// Fraction of samples that were disturbed, by either signal.
    ///
    /// This is a quality metric for the *measurement*, and simultaneously a
    /// real property of the *core*: a core that cannot be measured cleanly is a
    /// core that will not run your latency-critical thread cleanly either.
    pub fn contamination(&self) -> f64 {
        if self.samples_collected == 0 {
            return 1.0;
        }
        (self.samples_collected - self.samples_used) as f64 / self.samples_collected as f64
    }

    /// The value used for ranking, already oriented so that higher is better.
    pub fn ranking_value(&self) -> f64 {
        match self.direction {
            Direction::HigherIsBetter => self.ops_per_second,
            // Invert so every ranking works the same way downstream.
            Direction::LowerIsBetter => {
                if self.ns_per_op > 0.0 {
                    1.0 / self.ns_per_op
                } else {
                    0.0
                }
            }
        }
    }
}

/// Everything one `benchmark` run produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkResults {
    pub config: RunConfig,
    /// Cost of a single clock read, reported rather than subtracted.
    pub clock_overhead_ns: f64,
    pub clock_resolution_ns: u64,
    /// Unix timestamp when the run started.
    pub started_unix: u64,
    pub duration_seconds: f64,
    pub measurements: Vec<Measurement>,
    /// Conditions that make the numbers less trustworthy.
    pub warnings: Vec<String>,
}

impl BenchmarkResults {
    /// Measurements for one workload, in core order.
    pub fn by_workload(&self, workload_id: &str) -> Vec<&Measurement> {
        let mut v: Vec<&Measurement> = self
            .measurements
            .iter()
            .filter(|m| m.workload == workload_id)
            .collect();
        v.sort_by_key(|m| m.core);
        v
    }

    /// All measurements of a given family, grouped by core.
    pub fn by_core(&self, kind: WorkloadKind) -> BTreeMap<PhysicalId, Vec<&Measurement>> {
        let mut map: BTreeMap<PhysicalId, Vec<&Measurement>> = BTreeMap::new();
        for m in self.measurements.iter().filter(|m| m.kind == kind) {
            map.entry(m.core).or_default().push(m);
        }
        map
    }

    /// Distinct cores that were measured.
    pub fn cores(&self) -> Vec<PhysicalId> {
        let mut v: Vec<PhysicalId> = self.measurements.iter().map(|m| m.core).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Workload ids present, in the order they were run.
    pub fn workload_ids(&self) -> Vec<String> {
        let mut seen = Vec::new();
        for m in &self.measurements {
            if !seen.contains(&m.workload) {
                seen.push(m.workload.clone());
            }
        }
        seen
    }
}

/// Progress notification, so the CLI can show something during a long run
/// without the engine knowing anything about terminals.
pub enum Progress<'a> {
    RoundStarted {
        round: u32,
        of: u32,
        workload: &'a str,
    },
    CoreFinished {
        core: PhysicalId,
        cpu: LogicalId,
    },
}

/// The benchmark engine.
pub struct Runner<'a> {
    platform: &'a dyn Platform,
    config: RunConfig,
}

impl<'a> Runner<'a> {
    pub fn new(platform: &'a dyn Platform, config: RunConfig) -> Self {
        Runner { platform, config }
    }

    /// Measure every benchmarkable core with every supplied workload.
    ///
    /// Runs on the calling thread, which it repeatedly re-pins; the caller's
    /// original affinity is restored before returning, including on error.
    pub fn run(
        &self,
        topology: &Topology,
        workload_set: &[Box<dyn Workload>],
        progress: &mut dyn FnMut(Progress<'_>),
    ) -> Result<BenchmarkResults> {
        let targets = self.targets(topology)?;
        let original_affinity = self.platform.current_thread_affinity()?;
        let started = SystemTime::now();

        let clock_overhead_ns = clock::overhead_ns();
        let clock_resolution_ns = clock::resolution_ns();
        let mut warnings = self.preflight_warnings(topology, &targets, clock_resolution_ns);

        let start_ns = clock::now_ns();
        // The pin is restored even if a workload panics or a syscall fails,
        // so an interrupted run does not leave the shell's affinity narrowed.
        let outcome = self.run_inner(&targets, workload_set, progress);
        let duration_seconds = (clock::now_ns() - start_ns) as f64 / 1e9;
        let _ = self
            .platform
            .set_current_thread_affinity(&original_affinity);

        let measurements = outcome?;
        warnings.extend(postflight_warnings(&measurements));

        Ok(BenchmarkResults {
            config: self.config.clone(),
            clock_overhead_ns,
            clock_resolution_ns,
            started_unix: started
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            duration_seconds,
            measurements,
            warnings,
        })
    }

    /// (physical core, logical cpu) pairs to measure.
    fn targets(&self, topology: &Topology) -> Result<Vec<(PhysicalId, LogicalId)>> {
        let mut targets = Vec::new();
        for core in topology.benchmarkable_cores() {
            let usable: Vec<LogicalId> = core
                .logical_cpus
                .iter()
                .copied()
                .filter(|c| {
                    topology.online_cpus.contains(c) && topology.process_affinity.contains(c)
                })
                .collect();
            if usable.is_empty() {
                continue;
            }
            if self.config.all_logical_cpus {
                for cpu in usable {
                    targets.push((core.id, cpu));
                }
            } else {
                // One thread per physical core, so each measurement has the
                // core's full execution resources rather than half of them.
                targets.push((core.id, usable[0]));
            }
        }
        if targets.is_empty() {
            return Err(Error::invalid(
                "no cores are both online and inside this process's CPU affinity mask",
            ));
        }
        Ok(targets)
    }

    fn run_inner(
        &self,
        targets: &[(PhysicalId, LogicalId)],
        workload_set: &[Box<dyn Workload>],
        progress: &mut dyn FnMut(Progress<'_>),
    ) -> Result<Vec<Measurement>> {
        // samples[(core, workload)] -> (durations_ns, preempted_flags)
        let mut samples: BTreeMap<(PhysicalId, String), (Vec<f64>, Vec<bool>)> = BTreeMap::new();

        for workload in workload_set {
            let spec = workload.spec();
            let rounds = self.config.rounds * spec.sample_multiplier;

            for round in 0..rounds {
                // Reshuffle every round so no core is permanently downstream of
                // another core's thermal wake.
                let mut order: Vec<(PhysicalId, LogicalId)> = targets.to_vec();
                shuffle(
                    &mut order,
                    self.config.seed ^ (round as u64) << 32 ^ hash_id(spec.id),
                );

                progress(Progress::RoundStarted {
                    round: round + 1,
                    of: rounds,
                    workload: spec.name,
                });

                for (core, cpu) in order {
                    let (durations, preempted) = self.visit(cpu, workload.as_ref())?;
                    let entry = samples
                        .entry((core, spec.id.to_string()))
                        .or_insert_with(|| (Vec::new(), Vec::new()));
                    entry.0.extend(durations);
                    entry.1.extend(preempted);
                    progress(Progress::CoreFinished { core, cpu });
                }
            }
        }

        // Turn raw samples into measurements.
        let mut measurements = Vec::new();
        for workload in workload_set {
            let spec = workload.spec();
            for (core, cpu) in targets {
                let Some((durations, preempted)) = samples.get(&(*core, spec.id.to_string()))
                else {
                    continue;
                };
                if let Some(m) = summarise(*core, *cpu, &spec, durations, preempted, &self.config) {
                    measurements.push(m);
                }
            }
        }
        Ok(measurements)
    }

    /// One visit: pin, allocate, warm up, then collect timed samples.
    fn visit(&self, cpu: LogicalId, workload: &dyn Workload) -> Result<(Vec<f64>, Vec<bool>)> {
        self.platform.pin_current_thread(cpu)?;

        // Allocated *after* pinning: see the module docs on first touch.
        let mut run = workload.instantiate();

        let mut sink = 0u64;
        for _ in 0..self.config.warmup_iterations {
            sink ^= run.iterate();
        }

        let n = self.config.samples_per_visit as usize;
        let mut durations = Vec::with_capacity(n);
        let mut preempted = Vec::with_capacity(n);

        for _ in 0..n {
            // Counter reads bracket the timed region rather than sitting inside
            // it, so their cost is not attributed to the workload.
            let before = self.platform.thread_switch_counters();
            let t0 = clock::now_ns();
            let checksum = run.iterate();
            let t1 = clock::now_ns();
            let after = self.platform.thread_switch_counters();

            sink ^= checksum;
            durations.push(t1.saturating_sub(t0) as f64);
            preempted.push(match (before, after) {
                (Some(b), Some(a)) => a.delta(b).involuntary > 0,
                // No per-thread counters on this platform: fall back to the
                // statistical test alone.
                _ => false,
            });
        }

        // Consume the checksum so none of the work above can be elided.
        std::hint::black_box(sink);
        Ok((durations, preempted))
    }

    fn preflight_warnings(
        &self,
        topology: &Topology,
        targets: &[(PhysicalId, LogicalId)],
        clock_resolution_ns: u64,
    ) -> Vec<String> {
        let mut warnings = Vec::new();

        if targets.len() < topology.physical_count() {
            warnings.push(format!(
                "only {} of {} physical cores are available to this process; \
                 the rest are offline or outside its affinity mask",
                targets.len(),
                topology.physical_count()
            ));
        }
        if targets.len() < 2 {
            warnings.push(
                "only one core could be measured, so the rankings are not comparisons".to_string(),
            );
        }
        if clock_resolution_ns > 1_000 {
            warnings.push(format!(
                "clock resolution is {clock_resolution_ns} ns; jitter figures below a \
                 microsecond are not meaningful on this clocksource (typical on VMs \
                 without a stable TSC)"
            ));
        }
        if topology.smt_enabled() && !self.config.all_logical_cpus {
            warnings.push(
                "SMT is enabled: results describe a core with one thread active. A core \
                 shared with a busy sibling will perform worse than measured here"
                    .to_string(),
            );
        }
        warnings
    }
}

/// Fold raw samples into a [`Measurement`].
fn summarise(
    core: PhysicalId,
    cpu: LogicalId,
    spec: &WorkloadSpec,
    durations: &[f64],
    preempted: &[bool],
    config: &RunConfig,
) -> Option<Measurement> {
    let raw = Summary::from_samples(durations)?;

    // Two-stage cleaning. Known-bad samples (the kernel told us we were
    // preempted) go first, because leaving them in would inflate the MAD and
    // let the statistical test miss genuine outliers.
    let after_preemption: Vec<f64> = durations
        .iter()
        .zip(preempted)
        .filter(|(_, p)| !**p)
        .map(|(d, _)| *d)
        .collect();
    let preempted_count = durations.len() - after_preemption.len();

    let classes = stats::classify_outliers(&after_preemption, config.outlier_threshold);
    let clean_samples: Vec<f64> = after_preemption
        .iter()
        .zip(&classes)
        .filter(|(_, c)| **c == SampleClass::Clean)
        .map(|(d, _)| *d)
        .collect();
    let outliers = classes
        .iter()
        .filter(|c| **c == SampleClass::Outlier)
        .count();

    // If cleaning removed everything, the raw data is all we have; reporting it
    // with a high contamination figure is more useful than reporting nothing.
    let clean = Summary::from_samples(&clean_samples).unwrap_or(raw);

    let ns_per_op = clean.median / spec.ops_per_iteration as f64;
    let ops_per_second = if clean.median > 0.0 {
        spec.ops_per_iteration as f64 / (clean.median / 1e9)
    } else {
        0.0
    };

    Some(Measurement {
        core,
        cpu,
        workload: spec.id.to_string(),
        kind: spec.kind,
        direction: spec.direction,
        clean,
        raw,
        ops_per_second,
        ns_per_op,
        samples_collected: durations.len(),
        samples_used: clean.count,
        outliers,
        preempted: preempted_count,
    })
}

fn postflight_warnings(measurements: &[Measurement]) -> Vec<String> {
    let mut warnings = Vec::new();
    if measurements.is_empty() {
        return warnings;
    }
    let worst = measurements
        .iter()
        .max_by(|a, b| {
            a.contamination()
                .partial_cmp(&b.contamination())
                .expect("contamination is never NaN")
        })
        .expect("non-empty");
    if worst.contamination() > 0.25 {
        warnings.push(format!(
            "{:.0}% of samples on CPU {} were disturbed during `{}`; the machine is busy, \
             and these results describe a contended system rather than an idle one",
            worst.contamination() * 100.0,
            worst.cpu,
            worst.workload
        ));
    }
    warnings
}

/// Fisher-Yates using the deterministic workload RNG.
fn shuffle<T>(items: &mut [T], seed: u64) {
    let mut rng = Rng::new(seed);
    for i in (1..items.len()).rev() {
        let j = rng.next_below(i + 1);
        items.swap(i, j);
    }
}

/// Stable hash of a workload id, so each workload gets its own visit order.
fn hash_id(id: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64; // FNV-1a
    for b in id.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmarks::workloads::WorkloadSpec;

    fn spec() -> WorkloadSpec {
        WorkloadSpec {
            id: "test",
            name: "Test",
            description: "test workload",
            kind: WorkloadKind::Compute,
            direction: Direction::HigherIsBetter,
            ops_per_iteration: 1_000,
            sample_multiplier: 1,
        }
    }

    #[test]
    fn summarise_excludes_preempted_and_outlier_samples() {
        let mut durations = vec![1_000.0; 20];
        let mut preempted = vec![false; 20];
        // One sample the kernel told us was preempted...
        durations.push(1_050.0);
        preempted.push(true);
        // ...and one statistical outlier it did not.
        durations.push(90_000.0);
        preempted.push(false);

        let m = summarise(0, 0, &spec(), &durations, &preempted, &RunConfig::default()).unwrap();
        assert_eq!(m.samples_collected, 22);
        assert_eq!(m.preempted, 1);
        assert_eq!(m.outliers, 1);
        assert_eq!(m.samples_used, 20);
        assert!((m.clean.median - 1_000.0).abs() < 1e-9);
        assert!(m.raw.max > 89_000.0, "raw statistics keep everything");
    }

    #[test]
    fn contamination_is_the_discarded_fraction() {
        let durations = vec![100.0; 10];
        let preempted = vec![
            true, false, false, false, false, false, false, false, false, false,
        ];
        let m = summarise(0, 0, &spec(), &durations, &preempted, &RunConfig::default()).unwrap();
        assert!((m.contamination() - 0.1).abs() < 1e-9);
    }

    #[test]
    fn rates_are_derived_from_the_clean_median() {
        // 1000 ops in 1_000_000 ns = 1e6 ops/sec, 1000 ns/op.
        let durations = vec![1_000_000.0; 10];
        let m = summarise(
            0,
            0,
            &spec(),
            &durations,
            &[false; 10],
            &RunConfig::default(),
        )
        .unwrap();
        assert!((m.ops_per_second - 1_000_000.0).abs() < 1.0);
        assert!((m.ns_per_op - 1_000.0).abs() < 1e-9);
        assert!((m.ranking_value() - 1_000_000.0).abs() < 1.0);
    }

    #[test]
    fn lower_is_better_workloads_invert_their_ranking_value() {
        let mut s = spec();
        s.direction = Direction::LowerIsBetter;
        let fast = summarise(
            0,
            0,
            &s,
            &[1_000.0; 10],
            &[false; 10],
            &RunConfig::default(),
        )
        .unwrap();
        let slow = summarise(
            1,
            1,
            &s,
            &[2_000.0; 10],
            &[false; 10],
            &RunConfig::default(),
        )
        .unwrap();
        assert!(
            fast.ranking_value() > slow.ranking_value(),
            "lower latency must rank higher"
        );
    }

    #[test]
    fn empty_sample_set_produces_no_measurement() {
        assert!(summarise(0, 0, &spec(), &[], &[], &RunConfig::default()).is_none());
    }

    #[test]
    fn shuffle_is_deterministic_and_a_permutation() {
        let mut a: Vec<u32> = (0..32).collect();
        let mut b = a.clone();
        shuffle(&mut a, 7);
        shuffle(&mut b, 7);
        assert_eq!(a, b);
        let mut sorted = a.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..32).collect::<Vec<u32>>());
        assert_ne!(a, sorted, "shuffle left the order untouched");
    }

    #[test]
    fn different_rounds_get_different_orders() {
        let mut a: Vec<u32> = (0..32).collect();
        let mut b = a.clone();
        shuffle(&mut a, 1);
        shuffle(&mut b, 2);
        assert_ne!(a, b);
    }

    #[test]
    fn sample_counts_account_for_the_multiplier() {
        let config = RunConfig::default();
        let mut s = spec();
        assert_eq!(
            config.samples_for(&s),
            config.rounds * config.samples_per_visit
        );
        s.sample_multiplier = 6;
        assert_eq!(
            config.samples_for(&s),
            config.rounds * config.samples_per_visit * 6
        );
    }

    #[test]
    fn results_round_trip_through_json() {
        let m = summarise(
            0,
            0,
            &spec(),
            &[500.0; 8],
            &[false; 8],
            &RunConfig::default(),
        )
        .unwrap();
        let results = BenchmarkResults {
            config: RunConfig::default(),
            clock_overhead_ns: 20.0,
            clock_resolution_ns: 40,
            started_unix: 1_700_000_000,
            duration_seconds: 1.5,
            measurements: vec![m],
            warnings: vec!["test".into()],
        };
        let json = serde_json::to_string(&results).unwrap();
        let back: BenchmarkResults = serde_json::from_str(&json).unwrap();
        assert_eq!(results, back);
    }
}
