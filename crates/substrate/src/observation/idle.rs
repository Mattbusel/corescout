//! Idle state residency.
//!
//! | property | value |
//! |---|---|
//! | physical fact | how long each core has spent in each hardware idle state, and how often it entered |
//! | source | `/sys/devices/system/cpu/cpuN/cpuidle/stateK/{usage,time}` |
//! | sample rate | ~1 kHz; the counters update on every idle entry and exit |
//! | cost | two small file reads per CPU per idle state |
//! | perturbation | **Negligible**: the kernel already maintains these counters for its own governor, and reading them touches no other CPU |
//! | uncertainty | residency is accounted at state entry and exit, so a core currently idle has not yet accrued its present stay |
//!
//! # Why the counters and not "is it idle right now"
//!
//! "Is this core idle" is not a well-formed question at the resolution the
//! mirror samples at: a core enters and leaves C1 thousands of times a second,
//! so a boolean sampled at 100 Hz is a coin flip, not an observation.
//!
//! The cumulative counters are exact and complete. A consumer that wants
//! occupancy differences two snapshots and gets the true residency over that
//! interval, with no sampling error at all. This is the clearest example of why
//! the mirror publishes counters and leaves rates to the memory layer: the
//! counter is both cheaper *and* more accurate than any instantaneous reading
//! the mirror could synthesise.
//!
//! # Deeper states are the interesting ones
//!
//! Exit latency rises sharply with C-state depth, tens of microseconds for the
//! deep states on a server part. A core sitting in a deep state is a core that
//! will respond late to the next thing it is asked to do, which is exactly the
//! property a latency-sensitive consumer wants to know about and which no
//! frequency or temperature reading exposes.

use crate::observation::source;
use crate::observation::{
    BindContext, Perturbation, Sensor, SensorDescriptor, SensorId, SensorOutcome, StateWriter,
    Uncertainty,
};
use corescout_core::error::{Error, Result};
use corescout_mirror::entity::keys;
use corescout_mirror::state::{ChannelId, Semantics, Unit};
use std::path::PathBuf;

/// One (CPU, idle state) pair, resolved at bind time.
struct Target {
    row: u32,
    state: u32,
    usage: PathBuf,
    time: PathBuf,
}

pub struct IdleSensor {
    targets: Vec<Target>,
    /// One channel pair per state index, since state 2 on one core is the same
    /// hardware state as state 2 on another.
    usage_channels: Vec<ChannelId>,
    time_channels: Vec<ChannelId>,
    depth_channel: Option<ChannelId>,
}

impl IdleSensor {
    pub fn new() -> IdleSensor {
        IdleSensor {
            targets: Vec::new(),
            usage_channels: Vec::new(),
            time_channels: Vec::new(),
            depth_channel: None,
        }
    }
}

impl Default for IdleSensor {
    fn default() -> Self {
        Self::new()
    }
}

impl Sensor for IdleSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: SensorId(2),
            key: "idle",
            physical_fact: "cumulative time and entry count for each hardware idle state of \
                            each core",
            source: "/sys/devices/system/cpu/cpuN/cpuidle/stateK/",
            max_rate_hz: 1000.0,
            perturbation: Perturbation::Negligible,
            uncertainty: Uncertainty::unknown(
                "the kernel accounts residency on state exit, so a core's current stay in a \
                 state is not yet included in its total",
            ),
            requires_privilege: false,
        }
    }

    fn bind(&mut self, ctx: &mut BindContext<'_>) -> Result<()> {
        let cpu_dir = ctx.substrate().roots.sys.join("devices/system/cpu");
        let cpus: Vec<u32> = ctx
            .substrate()
            .topology
            .logical_cpus
            .iter()
            .filter(|c| c.online)
            .map(|c| c.id)
            .collect();

        let mut targets = Vec::new();
        let mut max_state = 0u32;
        for cpu in cpus {
            let Some(row) = ctx.row_of(&keys::logical_cpu(cpu)) else {
                continue;
            };
            let dir = cpu_dir.join(format!("cpu{cpu}/cpuidle"));
            for (state, path) in source::numbered_children(&dir, "state") {
                max_state = max_state.max(state);
                targets.push(Target {
                    row,
                    state,
                    usage: path.join("usage"),
                    time: path.join("time"),
                });
            }
        }

        if targets.is_empty() {
            return Err(Error::unsupported(
                "cpuidle is not exposed (no idle driver, or a container without \
                 /sys/devices/system/cpu/*/cpuidle)",
            ));
        }

        for state in 0..=max_state {
            self.usage_channels.push(ctx.declare_channel(
                format!("cpu.idle.state{state}.entries"),
                Unit::Count,
                Semantics::Cumulative,
            ));
            self.time_channels.push(ctx.declare_channel(
                format!("cpu.idle.state{state}.residency"),
                Unit::Microsecond,
                Semantics::Cumulative,
            ));
        }
        self.depth_channel = Some(ctx.declare_channel(
            "cpu.idle.states_available",
            Unit::Count,
            Semantics::Configured,
        ));
        self.targets = targets;
        Ok(())
    }

    fn observe(&mut self, out: &mut StateWriter<'_>) -> SensorOutcome {
        let mut outcome = SensorOutcome::default();
        let mut last_row: Option<u32> = None;
        let mut states_for_row = 0.0;

        for target in &self.targets {
            let index = target.state as usize;
            source::sample(
                out,
                &mut outcome,
                target.row,
                self.usage_channels.get(index).copied(),
                &target.usage,
                true,
            );
            source::sample(
                out,
                &mut outcome,
                target.row,
                self.time_channels.get(index).copied(),
                &target.time,
                true,
            );

            // Count states per CPU as we go; targets are grouped by CPU.
            match last_row {
                Some(row) if row == target.row => states_for_row += 1.0,
                Some(row) => {
                    source::emit(out, &mut outcome, row, self.depth_channel, states_for_row);
                    states_for_row = 1.0;
                }
                None => states_for_row = 1.0,
            }
            last_row = Some(target.row);
        }
        if let Some(row) = last_row {
            source::emit(out, &mut outcome, row, self.depth_channel, states_for_row);
        }
        outcome
    }
}
