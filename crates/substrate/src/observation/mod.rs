//! Observation: how the machine samples itself, and what that costs.
//!
//! # The rule this layer enforces
//!
//! > Observation belongs to the mirror. Intentional perturbation belongs to
//! > experimentation.
//!
//! Every sensor here is passive: it reads state the machine is maintaining
//! anyway, for its own reasons, whether or not anyone is looking. Nothing in
//! this module runs a workload, moves a thread, changes a frequency or requests
//! an idle state. Code that wants to do those things lives in
//! `corescout-experiment` or `corescout-agency`, and is not part of `M(t)`.
//!
//! # Observation is not free
//!
//! A computational mirror is unusual among mirrors: the act of looking consumes
//! the thing being looked at. Reading a sensor costs cycles on some CPU, evicts
//! cache lines, may take a kernel lock, and in several real cases sends an
//! inter-processor interrupt to the very core whose state is in question.
//!
//! So every sensor must declare, and this is enforced by the type system rather
//! than by convention:
//!
//! | property | why it is required |
//! |---|---|
//! | `physical_fact` | what real quantity the number approximates |
//! | `source` | where it comes from, so a reader can go and check |
//! | `max_rate_hz` | above this, sampling returns no new information |
//! | `perturbation` | how much observing changes what is observed |
//! | `uncertainty` | how wrong the number may be even when everything works |
//! | `requires_privilege` | whether it will be absent for ordinary users |
//!
//! # Never fabricate
//!
//! A sensor that cannot read something records **why** through
//! [`StateWriter::missing`], and the reason reaches the consumer in the
//! snapshot's availability matrix. A machine with no RAPL does not report zero
//! watts. The types make the honest path the easy one: writing a value and
//! marking it observed are the same call, and there is no way to write a value
//! without claiming you observed it.

#[cfg(target_os = "windows")]
pub mod windows;

pub mod counters;
pub mod frequency;
pub mod idle;
pub mod interrupts;
pub mod power;
pub mod scheduler;
pub mod source;
pub mod thermal;

use corescout_core::Result;
use corescout_mirror::entity::Entity;
use corescout_mirror::relation::Relation;
use corescout_mirror::schema::{Availability, AvailabilityMatrix};
use corescout_mirror::state::{ChannelId, ChannelSpec, Semantics, StateMatrix, Unit};

use crate::discovery::Substrate;

// The metadata a sensor declares is part of the *representation*, so it lives
// in `corescout-mirror` where a consumer that never links this crate can still
// read it. Re-exported here because this is where sensors are written.
pub use corescout_mirror::schema::{Perturbation, SensorId, SensorReport, Uncertainty};

/// Everything a sensor must declare about itself.
#[derive(Debug, Clone, PartialEq)]
pub struct SensorDescriptor {
    pub id: SensorId,
    /// Stable short name, e.g. `frequency`.
    pub key: &'static str,
    /// The physical quantity being approximated, in one sentence.
    pub physical_fact: &'static str,
    /// The kernel interface or instruction the value comes from.
    pub source: &'static str,
    /// Above this rate, sampling returns no new information.
    pub max_rate_hz: f64,
    pub perturbation: Perturbation,
    pub uncertainty: Uncertainty,
    /// True when the sensor needs privileges an ordinary user may not have, and
    /// will therefore be absent rather than wrong.
    pub requires_privilege: bool,
}

/// The result of one sensor's observation pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SensorOutcome {
    pub samples: u32,
    pub errors: u32,
    /// Age of the underlying values at the time they were read, for sensors
    /// whose source updates more slowly than the mirror ticks.
    pub sample_age_ns: u64,
}

impl SensorOutcome {
    pub fn sample(&mut self) {
        self.samples += 1;
    }

    pub fn error(&mut self) {
        self.errors += 1;
    }

    pub fn aged(&mut self, age_ns: u64) {
        self.sample_age_ns = self.sample_age_ns.max(age_ns);
    }
}

/// Handed to a sensor during binding so it can declare what it will observe.
///
/// Sensors may declare **entities of their own**, not only channels. A thermal
/// zone and a RAPL power domain are real parts of the machine that the CPU
/// topology knows nothing about, and forcing them to be attributes of a core
/// would be exactly the human-ontology imposition this architecture avoids.
pub struct BindContext<'a> {
    substrate: &'a Substrate,
    sensor: SensorId,
    entities: &'a mut Vec<Entity>,
    channels: &'a mut Vec<ChannelSpec>,
    relations: &'a mut Vec<Relation>,
}

