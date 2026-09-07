//! Energy and power limits.
//!
//! | property | value |
//! |---|---|
//! | physical fact | energy consumed by a power domain since boot, and the limits it is held to |
//! | source | `/sys/class/powercap/intel-rapl:*/` |
//! | sample rate | ~100 Hz; RAPL's own counters update roughly every millisecond |
//! | cost | one file read per domain |
//! | perturbation | **Negligible**: an MSR read on the local core, no cross-CPU work |
//! | uncertainty | RAPL is a model, not a wattmeter |
//!
//! # Why energy and not power
//!
//! RAPL exposes a monotonically increasing energy counter. Power is its
//! derivative, and computing it requires two readings and an interval, which
//! makes it a memory-layer quantity by the same rule that keeps rates out of
//! the mirror. Publishing `energy_uj` and letting a consumer difference it is
//! also *more accurate* than any power figure the mirror could compute, because
//! the consumer knows exactly which two samples it differenced.
//!
//! # The wrap
//!
//! The counter wraps at `max_energy_range_uj`, typically every 60 seconds or so
//! under load. That value is published as its own channel so a consumer
//! differencing samples can detect and correct a wrap. Doing the correction here
//! would require remembering the previous value, which is precisely what the
//! mirror is not allowed to do.
//!
//! # Privilege
//!
//! Since the PLATYPUS side-channel work, `energy_uj` is root-readable only on
//! most distributions: fine-grained energy readings leak information about what
//! other processes are computing. An unprivileged mirror will find this sensor
//! present and unreadable, which is reported as inactive rather than as an
//! error, and is one of the clearest cases where what the machine can know about
//! itself is deliberately limited.

use crate::observation::source;
use crate::observation::{
    BindContext, Perturbation, Sensor, SensorDescriptor, SensorId, SensorOutcome, StateWriter,
    Uncertainty,
};
use corescout_core::error::{Error, Result};
use corescout_mirror::entity::{keys, Entity, EntityClass};
use corescout_mirror::relation::{Relation, RelationKind};
use corescout_mirror::state::{ChannelId, Semantics, Unit};
use std::path::PathBuf;

struct Target {
    row: u32,
    energy: PathBuf,
    range: PathBuf,
    limit: PathBuf,
}

pub struct PowerSensor {
    targets: Vec<Target>,
    channel_energy: Option<ChannelId>,
    channel_range: Option<ChannelId>,
    channel_limit: Option<ChannelId>,
}

impl PowerSensor {
    pub fn new() -> PowerSensor {
        PowerSensor {
            targets: Vec::new(),
            channel_energy: None,
            channel_range: None,
            channel_limit: None,
        }
    }
}

impl Default for PowerSensor {
    fn default() -> Self {
        Self::new()
    }
}

impl Sensor for PowerSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: SensorId(4),
            key: "power",
            physical_fact: "cumulative energy consumed by each RAPL domain, and the power \
                            limits currently enforced on it",
            source: "/sys/class/powercap/intel-rapl:*/",
            max_rate_hz: 100.0,
            perturbation: Perturbation::Negligible,
            uncertainty: Uncertainty::relative(
                0.05,
                "RAPL is a firmware energy model derived from activity counters, not a \
                 measurement at the power rail; it tracks real consumption closely on CPU \
                 domains and less well on package and DRAM domains",
            ),
            requires_privilege: true,
        }
    }

    fn bind(&mut self, ctx: &mut BindContext<'_>) -> Result<()> {
        let powercap = ctx.substrate().roots.sys.join("class/powercap");
        let mut targets = Vec::new();

        for (dir_name, path) in source::named_children(&powercap, "intel-rapl:") {
            let energy = path.join("energy_uj");
            if !energy.exists() {
                continue;
            }
            // Reading once at bind time is how we discover whether we are
            // allowed to read at all. A permission failure here is the normal
            // unprivileged case.
            if source::u64(&energy).is_none() {
                continue;
            }
            let name = source::string(path.join("name")).unwrap_or(dir_name);
            let row = ctx.declare_entity(Entity::new(
                keys::power_domain(&name),
                EntityClass::PowerDomain,
                None,
            ));
            if let Some(machine) = ctx.row_of("machine") {
                ctx.declare_relation(Relation::new(row, machine, RelationKind::PowerDomain));
            }
            targets.push(Target {
                row,
                energy,
                range: path.join("max_energy_range_uj"),
                limit: path.join("constraint_0_power_limit_uw"),
            });
        }

        if targets.is_empty() {
            return Err(Error::unsupported(
                "no readable RAPL energy domains (absent on AMD without amd_energy, on most \
                 VMs, and root-only on distributions that restrict energy_uj)",
            ));
        }

        self.targets = targets;
        self.channel_energy =
            Some(ctx.declare_channel("power.energy", Unit::Microjoule, Semantics::Cumulative));
        self.channel_range = Some(ctx.declare_channel(
            "power.energy_wrap_at",
            Unit::Microjoule,
            Semantics::Configured,
        ));
        self.channel_limit =
            Some(ctx.declare_channel("power.limit", Unit::Microwatt, Semantics::Configured));
        Ok(())
    }

    fn observe(&mut self, out: &mut StateWriter<'_>) -> SensorOutcome {
        let mut outcome = SensorOutcome::default();
        for target in &self.targets {
            source::sample(
                out,
                &mut outcome,
                target.row,
                self.channel_energy,
                &target.energy,
                true,
            );
            source::sample(
                out,
                &mut outcome,
                target.row,
                self.channel_range,
                &target.range,
                false,
            );
            source::sample(
                out,
                &mut outcome,
                target.row,
                self.channel_limit,
                &target.limit,
                false,
            );
        }
        outcome
    }
}
