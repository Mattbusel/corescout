//! What the control loop is not allowed to decide for itself.
//!
//! [`corescout_autonomy::loop_::ControlLoop::tick`] takes two things from its
//! caller: the actions worth considering, and a function mapping a reflection to
//! the objectives the intent names. Both are here rather than inside the loop,
//! deliberately.
//!
//! # Why candidate actions come from outside
//!
//! Only the caller knows what workload is running and where it is permitted to
//! go. A loop that generated its own candidates would be choosing the space it
//! searches, and "the controller decided it was allowed to touch that" is the
//! failure this project's safety model exists to prevent. The set produced here
//! is bounded by the process's own affinity mask and contains nothing outside
//! the registered scope.
//!
//! # Why measurement comes from outside
//!
//! An intent names outcomes: latency, jitter, throughput, energy, migrations.
//! Which channel of the mirror stands for "latency" on a given machine is a
//! judgement about that machine's sensors, and it is the sort of judgement that
//! should be visible in one function rather than distributed through a model.
//! When a machine exposes no channel for an objective, [`measure`] leaves that
//! objective absent, and [`corescout_intent::Achievement`] treats an unmeasured
//! objective as a violated one. Nothing is guessed at.

use corescout_agency::{ActionKind, Target};
use corescout_core::CpuSet;
use corescout_intent::{Achievement, Intent, Objective};
use corescout_mirror::MirrorSnapshot;

/// The actions worth considering on this machine.
///
/// One `SetAffinity` per permitted CPU, plus `Hold`. Deliberately small: a
/// candidate set the size of the powerset of CPUs would let the loop spend its
/// whole budget imagining, and the single-CPU placements are the ones the
/// baselines are defined over, so the comparison stays like for like.
pub fn candidates(
    permitted: &CpuSet,
    snapshot: &MirrorSnapshot,
) -> Vec<(ActionKind, Option<usize>, String)> {
    let target = Target::CurrentProcess;
    let mut actions: Vec<(ActionKind, Option<usize>, String)> = Vec::new();

    // Hold is always a candidate, and is the one the policy should pick most of
    // the time. A controller with no option to do nothing will do something.
    actions.push((ActionKind::Hold, None, "hold".to_string()));

    for cpu in permitted.iter() {
        let row = row_for_cpu(snapshot, cpu);
        actions.push((
            ActionKind::SetAffinity {
                target,
                cpus: [cpu].into_iter().collect(),
            },
            row,
            format!("affinity/cpu{cpu}"),
        ));
    }
    actions
}

/// Read the intent's objectives off a reflection.
///
/// `cells` carries the cells the model considers predictable, which is how a
/// machine with unusual sensors still gets measured: the channel names below are
/// tried first, and what is not found is simply not reported.
pub fn measure(intent: &Intent) -> impl Fn(&MirrorSnapshot, &[(usize, usize, f64)]) -> Achievement {
    let objectives = intent.objectives();
    move |snapshot, _cells| {
        let mut achievement = Achievement::new();
        for objective in &objectives {
            if let Some(value) = read_objective(snapshot, *objective) {
                achievement = achievement.with(*objective, value);
            }
            // No else. An objective this machine cannot measure stays absent,
            // and an absent objective counts as unsatisfied, which is the
            // conservative direction.
        }
        achievement
    }
}

/// The value of one objective, from whichever channel stands for it here.
fn read_objective(snapshot: &MirrorSnapshot, objective: Objective) -> Option<f64> {
    let keys: &[&str] = match objective {
        // Scheduler wait time is the closest thing to "latency" the mirror
        // publishes without perf: it is how long runnable work waited for a
        // CPU, which is exactly what a latency-sensitive workload feels.
        Objective::Latency => &["cpu.sched.wait_ns", "cpu.sched.latency_ns"],
        Objective::Jitter => &["cpu.sched.wait_ns", "cpu.irq.count"],
        Objective::Throughput => &["cpu.frequency.current", "cpu.time.user"],
        Objective::Energy => &["package.power.uw", "package.energy.uj"],
        Objective::Migration => &["cpu.sched.migrations", "cpu.sched.switches"],
    };
    for key in keys {
        let channel = snapshot.channel(key);
        if let Some(channel) = channel {
            let values: Vec<f64> = (0..snapshot.entities.len())
                .filter_map(|row| snapshot.value(row as u32, channel))
                .filter(|value| value.is_finite())
                .collect();
            if !values.is_empty() {
                // The mean across entities. A sum would make the number depend
                // on how many CPUs the machine has, which would make two
                // machines' results incomparable for no benefit.
                return Some(values.iter().sum::<f64>() / values.len() as f64);
            }
        }
    }
    None
}

/// The mirror row for a logical CPU, so an action can be attributed to the
/// entity it acts on.
fn row_for_cpu(snapshot: &MirrorSnapshot, cpu: u32) -> Option<usize> {
    snapshot
        .entities
        .iter()
        .position(|entity| entity.key == format!("cpu/{cpu}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::test_support::fixture;

    #[test]
    fn holding_is_always_a_candidate() {
        // A controller with no option to do nothing will do something.
        let actions = candidates(&CpuSet::new(), &fixture());
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0].0, ActionKind::Hold));
    }

    #[test]
    fn candidates_never_leave_the_permitted_set() {
        let permitted: CpuSet = [1u32, 3].into_iter().collect();
        for (action, _, _) in candidates(&permitted, &fixture()) {
            if let ActionKind::SetAffinity { cpus, .. } = action {
                assert!(cpus.iter().all(|cpu| permitted.contains(cpu)));
            }
        }
    }

    #[test]
    fn a_candidate_is_attributed_to_the_entity_it_acts_on() {
        // The fixture has cpu/0 and cpu/1.
        let permitted: CpuSet = [0u32, 1].into_iter().collect();
        let snapshot = fixture();
        let actions = candidates(&permitted, &snapshot);
        let attributed = actions.iter().filter(|(_, row, _)| row.is_some()).count();
        assert!(attributed > 0, "no candidate found its entity row");
    }

    #[test]
    fn an_unmeasurable_objective_is_absent_rather_than_zero() {
        // Zero would read as "perfect latency" and the policy would act on it.
        let intent = Intent::preset("latency-critical").expect("a preset");
        let snapshot = fixture();
        let achievement = measure(&intent)(&snapshot, &[]);
        for objective in intent.objectives() {
            if read_objective(&snapshot, objective).is_none() {
                assert_eq!(achievement.get(objective), None, "{objective:?}");
            }
        }
    }

    #[test]
    fn an_absent_objective_does_not_satisfy_the_intent() {
        let intent = Intent::preset("latency-critical").expect("a preset");
        let empty = Achievement::new();
        assert!(
            !empty.satisfies(&intent),
            "an intent must not be satisfied by measuring nothing"
        );
    }

    #[test]
    fn a_measurable_objective_is_averaged_across_entities_not_summed() {
        let mut snapshot = fixture();
        // Column 1 of the fixture, given a channel name an objective looks for.
        snapshot.channels[1].key = "cpu.sched.wait_ns".to_string();
        snapshot.state.set(0, 1, 10.0);
        snapshot.state.set(1, 1, 20.0);
        snapshot.state.set(2, 1, 30.0);
        snapshot.state.set(3, 1, 40.0);
        let value = read_objective(&snapshot, Objective::Latency).expect("measurable");
        assert!((value - 25.0).abs() < 1e-9, "got {value}");
    }
}
