//! Benchmark workloads.
//!
//! # Design rules every workload here obeys
//!
//! 1. **Fixed work per iteration.** The amount of work is a compile-time
//!    constant, never calibrated at runtime. Calibration would make each run
//!    measure a slightly different thing, and cross-machine comparison of
//!    results would stop meaning anything. The constants are chosen so one
//!    iteration takes roughly 100-500 us on a current x86 core: long enough
//!    that clock overhead is negligible, short enough to usually fit inside one
//!    scheduler timeslice, so a preemption shows up as one ruined iteration
//!    rather than a tax spread invisibly across all of them.
//!
//! 2. **The result is consumed.** Every iteration returns a checksum that the
//!    runner folds into a `black_box`. Without this LLVM is entitled to delete
//!    the whole loop, and the benchmark would proudly report the speed of
//!    nothing.
//!
//! 3. **State is allocated after pinning.** [`Workload::instantiate`] is called
//!    on the pinned thread, so first-touch page allocation places the buffers on
//!    the NUMA node local to the core under test. Allocating up front and
//!    sharing the buffer would measure remote memory latency on every node but
//!    one.
//!
//! 4. **Deterministic inputs.** Any randomness comes from a fixed-seed
//!    generator, so two runs on the same machine exercise the identical access
//!    pattern and branch sequence.

pub mod compute;
pub mod latency;
pub mod memory;

use serde::{Deserialize, Serialize};

/// Broad family a workload belongs to. Rankings are computed per family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadKind {
    /// ALU, branch and FP work that stays in registers.
    Compute,
    /// Cache and memory hierarchy traversal.
    Memory,
    /// Short operations whose *distribution* is the point.
    Latency,
}

impl WorkloadKind {
    pub fn label(&self) -> &'static str {
        match self {
            WorkloadKind::Compute => "compute",
            WorkloadKind::Memory => "memory",
            WorkloadKind::Latency => "latency",
        }
    }
}

/// Which direction is "better" for a workload's raw measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// More operations per second is better (compute, memory bandwidth).
    HigherIsBetter,
    /// Fewer nanoseconds is better (latency, jitter).
    LowerIsBetter,
}

/// Static description of a workload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadSpec {
    /// Stable machine-readable id, used as the JSON key.
    pub id: &'static str,
    /// Short human-readable name.
    pub name: &'static str,
    /// What it measures and why the number moves.
    pub description: &'static str,
    pub kind: WorkloadKind,
    pub direction: Direction,
    /// Countable operations performed by one call to
    /// [`WorkloadRun::iterate`]. Used to normalise timings into a rate so
    /// workloads with different iteration costs are comparable.
    pub ops_per_iteration: u64,
    /// Relative number of iterations to collect, applied on top of the run
    /// configuration's base count. Cheap workloads with interesting tails
    /// (jitter) ask for more samples than expensive throughput ones.
    pub sample_multiplier: u32,
}

/// A workload definition: turns into a [`WorkloadRun`] on the target core.
pub trait Workload: Send + Sync {
    fn spec(&self) -> WorkloadSpec;

    /// Allocate per-core state.
    ///
    /// Called on the already-pinned benchmark thread. See rule 3 above.
    fn instantiate(&self) -> Box<dyn WorkloadRun>;
}

/// Live, per-core instance of a workload.
pub trait WorkloadRun {
    /// Perform exactly one iteration's worth of work and return a checksum
    /// derived from the result. The caller times this call.
    fn iterate(&mut self) -> u64;
}

/// The full default workload set, in report order.
pub fn default_workloads() -> Vec<Box<dyn Workload>> {
    vec![
        Box::new(compute::IntegerAlu),
        Box::new(compute::BranchHeavy),
        Box::new(compute::FloatScalar),
        Box::new(memory::L1Chase),
        Box::new(memory::L2Chase),
        Box::new(memory::DramChase),
        Box::new(latency::ShortOpJitter),
    ]
}

