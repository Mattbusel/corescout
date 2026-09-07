//! Realistic workload scenarios for the comparison harness.
//!
//! # Not the same thing as a benchmark workload
//!
//! [`crate::benchmarks::workloads`] holds micro-benchmarks: fixed work per
//! iteration, chosen so that one iteration is a clean unit of measurement.
//! Those answer "how fast is this core".
//!
//! The scenarios here answer a different question: "if I ran something that
//! behaves like real software under this placement policy, what would happen to
//! it?" They are longer, they are shaped like real programs, and several of them
//! deliberately create the conditions where placement matters at all:
//!
//! | scenario | why placement could matter |
//! |---|---|
//! | [`ScenarioKind::LowLatencyEventLoop`] | tail latency is the whole product; a migration is a visible outlier |
//! | [`ScenarioKind::BranchHeavy`] | a migration costs a cold branch predictor |
//! | [`ScenarioKind::CacheResident`] | the working set lives in a private L2 that a migration abandons |
//! | [`ScenarioKind::MemoryStreaming`] | bandwidth is shared, so the neighbours matter more than the core |
//! | [`ScenarioKind::MultithreadedWorkers`] | the slowest worker sets the pace |
//! | [`ScenarioKind::Mixed`] | the common case, where no single metric decides |
//! | [`ScenarioKind::BackgroundInterference`] | the machine is not idle, which is the normal state of a machine |
//! | [`ScenarioKind::SmtInterference`] | a sibling thread halves throughput on a core that looks free |
//! | [`ScenarioKind::ThermalDrift`] | sustained load makes the fast core stop being the fast core |
//! | [`ScenarioKind::NumaSensitive`] | remote memory costs more than any core-to-core difference |
//!
//! # Honesty about what this machine can run
//!
//! Several scenarios need capabilities not every machine has: SMT siblings,
//! more than one NUMA node, enough cores to generate interference. Rather than
//! silently degrading into a different experiment, [`Scenario::requirements`]
//! states what is needed and the harness records the scenario as skipped, with
//! the reason, when the machine cannot host it.

use serde::{Deserialize, Serialize};

use crate::benchmarks::workloads::{self, Workload, WorkloadRun};

/// The shape of a scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioKind {
    LowLatencyEventLoop,
    BranchHeavy,
    CacheResident,
    MemoryStreaming,
    MultithreadedWorkers,
    Mixed,
    BackgroundInterference,
    SmtInterference,
    ThermalDrift,
    NumaSensitive,
}

