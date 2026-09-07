//! The reflector: the loop that produces `M(t)` from a real machine.
//!
//! # Why this is called a reflector and not a mirror
//!
//! `corescout-mirror` holds the *reflection*: a snapshot, and the shared memory
//! it is published through. Both are inert data. This is the machinery that
//! produces one, and it is the part that needs hardware, sensors and
//! privileges.
//!
//! Keeping the two apart is what lets a consumer link the reflection without
//! linking the thing that makes it. An observer holding a `MirrorSnapshot` has
//! no `Reflector`, no sensors and no `/sys`.
//!
//! # One pass
//!
//! ```text
//! clear the matrix        no stale value survives into a new reflection
//! stamp the clocks
//! for each bound sensor:
//!     time it, run it, record what it cost and how confident it is
//! ```
//!
//! Clearing first is the anti-staleness rule. A sensor that fails this tick
//! leaves a hole with a reason attached, not last minute's temperature wearing
//! a fresh timestamp. A consumer cannot tell those apart, so the mirror must
//! not offer it the chance to be wrong.

use corescout_core::clock;
use corescout_core::Result;
use corescout_mirror::schema::{Availability, AvailabilityMatrix, SensorReport};
use corescout_mirror::{
    ChannelSpec, Entity, MirrorSnapshot, Relation, StateMatrix, FORMAT_VERSION,
};

use crate::discovery::Substrate;
use crate::observation::{AvailabilityWriter, BindContext, Sensor, StateWriter};

/// A live producer of reflections.
///
/// One `Reflector` owns one machine's observation loop. Calling
/// [`Reflector::observe`] advances it to the present; nothing else changes it.
/// No consumer, model or agent can write to a reflection. The only way to
/// change what the mirror shows is to change the machine.
pub struct Reflector {
    substrate: Substrate,
    sensors: Vec<Box<dyn Sensor>>,
    entities: Vec<Entity>,
    channels: Vec<ChannelSpec>,
    relations: Vec<Relation>,
    state: StateMatrix,
    availability: AvailabilityMatrix,
    reports: Vec<SensorReport>,
    epoch: u64,
    sequence: u64,
    monotonic_ns: u64,
    realtime_ns: u64,
}

impl std::fmt::Debug for Reflector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reflector")
            .field("entities", &self.entities.len())
            .field("channels", &self.channels.len())
            .field("relations", &self.relations.len())
            .field("sensors", &self.sensors.len())
            .field("epoch", &self.epoch)
            .field("sequence", &self.sequence)
            .finish()
    }
}

impl Reflector {
    /// Discover structure, then bind every sensor to it.
    ///
    /// Binding is where all the once-per-epoch work happens: paths are
    /// constructed, capabilities probed, descriptors opened, entities and
    /// channels declared. A sensor that cannot bind is not an error. It is
    /// recorded as inactive **with a reason**, and the mirror says so rather
    /// than implying this machine has no thermal sensors.
    pub fn build(substrate: Substrate, sensors: Vec<Box<dyn Sensor>>) -> Result<Reflector> {
        Reflector::build_at_epoch(substrate, sensors, 1)
    }

    /// Build at a given epoch, for a rebuild after the machine's shape changed.
    pub fn build_at_epoch(
        substrate: Substrate,
        sensors: Vec<Box<dyn Sensor>>,
        epoch: u64,
    ) -> Result<Reflector> {
        let (mut entities, mut relations) = substrate.structure();
        let mut channels: Vec<ChannelSpec> = Vec::new();
        let mut reports = Vec::new();
        let mut bound = Vec::new();

        for mut sensor in sensors {
            let descriptor = sensor.descriptor();
            debug_assert!(
                descriptor.perturbation.is_passive(),
                "sensor `{}` declares Material perturbation and belongs in experiment/",
                descriptor.key
            );

            let before = channels.len();
            let outcome = {
                let mut ctx = BindContext::new(
                    &substrate,
                    descriptor.id,
                    &mut entities,
                    &mut channels,
                    &mut relations,
                );
                sensor.bind(&mut ctx)
            };

            reports.push(match outcome {
                Ok(()) => {
                    SensorReport::pending(descriptor.id, descriptor.key, descriptor.perturbation)
                }
                Err(error) => {
                    // Why it could not bind is the useful part. "Permission
                    // denied" is actionable; "unsupported" never will be.
                    let availability = classify(&error, descriptor.requires_privilege);
                    // A sensor that failed to bind must not leave half-declared
                    // columns behind.
                    channels.truncate(before);
                    SensorReport::unavailable(
                        descriptor.id,
                        descriptor.key,
                        descriptor.perturbation,
                        availability,
                    )
                }
            });
            bound.push(sensor);
        }

        let state = StateMatrix::new(entities.len(), channels.len());
        let availability = AvailabilityMatrix::new(entities.len(), channels.len());
        Ok(Reflector {
            substrate,
            sensors: bound,
            entities,
            channels,
            relations,
            state,
            availability,
            reports,
            epoch,
            sequence: 0,
            monotonic_ns: 0,
            realtime_ns: 0,
        })
    }

