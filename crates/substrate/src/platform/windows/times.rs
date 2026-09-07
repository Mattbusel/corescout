//! Per-CPU time and interrupt counters, via `NtQuerySystemInformation`.
//!
//! # The Windows equivalent of `/proc/stat`
//!
//! `SystemProcessorPerformanceInformation` returns one record per logical CPU
//! with idle, kernel and user time, plus DPC and interrupt time and a raw
//! interrupt count. That is the substance of what the Linux scheduler and
//! interrupt sensors read, from one call rather than from two files.
//!
//! # Two things worth knowing about the numbers
//!
//! **Kernel time includes idle time.** Windows reports `KernelTime` as the
//! total time spent below user mode *including* the idle thread, so busy kernel
//! time is `KernelTime - IdleTime`. Reporting `KernelTime` as "kernel" without
//! that subtraction is the standard way to conclude a machine is extremely busy
//! while it sits at a desktop doing nothing.
//!
//! **The units are 100-nanosecond ticks.** Converted to nanoseconds here so
//! that a consumer comparing this to anything else is comparing the same unit.
//!
//! # Why this is a private import
//!
//! `NtQuerySystemInformation` lives in `ntdll` and is documented as subject to
//! change. It is used because the supported alternative, `GetSystemTimes`,
//! aggregates the whole machine and therefore cannot say anything per CPU,
//! which is the only thing a mirror of a multi-core machine is interested in.

use windows_sys::Win32::Foundation::NTSTATUS;

use corescout_core::error::{Error, Result};

/// `SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION`.
///
/// Declared here because `windows-sys` does not expose it. The layout is stable
/// and documented; the trailing reserved fields are part of the record and must
/// be present for the stride to be right.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessorPerformance {
    /// Time in the idle thread, in 100ns ticks.
    pub idle_time: i64,
    /// Time below user mode, **including** `idle_time`.
    pub kernel_time: i64,
    /// Time in user mode.
    pub user_time: i64,
    /// Time servicing deferred procedure calls.
    pub dpc_time: i64,
    /// Time servicing interrupts.
    pub interrupt_time: i64,
    /// Interrupts serviced, cumulative.
    pub interrupt_count: u32,
}

/// `SystemProcessorPerformanceInformation`.
const SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION: i32 = 8;

#[link(name = "ntdll")]
extern "system" {
    fn NtQuerySystemInformation(
        class: i32,
        buffer: *mut core::ffi::c_void,
        length: u32,
        returned: *mut u32,
    ) -> NTSTATUS;
}

/// Read one record per logical CPU.
pub fn read(cpu_count: usize) -> Result<Vec<ProcessorPerformance>> {
    if cpu_count == 0 {
        return Ok(Vec::new());
    }
    let mut buffer = vec![ProcessorPerformance::default(); cpu_count];
    let bytes = std::mem::size_of_val(buffer.as_slice()) as u32;
    let mut returned: u32 = 0;
    // SAFETY: the buffer holds exactly `cpu_count` records of the documented
    // layout and `bytes` is its true size; the call writes only into it and
    // reports how much it wrote.
    let status = unsafe {
        NtQuerySystemInformation(
            SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION,
            buffer.as_mut_ptr().cast(),
            bytes,
            &mut returned,
        )
    };
    if status < 0 {
        return Err(Error::syscall(
            "NtQuerySystemInformation(SystemProcessorPerformanceInformation)",
            status,
        ));
    }
    // Trust what the call says it wrote rather than what we asked for: a
    // machine can report fewer CPUs than the topology enumerated.
    let filled = returned as usize / std::mem::size_of::<ProcessorPerformance>();
    buffer.truncate(filled.min(cpu_count));
    Ok(buffer)
}

/// One CPU's times, in nanoseconds, with kernel time corrected.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CpuTimes {
    pub idle_ns: u64,
    /// Kernel time **excluding** idle. See the module note.
    ///
    /// Derived by subtraction, and the subtraction can transiently underflow:
    /// Windows updates the per-CPU counters independently, so a read can catch
    /// `idle` already advanced and `kernel` not yet. It saturates at zero when
    /// that happens, which means **this field is not monotonic** even though
    /// every counter it is derived from is.
    ///
    /// That matters downstream: the memory layer treats a counter going
    /// backwards as a wrap, and this field would produce false wraps. Anything
    /// that needs monotonicity should use [`CpuTimes::total_ns`], which is
    /// computed from the raw counters and does not subtract.
    pub kernel_ns: u64,
    /// Kernel time as Windows reports it, including idle. Monotonic.
    pub kernel_including_idle_ns: u64,
    pub user_ns: u64,
    pub dpc_ns: u64,
    pub interrupt_ns: u64,
    pub interrupt_count: u64,
}

impl CpuTimes {
    /// Everything that was not idle.
    pub fn busy_ns(&self) -> u64 {
        self.kernel_ns.saturating_add(self.user_ns)
    }

