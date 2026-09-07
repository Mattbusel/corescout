//! Platform abstraction layer.
//!
//! Everything OS-specific in CoreScout is reached through the [`Platform`]
//! trait. Topology discovery, benchmarking, ranking and output are written
//! against this trait and contain no `cfg(target_os)` of their own.
//!
//! # Adding a new platform
//!
//! Implement [`Platform`] in `platform/<os>/`, return it from [`detect`] under
//! the appropriate `cfg`, and nothing above this module needs to change. For
//! Windows the mapping is:
//!
//! | trait method                | Win32 |
//! |-----------------------------|-------|
//! | `discover_topology`         | `GetLogicalProcessorInformationEx` (+ `CallNtPowerInformation` for frequency, `GetSystemCpuSetInformation` for E/P class and `EfficiencyClass`) |
//! | `pin_current_thread`        | `SetThreadAffinityMask` / `SetThreadGroupAffinity` |
//! | `process_affinity`          | `GetProcessAffinityMask` |
//! | `spawn_with_affinity`       | `CreateProcess` suspended + `SetProcessAffinityMask` + `ResumeThread` |
//! | `thread_switch_counters`    | `QueryThreadCycleTime` / ETW (approximate) |
//!
//! The one place that needs care on Windows is processor *groups*: a machine
//! with more than 64 logical CPUs splits into groups, so [`crate::topology::LogicalId`]
//! must be mapped to a `(group, index)` pair inside the backend, never leaked
//! upward.

use std::process::{Child, Command};

use crate::topology::{LogicalId, Topology};
use corescout_core::cpuset::CpuSet;
use corescout_core::error::Result;

// Compiled everywhere: the sysfs *parser* inside is portable and its tests
// must run on any developer machine. Only its syscall submodule is gated.
pub mod linux;

pub mod cpuid;
pub mod stub;
#[cfg(target_os = "windows")]
pub mod windows;

/// Counters describing how much the OS interfered with a thread.
///
/// An *involuntary* context switch means the scheduler took the CPU away
/// mid-computation: the single most common cause of a benchmark outlier that
/// has nothing to do with the core being measured.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SwitchCounters {
    pub voluntary: u64,
    pub involuntary: u64,
}

impl SwitchCounters {
    /// Counters accumulated between two samples.
    pub fn delta(self, earlier: SwitchCounters) -> SwitchCounters {
        SwitchCounters {
            voluntary: self.voluntary.saturating_sub(earlier.voluntary),
            involuntary: self.involuntary.saturating_sub(earlier.involuntary),
        }
    }
}

/// The OS-specific operations CoreScout needs.
///
/// Implementations must be `Sync` because the benchmark runner hands the
/// platform to worker threads.
pub trait Platform: Send + Sync {
    /// Human-readable backend name, e.g. `"linux"`.
    fn name(&self) -> &'static str;

    /// Discover the full machine topology.
    fn discover_topology(&self) -> Result<Topology>;

    /// Restrict the *calling thread* to a single logical CPU.
    ///
    /// This is a hard pin, not a hint: after it returns successfully the
    /// thread must not run anywhere else, or benchmark attribution is void.
    fn pin_current_thread(&self, cpu: LogicalId) -> Result<()>;

    /// Set the calling thread's affinity to an arbitrary set.
    fn set_current_thread_affinity(&self, cpus: &CpuSet) -> Result<()>;

    /// Read the calling thread's current affinity mask.
    fn current_thread_affinity(&self) -> Result<CpuSet>;

    /// Read the affinity mask of the whole process.
    fn process_affinity(&self) -> Result<CpuSet>;

    /// Sample the calling thread's context-switch counters, when the platform
    /// can report them per-thread. `None` means "no interference data", and
    /// callers degrade to statistical outlier detection alone.
    fn thread_switch_counters(&self) -> Option<SwitchCounters>;

    /// Spawn a child process confined to `cpus`.
    ///
    /// The affinity must be applied before the child's `main` runs, otherwise
    /// early allocations and thread spawns land on the wrong cores.
    fn spawn_with_affinity(&self, command: &mut Command, cpus: &CpuSet) -> Result<Child>;
}

/// Return the backend for the platform we are running on.
///
/// A stub backend is returned on unsupported systems: `corescout info` then
/// fails with a clear message instead of the binary refusing to build.
pub fn detect() -> Box<dyn Platform> {
    #[cfg(target_os = "windows")]
    {
        Box::new(windows::WindowsPlatform::new())
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::LinuxPlatform::new())
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        Box::new(stub::StubPlatform::new())
    }
}
