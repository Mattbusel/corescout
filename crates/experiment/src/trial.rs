//! One unit of real work, on a real CPU, measured.
//!
//! # What a trial is
//!
//! Pin this thread to a placement, run a workload that computes something,
//! check the answer, and report what it cost in cycles and in wall time.
//!
//! Everything in that sentence is literal. The workload is the same
//! [`crate::benchmarks::workloads`] code the original benchmark used: it chases
//! pointers through real memory and folds real arithmetic, and its result is
//! consumed so the optimiser cannot delete it. The placement is a real
//! `SetThreadGroupAffinity`. The cost is `QueryThreadCycleTime`, which counts
//! cycles the thread actually executed rather than time that elapsed while it
//! was waiting.
//!
//! # Verification is what makes the cost meaningful
//!
//! A trial returns a checksum folded from every iteration. The expected value is
//! established once, on a reference run, and never shown to anything that
//! chooses placements. A configuration that got faster by doing less work
//! produces a different checksum and is reported as [`Trial::verified`] false,
//! and its cost still counts.
//!
//! Without that, "this placement is faster" and "this placement skipped the
//! work" are the same measurement.
//!
//! # Two costs, because they answer different questions
//!
//! | measure | question |
//! |---|---|
//! | cycles | how much of the machine did this consume |
//! | wall nanoseconds | how long did the caller wait |
//!
//! On a hybrid part these disagree sharply, and neither is the "real" one. A
//! P-core finishes sooner and burns more cycles doing it; an E-core is the
//! reverse. Reporting one alone would answer a question nobody asked.

use corescout_core::clock;
use corescout_core::error::Result;
use corescout_core::CpuSet;
use corescout_substrate::platform::Platform;

use crate::benchmarks::workloads::{Workload, WorkloadKind};

/// What one trial cost and whether it was right.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trial {
    /// Cycles the thread actually executed. `None` where the platform cannot
    /// report them, in which case only wall time is available.
    pub cycles: Option<u64>,
    /// Wall time from first iteration to last.
    pub wall_ns: u64,
    /// Iterations completed.
    pub iterations: u32,
    /// Whether the result matched the reference.
    pub verified: bool,
    /// The checksum this trial produced, for a caller establishing a reference.
    pub checksum: u64,
}

impl Trial {
    /// Cycles per iteration, when cycles are available.
    pub fn cycles_per_iteration(&self) -> Option<f64> {
        let cycles = self.cycles?;
        (self.iterations > 0).then(|| cycles as f64 / self.iterations as f64)
    }

    /// Wall nanoseconds per iteration.
    pub fn ns_per_iteration(&self) -> f64 {
        if self.iterations == 0 {
            return f64::NAN;
        }
        self.wall_ns as f64 / self.iterations as f64
    }

    /// The cost a productivity ratio should use.
    ///
    /// Cycles when they are available, because they measure the machine
    /// consumed rather than the time waited. Wall time otherwise, and a caller
    /// that cares which should ask [`Trial::cycles`].
    pub fn cost(&self) -> f64 {
        match self.cycles {
            Some(cycles) => cycles as f64,
            None => self.wall_ns as f64,
        }
    }
}

/// Run one trial under a placement.
///
/// Restores the thread's original affinity before returning, on every path
/// including a failure. A measurement harness that leaves a thread pinned has
/// altered every measurement that follows it.
pub fn run(
    platform: &dyn Platform,
    cpus: &CpuSet,
    workload: &dyn Workload,
    iterations: u32,
    warmup: u32,
    expected: Option<u64>,
) -> Result<Trial> {
    let original = platform.current_thread_affinity().ok();
    let placed = platform.set_current_thread_affinity(cpus);
    if let Err(error) = placed {
        if let Some(original) = &original {
            let _ = platform.set_current_thread_affinity(original);
        }
        return Err(error);
    }

    let mut run = workload.instantiate();
    let mut sink = 0u64;
    // A core that has been idle is at its lowest P-state with cold caches and
    // an empty branch predictor. Measuring that would rank placements by how
    // recently each happened to run something.
    for _ in 0..warmup {
        sink = sink.wrapping_add(run.iterate());
    }
    std::hint::black_box(sink);

    let cycles_before = platform.thread_cycles();
    let started = clock::now_ns();
    let mut checksum = 0u64;
    for _ in 0..iterations {
        checksum = checksum.wrapping_mul(31).wrapping_add(run.iterate());
    }
    let wall_ns = clock::now_ns().saturating_sub(started);
    let cycles_after = platform.thread_cycles();
    std::hint::black_box(checksum);

    if let Some(original) = &original {
        let _ = platform.set_current_thread_affinity(original);
    }

    Ok(Trial {
        cycles: match (cycles_before, cycles_after) {
            (Some(before), Some(after)) => Some(after.saturating_sub(before)),
            _ => None,
        },
        wall_ns,
        iterations,
        // No reference yet means this run is establishing one, so it cannot be
        // wrong. Marking it unverified would make every first measurement a
        // failure.
        verified: expected.map_or(true, |value| value == checksum),
        checksum,
    })
}

/// Establish what a workload's checksum should be.
///
/// Run once, unpinned, before anything starts choosing placements. The value it
/// returns is the reference every later trial is checked against, and nothing
/// that selects a placement is ever shown it.
///
/// # Warmup is part of the identity of a checksum
///
/// The workloads are stateful: a pointer chase is somewhere in its cycle, a
/// branch generator is somewhere in its sequence. Iterating advances that
/// state, so a run preceded by six warmup iterations does not produce the same
/// checksum as one preceded by two.
///
/// `warmup` is therefore a parameter here and must match what the trials use.
/// It is not a detail: getting it wrong makes every honest trial report itself
/// as corrupt, which is how this was found.
pub fn reference(
    platform: &dyn Platform,
    workload: &dyn Workload,
    iterations: u32,
    warmup: u32,
) -> Result<u64> {
    let permitted = platform.process_affinity()?;
    let trial = run(platform, &permitted, workload, iterations, warmup, None)?;
    Ok(trial.checksum)
}

