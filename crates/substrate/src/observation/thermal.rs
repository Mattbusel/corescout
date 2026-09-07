//! Thermal state.
//!
//! | property | value |
//! |---|---|
//! | physical fact | the temperature reported by an on-die or on-board sensor |
//! | source | `/sys/class/thermal/thermal_zoneN/`, `/sys/class/hwmon/hwmonN/tempX_input` |
//! | sample rate | ~10 Hz; the underlying sensors update on the order of milliseconds and are heavily filtered |
//! | cost | one file read per sensor |
//! | perturbation | **Low** |
//! | uncertainty | +/- 1 C at best, and the reading lags the junction it describes |
//!
//! # Why reading a temperature is not free
//!
//! On `coretemp` a read is an `rdmsr` of `IA32_THERM_STATUS` **on the target
//! core**, dispatched by IPI. Observing a core's temperature therefore
//! interrupts it, and if the core was in a deep C-state, wakes it, which changes
//! the temperature. On board sensors behind SMBus or PECI the transaction takes
//! milliseconds of bus time.
//!
//! This is the sharpest illustration of the observer effect in the whole system:
//! a naive implementation polling every core's temperature at 100 Hz would be
//! preventing those cores from reaching the idle states whose thermal effect it
//! was trying to measure. Hence a declared ceiling of 10 Hz.
//!
//! # Thermal zones are their own entities
//!
//! A thermal zone is not a property of a core. It is a distinct physical thing
//! with its own identity, its own sampling behaviour, and a many-to-many
//! relationship with the cores it covers: a package sensor covers every core, a
//! per-core sensor covers one, and the two disagree by design.
//!
//! Modelling temperature as a `f64` field on a core would force a choice between
//! those and throw the rest away. Instead each sensor becomes an entity and is
//! linked to what it measures by a `ThermalDomain` edge, so a learner can
//! discover for itself which sensors move together and which lead which.

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

/// Divisor from the kernel's millidegree convention to Celsius.
const MILLIDEGREES: f64 = 1000.0;

struct Target {
    row: u32,
    input: PathBuf,
}

pub struct ThermalSensor {
    targets: Vec<Target>,
    channel: Option<ChannelId>,
}

impl ThermalSensor {
    pub fn new() -> ThermalSensor {
        ThermalSensor {
            targets: Vec::new(),
            channel: None,
        }
    }
}

impl Default for ThermalSensor {
    fn default() -> Self {
        Self::new()
    }
}

impl Sensor for ThermalSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: SensorId(3),
            key: "thermal",
            physical_fact: "temperature at each exposed thermal sensor, in degrees Celsius",
            source: "/sys/class/thermal and /sys/class/hwmon",
            max_rate_hz: 10.0,
            // See the module docs: a coretemp read is an IPI to the core being
            // measured.
            perturbation: Perturbation::Low,
            uncertainty: Uncertainty::absolute(
                1.0,
                "on-die thermal diodes are specified to about 1 C and are filtered, so the \
                 reading lags the junction temperature it describes",
            ),
            requires_privilege: false,
        }
    }

    fn bind(&mut self, ctx: &mut BindContext<'_>) -> Result<()> {
        let sys = ctx.substrate().roots.sys.clone();
        let mut targets = Vec::new();

        // Thermal zones: whole-package and platform sensors.
        for (index, path) in source::numbered_children(sys.join("class/thermal"), "thermal_zone") {
            let Some(temp) = path.join("temp").exists().then(|| path.join("temp")) else {
                continue;
            };
            let zone_type = source::string(path.join("type")).unwrap_or_else(|| "zone".to_string());
            let row = ctx.declare_entity(Entity::new(
                keys::thermal_zone(&zone_type, index),
                EntityClass::ThermalZone,
                Some(index),
            ));
            // A zone whose type names the package covers the whole machine.
            if let Some(machine) = ctx.row_of("machine") {
                ctx.declare_relation(Relation::new(row, machine, RelationKind::ThermalDomain));
            }
            targets.push(Target { row, input: temp });
        }

        // hwmon: where per-core temperatures actually live on x86.
        for (_, path) in source::numbered_children(sys.join("class/hwmon"), "hwmon") {
            let chip = source::string(path.join("name")).unwrap_or_else(|| "hwmon".to_string());
            for index in 1..=32u32 {
                let input = path.join(format!("temp{index}_input"));
                if !input.exists() {
                    continue;
                }
                let label = source::string(path.join(format!("temp{index}_label")))
                    .unwrap_or_else(|| format!("temp{index}"));
                let row = ctx.declare_entity(Entity::new(
                    keys::thermal_zone(&format!("{chip}/{label}"), index),
                    EntityClass::ThermalZone,
                    Some(index),
                ));

                // `coretemp` labels per-core sensors "Core N", where N is the
                // *kernel core id* within the package. Linking the sensor to the
                // core it names is the only place the mirror uses a label to
                // infer structure, and it is done here, once, at bind time,
                // rather than being left for every consumer to guess at.
                let linked = label
                    .strip_prefix("Core ")
                    .and_then(|n| n.trim().parse::<u32>().ok())
                    .and_then(|core_id| {
                        ctx.substrate()
                            .topology
                            .physical_cores
                            .iter()
                            .find(|c| c.core_id == core_id)
                            .map(|c| (c.package_id, c.core_id))
                    })
                    .and_then(|(package, core)| ctx.row_of(&keys::physical_core(package, core)));
                let target_row = linked.or_else(|| ctx.row_of("machine"));
                if let Some(target) = target_row {
                    ctx.declare_relation(Relation::new(row, target, RelationKind::ThermalDomain));
                }
                targets.push(Target { row, input });
            }
        }

        if targets.is_empty() {
            return Err(Error::unsupported(
                "no thermal sensors are exposed (common in VMs and containers)",
            ));
        }

        self.targets = targets;
        self.channel =
            Some(ctx.declare_channel("thermal.temperature", Unit::Celsius, Semantics::Instant));
        Ok(())
    }

    fn observe(&mut self, out: &mut StateWriter<'_>) -> SensorOutcome {
        let mut outcome = SensorOutcome::default();
        let Some(channel) = self.channel else {
            return outcome;
        };
        for target in &self.targets {
            match source::f64(&target.input) {
                // The kernel reports millidegrees. Converting here, once, is
                // exactly the normalisation the plane exists to centralise.
                Some(millidegrees) => {
                    out.set(target.row, channel, millidegrees / MILLIDEGREES);
                    outcome.sample();
                }
                None => outcome.error(),
            }
        }
        outcome
    }
}
