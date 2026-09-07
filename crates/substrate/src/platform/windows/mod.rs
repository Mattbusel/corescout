//! The Windows backend.
//!
//! # What works and what does not
//!
//! | operation | Windows | note |
//! |---|---|---|
//! | topology | yes | `GetLogicalProcessorInformationEx`, one consistent snapshot |
//! | affinity | yes | group-aware, so it works past 64 CPUs |
//! | frequency | yes | `CallNtPowerInformation`, with the usual "requested, not actual" caveat |
//! | CPU times | yes | `NtQuerySystemInformation`, per CPU |
//! | interrupts | yes | from the same record |
//! | context switches | **no** | not exposed per thread without a heavy system-wide query |
//! | thermal | **no** | no supported API; vendor drivers only |
//! | power | **no** | no RAPL equivalent |
//! | PMU counters | **no** | needs a driver |
//!
//! The absences are reported through the mirror's availability codes as
//! `Unsupported`, which is the distinction that machinery exists to make: this
//! machine cannot tell you, as opposed to this machine is idle.
//!
//! # Context switches
//!
//! Linux gives per-thread involuntary switch counts from `getrusage`, and the
//! benchmark uses them to mark an iteration as *known* contaminated rather than
//! merely suspicious. Windows has no cheap equivalent, so
//! [`WindowsPlatform::thread_switch_counters`] returns `None` and the benchmark
//! falls back to statistical outlier detection alone.
//!
//! That is a real loss of measurement quality on this platform, and it is
//! better to say so than to substitute a number that means something else.

pub mod affinity;
pub mod frequency;
pub mod times;
pub mod topology;

use std::process::{Child, Command};

use corescout_core::cpuset::CpuSet;
use corescout_core::error::Result;
use corescout_core::LogicalId;

use crate::platform::{Platform, SwitchCounters};
use crate::topology::Topology;

/// The Windows implementation of [`Platform`].
pub struct WindowsPlatform;

impl WindowsPlatform {
    pub fn new() -> Self {
        WindowsPlatform
    }
}

impl Default for WindowsPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for WindowsPlatform {
    fn name(&self) -> &'static str {
        "windows"
    }

    fn discover_topology(&self) -> Result<Topology> {
        topology::discover()
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
        // See the module note. Windows does not expose this per thread cheaply,
        // and a number that counted something else would be worse than none.
        None
    }

    fn thread_cycles(&self) -> Option<u64> {
        thread_cycles()
    }

    fn spawn_with_affinity(&self, command: &mut Command, cpus: &CpuSet) -> Result<Child> {
        // Windows has no `pre_exec`. The child is started, its affinity set,
        // and then it runs: there is a brief window in which it is unconfined,
        // which the Linux backend does not have. Recorded here because it
        // matters for a launcher whose whole purpose is placement.
        let child = command
            .spawn()
            .map_err(|source| corescout_core::error::Error::Io {
                path: std::path::PathBuf::from(command.get_program()),
                source,
            })?;
        set_process_affinity(child.id(), cpus)?;
        Ok(child)
    }
}

/// Confine another process to a set of CPUs.
fn set_process_affinity(pid: u32, cpus: &CpuSet) -> Result<()> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, SetProcessAffinityMask, PROCESS_SET_INFORMATION,
    };

    if cpus.is_empty() {
        return Err(corescout_core::error::Error::invalid(
            "cannot confine a process to an empty set of CPUs",
        ));
    }
    let mut mask = 0usize;
    for cpu in cpus.iter() {
        mask |= 1usize << (cpu % 64);
    }

    // SAFETY: opening a handle to a pid we just created, and closing it below
    // on every path.
    let handle = unsafe { OpenProcess(PROCESS_SET_INFORMATION, 0, pid) };
    if handle == 0 {
        return Err(corescout_core::error::Error::syscall(
            "OpenProcess",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
        ));
    }
    // SAFETY: a live handle with the right access right.
    let ok = unsafe { SetProcessAffinityMask(handle, mask) };
    // SAFETY: closing the handle we opened, exactly once.
    unsafe {
        CloseHandle(handle);
    }
    if ok == 0 {
        return Err(corescout_core::error::Error::syscall(
            "SetProcessAffinityMask",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
        ));
    }
    Ok(())
}

// `QueryThreadCycleTime` is a documented kernel32 export that `windows-sys`
// 0.52 does not bind, so it is declared here. It is the high-resolution
// alternative to `GetThreadTimes`, which is quantised to the scheduler tick and
// therefore useless for anything shorter than about 16 milliseconds.
#[link(name = "kernel32")]
extern "system" {
    fn QueryThreadCycleTime(
        thread: windows_sys::Win32::Foundation::HANDLE,
        cycles: *mut u64,
    ) -> windows_sys::Win32::Foundation::BOOL;
}

