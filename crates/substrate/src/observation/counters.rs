//! Hardware performance counters.
//!
//! | property | value |
//! |---|---|
//! | physical fact | cycles, retired instructions, cache misses and branch mispredictions actually executed by each core |
//! | source | `perf_event_open(2)` in per-CPU counting mode |
//! | sample rate | ~1 kHz; each read is one `read(2)` on an open descriptor |
//! | cost | one syscall per counter per CPU per tick |
//! | perturbation | **Low** |
//! | uncertainty | exact within the event's own definition, which varies between microarchitectures |
//!
//! # This is the closest thing to direct hardware self-observation
//!
//! Every other sensor reads a number the kernel computed. These counters are
//! read from the PMU registers of the core itself: the machine counting its own
//! instructions as it executes them. If any observation deserves the word
//! proprioception, it is this one.
//!
//! # Why it is nonetheless `Low` rather than `Negligible`
//!
//! Not because reading costs much, but because *holding* the counters does. A
//! core has a small number of general-purpose PMU counters, typically four to
//! eight. CoreScout occupying four of them means any other profiler on the
//! machine gets time-multiplexed onto what is left, and multiplexed counts are
//! scaled estimates rather than measurements.
//!
//! So this sensor degrades other observers' accuracy while improving its own.
//! That is a genuine perturbation of the machine's observable state, even though
//! no physical quantity changed, and it is exactly the kind of effect the
//! perturbation declaration exists to surface.
//!
//! # Privilege and availability
//!
//! Per-CPU system-wide counting needs `perf_event_paranoid <= 0` or
//! `CAP_PERFMON`. It is also usually unavailable inside VMs without a virtual
//! PMU, and inside containers by default. All of these appear as a bind failure,
//! which marks the sensor inactive and leaves its columns unobserved.
//!
//! # File descriptors
//!
//! Four events times the number of CPUs. On a large server that is several
//! hundred descriptors held open for the lifetime of the mirror, which can
//! exceed a default `RLIMIT_NOFILE` of 1024 on a 256-CPU machine. A partial
//! open is treated as success for the CPUs that worked.

use crate::observation::{
    BindContext, Perturbation, Sensor, SensorDescriptor, SensorId, SensorOutcome, StateWriter,
    Uncertainty,
};
use corescout_core::error::{Error, Result};
#[cfg_attr(not(target_os = "linux"), allow(unused_imports))]
use corescout_mirror::state::{ChannelId, Semantics, Unit};

/// The events CoreScout counts, as `(channel key, PERF_COUNT_HW_* config)`.
///
/// Deliberately four: enough to characterise what a core is doing, few enough
/// to fit in the general-purpose counters of every current x86 part without
/// forcing multiplexing on itself.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const EVENTS: [(&str, u64); 4] = [
    ("cpu.pmu.cycles", 0),        // PERF_COUNT_HW_CPU_CYCLES
    ("cpu.pmu.instructions", 1),  // PERF_COUNT_HW_INSTRUCTIONS
    ("cpu.pmu.cache_misses", 3),  // PERF_COUNT_HW_CACHE_MISSES
    ("cpu.pmu.branch_misses", 5), // PERF_COUNT_HW_BRANCH_MISSES
];

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct CounterSensor {
    /// `(entity row, event index, file descriptor)`.
    counters: Vec<(u32, usize, RawFd)>,
    channels: Vec<ChannelId>,
}

#[cfg(target_os = "linux")]
type RawFd = std::os::unix::io::RawFd;
#[cfg(not(target_os = "linux"))]
type RawFd = i32;

impl CounterSensor {
    pub fn new() -> CounterSensor {
        CounterSensor {
            counters: Vec::new(),
            channels: Vec::new(),
        }
    }
}

impl Default for CounterSensor {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for CounterSensor {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        for (_, _, fd) in &self.counters {
            // SAFETY: each fd was returned by perf_event_open and is closed
            // exactly once, here.
            unsafe {
                libc::close(*fd);
            }
        }
    }
}

