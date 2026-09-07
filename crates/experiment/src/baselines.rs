//! The policies to beat.
//!
//! # The bar is high and should be
//!
//! A self-modelling runtime that cannot beat `taskset` has not earned its
//! complexity, and one that cannot beat the Linux scheduler has not earned
//! being run at all. So the baselines here are chosen to be genuinely hard:
//!
//! | baseline | why it is hard to beat |
//! |---|---|
//! | [`Baseline::Scheduler`] | CFS/EEVDF sees the whole machine and rebalances continuously |
//! | [`Baseline::FirstCpu`] | naive, but stable, and stability is most of the benefit of pinning |
//! | [`Baseline::PreferredCore`] | the firmware's own bin sort, measured with equipment we do not have |
//! | [`Baseline::CoreScoutRanking`] | the original project: a careful empirical benchmark |
//! | [`Baseline::Utilization`] | what a simple load balancer would do |
//! | [`Baseline::Random`] | the control: any policy that cannot beat this is noise |
//!
//! `Random` is the important one. A comparison without it cannot distinguish a
//! policy that works from a policy that happens to have been lucky on the
//! machine it was developed on.

use corescout_core::CpuSet;
use corescout_mirror::MirrorSnapshot;
use serde::{Deserialize, Serialize};

/// A way of deciding where work goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Baseline {
    /// Set no affinity at all. The operating system decides.
    Scheduler,
    /// Pin to the lowest-numbered permitted CPU. What a hurried person does.
    FirstCpu,
    /// Pin to the firmware's preferred core, where one is exposed.
    PreferredCore,
    /// Pin to whatever the original CoreScout benchmark ranked first.
    CoreScoutRanking,
    /// Pin to the least busy CPU, by idle residency.
    Utilization,
    /// Pin to a CPU chosen by a fixed-seed PRNG. The control.
    Random,
    /// Whatever the autonomous controller chooses. The thing being tested.
    SelfModel,
}

impl Baseline {
    pub fn label(self) -> &'static str {
        match self {
            Baseline::Scheduler => "os-scheduler",
            Baseline::FirstCpu => "first-cpu",
            Baseline::PreferredCore => "preferred-core",
            Baseline::CoreScoutRanking => "corescout-ranking",
            Baseline::Utilization => "least-utilised",
            Baseline::Random => "random",
            Baseline::SelfModel => "self-model",
        }
    }

    /// One line on what this policy does, for a report.
    pub fn description(self) -> &'static str {
        match self {
            Baseline::Scheduler => "no affinity; the OS scheduler decides and rebalances",
            Baseline::FirstCpu => "pinned to the lowest-numbered permitted CPU",
            Baseline::PreferredCore => "pinned to the firmware's preferred core",
            Baseline::CoreScoutRanking => "pinned to the core the original benchmark ranked first",
            Baseline::Utilization => "pinned to the CPU with the most idle residency",
            Baseline::Random => "pinned to a CPU chosen at random with a fixed seed",
            Baseline::SelfModel => "placed by the autonomous controller",
        }
    }

    /// Whether this baseline needs a mirror to decide.
    pub fn needs_mirror(self) -> bool {
        matches!(
            self,
            Baseline::PreferredCore | Baseline::Utilization | Baseline::SelfModel
        )
    }

    /// Every baseline that can be run without a controller.
    pub fn all_static() -> Vec<Baseline> {
        vec![
            Baseline::Scheduler,
            Baseline::FirstCpu,
            Baseline::PreferredCore,
            Baseline::CoreScoutRanking,
            Baseline::Utilization,
            Baseline::Random,
        ]
    }
}

/// Where a baseline decided to put the work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub baseline: Baseline,
    /// `None` means "no affinity", which is what the scheduler baseline does
    /// and is materially different from "every CPU": the kernel is free to
    /// rebalance rather than being told it may.
    pub cpus: Option<CpuSet>,
    /// Why, in one line.
    pub reason: String,
}

impl Placement {
    pub fn none(baseline: Baseline, reason: impl Into<String>) -> Placement {
        Placement {
            baseline,
            cpus: None,
            reason: reason.into(),
        }
    }

    pub fn pinned(baseline: Baseline, cpus: CpuSet, reason: impl Into<String>) -> Placement {
        Placement {
            baseline,
            cpus: Some(cpus),
            reason: reason.into(),
        }
    }
}

