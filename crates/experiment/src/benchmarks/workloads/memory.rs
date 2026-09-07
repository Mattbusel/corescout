//! Memory hierarchy workloads: L1-resident, L2-sized, and out-of-cache.
//!
//! All three are *pointer chases* around a single random cycle, one cache line
//! per node. That choice is deliberate:
//!
//! - Each load's address depends on the previous load's result, so the core
//!   cannot overlap them. The number that comes out is genuine load-to-use
//!   latency rather than the memory controller's peak bandwidth.
//! - A random cycle defeats both the sequential and the stride prefetchers. A
//!   sequential walk of a 64 MiB buffer measures the prefetcher, which is a
//!   real and useful thing to know but tells you nothing about how a core
//!   handles a cache miss it could not see coming.
//! - One line per node means the working set size in bytes equals the number of
//!   nodes times 64, so "does this fit in L2" is exact rather than approximate.
//!
//! Sizes are fixed constants rather than derived from the detected cache sizes.
//! Deriving them would make two machines run different benchmarks and make
//! their scores incomparable; the fixed sizes are instead chosen to sit
//! unambiguously inside L1, inside L2 but outside L1, and outside any L3
//! shipping today.

use std::hint::black_box;

use super::{build_chase_cycle, Direction, Workload, WorkloadKind, WorkloadRun, WorkloadSpec};

/// One node per cache line, so the buffer size is exactly `nodes * 64` bytes.
const LINE: usize = 64;

/// 16 KiB: fits in every x86 L1d (32-48 KiB) alongside the rest of the loop.
const L1_BYTES: usize = 16 * 1024;
/// 512 KiB: larger than any current L1, smaller than the 1-2 MiB L2 slices on
/// current Intel and AMD cores.
const L2_BYTES: usize = 512 * 1024;
/// 64 MiB: larger than the L3 of mainstream desktop and most server parts, so
/// the chase reaches DRAM. On a large-L3 part (X3D, big Xeon) this degrades
/// into an L3 latency measurement, which the report notes rather than hides.
const DRAM_BYTES: usize = 64 * 1024 * 1024;

// Compile-time guards on the sizing above. These are the assumptions the whole
// memory family rests on, so a bad edit should fail the build rather than
// quietly measure the wrong level of the hierarchy.
const _: () = assert!(L1_BYTES < 32 * 1024, "the L1 set must fit a 32 KiB L1d");
const _: () = assert!(L2_BYTES > 64 * 1024, "the L2 set must not fit in any L1");
const _: () = assert!(L2_BYTES < 1024 * 1024, "the L2 set must fit a 1 MiB L2");
const _: () = assert!(
    DRAM_BYTES > 32 * 1024 * 1024,
    "the DRAM set must exceed most L3s"
);
const _: () = assert!(L1_BYTES % LINE == 0 && L2_BYTES % LINE == 0 && DRAM_BYTES % LINE == 0);

const L1_HOPS: u64 = 150_000;
const L2_HOPS: u64 = 60_000;
const DRAM_HOPS: u64 = 8_000;

/// Slots per cache line: we store one `usize` pointer per line and leave the
/// rest of the line unused, so that touching `n` nodes really does touch
/// `n * 64` bytes rather than `n * 8`.
const SLOTS_PER_LINE: usize = LINE / std::mem::size_of::<usize>();

/// Shared implementation of a pointer chase over `bytes` of memory.
struct Chase {
    /// Flat buffer of exactly `bytes` bytes. Only every `SLOTS_PER_LINE`-th
    /// element is a live next-pointer; the gaps are padding that forces each
    /// hop onto its own cache line.
    slots: Vec<usize>,
    hops: u64,
    cursor: usize,
}

impl Chase {
    fn new(bytes: usize, hops: u64, seed: u64) -> Chase {
        let nodes = bytes / LINE;
        assert!(nodes > 1, "chase working set must span at least two lines");
        let cycle = build_chase_cycle(nodes, seed);

        // Expand node indices into slot indices, one live pointer per line.
        let mut slots = vec![0usize; nodes * SLOTS_PER_LINE];
        for (node, next_node) in cycle.iter().enumerate() {
            slots[node * SLOTS_PER_LINE] = next_node * SLOTS_PER_LINE;
        }

        // Touch every page once so they are faulted in and, on a NUMA machine,
        // first-touch places them on the node local to the pinned thread.
        // Without this the first timed iteration would be measuring page
        // faults, not memory latency.
        let mut warm = 0usize;
        for v in slots.iter().step_by(SLOTS_PER_LINE) {
            warm = warm.wrapping_add(*v);
        }
        black_box(warm);

        Chase {
            slots,
            hops,
            cursor: 0,
        }
    }

    /// Bytes of memory the chase actually walks. Used to assert the padding
    /// scheme in tests; the running benchmark has no need for it.
    #[cfg(test)]
    fn footprint(&self) -> usize {
        self.slots.len() * std::mem::size_of::<usize>()
    }
}