impl Sensor for CounterSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: SensorId(7),
            key: "counters",
            physical_fact: "cycles, retired instructions, cache misses and branch \
                            mispredictions executed by each core since the counter was opened",
            source: "perf_event_open(2), PERF_TYPE_HARDWARE, per-CPU counting mode",
            max_rate_hz: 1000.0,
            // Holding PMU counters degrades every other profiler on the machine;
            // see the module docs.
            perturbation: Perturbation::Low,
            uncertainty: Uncertainty::unknown(
                "counts are exact for the event as the microarchitecture defines it, but \
                 event definitions differ between vendors and generations, so absolute \
                 comparison across machines is not meaningful",
            ),
            requires_privilege: true,
        }
    }

    fn bind(&mut self, ctx: &mut BindContext<'_>) -> Result<()> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = ctx;
            Err(Error::unsupported(
                "hardware performance counters (perf_event_open is Linux only)",
            ))
        }

        #[cfg(target_os = "linux")]
        {
            let cpus: Vec<u32> = ctx
                .substrate()
                .topology
                .logical_cpus
                .iter()
                .filter(|c| c.online)
                .map(|c| c.id)
                .collect();

            let mut counters = Vec::new();
            for cpu in cpus {
                let Some(row) = ctx.row_of(&corescout_mirror::entity::keys::logical_cpu(cpu))
                else {
                    continue;
                };
                for (index, (_, config)) in EVENTS.iter().enumerate() {
                    match perf::open_counting_event(*config, cpu as i32) {
                        Ok(fd) => counters.push((row, index, fd)),
                        // A partial open is normal: descriptor limits, a CPU
                        // that went offline, or an event this part does not
                        // implement. Whatever opened, we use.
                        Err(_) => continue,
                    }
                }
            }

            if counters.is_empty() {
                return Err(Error::unsupported(
                    "perf_event_open returned nothing usable (perf_event_paranoid may be \
                     above 0, or this machine has no accessible PMU)",
                ));
            }

            self.counters = counters;
            for (key, _) in EVENTS {
                self.channels
                    .push(ctx.declare_channel(key, Unit::Count, Semantics::Cumulative));
            }
            Ok(())
        }
    }

    fn observe(&mut self, out: &mut StateWriter<'_>) -> SensorOutcome {
        #[allow(unused_mut)]
        let mut outcome = SensorOutcome::default();
        #[cfg(target_os = "linux")]
        for (row, event, fd) in &self.counters {
            match perf::read_counter(*fd) {
                Some(value) => {
                    if let Some(channel) = self.channels.get(*event) {
                        out.set(*row, *channel, value as f64);
                        outcome.sample();
                    }
                }
                None => outcome.error(),
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = out;
        outcome
    }
}

/// The `perf_event_open` ABI, defined here rather than taken from a binding.
///
/// The struct below is exactly `PERF_ATTR_SIZE_VER0`, the original 64-byte
/// layout, and `size` is set to match. The kernel accepts any known size and
/// zero-fills the rest, so using the oldest layout is both the simplest and the
/// most portable option: it cannot drift with the kernel, and it avoids
/// depending on a generated binding whose padding is not part of any stable
/// contract.
///
/// Every flag bit is left zero, which is precisely the configuration wanted:
/// the counter starts enabled, counts both user and kernel, and does not
/// inherit across `fork`.
#[cfg(target_os = "linux")]
mod perf {
    /// `PERF_TYPE_HARDWARE`.
    const PERF_TYPE_HARDWARE: u32 = 0;
    /// `PERF_FLAG_FD_CLOEXEC`: never leak counters into a child process.
    const PERF_FLAG_FD_CLOEXEC: u64 = 8;

    /// Size of the attribute struct, which is also the value of its `size`
    /// field. Exposed so a test can assert the ABI has not drifted.
    #[allow(dead_code)]
    pub const ATTR_SIZE: usize = std::mem::size_of::<PerfEventAttrV0>();

    #[repr(C)]
    #[derive(Default)]
    struct PerfEventAttrV0 {
        type_: u32,
        size: u32,
        config: u64,
        sample_period_or_freq: u64,
        sample_type: u64,
        read_format: u64,
        /// The flag bitfield. All zero: enabled, not inherited, counting both
        /// user and kernel time.
        flags: u64,
        wakeup: u32,
        bp_type: u32,
        config1: u64,
    }

    /// Open one counting event on one CPU, across all processes.
    pub fn open_counting_event(config: u64, cpu: i32) -> Result<i32, i32> {
        let attr = PerfEventAttrV0 {
            type_: PERF_TYPE_HARDWARE,
            size: std::mem::size_of::<PerfEventAttrV0>() as u32,
            config,
            ..Default::default()
        };
        debug_assert_eq!(std::mem::size_of::<PerfEventAttrV0>(), 64);

        // SAFETY: `attr` is a valid, fully initialised struct of the size it
        // declares. pid = -1 with a specific cpu means "all processes on this
        // CPU", which is the system-wide counting mode.
        let fd = unsafe {
            libc::syscall(
                libc::SYS_perf_event_open,
                &attr as *const PerfEventAttrV0,
                -1i32, // pid: all processes
                cpu,   // this CPU only
                -1i32, // group_fd: not in a group
                PERF_FLAG_FD_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0));
        }
        Ok(fd as i32)
    }

    /// Read a counter's current value.
    ///
    /// With `read_format` left at zero, the kernel returns a single `u64`.
    pub fn read_counter(fd: i32) -> Option<u64> {
        let mut value: u64 = 0;
        // SAFETY: reading 8 bytes into an 8-byte stack variable.
        let read = unsafe {
            libc::read(
                fd,
                &mut value as *mut u64 as *mut libc::c_void,
                std::mem::size_of::<u64>(),
            )
        };
        if read == std::mem::size_of::<u64>() as isize {
            Some(value)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_event_set_is_distinct_and_named() {
        let mut keys: Vec<&str> = EVENTS.iter().map(|(k, _)| *k).collect();
        let mut configs: Vec<u64> = EVENTS.iter().map(|(_, c)| *c).collect();
        keys.sort_unstable();
        configs.sort_unstable();
        let unique_keys = {
            let mut v = keys.clone();
            v.dedup();
            v.len()
        };
        let unique_configs = {
            let mut v = configs.clone();
            v.dedup();
            v.len()
        };
        assert_eq!(unique_keys, EVENTS.len());
        assert_eq!(unique_configs, EVENTS.len());
    }

    #[test]
    fn the_event_set_fits_in_the_general_purpose_counters() {
        // Four events is the budget: more would force the PMU to multiplex,
        // which would make our own counts estimates rather than measurements.
        assert!(EVENTS.len() <= 4, "the PMU budget was exceeded");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn the_attr_struct_matches_the_kernel_abi_size() {
        // PERF_ATTR_SIZE_VER0. If this ever changes, the kernel will reject
        // every open with EINVAL, so assert it here rather than discovering it
        // at runtime on a machine we cannot debug.
        assert_eq!(super::perf::ATTR_SIZE, 64);
    }
}