impl<'a> BindContext<'a> {
    pub fn new(
        substrate: &'a Substrate,
        sensor: SensorId,
        entities: &'a mut Vec<Entity>,
        channels: &'a mut Vec<ChannelSpec>,
        relations: &'a mut Vec<Relation>,
    ) -> BindContext<'a> {
        BindContext {
            substrate,
            sensor,
            entities,
            channels,
            relations,
        }
    }

    /// The structural machine description, including filesystem roots.
    pub fn substrate(&self) -> &Substrate {
        self.substrate
    }

    /// Row index of an already-declared entity, by natural key.
    pub fn row_of(&self, key: &str) -> Option<u32> {
        self.entities
            .iter()
            .position(|e| e.key == key)
            .map(|i| i as u32)
    }

    /// Declare an entity, or return the existing row if its key is already
    /// present. Idempotent, so two sensors observing the same physical thing
    /// agree about which row it is rather than duplicating it.
    pub fn declare_entity(&mut self, entity: Entity) -> u32 {
        if let Some(row) = self.row_of(&entity.key) {
            return row;
        }
        self.entities.push(entity);
        (self.entities.len() - 1) as u32
    }

    /// Declare a channel and get its column index.
    pub fn declare_channel(
        &mut self,
        key: impl Into<String>,
        unit: Unit,
        semantics: Semantics,
    ) -> ChannelId {
        let key = key.into();
        if let Some(existing) = self.channels.iter().find(|c| c.key == key) {
            return existing.id;
        }
        let id = ChannelId(self.channels.len() as u16);
        self.channels.push(ChannelSpec {
            id,
            key,
            unit,
            semantics,
            sensor: self.sensor,
        });
        id
    }

    /// Declare an edge.
    pub fn declare_relation(&mut self, relation: Relation) {
        self.relations.push(relation);
    }
}

/// Records *why* cells are empty. Held by [`StateWriter`].
pub struct AvailabilityWriter<'a> {
    matrix: &'a mut AvailabilityMatrix,
}

impl<'a> AvailabilityWriter<'a> {
    pub fn new(matrix: &'a mut AvailabilityMatrix) -> AvailabilityWriter<'a> {
        AvailabilityWriter { matrix }
    }

    #[inline]
    pub fn set(&mut self, row: u32, channel: ChannelId, availability: Availability) {
        self.matrix.set(row as usize, channel.index(), availability);
    }
}

/// Handed to a sensor during observation. The only things it can do are record
/// a number, or record why there is not one.
pub struct StateWriter<'a> {
    matrix: &'a mut StateMatrix,
    availability: AvailabilityWriter<'a>,
}

impl<'a> StateWriter<'a> {
    pub fn new(
        matrix: &'a mut StateMatrix,
        availability: AvailabilityWriter<'a>,
    ) -> StateWriter<'a> {
        StateWriter {
            matrix,
            availability,
        }
    }

    /// Record an observation.
    ///
    /// Out-of-range rows and columns are ignored rather than panicking: a
    /// sensor racing a CPU hotplug event should degrade to a missing cell, not
    /// take the whole mirror down.
    #[inline]
    pub fn set(&mut self, row: u32, channel: ChannelId, value: f64) {
        let (r, c) = (row as usize, channel.index());
        if r < self.matrix.rows() && c < self.matrix.cols() {
            self.matrix.set(r, c, value);
            self.availability.set(row, channel, Availability::Observed);
        }
    }

    /// Record that a cell has no value, and why.
    ///
    /// The alternative, leaving it silently `NaN`, loses the distinction
    /// between "this machine cannot tell you" and "nobody asked".
    #[inline]
    pub fn missing(&mut self, row: u32, channel: ChannelId, why: Availability) {
        let (r, c) = (row as usize, channel.index());
        if r < self.matrix.rows() && c < self.matrix.cols() {
            self.availability.set(row, channel, why);
        }
    }
}

/// A passive source of observations.
///
/// Implementors must be honest about [`Perturbation`]. The reflector checks
/// that no registered sensor declares [`Perturbation::Material`], but it cannot
/// check that a sensor declaring `Negligible` is telling the truth. That is a
/// review obligation, and it is why each sensor's module documentation states
/// the physical basis for its claim.
pub trait Sensor: Send {
    fn descriptor(&self) -> SensorDescriptor;

