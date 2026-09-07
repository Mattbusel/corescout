//! The Windows sensor set.
//!
//! # What this machine will tell us
//!
//! Two calls carry almost everything Windows exposes passively about its own
//! processors:
//!
//! | sensor | source | channels |
//! |---|---|---|
//! | `win_times` | `NtQuerySystemInformation` | idle, kernel, user, DPC, interrupt time, interrupt count |
//! | `win_frequency` | `CallNtPowerInformation` | current and maximum MHz, idle state |
//!
//! # What it will not
//!
//! No temperature, no package power, no performance counters. Windows has no
//! supported user-mode interface for any of them: thermals come from vendor
//! drivers, there is no RAPL equivalent, and the PMU needs a kernel driver.
//!
//! Those absences are published as [`Availability::Unsupported`] rather than
//! omitted, which is the whole point of the availability machinery. A consumer
//! can tell "this machine has no thermal sensor" from "this machine is cold",
//! and a model trained here will not silently treat a missing channel as a
//! measured zero.
//!
//! # Cumulative, never rates
//!
//! Every time channel is published as a cumulative total, exactly as the Linux
//! sensors publish `/proc/stat` fields. Differencing is memory's job, and a
//! sensor that helpfully computed a rate would be putting history into a
//! representation of the present.

use corescout_core::error::{Error, Result};
use corescout_mirror::entity::{keys, Entity, EntityClass};
use corescout_mirror::schema::{Availability, Perturbation, Uncertainty};
use corescout_mirror::state::{ChannelId, Semantics, Unit};

use crate::observation::{BindContext, Sensor, SensorDescriptor, SensorOutcome, StateWriter};
use crate::platform::windows::{frequency, times};

/// Per-CPU time and interrupt counters.
pub struct WindowsTimesSensor {
    /// `(row, cpu index)` for each logical CPU, resolved once at bind.
    rows: Vec<(u32, usize)>,
    idle: ChannelId,
    kernel: ChannelId,
    user: ChannelId,
    dpc: ChannelId,
    interrupt_time: ChannelId,
    interrupt_count: ChannelId,
    cpu_count: usize,
}

impl WindowsTimesSensor {
    pub fn new() -> WindowsTimesSensor {
        WindowsTimesSensor {
            rows: Vec::new(),
            idle: ChannelId(0),
            kernel: ChannelId(0),
            user: ChannelId(0),
            dpc: ChannelId(0),
            interrupt_time: ChannelId(0),
            interrupt_count: ChannelId(0),
            cpu_count: 0,
        }
    }
}

impl Default for WindowsTimesSensor {
    fn default() -> Self {
        Self::new()
    }
}

impl Sensor for WindowsTimesSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            // Ids continue the Linux numbering rather than restarting, so a
            // recording says which sensor produced a channel without also
            // needing to record which platform wrote it.
            id: corescout_mirror::schema::SensorId(11),
            key: "win_times",
            physical_fact:
                "cumulative time each logical CPU spent idle, in the kernel, in user mode, \
                 servicing DPCs and servicing interrupts, plus interrupts serviced",
            source: "NtQuerySystemInformation(SystemProcessorPerformanceInformation)",
            // The underlying counters advance on the scheduler tick, so reading
            // faster than that returns the same numbers.
            max_rate_hz: 64.0,
            // One system call that copies a small array. It does not interrupt
            // other cores.
            perturbation: Perturbation::Negligible,
            // Windows accounts CPU time in 100ns units but attributes it in
            // whole scheduler ticks, so the real granularity is far coarser
            // than the unit suggests: about 15.6 ms.
            uncertainty: Uncertainty::absolute(
                15_600_000.0,
                "CPU time is attributed in whole scheduler ticks, not in the 100ns units                  the interface reports",
            ),
            requires_privilege: false,
        }
    }

    fn bind(&mut self, ctx: &mut BindContext<'_>) -> Result<()> {
        // Copied out before anything is declared: `declare_channel` needs the
        // context mutably, and the topology is borrowed from it.
        let cpus: Vec<u32> = ctx
            .substrate()
            .topology
            .logical_cpus
            .iter()
            .map(|cpu| cpu.id)
            .collect();
        self.cpu_count = cpus.len();
        if self.cpu_count == 0 {
            return Err(Error::unsupported("no logical CPUs to observe"));
        }
        // Prove the source works before declaring channels, so a machine that
        // will not answer produces an inactive sensor rather than a column of
        // permanent gaps.
        times::sample(self.cpu_count)?;

        self.idle = ctx.declare_channel("cpu.time.idle", Unit::Nanosecond, Semantics::Cumulative);
        self.kernel =
            ctx.declare_channel("cpu.time.kernel", Unit::Nanosecond, Semantics::Cumulative);
        self.user = ctx.declare_channel("cpu.time.user", Unit::Nanosecond, Semantics::Cumulative);
        self.dpc = ctx.declare_channel("cpu.time.dpc", Unit::Nanosecond, Semantics::Cumulative);
        self.interrupt_time = ctx.declare_channel(
            "cpu.time.interrupt",
            Unit::Nanosecond,
            Semantics::Cumulative,
        );
        self.interrupt_count =
            ctx.declare_channel("cpu.irq.count", Unit::Count, Semantics::Cumulative);

        self.rows.clear();
        for (index, cpu) in cpus.iter().enumerate() {
            let row = ctx.declare_entity(Entity::new(
                keys::logical_cpu(*cpu),
                EntityClass::LogicalCpu,
                Some(*cpu),
            ));
            self.rows.push((row, index));
        }
        Ok(())
    }

    fn observe(&mut self, out: &mut StateWriter<'_>) -> SensorOutcome {
        let mut outcome = SensorOutcome::default();
        let samples = match times::sample(self.cpu_count) {
            Ok(samples) => samples,
            Err(_) => {
                for (row, _) in &self.rows {
                    for channel in [
                        self.idle,
                        self.kernel,
                        self.user,
                        self.dpc,
                        self.interrupt_time,
                        self.interrupt_count,
                    ] {
                        out.missing(*row, channel, Availability::Unavailable);
                    }
                    outcome.error();
                }
                return outcome;
            }
        };

        for (row, index) in &self.rows {
            let Some(cpu) = samples.get(*index) else {
                // The topology enumerated a CPU the performance query did not
                // report. Not an error in this sensor, and not a zero either.
                for channel in [
                    self.idle,
                    self.kernel,
                    self.user,
                    self.dpc,
                    self.interrupt_time,
                    self.interrupt_count,
                ] {
                    out.missing(*row, channel, Availability::Unavailable);
                }
                outcome.error();
                continue;
            };
            out.set(*row, self.idle, cpu.idle_ns as f64);
            out.set(*row, self.kernel, cpu.kernel_ns as f64);
            out.set(*row, self.user, cpu.user_ns as f64);
            out.set(*row, self.dpc, cpu.dpc_ns as f64);
            out.set(*row, self.interrupt_time, cpu.interrupt_ns as f64);
            out.set(*row, self.interrupt_count, cpu.interrupt_count as f64);
            outcome.sample();
        }
        outcome
    }
}