/// Decide a placement for a baseline.
///
/// `permitted` is the set of CPUs this process may use; a baseline never
/// escapes it. `ranking` is the original CoreScout ranking, when one is
/// available.
pub fn decide(
    baseline: Baseline,
    permitted: &CpuSet,
    snapshot: Option<&MirrorSnapshot>,
    ranking: Option<u32>,
    seed: u64,
) -> Placement {
    if permitted.is_empty() {
        return Placement::none(baseline, "no CPUs are permitted to this process");
    }

    match baseline {
        Baseline::Scheduler => Placement::none(
            baseline,
            "no affinity set; the kernel places and rebalances freely",
        ),
        Baseline::FirstCpu => {
            let cpu = permitted.iter().next().expect("non-empty");
            Placement::pinned(
                baseline,
                [cpu].into_iter().collect(),
                format!("lowest permitted CPU is {cpu}"),
            )
        }
        Baseline::Random => {
            // A fixed-seed PRNG, so "random" is reproducible across runs and
            // two policies are compared on the same rolls.
            let mut state = seed | 1;
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            let cpus = permitted.to_vec();
            let cpu = cpus[(state.wrapping_mul(0x2545_F491_4F6C_DD1D) as usize) % cpus.len()];
            Placement::pinned(
                baseline,
                [cpu].into_iter().collect(),
                format!("randomly selected CPU {cpu} from seed {seed}"),
            )
        }
        Baseline::CoreScoutRanking => match ranking {
            Some(cpu) if permitted.contains(cpu) => Placement::pinned(
                baseline,
                [cpu].into_iter().collect(),
                format!("the original benchmark ranked CPU {cpu} first"),
            ),
            Some(cpu) => Placement::none(
                baseline,
                format!("CPU {cpu} was ranked first but is not permitted to this process"),
            ),
            None => Placement::none(baseline, "no ranking has been computed on this machine"),
        },
        Baseline::PreferredCore => {
            let Some(snapshot) = snapshot else {
                return Placement::none(baseline, "no reflection available");
            };
            match preferred_cpu(snapshot, permitted) {
                Some(cpu) => Placement::pinned(
                    baseline,
                    [cpu].into_iter().collect(),
                    format!("firmware ranks CPU {cpu} highest"),
                ),
                None => Placement::none(
                    baseline,
                    "this machine exposes no preferred-core information",
                ),
            }
        }
        Baseline::Utilization => {
            let Some(snapshot) = snapshot else {
                return Placement::none(baseline, "no reflection available");
            };
            match least_busy_cpu(snapshot, permitted) {
                Some(cpu) => Placement::pinned(
                    baseline,
                    [cpu].into_iter().collect(),
                    format!("CPU {cpu} has the most idle residency"),
                ),
                None => Placement::none(
                    baseline,
                    "the mirror exposes no idle residency on this machine",
                ),
            }
        }
        Baseline::SelfModel => Placement::none(
            baseline,
            "placement comes from the controller, not from this function",
        ),
    }
}

