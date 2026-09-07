//! Latency and jitter workload.
//!
//! # What this measures, and why it is a separate family
//!
//! The compute and memory workloads ask "how much work per second". This one
//! asks a different question: **when I ask this core to do a small, fixed
//! amount of work, how consistent is the answer?**
//!
//! The unit of work is deliberately tiny, a few microseconds. At that scale the
//! iteration time is dominated not by the arithmetic but by everything the
//! machine does *around* the arithmetic: a timer interrupt, an IPI, an SMI, the
//! SMT sibling waking and stealing front-end slots, a cpufreq transition, a
//! migration the pin was supposed to prevent. Those events are rare per
//! iteration, so the median stays close to the pure compute cost and the tail
//! shows the interference. The gap between them (`p99 - median`) is the number
//! a latency-sensitive user actually has to budget for.
//!
//! Because the events are rare, this workload asks for several times more
//! samples than the throughput ones: a p99 estimated from 30 samples is barely
//! an estimate at all.

use std::hint::black_box;

use super::{Direction, Workload, WorkloadKind, WorkloadRun, WorkloadSpec};

/// Dependent operations per iteration. Sized for roughly 3-6 us on a current
/// core: short enough that a single 4 ms scheduler tick is a dramatic outlier
/// rather than a background tax, long enough to be far above clock resolution.
const OPS: u64 = 4_000;

/// Repeated short fixed-cost operations, measured as a distribution.
pub struct ShortOpJitter;

impl Workload for ShortOpJitter {
    fn spec(&self) -> WorkloadSpec {
        WorkloadSpec {
            id: "jitter",
            name: "Short-operation jitter",
            description:
                "A few microseconds of fixed dependent work, repeated many times. The median \
                 is the core's clean latency; the p99 is what interrupts, the SMT sibling and \
                 the scheduler add on top.",
            kind: WorkloadKind::Latency,
            direction: Direction::LowerIsBetter,
            ops_per_iteration: OPS,
            // Tail estimates need samples. Six times the base count keeps the
            // p99 meaningful without making a full analysis take minutes.
            sample_multiplier: 6,
        }
    }

    fn instantiate(&self) -> Box<dyn WorkloadRun> {
        Box::new(ShortOpJitterRun {
            state: 0x243F_6A88_85A3_08D3,
        })
    }
}

struct ShortOpJitterRun {
    state: u64,
}

impl WorkloadRun for ShortOpJitterRun {
    #[inline(never)]
    fn iterate(&mut self) -> u64 {
        // One serial dependency chain, on purpose. Independent chains would let
        // the core hide small stalls behind instruction-level parallelism,
        // which is exactly the smoothing we are trying *not* to apply: we want
        // any disturbance to land visibly in the iteration time.
        let mut x = black_box(self.state);
        for _ in 0..OPS {
            x = x
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407)
                .rotate_left(21);
        }
        self.state = x | 1;
        x | 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_core::clock;

    #[test]
    fn jitter_workload_asks_for_extra_samples() {
        let spec = ShortOpJitter.spec();
        assert_eq!(spec.kind, WorkloadKind::Latency);
        assert_eq!(spec.direction, Direction::LowerIsBetter);
        assert!(
            spec.sample_multiplier > 1,
            "tail statistics need more than the base sample count"
        );
    }

    #[test]
    fn iteration_is_short_but_measurable() {
        let mut run = ShortOpJitter.instantiate();
        run.iterate(); // warm
        let start = clock::now_ns();
        for _ in 0..20 {
            black_box(run.iterate());
        }
        let per_iter = (clock::now_ns() - start) / 20;
        // Very wide bounds so this holds on a slow VM as well as a fast
        // desktop; the point is only that we are in microseconds, not
        // nanoseconds or milliseconds.
        assert!(
            (300..2_000_000).contains(&per_iter),
            "jitter iteration took {per_iter} ns, outside the intended range"
        );
    }

    #[test]
    fn state_feeds_forward() {
        let mut run = ShortOpJitter.instantiate();
        assert_ne!(run.iterate(), run.iterate());
    }
}