/// Per-CPU frequency and idle state.
pub struct WindowsFrequencySensor {
    rows: Vec<(u32, usize)>,
    current: ChannelId,
    max: ChannelId,
    idle_state: ChannelId,
    cpu_count: usize,
}

impl WindowsFrequencySensor {
    pub fn new() -> WindowsFrequencySensor {
        WindowsFrequencySensor {
            rows: Vec::new(),
            current: ChannelId(0),
            max: ChannelId(0),
            idle_state: ChannelId(0),
            cpu_count: 0,
        }
    }
}

impl Default for WindowsFrequencySensor {
    fn default() -> Self {
        Self::new()
    }
}

impl Sensor for WindowsFrequencySensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: corescout_mirror::schema::SensorId(10),
            key: "win_frequency",
            physical_fact:
                "the clock frequency the power manager last requested for each logical CPU, \
                 and the deepest idle state it is currently in. Not the instantaneous clock: \
                 the core's real frequency may change many times during one read",
            source: "CallNtPowerInformation(ProcessorInformation)",
            max_rate_hz: 20.0,
            perturbation: Perturbation::Negligible,
            // Reported in whole MHz, and quantised far more coarsely than that
            // in practice by the P-state table.
            uncertainty: Uncertainty::absolute(
                1000.0,
                "reported in whole MHz, and the underlying value is the requested P-state                  rather than a measured clock",
            ),
            requires_privilege: false,
        }
    }

    fn bind(&mut self, ctx: &mut BindContext<'_>) -> Result<()> {
        let cpus: Vec<u32> = ctx
            .substrate()
            .topology
            .logical_cpus
            .iter()
            .map(|cpu| cpu.id)
            .collect();
        self.cpu_count = cpus.len();
        if self.cpu_count == 0 {
            return Err(Error::unsupported("no logical CPUs to observe"));
        }
        frequency::read(self.cpu_count)?;

        self.current =
            ctx.declare_channel("cpu.frequency.current", Unit::Kilohertz, Semantics::Instant);
        self.max = ctx.declare_channel("cpu.frequency.max", Unit::Kilohertz, Semantics::Instant);
        self.idle_state =
            ctx.declare_channel("cpu.idle.state", Unit::Dimensionless, Semantics::Instant);

        self.rows.clear();
        for (index, cpu) in cpus.iter().enumerate() {
            let row = ctx.declare_entity(Entity::new(
                keys::logical_cpu(*cpu),
                EntityClass::LogicalCpu,
                Some(*cpu),
            ));
            self.rows.push((row, index));
        }
        Ok(())
    }

    fn observe(&mut self, out: &mut StateWriter<'_>) -> SensorOutcome {
        let mut outcome = SensorOutcome::default();
        let records = match frequency::read(self.cpu_count) {
            Ok(records) => records,
            Err(_) => {
                for (row, _) in &self.rows {
                    for channel in [self.current, self.max, self.idle_state] {
                        out.missing(*row, channel, Availability::Unavailable);
                    }
                    outcome.error();
                }
                return outcome;
            }
        };

        for (row, index) in &self.rows {
            let Some(record) = records.get(*index) else {
                for channel in [self.current, self.max, self.idle_state] {
                    out.missing(*row, channel, Availability::Unavailable);
                }
                outcome.error();
                continue;
            };
            // A zero is the power manager declining to say, not a stopped
            // clock, so it is published as a gap rather than as zero MHz.
            if record.current_mhz > 0 {
                out.set(*row, self.current, record.current_mhz as f64 * 1000.0);
            } else {
                out.missing(*row, self.current, Availability::Unavailable);
            }
            if record.max_mhz > 0 {
                out.set(*row, self.max, record.max_mhz as f64 * 1000.0);
            } else {
                out.missing(*row, self.max, Availability::Unavailable);
            }
            out.set(*row, self.idle_state, record.current_idle_state as f64);
            outcome.sample();
        }
        outcome
    }
}