    pub fn substrate(&self) -> &Substrate {
        &self.substrate
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    pub fn channels(&self) -> &[ChannelSpec] {
        &self.channels
    }

    pub fn relations(&self) -> &[Relation] {
        &self.relations
    }

    /// Advance the reflection to the present.
    pub fn observe(&mut self) {
        self.state.clear();
        self.availability.clear();
        self.monotonic_ns = clock::now_ns();
        self.realtime_ns = realtime_ns();
        self.sequence += 1;

        // Columns belonging to a sensor that could not bind are marked once,
        // with that sensor's reason, so a consumer sees "unsupported on this
        // machine" rather than an undifferentiated hole.
        for report in self.reports.iter().filter(|r| r.inactive) {
            for (col, channel) in self.channels.iter().enumerate() {
                if channel.sensor == report.id {
                    for row in 0..self.availability.rows() {
                        self.availability.set(row, col, report.availability);
                    }
                }
            }
        }

        for (sensor, report) in self.sensors.iter_mut().zip(self.reports.iter_mut()) {
            if report.inactive {
                continue;
            }
            let started = clock::now_ns();
            let outcome = {
                let mut writer = StateWriter::new(
                    &mut self.state,
                    AvailabilityWriter::new(&mut self.availability),
                );
                sensor.observe(&mut writer)
            };
            // The cost of looking is itself observed, and published.
            report.last_cost_ns = clock::now_ns().saturating_sub(started);
            report.samples = outcome.samples;
            report.errors = outcome.errors;
            report.sample_age_ns = outcome.sample_age_ns;
            report.sampling_latency_ns = report.last_cost_ns;
            let attempted = outcome.samples + outcome.errors;
            report.confidence = if attempted == 0 {
                0.0
            } else {
                outcome.samples as f64 / attempted as f64
            };
            report.availability = if outcome.samples > 0 {
                Availability::Observed
            } else {
                Availability::Unavailable
            };
        }
    }

    /// The current reflection, as an owned snapshot.
    pub fn snapshot(&self) -> MirrorSnapshot {
        MirrorSnapshot {
            format_version: FORMAT_VERSION,
            epoch: self.epoch,
            sequence: self.sequence,
            monotonic_ns: self.monotonic_ns,
            realtime_ns: self.realtime_ns,
            entities: self.entities.clone(),
            channels: self.channels.clone(),
            relations: self.relations.clone(),
            state: self.state.clone(),
            availability: self.availability.clone(),
            sensors: self.reports.clone(),
        }
    }

    /// Total time the last observation pass spent looking.
    pub fn last_observation_cost_ns(&self) -> u64 {
        self.reports.iter().map(|r| r.last_cost_ns).sum()
    }

    /// Sensors that could not bind on this machine, and why.
    pub fn inactive_sensors(&self) -> Vec<&SensorReport> {
        self.reports.iter().filter(|r| r.inactive).collect()
    }

    /// Whether the machine's shape has changed since this reflector was built.
    ///
    /// Cheap enough to call every tick. When it returns true the caller should
    /// rebuild at a higher epoch, because every row index it holds is stale.
    pub fn shape_changed(&self) -> bool {
        match self.substrate.rediscover() {
            Ok(current) => {
                let (entities, _) = current.structure();
                entities.len() != self.entities.len()
                    || entities
                        .iter()
                        .zip(&self.entities)
                        .any(|(now, before)| now.id != before.id)
            }
            // If we cannot tell, do not claim the machine changed: a spurious
            // epoch bump discards every consumer's cached indices.
            Err(_) => false,
        }
    }
}

/// Work out why a sensor could not bind.
fn classify(error: &corescout_core::Error, requires_privilege: bool) -> Availability {
    use corescout_core::Error;
    match error {
        Error::Io { source, .. } => match source.kind() {
            std::io::ErrorKind::PermissionDenied => Availability::PermissionDenied,
            std::io::ErrorKind::NotFound => Availability::Unsupported,
            _ => Availability::Unavailable,
        },
        Error::Unsupported(_) => {
            // A sensor that needs privilege and reports "unsupported" is
            // ambiguous on a machine where the interface exists but is
            // unreadable. Attribute it to privilege, which is the actionable
            // reading, and let the sensor say otherwise if it knows better.
            if requires_privilege {
                Availability::PermissionDenied
            } else {
                Availability::Unsupported
            }
        }
        Error::Syscall { errno, .. } => {
            if *errno == 1 || *errno == 13 {
                Availability::PermissionDenied
            } else {
                Availability::Unavailable
            }
        }
        _ => Availability::Unknown,
    }
}

/// Wall-clock nanoseconds since the Unix epoch.
///
/// For correlating a reflection with logs and with other machines. Never for
/// measuring intervals: it is not monotonic, and `monotonic_ns` exists for that.
fn realtime_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::Roots;
    use crate::observation::{Perturbation, SensorDescriptor, SensorOutcome, Uncertainty};
    use crate::test_support::fake_topology;
    use corescout_mirror::schema::SensorId;
    use corescout_mirror::state::{Semantics, Unit};
    use corescout_mirror::ChannelId;