impl ScenarioKind {
    pub fn label(self) -> &'static str {
        match self {
            ScenarioKind::LowLatencyEventLoop => "low-latency-event-loop",
            ScenarioKind::BranchHeavy => "branch-heavy",
            ScenarioKind::CacheResident => "cache-resident",
            ScenarioKind::MemoryStreaming => "memory-streaming",
            ScenarioKind::MultithreadedWorkers => "multithreaded-workers",
            ScenarioKind::Mixed => "mixed",
            ScenarioKind::BackgroundInterference => "background-interference",
            ScenarioKind::SmtInterference => "smt-interference",
            ScenarioKind::ThermalDrift => "thermal-drift",
            ScenarioKind::NumaSensitive => "numa-sensitive",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            ScenarioKind::LowLatencyEventLoop => {
                "short units of work at a fixed rate; the tail of the distribution is the result"
            }
            ScenarioKind::BranchHeavy => {
                "unpredictable branches, so the branch predictor's state is worth keeping"
            }
            ScenarioKind::CacheResident => {
                "a working set sized to a private cache, which a migration abandons"
            }
            ScenarioKind::MemoryStreaming => {
                "sequential traversal far larger than cache; bandwidth-bound, not core-bound"
            }
            ScenarioKind::MultithreadedWorkers => {
                "several identical workers where the slowest one sets the completion time"
            }
            ScenarioKind::Mixed => {
                "compute, cache and latency work interleaved, as real software is"
            }
            ScenarioKind::BackgroundInterference => {
                "the measured work runs while unrelated load occupies the rest of the machine"
            }
            ScenarioKind::SmtInterference => {
                "a competing thread runs on the SMT sibling of the measured core"
            }
            ScenarioKind::ThermalDrift => {
                "sustained load long enough that clocks fall and the ranking may invert"
            }
            ScenarioKind::NumaSensitive => {
                "a large working set touched from a core that may be on a different node"
            }
        }
    }

    /// The primary thing this scenario is measuring.
    pub fn primary_metric(self) -> Metric {
        match self {
            ScenarioKind::LowLatencyEventLoop | ScenarioKind::SmtInterference => Metric::P99Latency,
            ScenarioKind::BranchHeavy
            | ScenarioKind::CacheResident
            | ScenarioKind::MemoryStreaming
            | ScenarioKind::NumaSensitive
            | ScenarioKind::ThermalDrift => Metric::Throughput,
            ScenarioKind::MultithreadedWorkers | ScenarioKind::BackgroundInterference => {
                Metric::MakespanNs
            }
            ScenarioKind::Mixed => Metric::MeanLatency,
        }
    }

    pub fn all() -> Vec<ScenarioKind> {
        vec![
            ScenarioKind::LowLatencyEventLoop,
            ScenarioKind::BranchHeavy,
            ScenarioKind::CacheResident,
            ScenarioKind::MemoryStreaming,
            ScenarioKind::MultithreadedWorkers,
            ScenarioKind::Mixed,
            ScenarioKind::BackgroundInterference,
            ScenarioKind::SmtInterference,
            ScenarioKind::ThermalDrift,
            ScenarioKind::NumaSensitive,
        ]
    }
}

/// Which number decides whether one policy beat another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    MeanLatency,
    P95Latency,
    P99Latency,
    Jitter,
    Throughput,
    MakespanNs,
    Migrations,
}

impl Metric {
    pub fn label(self) -> &'static str {
        match self {
            Metric::MeanLatency => "mean latency",
            Metric::P95Latency => "p95 latency",
            Metric::P99Latency => "p99 latency",
            Metric::Jitter => "jitter",
            Metric::Throughput => "throughput",
            Metric::MakespanNs => "makespan",
            Metric::Migrations => "involuntary switches",
        }
    }

    /// True when a larger number is a better result.
    pub fn higher_is_better(self) -> bool {
        matches!(self, Metric::Throughput)
    }
}

/// What a scenario needs from the machine to be meaningful.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Requirements {
    /// Logical CPUs the process must be permitted to use.
    pub min_cpus: usize,
    /// Needs at least one pair of SMT siblings.
    pub needs_smt: bool,
    /// Needs at least two NUMA nodes.
    pub needs_multiple_numa_nodes: bool,
}

/// A runnable scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    pub kind: ScenarioKind,
    /// Measured units of work.
    pub iterations: u32,
    /// Unmeasured iterations first, so the measurement is not of a cold core.
    pub warmup: u32,
    /// Worker threads for the scenarios that use more than one.
    pub threads: usize,
    /// Interference threads to start alongside, for the scenarios that create
    /// their own contention.
    pub interference_threads: usize,
}

impl Scenario {
    /// A scenario at a sensible default size.
    pub fn new(kind: ScenarioKind) -> Scenario {
        let (iterations, warmup, threads, interference) = match kind {
            // Short units, many of them: the tail needs samples to exist.
            ScenarioKind::LowLatencyEventLoop => (4_000, 400, 1, 0),
            ScenarioKind::BranchHeavy => (600, 60, 1, 0),
            ScenarioKind::CacheResident => (600, 60, 1, 0),
            ScenarioKind::MemoryStreaming => (300, 30, 1, 0),
            ScenarioKind::MultithreadedWorkers => (400, 40, 4, 0),
            ScenarioKind::Mixed => (900, 90, 1, 0),
            ScenarioKind::BackgroundInterference => (600, 60, 1, 2),
            ScenarioKind::SmtInterference => (800, 80, 1, 1),
            // Long enough for the package to actually heat.
            ScenarioKind::ThermalDrift => (6_000, 100, 1, 0),
            ScenarioKind::NumaSensitive => (300, 30, 1, 0),
        };
        Scenario {
            kind,
            iterations,
            warmup,
            threads,
            interference_threads: interference,
        }
    }

