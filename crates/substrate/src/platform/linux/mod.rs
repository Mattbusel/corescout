//! Linux backend.
//!
//! Topology comes from sysfs ([`sysfs`]); pinning, affinity and thread
//! accounting come from syscalls (`affinity`).
//!
//! Note that [`sysfs`] is compiled on *every* platform, not just Linux: it is
//! pure file parsing, and keeping it portable is what lets its tests run on any
//! developer machine against captured sysfs trees. Only the syscall layer is
//! gated behind `cfg(target_os = "linux")`.

pub mod sysfs;

#[cfg(target_os = "linux")]
mod affinity;

#[cfg(target_os = "linux")]
pub use linux_platform::LinuxPlatform;

#[cfg(target_os = "linux")]
mod linux_platform {
    use std::process::{Child, Command};

    use super::{affinity, sysfs::Sysfs};
    use crate::platform::{Platform, SwitchCounters};
    use crate::topology::{LogicalId, Topology};
    use corescout_core::cpuset::CpuSet;
    use corescout_core::error::Result;

    /// The Linux implementation of [`Platform`].
    pub struct LinuxPlatform {
        sysfs: Sysfs,
    }

    impl LinuxPlatform {
        pub fn new() -> Self {
            LinuxPlatform {
                sysfs: Sysfs::system(),
            }
        }

        /// Read topology from an alternate filesystem root. Used by tests and
        /// by the hidden `--sysroot` flag for inspecting a captured tree.
        pub fn with_sysfs(sysfs: Sysfs) -> Self {
            LinuxPlatform { sysfs }
        }
    }

    impl Default for LinuxPlatform {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Platform for LinuxPlatform {
        fn name(&self) -> &'static str {
            "linux"
        }

        fn discover_topology(&self) -> Result<Topology> {
            // The process mask is read first so the topology records what we
            // were actually allowed to use at discovery time, before any
            // benchmark thread starts narrowing its own affinity.
            let affinity = affinity::process_affinity().ok();
            self.sysfs.read_topology(affinity)
        }

        fn pin_current_thread(&self, cpu: LogicalId) -> Result<()> {
            affinity::pin_current_thread(cpu)
        }

        fn set_current_thread_affinity(&self, cpus: &CpuSet) -> Result<()> {
            affinity::set_current_thread_affinity(cpus)
        }

        fn current_thread_affinity(&self) -> Result<CpuSet> {
            affinity::current_thread_affinity()
        }

        fn process_affinity(&self) -> Result<CpuSet> {
            affinity::process_affinity()
        }

        fn thread_switch_counters(&self) -> Option<SwitchCounters> {
            affinity::thread_switch_counters()
        }

        fn spawn_with_affinity(&self, command: &mut Command, cpus: &CpuSet) -> Result<Child> {
            affinity::spawn_with_affinity(command, cpus)
        }
    }
}