    /// A sensor that observes nothing real. Enough to exercise the loop.
    struct FakeSensor {
        rows: Vec<u32>,
        channel: Option<ChannelId>,
        bind_error: Option<corescout_core::Error>,
    }

    impl FakeSensor {
        fn working() -> FakeSensor {
            FakeSensor {
                rows: Vec::new(),
                channel: None,
                bind_error: None,
            }
        }

        fn failing(error: corescout_core::Error) -> FakeSensor {
            FakeSensor {
                rows: Vec::new(),
                channel: None,
                bind_error: Some(error),
            }
        }
    }

    impl Sensor for FakeSensor {
        fn descriptor(&self) -> SensorDescriptor {
            SensorDescriptor {
                id: SensorId(900),
                key: "fake",
                physical_fact: "nothing; this sensor exists for tests",
                source: "none",
                max_rate_hz: 1000.0,
                perturbation: Perturbation::None,
                uncertainty: Uncertainty::unknown("not a real measurement"),
                requires_privilege: false,
            }
        }

        fn bind(&mut self, ctx: &mut BindContext<'_>) -> Result<()> {
            if let Some(error) = self.bind_error.take() {
                return Err(error);
            }
            self.channel =
                Some(ctx.declare_channel("fake.value", Unit::Dimensionless, Semantics::Instant));
            for cpu in &ctx.substrate().topology.logical_cpus {
                if let Some(row) = ctx.row_of(&corescout_mirror::entity::keys::logical_cpu(cpu.id))
                {
                    self.rows.push(row);
                }
            }
            Ok(())
        }

        fn observe(&mut self, out: &mut StateWriter<'_>) -> SensorOutcome {
            let mut outcome = SensorOutcome::default();
            let Some(channel) = self.channel else {
                return outcome;
            };
            for row in &self.rows {
                out.set(*row, channel, 1.0);
                outcome.sample();
            }
            outcome
        }
    }

    fn substrate() -> Substrate {
        Substrate::new(fake_topology(), Roots::new("/nonexistent", "/nonexistent"))
    }

    #[test]
    fn observing_fills_the_matrix_and_marks_it_observed() {
        let mut reflector =
            Reflector::build(substrate(), vec![Box::new(FakeSensor::working())]).unwrap();
        reflector.observe();
        let snapshot = reflector.snapshot();

        assert_eq!(snapshot.sequence, 1);
        assert_eq!(snapshot.lookup("cpu/0", "fake.value"), Some(1.0));

        let row = snapshot.row_of_key("cpu/0").unwrap();
        let col = snapshot.channel("fake.value").unwrap();
        assert_eq!(snapshot.availability(row, col), Availability::Observed);

        // A cell no sensor claims is "not applicable", not "unknown".
        let machine = snapshot.row_of_key("machine").unwrap();
        assert_eq!(
            snapshot.availability(machine, col),
            Availability::NotApplicable
        );
    }

    #[test]
    fn a_sensor_that_cannot_bind_reports_why() {
        let reflector = Reflector::build(
            substrate(),
            vec![Box::new(FakeSensor::failing(
                corescout_core::Error::unsupported("no such interface"),
            ))],
        )
        .unwrap();
        let inactive = reflector.inactive_sensors();
        assert_eq!(inactive.len(), 1);
        assert_eq!(inactive[0].availability, Availability::Unsupported);
        assert_eq!(inactive[0].confidence, 0.0);
    }

    #[test]
    fn a_permission_failure_is_distinguished_from_an_absent_interface() {
        // The distinction that tells an operator whether running as root would
        // help.
        let denied = corescout_core::Error::io(
            "/sys/class/powercap/intel-rapl:0/energy_uj",
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        );
        let reflector =
            Reflector::build(substrate(), vec![Box::new(FakeSensor::failing(denied))]).unwrap();
        assert_eq!(
            reflector.inactive_sensors()[0].availability,
            Availability::PermissionDenied
        );
    }

    #[test]
    fn each_pass_starts_from_a_clean_matrix() {
        let mut reflector =
            Reflector::build(substrate(), vec![Box::new(FakeSensor::working())]).unwrap();
        reflector.observe();
        assert!(reflector.snapshot().state.observed_cells() > 0);

        reflector.sensors.clear();
        reflector.observe();
        assert_eq!(
            reflector.snapshot().state.observed_cells(),
            0,
            "stale values must not survive into a later reflection"
        );
    }

    #[test]
    fn the_reflector_records_what_looking_cost() {
        let mut reflector =
            Reflector::build(substrate(), vec![Box::new(FakeSensor::working())]).unwrap();
        reflector.observe();
        let snapshot = reflector.snapshot();
        assert_eq!(snapshot.sensors.len(), 1);
        assert!(snapshot.sensors[0].samples > 0);
        assert_eq!(snapshot.sensors[0].confidence, 1.0);
        assert_eq!(
            snapshot.observation_cost_ns(),
            reflector.last_observation_cost_ns()
        );
    }
}