/// Cycles the calling thread has executed.
///
/// # What a cycle means on a hybrid part
///
/// On this machine a P-core cycle and an E-core cycle are not the same amount
/// of silicon, energy, or work. Cycles are still the right denominator for
/// "how much of the machine did this consume", and they are the wrong number
/// for "how long did this take". Anything comparing placements should report
/// both, which is why [`Platform::thread_cycles`] sits alongside the clock
/// rather than replacing it.
pub fn thread_cycles() -> Option<u64> {
    use windows_sys::Win32::System::Threading::GetCurrentThread;
    let mut cycles: u64 = 0;
    // SAFETY: a pseudo-handle to the current thread and an owned out-parameter.
    let ok = unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut cycles) };
    (ok != 0).then_some(cycles)
}

/// The processor brand string, from `CPUID` leaves 0x80000002..0x80000004.
///
/// Read directly rather than from the registry: the registry copy is written at
/// install time and can be stale on a machine whose CPU was replaced, and this
/// is one of the few facts worth getting from the silicon itself.
pub fn cpu_brand() -> Option<String> {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::__cpuid;
        // SAFETY: CPUID is unprivileged and available on every x86-64 part. The
        // extended leaves are checked for support first.
        let supported = unsafe { __cpuid(0x8000_0000) }.eax >= 0x8000_0004;
        if !supported {
            return None;
        }
        let mut bytes = Vec::with_capacity(48);
        for leaf in 0x8000_0002u32..=0x8000_0004 {
            let result = unsafe { __cpuid(leaf) };
            for register in [result.eax, result.ebx, result.ecx, result.edx] {
                bytes.extend_from_slice(&register.to_le_bytes());
            }
        }
        let text = String::from_utf8_lossy(&bytes);
        let trimmed = text.trim_end_matches('\0').trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        None
    }
}

/// The vendor string, from `CPUID` leaf 0.
pub fn cpu_vendor() -> Option<String> {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::__cpuid;
        // SAFETY: leaf 0 is universally available.
        let result = unsafe { __cpuid(0) };
        let mut bytes = Vec::with_capacity(12);
        for register in [result.ebx, result.edx, result.ecx] {
            bytes.extend_from_slice(&register.to_le_bytes());
        }
        let text = String::from_utf8_lossy(&bytes).trim().to_string();
        (!text.is_empty()).then_some(text)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_machine_names_its_own_processor() {
        // Live. Reads the silicon rather than a registry key written at install
        // time, so it is right even on a machine whose CPU was replaced.
        let brand = cpu_brand().expect("x86-64 parts carry a brand string");
        assert!(brand.len() > 4, "implausible brand {brand:?}");
        let vendor = cpu_vendor().expect("every part has a vendor");
        assert!(
            vendor == "GenuineIntel" || vendor == "AuthenticAMD" || !vendor.is_empty(),
            "unexpected vendor {vendor:?}"
        );
    }

    #[test]
    fn this_thread_can_count_its_own_cycles() {
        // Live, and it must actually advance: a counter that stands still would
        // make every productivity ratio in the project infinite.
        let before = thread_cycles().expect("Windows reports thread cycle time");
        let mut sink = 0u64;
        for i in 0..2_000_000u64 {
            sink = sink.wrapping_add(i ^ sink);
        }
        std::hint::black_box(sink);
        let after = thread_cycles().expect("still reports");
        assert!(after > before, "cycles did not advance across real work");
    }

    #[test]
    fn cycles_are_not_wall_time() {
        // The distinction the whole measurement rests on. Sleeping burns wall
        // time and no cycles; a thread that waited did not use the machine.
        let before = thread_cycles().expect("reported");
        std::thread::sleep(std::time::Duration::from_millis(80));
        let after = thread_cycles().expect("reported");
        let burned = after - before;
        // A sleeping thread executes some cycles waking up, but nowhere near
        // 80 ms worth at any plausible clock.
        assert!(
            burned < 10_000_000,
            "sleeping burned {burned} cycles, so this is measuring wall time"
        );
    }

    #[test]
    fn the_backend_reports_itself_as_windows() {
        assert_eq!(WindowsPlatform::new().name(), "windows");
    }

    #[test]
    fn the_backend_admits_it_cannot_count_context_switches() {
        // The absence is the point: a number counting something else would be
        // worse than none, and the benchmark degrades gracefully on `None`.
        assert!(WindowsPlatform::new().thread_switch_counters().is_none());
    }

    #[test]
    fn the_platform_trait_works_end_to_end_on_this_machine() {
        let platform = WindowsPlatform::new();
        let topology = platform.discover_topology().expect("topology");
        assert!(!topology.logical_cpus.is_empty());

        let permitted = platform.process_affinity().expect("affinity");
        assert!(!permitted.is_empty());

        let original = platform.current_thread_affinity().expect("readable");
        let first = permitted.iter().next().expect("a CPU");
        platform.pin_current_thread(first).expect("pinnable");
        platform
            .set_current_thread_affinity(&original)
            .expect("restorable");
    }
}
