//! CPU frequency state.
//!
//! | property | value |
//! |---|---|
//! | physical fact | the clock rate the core's PLL is running at |
//! | source | `/sys/devices/system/cpu/cpuN/cpufreq/` |
//! | sample rate | ~100 Hz useful; the governor re-evaluates on the order of milliseconds |
//! | cost | one small file read per CPU per channel |
//! | perturbation | **Low**, and this one deserves a paragraph |
//! | uncertainty | `scaling_cur_freq` is a request, not a measurement |
//!
//! # Why this sensor perturbs the machine
//!
//! Reading `scaling_cur_freq` is not free and does not merely cost the reader.
//! On `acpi-cpufreq` the kernel reads `APERF`/`MPERF` on the *target* CPU, which
//! it does with `smp_call_function_single`: an inter-processor interrupt that
//! pulls the observed core out of whatever it was doing. On `intel_pstate` the
//! same interface returns the driver's cached request instead, which is cheap
//! and is also not a measurement of anything physical.
//!
//! So this sensor sits exactly on the awkward edge the architecture exists to
//! make visible: the reading is either expensive and true, or cheap and
//! approximate, and which one you get depends on a driver the mirror does not
//! choose. Both cases are declared as `Low` because the mirror cannot tell which
//! it is on, and over-declaring perturbation is the safe direction.
//!
//! The original CoreScout read this file in its topology walk without comment.
//! That was the clearest instance of assuming observation is free.
//!
//! # What `current` actually means
//!
//! `scaling_cur_freq` is the frequency the governor last asked for.
//! `cpuinfo_cur_freq` is closer to what the hardware is doing but usually
//! requires privilege. Neither is the instantaneous clock, which varies over
//! microseconds and cannot be sampled from software at all. The channel is
//! published for what it is.

use crate::observation::source;
use crate::observation::{
    BindContext, Perturbation, Sensor, SensorDescriptor, SensorId, SensorOutcome, StateWriter,
    Uncertainty,
};
use corescout_core::cpuset::CpuSet;
use corescout_core::error::{Error, Result};
use corescout_mirror::entity::keys;
use corescout_mirror::relation::{Relation, RelationKind};
use corescout_mirror::state::{ChannelId, Semantics, Unit};
use std::path::PathBuf;

/// One CPU's resolved cpufreq paths, worked out once at bind time.
struct Target {
    row: u32,
    current: PathBuf,
    min: PathBuf,
    max: PathBuf,
    base: PathBuf,
}

pub struct FrequencySensor {
    targets: Vec<Target>,
    channel_current: Option<ChannelId>,
    channel_min: Option<ChannelId>,
    channel_max: Option<ChannelId>,
    channel_base: Option<ChannelId>,
}

impl FrequencySensor {
    pub fn new() -> FrequencySensor {
        FrequencySensor {
            targets: Vec::new(),
            channel_current: None,
            channel_min: None,
            channel_max: None,
            channel_base: None,
        }
    }
}

impl Default for FrequencySensor {
    fn default() -> Self {
        Self::new()
    }
}

impl Sensor for FrequencySensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: SensorId(1),
            key: "frequency",
            physical_fact: "the clock rate each core is running at, as the cpufreq driver \
                            reports it",
            source: "/sys/devices/system/cpu/cpuN/cpufreq/",
            max_rate_hz: 100.0,
            // See the module docs: on some drivers this costs the observed core
            // an IPI.
            perturbation: Perturbation::Low,
            uncertainty: Uncertainty::relative(
                0.05,
                "scaling_cur_freq is the governor's request, not a measurement; the core \
                 may be at a different point in its ramp, or held lower by a power limit",
            ),
            requires_privilege: false,
        }
    }

    fn bind(&mut self, ctx: &mut BindContext<'_>) -> Result<()> {
        let cpu_dir = ctx.substrate().roots.sys.join("devices/system/cpu");
        let online: Vec<u32> = ctx
            .substrate()
            .topology
            .logical_cpus
            .iter()
            .filter(|c| c.online)
            .map(|c| c.id)
            .collect();

        let mut targets = Vec::new();
        let mut domains: Vec<(String, Vec<u32>)> = Vec::new();
        for cpu in online {
            let dir = cpu_dir.join(format!("cpu{cpu}/cpufreq"));
            if !dir.exists() {
                continue;
            }
            let Some(row) = ctx.row_of(&keys::logical_cpu(cpu)) else {
                continue;
            };
            targets.push(Target {
                row,
                current: dir.join("scaling_cur_freq"),
                min: dir.join("cpuinfo_min_freq"),
                max: dir.join("cpuinfo_max_freq"),
                base: dir.join("base_frequency"),
            });

            // Which CPUs change frequency together is a real relation between
            // entities, and one that no other part of the topology exposes.
            if let Some(related) = source::string(dir.join("related_cpus")) {
                if let Ok(set) = CpuSet::parse_list(&related) {
                    let members = set.to_vec();
                    if members.len() > 1 {
                        let key = corescout_core::cpuset::format_list(&members);
                        if !domains.iter().any(|(k, _)| *k == key) {
                            domains.push((key, members));
                        }
                    }
                }
            }
        }

        if targets.is_empty() {
            return Err(Error::unsupported(
                "cpufreq is not present on this machine (no scaling driver, or a container \
                 without /sys/devices/system/cpu/*/cpufreq)",
            ));
        }

        // Frequency domains, as edges between the CPUs that share them.
        for (_, members) in domains {
            for a in &members {
                for b in &members {
                    if a == b {
                        continue;
                    }
                    if let (Some(from), Some(to)) = (
                        ctx.row_of(&keys::logical_cpu(*a)),
                        ctx.row_of(&keys::logical_cpu(*b)),
                    ) {
                        ctx.declare_relation(Relation::new(
                            from,
                            to,
                            RelationKind::FrequencyDomain,
                        ));
                    }
                }
            }
        }

        self.targets = targets;
        self.channel_current =
            Some(ctx.declare_channel("cpu.frequency.current", Unit::Kilohertz, Semantics::Instant));
        self.channel_min =
            Some(ctx.declare_channel("cpu.frequency.min", Unit::Kilohertz, Semantics::Configured));
        self.channel_max =
            Some(ctx.declare_channel("cpu.frequency.max", Unit::Kilohertz, Semantics::Configured));
        self.channel_base =
            Some(ctx.declare_channel("cpu.frequency.base", Unit::Kilohertz, Semantics::Configured));
        Ok(())
    }

    fn observe(&mut self, out: &mut StateWriter<'_>) -> SensorOutcome {
        let mut outcome = SensorOutcome::default();
        for target in &self.targets {
            // A CPU can go offline between binding and reading. That is a hole
            // in this snapshot, not a failure of the mirror, but it is worth
            // counting.
            source::sample(
                out,
                &mut outcome,
                target.row,
                self.channel_current,
                &target.current,
                true,
            );
            source::sample(
                out,
                &mut outcome,
                target.row,
                self.channel_min,
                &target.min,
                true,
            );
            source::sample(
                out,
                &mut outcome,
                target.row,
                self.channel_max,
                &target.max,
                true,
            );
            // base_frequency exists only on intel_pstate and amd_pstate. Its
            // absence is expected and is not an error.
            source::sample(
                out,
                &mut outcome,
                target.row,
                self.channel_base,
                &target.base,
                false,
            );
        }
        outcome
    }
}