/// Workloads belonging to a given family.
pub fn workloads_of_kind(kind: WorkloadKind) -> Vec<Box<dyn Workload>> {
    default_workloads()
        .into_iter()
        .filter(|w| w.spec().kind == kind)
        .collect()
}

/// A tiny, fast, deterministic PRNG (xorshift64*).
///
/// Used to build unpredictable-but-reproducible branch patterns and pointer
/// chase permutations. Not cryptographic, and does not need to be: the only
/// requirement is that the hardware's predictors and prefetchers cannot learn
/// the pattern, while two runs produce the identical pattern.
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        // Zero is a fixed point of xorshift; forbid it.
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    #[inline(always)]
    pub(crate) fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-ish value in `0..n`. The modulo bias is irrelevant here.
    #[inline(always)]
    pub(crate) fn next_below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// Build a single cycle visiting every index of a `len`-element array exactly
/// once, in random order (a random cyclic permutation via Sattolo's algorithm).
///
/// A *cycle* rather than a shuffle matters: pointer chasing round a single
/// cycle guarantees every load depends on the previous one, so the measurement
/// is memory *latency* and cannot be hidden by the core's memory-level
/// parallelism. A shuffled non-cyclic order could contain short loops.
pub(crate) fn build_chase_cycle(len: usize, seed: u64) -> Vec<usize> {
    assert!(len > 1, "a chase needs at least two slots");
    let mut order: Vec<usize> = (0..len).collect();
    let mut rng = Rng::new(seed);
    // Sattolo: swap with a strictly earlier element, which produces a
    // permutation consisting of exactly one cycle.
    for i in (1..len).rev() {
        let j = rng.next_below(i);
        order.swap(i, j);
    }
    // `order` is a cyclic sequence of slots; turn it into next-pointers.
    let mut next = vec![0usize; len];
    for w in 0..len {
        next[order[w]] = order[(w + 1) % len];
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn rng_is_deterministic_and_not_stuck() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(1);
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            let v = a.next_u64();
            assert_eq!(v, b.next_u64());
            seen.insert(v);
        }
        assert_eq!(seen.len(), 1000, "generator repeated within 1000 draws");
    }

    #[test]
    fn rng_zero_seed_still_works() {
        let mut r = Rng::new(0);
        assert_ne!(r.next_u64(), 0);
    }

    #[test]
    fn chase_visits_every_slot_exactly_once() {
        let len = 1024;
        let next = build_chase_cycle(len, 42);
        let mut seen = vec![false; len];
        let mut p = 0usize;
        for _ in 0..len {
            assert!(!seen[p], "cycle revisited slot {p} early");
            seen[p] = true;
            p = next[p];
        }
        assert_eq!(
            p, 0,
            "chase did not return to its start: not a single cycle"
        );
        assert!(seen.iter().all(|s| *s));
    }

    #[test]
    fn chase_is_reproducible() {
        assert_eq!(build_chase_cycle(256, 7), build_chase_cycle(256, 7));
        assert_ne!(build_chase_cycle(256, 7), build_chase_cycle(256, 8));
    }

    #[test]
    fn every_default_workload_has_a_unique_id_and_sane_spec() {
        let mut ids = HashSet::new();
        for w in default_workloads() {
            let spec = w.spec();
            assert!(ids.insert(spec.id), "duplicate workload id {}", spec.id);
            assert!(spec.ops_per_iteration > 0);
            assert!(spec.sample_multiplier >= 1);
            assert!(!spec.description.is_empty());
        }
    }

    #[test]
    fn each_family_is_represented() {
        for kind in [
            WorkloadKind::Compute,
            WorkloadKind::Memory,
            WorkloadKind::Latency,
        ] {
            assert!(
                !workloads_of_kind(kind).is_empty(),
                "no workloads for {:?}",
                kind
            );
        }
    }

    /// Every workload must actually compute something that varies with its
    /// input state, and must not be optimised into a constant.
    #[test]
    fn workloads_produce_checksums() {
        for w in default_workloads() {
            let mut run = w.instantiate();
            let first = run.iterate();
            assert_ne!(first, 0, "{} produced a zero checksum", w.spec().id);
        }
    }
}