/// A workload of each family, for a caller that wants a mixed load.
pub fn workload_set() -> Vec<Box<dyn Workload>> {
    vec![
        Box::new(crate::benchmarks::workloads::compute::IntegerAlu),
        Box::new(crate::benchmarks::workloads::memory::L2Chase),
        Box::new(crate::benchmarks::workloads::latency::ShortOpJitter),
    ]
}

/// Which family a workload belongs to, for reporting.
pub fn family_of(workload: &dyn Workload) -> WorkloadKind {
    workload.spec().kind
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_substrate::platform;

    fn workload() -> Box<dyn Workload> {
        Box::new(crate::benchmarks::workloads::compute::IntegerAlu)
    }

    #[test]
    fn a_trial_does_real_work_and_costs_real_cycles() {
        // Live, on the machine running the test.
        let platform = platform::detect();
        let permitted = platform.process_affinity().expect("permitted");
        let trial = run(
            platform.as_ref(),
            &permitted,
            workload().as_ref(),
            40,
            4,
            None,
        )
        .expect("runnable");
        assert_eq!(trial.iterations, 40);
        assert!(trial.wall_ns > 0, "the work took no time at all");
        if let Some(cycles) = trial.cycles {
            assert!(cycles > 0, "the work burned no cycles");
        }
    }

    #[test]
    fn the_same_workload_produces_the_same_answer_wherever_it_runs() {
        // The property verification depends on. If a checksum varied by
        // placement, an honest run would be indistinguishable from a corrupt
        // one.
        let platform = platform::detect();
        let permitted = platform.process_affinity().expect("permitted");
        let expected = reference(platform.as_ref(), workload().as_ref(), 30, 2).expect("reference");

        for cpu in permitted.iter().take(4) {
            let one: CpuSet = [cpu].into_iter().collect();
            let trial = run(
                platform.as_ref(),
                &one,
                workload().as_ref(),
                30,
                2,
                Some(expected),
            )
            .expect("runnable");
            assert!(
                trial.verified,
                "CPU {cpu} produced a different answer: {} vs {expected}",
                trial.checksum
            );
        }
    }

    #[test]
    fn a_trial_restores_the_threads_affinity() {
        // A harness that leaves a thread pinned has altered every measurement
        // that follows it.
        let platform = platform::detect();
        let permitted = platform.process_affinity().expect("permitted");
        let before = platform.current_thread_affinity().expect("readable");
        let one: CpuSet = [permitted.iter().next().expect("a cpu")]
            .into_iter()
            .collect();
        run(platform.as_ref(), &one, workload().as_ref(), 10, 1, None).expect("runnable");
        let after = platform.current_thread_affinity().expect("readable");
        assert_eq!(before.to_vec(), after.to_vec());
    }

    #[test]
    fn cycles_and_wall_time_both_advance_and_are_not_the_same_number() {
        let platform = platform::detect();
        let permitted = platform.process_affinity().expect("permitted");
        let trial = run(
            platform.as_ref(),
            &permitted,
            workload().as_ref(),
            60,
            4,
            None,
        )
        .expect("runnable");
        assert!(trial.ns_per_iteration() > 0.0);
        if let Some(per) = trial.cycles_per_iteration() {
            assert!(per > 0.0);
            // At any plausible clock these differ; if they were equal the
            // cycle counter would be returning nanoseconds.
            assert!(
                (per - trial.ns_per_iteration()).abs() > 1.0,
                "cycles and nanoseconds are suspiciously identical"
            );
        }
    }

    #[test]
    fn an_empty_placement_is_refused_rather_than_ignored() {
        let platform = platform::detect();
        assert!(run(
            platform.as_ref(),
            &CpuSet::new(),
            workload().as_ref(),
            10,
            1,
            None
        )
        .is_err());
    }
}

#[cfg(test)]
mod warmup_tests {
    use super::*;
    use corescout_substrate::platform;

    fn workload() -> Box<dyn Workload> {
        Box::new(crate::benchmarks::workloads::compute::IntegerAlu)
    }

    #[test]
    fn a_checksum_depends_on_how_much_warmup_preceded_it() {
        // The workloads are stateful and iterating advances them. This is not a
        // defect in them; it is a fact a verification scheme has to respect, and
        // not respecting it made every honest trial report itself as corrupt.
        let platform = platform::detect();
        let permitted = platform.process_affinity().expect("permitted");
        let two = run(
            platform.as_ref(),
            &permitted,
            workload().as_ref(),
            20,
            2,
            None,
        )
        .expect("runnable");
        let six = run(
            platform.as_ref(),
            &permitted,
            workload().as_ref(),
            20,
            6,
            None,
        )
        .expect("runnable");
        assert_ne!(
            two.checksum, six.checksum,
            "if warmup did not change the checksum, this parameter would be pointless"
        );
    }

    #[test]
    fn matching_warmup_and_iterations_verify_everywhere() {
        let platform = platform::detect();
        let permitted = platform.process_affinity().expect("permitted");
        let expected =
            reference(platform.as_ref(), workload().as_ref(), 25, 5).expect("a reference");
        for cpu in permitted.iter().take(6) {
            let one: CpuSet = [cpu].into_iter().collect();
            let trial = run(
                platform.as_ref(),
                &one,
                workload().as_ref(),
                25,
                5,
                Some(expected),
            )
            .expect("runnable");
            assert!(trial.verified, "CPU {cpu} disagreed with the reference");
        }
    }
}