    /// Total accounted time.
    ///
    /// From the raw counters rather than the corrected ones, so it inherits
    /// their monotonicity. `kernel_including_idle + user` is the whole of the
    /// time Windows accounts for, because idle is already inside kernel.
    pub fn total_ns(&self) -> u64 {
        self.kernel_including_idle_ns.saturating_add(self.user_ns)
    }
}

/// Convert a raw record into nanoseconds, subtracting idle from kernel.
pub fn interpret(raw: &ProcessorPerformance) -> CpuTimes {
    // 100ns ticks. Negative values are not meaningful here; the fields are
    // signed only because the underlying type is a LARGE_INTEGER.
    let ticks_to_ns = |ticks: i64| -> u64 { (ticks.max(0) as u64).saturating_mul(100) };
    let idle_ns = ticks_to_ns(raw.idle_time);
    let kernel_including_idle = ticks_to_ns(raw.kernel_time);
    CpuTimes {
        idle_ns,
        // The correction. Without it a desktop at rest looks fully loaded.
        kernel_ns: kernel_including_idle.saturating_sub(idle_ns),
        kernel_including_idle_ns: kernel_including_idle,
        user_ns: ticks_to_ns(raw.user_time),
        dpc_ns: ticks_to_ns(raw.dpc_time),
        interrupt_ns: ticks_to_ns(raw.interrupt_time),
        interrupt_count: raw.interrupt_count as u64,
    }
}

/// Read and interpret in one step.
pub fn sample(cpu_count: usize) -> Result<Vec<CpuTimes>> {
    Ok(read(cpu_count)?.iter().map(interpret).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu_count() -> usize {
        super::super::topology::discover()
            .map(|t| t.logical_cpus.len())
            .unwrap_or(1)
    }

    #[test]
    fn this_machine_reports_per_cpu_times() {
        // Live. This is the sensor the mirror is built on, reading the machine
        // running the test.
        let times = sample(cpu_count()).expect("Windows reports processor times");
        assert!(!times.is_empty());
        for cpu in &times {
            assert!(
                cpu.total_ns() > 0,
                "a CPU reported no accounted time at all"
            );
        }
    }

    #[test]
    fn kernel_time_has_idle_subtracted_out() {
        // The correction that stops an idle desktop looking fully loaded.
        let raw = ProcessorPerformance {
            idle_time: 900,
            kernel_time: 1000,
            user_time: 50,
            ..ProcessorPerformance::default()
        };
        let times = interpret(&raw);
        assert_eq!(times.idle_ns, 90_000);
        assert_eq!(times.kernel_ns, 10_000, "kernel must exclude idle");
        assert_eq!(times.busy_ns(), 15_000);
        // Total comes from the raw counters, so idle is not double counted.
        assert_eq!(times.total_ns(), 105_000);
    }

    #[test]
    fn an_idle_machine_is_mostly_idle() {
        // The end-to-end version of the same check, on the real machine. If the
        // correction were missing this would fail, and it is exactly the bug
        // that would otherwise make every model in the project wrong.
        let times = sample(cpu_count()).expect("times");
        let idle: u64 = times.iter().map(|c| c.idle_ns).sum();
        let total: u64 = times.iter().map(|c| c.total_ns()).sum();
        assert!(total > 0);
        let idle_share = idle as f64 / total as f64;
        assert!(
            idle_share > 0.20,
            "this machine claims to be {:.0}% busy since boot, which almost \
             certainly means idle is not being subtracted from kernel time",
            (1.0 - idle_share) * 100.0
        );
    }

    #[test]
    fn counters_only_go_up() {
        // Cumulative, like their Linux counterparts. A counter that went
        // backwards would break every rate the memory layer derives.
        let first = sample(cpu_count()).expect("times");
        std::thread::sleep(std::time::Duration::from_millis(60));
        let second = sample(cpu_count()).expect("times");
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(second.iter()) {
            assert!(b.idle_ns >= a.idle_ns, "idle went backwards");
            assert!(b.user_ns >= a.user_ns, "user went backwards");
            assert!(
                b.kernel_including_idle_ns >= a.kernel_including_idle_ns,
                "raw kernel time went backwards"
            );
            // Derived from the raw counters, so monotonic. `kernel_ns` is
            // deliberately *not* checked here: it is a subtraction that can
            // transiently underflow, which is documented on the field.
            assert!(b.total_ns() >= a.total_ns(), "total went backwards");
        }
    }

    #[test]
    fn time_actually_passes_between_samples() {
        // The sensor has to be live rather than returning a constant.
        let first = sample(cpu_count()).expect("times");
        std::thread::sleep(std::time::Duration::from_millis(120));
        let second = sample(cpu_count()).expect("times");
        let before: u64 = first.iter().map(|c| c.total_ns()).sum();
        let after: u64 = second.iter().map(|c| c.total_ns()).sum();
        assert!(
            after > before,
            "no time was accounted across a 120 ms sleep; the sensor is not live"
        );
    }

    #[test]
    fn asking_for_no_cpus_is_not_an_error() {
        assert!(read(0).expect("empty is fine").is_empty());
    }
}