/// The sensors this platform actually has.
///
/// Thermal, power and PMU sensors are absent rather than present-and-failing.
/// A sensor that binds and then reports nothing forever adds columns of
/// permanent gaps to every reflection, which costs every consumer memory and
/// attention for no information.
pub fn sensors() -> Vec<Box<dyn Sensor>> {
    vec![
        Box::new(WindowsFrequencySensor::new()),
        Box::new(WindowsTimesSensor::new()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::Substrate;
    use crate::platform;

    fn substrate() -> Substrate {
        let platform = platform::detect();
        Substrate::discover(platform.as_ref()).expect("this machine is discoverable")
    }

    #[test]
    fn both_sensors_bind_on_this_machine() {
        // Live. If a sensor cannot bind here it is not a sensor for this
        // platform, and the mirror would be a set of empty columns.
        let substrate = substrate();
        for mut sensor in sensors() {
            let mut entities = Vec::new();
            let mut channels = Vec::new();
            let mut relations = Vec::new();
            let mut ctx = BindContext::new(
                &substrate,
                corescout_mirror::schema::SensorId(1),
                &mut entities,
                &mut channels,
                &mut relations,
            );
            sensor
                .bind(&mut ctx)
                .unwrap_or_else(|e| panic!("{} did not bind: {e}", sensor.descriptor().key));
            assert!(
                !channels.is_empty(),
                "{} declared no channels",
                sensor.descriptor().key
            );
            assert!(
                !entities.is_empty(),
                "{} declared no entities",
                sensor.descriptor().key
            );
        }
    }

    #[test]
    fn every_sensor_documents_what_it_approximates() {
        // The mirror's standing rule: a channel says what physical fact it is
        // an approximation of, not merely what it is called.
        for sensor in sensors() {
            let descriptor = sensor.descriptor();
            assert!(
                descriptor.physical_fact.len() > 40,
                "{} does not say what it approximates",
                descriptor.key
            );
            assert!(!descriptor.source.is_empty());
            assert!(descriptor.max_rate_hz > 0.0);
        }
    }

    #[test]
    fn no_windows_sensor_claims_to_need_privilege_it_does_not() {
        // Both read unprivileged interfaces. If one starts needing rights, the
        // descriptor has to say so or the mirror will report the wrong reason
        // for its absence.
        for sensor in sensors() {
            assert!(
                !sensor.descriptor().requires_privilege,
                "{} claims privilege it does not use",
                sensor.descriptor().key
            );
        }
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn the_windows_sensors_have_distinct_ids_and_keys() {
        // Two sensors sharing an id would make a recording ambiguous about
        // which one produced a channel.
        let sensors = sensors();
        let ids: Vec<u16> = sensors.iter().map(|s| s.descriptor().id.0).collect();
        let keys: Vec<&str> = sensors.iter().map(|s| s.descriptor().key).collect();
        let mut unique_ids = ids.clone();
        unique_ids.sort_unstable();
        unique_ids.dedup();
        assert_eq!(unique_ids.len(), ids.len(), "duplicate sensor id: {ids:?}");
        let mut unique_keys = keys.clone();
        unique_keys.sort_unstable();
        unique_keys.dedup();
        assert_eq!(unique_keys.len(), keys.len(), "duplicate key: {keys:?}");
    }

    #[test]
    fn windows_sensor_ids_do_not_collide_with_the_sysfs_set() {
        // The two sets never run together today, and an id that meant one thing
        // on Linux and another on Windows would make a trace unreadable without
        // knowing where it came from.
        let windows: Vec<u16> = sensors().iter().map(|s| s.descriptor().id.0).collect();
        let linux: Vec<u16> = crate::observation::linux_sensors()
            .iter()
            .map(|s| s.descriptor().id.0)
            .collect();
        for id in &windows {
            assert!(!linux.contains(id), "sensor id {id} is used by both sets");
        }
    }
}