/// The CPU the firmware ranks highest, from the reflection.
fn preferred_cpu(snapshot: &MirrorSnapshot, permitted: &CpuSet) -> Option<u32> {
    // The mirror does not publish CPPC ranking as a channel, so this reads the
    // entity metadata rather than inventing a channel for it. If a future
    // sensor publishes it, this becomes a channel lookup.
    let channel = snapshot.channel("cpu.frequency.max")?;
    snapshot
        .entities
        .iter()
        .enumerate()
        .filter(|(_, entity)| entity.key.starts_with("cpu/"))
        .filter_map(|(row, entity)| {
            let cpu = entity.natural_index?;
            if !permitted.contains(cpu) {
                return None;
            }
            let value = snapshot.value(row as u32, channel)?;
            Some((cpu, value))
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(cpu, _)| cpu)
}

/// The CPU with the most accumulated idle time.
fn least_busy_cpu(snapshot: &MirrorSnapshot, permitted: &CpuSet) -> Option<u32> {
    let channel = snapshot
        .channel("cpu.time.idle")
        .or_else(|| snapshot.channel("cpu.idle.state0.residency"))?;
    snapshot
        .entities
        .iter()
        .enumerate()
        .filter(|(_, entity)| entity.key.starts_with("cpu/"))
        .filter_map(|(row, entity)| {
            let cpu = entity.natural_index?;
            if !permitted.contains(cpu) {
                return None;
            }
            let value = snapshot.value(row as u32, channel)?;
            Some((cpu, value))
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(cpu, _)| cpu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::test_support::fixture;

    fn permitted() -> CpuSet {
        (0u32..4).collect()
    }

    #[test]
    fn the_scheduler_baseline_sets_no_affinity_at_all() {
        // Materially different from "all CPUs": the kernel may rebalance.
        let placement = decide(Baseline::Scheduler, &permitted(), None, None, 1);
        assert!(placement.cpus.is_none());
        assert!(placement.reason.contains("rebalance"));
    }

    #[test]
    fn the_first_cpu_baseline_is_deterministic() {
        let a = decide(Baseline::FirstCpu, &permitted(), None, None, 1);
        let b = decide(Baseline::FirstCpu, &permitted(), None, None, 99);
        assert_eq!(a.cpus, b.cpus);
        assert_eq!(a.cpus.unwrap().to_vec(), vec![0]);
    }

    #[test]
    fn the_random_baseline_is_reproducible_and_stays_in_bounds() {
        // "Random" must be the same across runs, or two policies are not being
        // compared on the same rolls.
        let a = decide(Baseline::Random, &permitted(), None, None, 7);
        let b = decide(Baseline::Random, &permitted(), None, None, 7);
        assert_eq!(a.cpus, b.cpus);
        let cpu = a.cpus.unwrap().iter().next().unwrap();
        assert!(permitted().contains(cpu));

        let other = decide(Baseline::Random, &permitted(), None, None, 8);
        assert_ne!(a.reason, other.reason);
    }

    #[test]
    fn a_ranking_outside_the_permitted_set_is_declined_rather_than_forced() {
        let placement = decide(Baseline::CoreScoutRanking, &permitted(), None, Some(99), 1);
        assert!(placement.cpus.is_none());
        assert!(placement.reason.contains("not permitted"));
    }

    #[test]
    fn a_missing_ranking_is_reported_not_faked() {
        let placement = decide(Baseline::CoreScoutRanking, &permitted(), None, None, 1);
        assert!(placement.cpus.is_none());
        assert!(placement.reason.contains("no ranking"));
    }

    #[test]
    fn utilisation_picks_the_most_idle_cpu_from_the_reflection() {
        let mut snapshot = fixture();
        // Column 1 is cpu.time.idle. CPU 1 (row 3) is more idle than CPU 0.
        snapshot.state.set(2, 1, 100.0);
        snapshot.state.set(3, 1, 900.0);
        let placement = decide(
            Baseline::Utilization,
            &(0u32..2).collect(),
            Some(&snapshot),
            None,
            1,
        );
        assert_eq!(placement.cpus.unwrap().to_vec(), vec![1]);
    }

    #[test]
    fn a_machine_without_the_needed_channel_declines() {
        let mut snapshot = fixture();
        snapshot.channels.clear();
        let placement = decide(
            Baseline::Utilization,
            &permitted(),
            Some(&snapshot),
            None,
            1,
        );
        assert!(placement.cpus.is_none());
        assert!(placement.reason.contains("no idle residency"));
    }

    #[test]
    fn no_baseline_escapes_the_permitted_set() {
        let narrow: CpuSet = [2u32].into_iter().collect();
        for baseline in Baseline::all_static() {
            let placement = decide(baseline, &narrow, Some(&fixture()), Some(0), 3);
            if let Some(cpus) = placement.cpus {
                assert!(
                    cpus.iter().all(|cpu| narrow.contains(cpu)),
                    "{} escaped the permitted set",
                    baseline.label()
                );
            }
        }
    }

    #[test]
    fn an_empty_permitted_set_yields_no_placement() {
        for baseline in Baseline::all_static() {
            let placement = decide(baseline, &CpuSet::new(), None, None, 1);
            assert!(placement.cpus.is_none());
        }
    }

    #[test]
    fn every_baseline_describes_itself() {
        for baseline in Baseline::all_static() {
            assert!(!baseline.label().is_empty());
            assert!(!baseline.description().is_empty());
        }
        assert!(Baseline::Random.description().contains("random"));
    }
}