    /// Resolve everything resolvable once: which entities exist, which files to
    /// read, which descriptors to open, which columns to fill.
    ///
    /// This is where CoreScout earns its keep as a normalisation layer. Path
    /// construction, parsing setup and capability probing happen here, once per
    /// epoch, so that [`Sensor::observe`] is close to pure I/O.
    ///
    /// Returning `Err` means the sensor is unavailable on this machine, which
    /// is a normal outcome. The error's kind decides what the mirror reports:
    /// permission denied, unsupported, or merely unavailable.
    fn bind(&mut self, ctx: &mut BindContext<'_>) -> Result<()>;

    /// Sample the machine once.
    fn observe(&mut self, out: &mut StateWriter<'_>) -> SensorOutcome;
}

/// The default passive sensor set for this machine.
///
/// Ordered cheapest-first, so a mirror running under a tight tick budget
/// degrades by dropping the expensive sensors at the end rather than by
/// randomly missing whichever ones ran late.
pub fn default_sensors() -> Vec<Box<dyn Sensor>> {
    // Windows exposes a different, smaller set through entirely different
    // interfaces. See `linux_sensors` for the set that reads sysfs and procfs,
    // which a test with a synthetic tree wants by name rather than by default.
    #[cfg(target_os = "windows")]
    {
        return windows::sensors();
    }
    #[allow(unreachable_code)]
    linux_sensors()
}

/// The sysfs and procfs sensor set.
///
/// Named separately from [`default_sensors`] because it is meaningful on any
/// platform: the sensors read files by path, so a test can point them at a
/// synthetic tree and exercise them anywhere. What varies by platform is which
/// set is the *default*, not which set can be constructed.
pub fn linux_sensors() -> Vec<Box<dyn Sensor>> {
    vec![
        Box::new(frequency::FrequencySensor::new()),
        Box::new(idle::IdleSensor::new()),
        Box::new(thermal::ThermalSensor::new()),
        Box::new(power::PowerSensor::new()),
        Box::new(scheduler::SchedulerSensor::new()),
        Box::new(interrupts::InterruptSensor::new()),
        Box::new(counters::CounterSensor::new()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_default_sensor_claims_to_be_material() {
        // The architectural invariant, asserted rather than trusted.
        for sensor in default_sensors() {
            let d = sensor.descriptor();
            assert!(
                d.perturbation.is_passive(),
                "sensor `{}` declares Material perturbation and is an experiment",
                d.key
            );
        }
    }

    #[test]
    fn every_sensor_documents_its_physical_basis() {
        for sensor in default_sensors() {
            let d = sensor.descriptor();
            assert!(
                !d.physical_fact.is_empty(),
                "{} has no physical_fact",
                d.key
            );
            assert!(!d.source.is_empty(), "{} has no source", d.key);
            assert!(
                !d.uncertainty.basis.is_empty(),
                "{} has no uncertainty basis",
                d.key
            );
            assert!(
                d.max_rate_hz > 0.0,
                "{} declares no sampling ceiling",
                d.key
            );
        }
    }

    #[test]
    fn sensor_ids_and_keys_are_unique() {
        let sensors = default_sensors();
        let mut ids: Vec<u16> = sensors.iter().map(|s| s.descriptor().id.0).collect();
        let mut keys: Vec<&str> = sensors.iter().map(|s| s.descriptor().key).collect();
        ids.sort_unstable();
        keys.sort_unstable();
        let unique_ids = {
            let mut v = ids.clone();
            v.dedup();
            v.len()
        };
        let unique_keys = {
            let mut v = keys.clone();
            v.dedup();
            v.len()
        };
        assert_eq!(unique_ids, ids.len(), "duplicate sensor id");
        assert_eq!(unique_keys, keys.len(), "duplicate sensor key");
    }

    #[test]
    fn writing_a_value_marks_it_observed_and_missing_records_a_reason() {
        let mut matrix = StateMatrix::new(2, 2);
        let mut availability = AvailabilityMatrix::new(2, 2);
        {
            let mut writer =
                StateWriter::new(&mut matrix, AvailabilityWriter::new(&mut availability));
            writer.set(0, ChannelId(0), 5.0);
            writer.missing(1, ChannelId(1), Availability::PermissionDenied);
            // Out of range must not panic.
            writer.set(99, ChannelId(0), 1.0);
            writer.missing(0, ChannelId(99), Availability::Unknown);
        }
        assert_eq!(matrix.get(0, 0), 5.0);
        assert_eq!(availability.get(0, 0), Availability::Observed);
        assert!(matrix.get(1, 1).is_nan());
        assert_eq!(availability.get(1, 1), Availability::PermissionDenied);
        assert_eq!(matrix.observed_cells(), 1);
    }
}