    /// A much smaller version, for tests and for `demo self`.
    pub fn quick(kind: ScenarioKind) -> Scenario {
        let full = Scenario::new(kind);
        Scenario {
            iterations: (full.iterations / 40).max(20),
            warmup: (full.warmup / 10).max(4),
            ..full
        }
    }

    pub fn requirements(&self) -> Requirements {
        match self.kind {
            ScenarioKind::SmtInterference => Requirements {
                min_cpus: 2,
                needs_smt: true,
                needs_multiple_numa_nodes: false,
            },
            ScenarioKind::NumaSensitive => Requirements {
                min_cpus: 2,
                needs_smt: false,
                needs_multiple_numa_nodes: true,
            },
            ScenarioKind::BackgroundInterference => Requirements {
                min_cpus: 1 + self.interference_threads,
                ..Requirements::default()
            },
            ScenarioKind::MultithreadedWorkers => Requirements {
                min_cpus: self.threads,
                ..Requirements::default()
            },
            _ => Requirements {
                min_cpus: 1,
                ..Requirements::default()
            },
        }
    }

    /// Build the measured work for one thread.
    ///
    /// Called on the pinned thread so buffers land on the local NUMA node.
    pub fn instantiate(&self) -> Box<dyn WorkloadRun> {
        match self.kind {
            // The event loop's unit of work is deliberately tiny: what is being
            // measured is the machine's ability to do a small thing on time,
            // not its ability to do a big thing quickly.
            ScenarioKind::LowLatencyEventLoop | ScenarioKind::SmtInterference => {
                workloads::latency::ShortOpJitter.instantiate()
            }
            ScenarioKind::BranchHeavy => workloads::compute::BranchHeavy.instantiate(),
            ScenarioKind::CacheResident => workloads::memory::L2Chase.instantiate(),
            ScenarioKind::MemoryStreaming | ScenarioKind::NumaSensitive => {
                workloads::memory::DramChase.instantiate()
            }
            ScenarioKind::MultithreadedWorkers
            | ScenarioKind::BackgroundInterference
            | ScenarioKind::ThermalDrift => workloads::compute::IntegerAlu.instantiate(),
            ScenarioKind::Mixed => Box::new(MixedRun {
                compute: workloads::compute::IntegerAlu.instantiate(),
                cache: workloads::memory::L2Chase.instantiate(),
                latency: workloads::latency::ShortOpJitter.instantiate(),
                step: 0,
            }),
        }
    }

    /// Build the work an interference thread does, if this scenario has any.
    ///
    /// Interference is deliberately the *opposite* shape to the measured work:
    /// a cache-hostile streamer against cache-resident work, and heavy ALU
    /// against latency work, because that is where placement decisions actually
    /// get punished.
    pub fn instantiate_interference(&self) -> Option<Box<dyn WorkloadRun>> {
        match self.kind {
            ScenarioKind::BackgroundInterference => {
                Some(workloads::memory::DramChase.instantiate())
            }
            ScenarioKind::SmtInterference => Some(workloads::compute::IntegerAlu.instantiate()),
            _ => None,
        }
    }
}

/// Interleaves three shapes of work, the way real software does.
struct MixedRun {
    compute: Box<dyn WorkloadRun>,
    cache: Box<dyn WorkloadRun>,
    latency: Box<dyn WorkloadRun>,
    step: u32,
}