impl WorkloadRun for Chase {
    #[inline(never)]
    fn iterate(&mut self) -> u64 {
        let slots = black_box(&self.slots);
        let mut p = black_box(self.cursor);
        let mut checksum = 0u64;
        for _ in 0..self.hops {
            // The single dependent load. Everything about this workload exists
            // to make sure this line is the bottleneck.
            p = slots[p];
            checksum = checksum.wrapping_add(p as u64);
        }
        // Carry the cursor forward so consecutive iterations start at different
        // points of the cycle, which stops any one iteration's tail from being
        // systematically warmer than the others.
        self.cursor = p;
        checksum | 1
    }
}

/// L1-resident chase: measures the core's best case load-to-use latency.
pub struct L1Chase;

impl Workload for L1Chase {
    fn spec(&self) -> WorkloadSpec {
        WorkloadSpec {
            id: "mem-l1",
            name: "L1 pointer chase",
            description: "Dependent loads around a 16 KiB random cycle: L1 hit latency.",
            kind: WorkloadKind::Memory,
            direction: Direction::HigherIsBetter,
            ops_per_iteration: L1_HOPS,
            sample_multiplier: 1,
        }
    }

    fn instantiate(&self) -> Box<dyn WorkloadRun> {
        Box::new(Chase::new(L1_BYTES, L1_HOPS, 0x1111_2222))
    }
}

/// L2-sized chase: misses L1 on essentially every hop.
pub struct L2Chase;

impl Workload for L2Chase {
    fn spec(&self) -> WorkloadSpec {
        WorkloadSpec {
            id: "mem-l2",
            name: "L2 pointer chase",
            description:
                "Dependent loads around a 512 KiB random cycle: L2 latency, and the first \
                 place an SMT sibling's memory traffic starts to show up.",
            kind: WorkloadKind::Memory,
            direction: Direction::HigherIsBetter,
            ops_per_iteration: L2_HOPS,
            sample_multiplier: 1,
        }
    }

    fn instantiate(&self) -> Box<dyn WorkloadRun> {
        Box::new(Chase::new(L2_BYTES, L2_HOPS, 0x3333_4444))
    }
}

/// 64 MiB chase: reaches DRAM on most machines.
pub struct DramChase;

impl Workload for DramChase {
    fn spec(&self) -> WorkloadSpec {
        WorkloadSpec {
            id: "mem-dram",
            name: "Out-of-cache pointer chase",
            description: "Dependent loads around a 64 MiB random cycle: main memory latency, \
                 including NUMA distance and any interconnect hop. Becomes an L3 \
                 measurement on parts with very large caches.",
            kind: WorkloadKind::Memory,
            direction: Direction::HigherIsBetter,
            ops_per_iteration: DRAM_HOPS,
            sample_multiplier: 1,
        }
    }

    fn instantiate(&self) -> Box<dyn WorkloadRun> {
        Box::new(Chase::new(DRAM_BYTES, DRAM_HOPS, 0x5555_6666))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chase_iterates_and_advances_its_cursor() {
        let mut chase = Chase::new(64 * LINE, 100, 9);
        let start = chase.cursor;
        let sum = chase.iterate();
        assert_ne!(sum, 0);
        assert_ne!(
            chase.cursor, start,
            "cursor did not advance; the chase is not moving"
        );
    }

    #[test]
    fn footprint_matches_the_requested_working_set() {
        // The padding scheme is the whole reason the L1/L2 sizing means
        // anything, so assert it directly.
        for bytes in [L1_BYTES, L2_BYTES, 4 * 1024 * 1024] {
            let chase = Chase::new(bytes, 10, 1);
            assert_eq!(chase.footprint(), bytes);
        }
    }

    #[test]
    fn every_hop_lands_on_a_distinct_cache_line() {
        let chase = Chase::new(64 * LINE, 10, 5);
        let mut p = 0usize;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            assert_eq!(p % SLOTS_PER_LINE, 0, "hop landed mid-line at slot {p}");
            assert!(seen.insert(p / SLOTS_PER_LINE), "line revisited early");
            p = chase.slots[p];
        }
        assert_eq!(seen.len(), 64);
    }

    #[test]
    fn l1_and_l2_working_sets_are_the_documented_sizes() {
        assert_eq!(L1_BYTES / LINE * LINE, L1_BYTES);
        assert_eq!(L2_BYTES / LINE * LINE, L2_BYTES);
        assert_eq!(DRAM_BYTES / LINE * LINE, DRAM_BYTES);
    }

    #[test]
    fn larger_working_sets_are_not_faster() {
        // A weak but genuine ordering check: chasing 64 MiB of random lines
        // cannot be quicker per hop than chasing 16 KiB. If this ever fails,
        // the compiler has hoisted the loads or the cycle is degenerate.
        let per_hop = |bytes: usize, hops: u64| {
            let mut c = Chase::new(bytes, hops, 3);
            c.iterate(); // warm
            let start = corescout_core::clock::now_ns();
            c.iterate();
            let end = corescout_core::clock::now_ns();
            (end - start) as f64 / hops as f64
        };
        let l1 = per_hop(L1_BYTES, 20_000);
        let big = per_hop(4 * 1024 * 1024, 20_000);
        assert!(
            big > l1,
            "4 MiB chase ({big:.1} ns/hop) was not slower than a 16 KiB chase ({l1:.1} ns/hop)"
        );
    }
}