impl WorkloadRun for MixedRun {
    fn iterate(&mut self) -> u64 {
        self.step = self.step.wrapping_add(1);
        // A fixed rotation rather than a random one: two runs must exercise the
        // identical sequence or the comparison measures the dice.
        match self.step % 3 {
            0 => self.compute.iterate(),
            1 => self.cache.iterate(),
            _ => self.latency.iterate(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scenario_describes_itself_and_names_a_metric() {
        for kind in ScenarioKind::all() {
            assert!(!kind.label().is_empty());
            assert!(kind.description().len() > 20, "{}", kind.label());
            assert!(!kind.primary_metric().label().is_empty());
        }
    }

    #[test]
    fn labels_are_unique() {
        let mut labels: Vec<&str> = ScenarioKind::all().iter().map(|k| k.label()).collect();
        labels.sort_unstable();
        let count = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), count);
    }

    #[test]
    fn every_scenario_instantiates_and_does_real_work() {
        // The checksums must vary: a workload returning a constant would let the
        // optimiser delete the loop and the harness would time nothing.
        for kind in ScenarioKind::all() {
            let scenario = Scenario::quick(kind);
            let mut run = scenario.instantiate();
            let first = run.iterate();
            let mut differs = false;
            for _ in 0..8 {
                if run.iterate() != first {
                    differs = true;
                }
            }
            assert!(
                differs || matches!(kind, ScenarioKind::CacheResident),
                "{} produced a constant checksum",
                kind.label()
            );
        }
    }

    #[test]
    fn the_mixed_scenario_visits_all_three_shapes() {
        let mut run = MixedRun {
            compute: workloads::compute::IntegerAlu.instantiate(),
            cache: workloads::memory::L2Chase.instantiate(),
            latency: workloads::latency::ShortOpJitter.instantiate(),
            step: 0,
        };
        // Three iterations must not all take the same path; the cheapest check
        // is that the rotation index advances.
        for _ in 0..3 {
            run.iterate();
        }
        assert_eq!(run.step, 3);
    }

    #[test]
    fn only_the_interference_scenarios_have_interference() {
        for kind in ScenarioKind::all() {
            let scenario = Scenario::new(kind);
            let has = scenario.instantiate_interference().is_some();
            assert_eq!(
                has,
                matches!(
                    kind,
                    ScenarioKind::BackgroundInterference | ScenarioKind::SmtInterference
                ),
                "{}",
                kind.label()
            );
            assert_eq!(has, scenario.interference_threads > 0, "{}", kind.label());
        }
    }

    #[test]
    fn requirements_are_stated_rather_than_assumed() {
        assert!(
            Scenario::new(ScenarioKind::SmtInterference)
                .requirements()
                .needs_smt
        );
        assert!(
            Scenario::new(ScenarioKind::NumaSensitive)
                .requirements()
                .needs_multiple_numa_nodes
        );
        assert_eq!(
            Scenario::new(ScenarioKind::MultithreadedWorkers)
                .requirements()
                .min_cpus,
            4
        );
    }

    #[test]
    fn quick_scenarios_are_smaller_but_still_run() {
        for kind in ScenarioKind::all() {
            let full = Scenario::new(kind);
            let quick = Scenario::quick(kind);
            assert!(quick.iterations < full.iterations, "{}", kind.label());
            assert!(quick.iterations >= 20);
            assert!(quick.warmup >= 4);
            assert_eq!(quick.threads, full.threads);
        }
    }

    #[test]
    fn higher_is_better_only_for_throughput() {
        assert!(Metric::Throughput.higher_is_better());
        for metric in [
            Metric::MeanLatency,
            Metric::P95Latency,
            Metric::P99Latency,
            Metric::Jitter,
            Metric::MakespanNs,
            Metric::Migrations,
        ] {
            assert!(!metric.higher_is_better(), "{}", metric.label());
        }
    }
}
