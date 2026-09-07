//! Linux affinity and thread-accounting syscalls.
//!
//! This module deliberately talks to the kernel through `syscall(2)` rather
//! than glibc's `CPU_SET` macros. `cpu_set_t` is a fixed 1024-bit bitmap; large
//! servers and future machines exceed it, and glibc's dynamic `CPU_ALLOC` API
//! is awkward to use safely from Rust. Passing our own `u64` mask gives exact
//! control over the bitmap length, which is what the raw syscall ABI actually
//! wants.

use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

use crate::platform::SwitchCounters;
use crate::topology::LogicalId;
use corescout_core::cpuset::CpuSet;
use corescout_core::error::{Error, Result};

/// Number of `u64` words needed to hold `max_cpu`, with headroom.
///
/// The kernel requires the buffer length to be a multiple of the word size and
/// at least as large as its internal `cpumask_t`; it returns `EINVAL` when the
/// buffer is too small, which [`get_affinity`] handles by growing.
fn words_for(max_cpu: u32) -> usize {
    (max_cpu as usize / 64) + 1
}

/// Convert a [`CpuSet`] into the kernel's bitmap representation.
fn to_mask(cpus: &CpuSet) -> Result<Vec<u64>> {
    let max = cpus
        .max()
        .ok_or_else(|| Error::invalid("cannot set an empty CPU affinity mask"))?;
    let mut mask = vec![0u64; words_for(max)];
    for cpu in cpus.iter() {
        mask[cpu as usize / 64] |= 1u64 << (cpu % 64);
    }
    Ok(mask)
}

/// Convert a kernel bitmap back into a [`CpuSet`].
fn from_mask(mask: &[u64]) -> CpuSet {
    let mut set = CpuSet::new();
    for (word_index, word) in mask.iter().enumerate() {
        let mut bits = *word;
        while bits != 0 {
            let bit = bits.trailing_zeros() as usize;
            set.insert((word_index * 64 + bit) as LogicalId);
            bits &= bits - 1;
        }
    }
    set
}

/// `sched_setaffinity(pid, ...)`. `pid == 0` means the calling *thread*.
fn set_affinity(pid: libc::pid_t, cpus: &CpuSet) -> Result<()> {
    let mask = to_mask(cpus)?;
    let len = std::mem::size_of_val(&mask[..]);
    // SAFETY: `mask` is a live allocation of exactly `len` bytes and the kernel
    // only reads from it.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_sched_setaffinity,
            pid as libc::c_long,
            len as libc::c_long,
            mask.as_ptr(),
        )
    };
    if rc < 0 {
        return Err(Error::Syscall {
            call: "sched_setaffinity",
            errno: errno(),
        });
    }
    Ok(())
}

/// `sched_getaffinity(pid, ...)`, growing the buffer until the kernel is happy.
fn get_affinity(pid: libc::pid_t) -> Result<CpuSet> {
    let mut words = 16; // 1024 CPUs, the historical cpu_set_t size.
    loop {
        let mut mask = vec![0u64; words];
        let len = std::mem::size_of_val(&mask[..]);
        // SAFETY: the kernel writes at most `len` bytes into our allocation.
        let rc = unsafe {
            libc::syscall(
                libc::SYS_sched_getaffinity,
                pid as libc::c_long,
                len as libc::c_long,
                mask.as_mut_ptr(),
            )
        };
        if rc >= 0 {
            // The return value is the number of *bytes* filled in; anything
            // past it is untouched, and our buffer started zeroed.
            return Ok(from_mask(&mask));
        }
        let err = errno();
        if err == libc::EINVAL && words < 1024 {
            // Buffer too small for this kernel's cpumask. Double and retry.
            words *= 2;
            continue;
        }
        return Err(Error::Syscall {
            call: "sched_getaffinity",
            errno: err,
        });
    }
}

/// Pin the calling thread to exactly one logical CPU.
pub fn pin_current_thread(cpu: LogicalId) -> Result<()> {
    let mut set = CpuSet::new();
    set.insert(cpu);
    set_affinity(0, &set)
}

/// Set the calling thread's affinity to `cpus`.
pub fn set_current_thread_affinity(cpus: &CpuSet) -> Result<()> {
    set_affinity(0, cpus)
}

/// Read the calling thread's affinity.
pub fn current_thread_affinity() -> Result<CpuSet> {
    get_affinity(0)
}

/// Read the affinity of the process as a whole (its main thread).
pub fn process_affinity() -> Result<CpuSet> {
    // SAFETY: getpid() cannot fail.
    let pid = unsafe { libc::getpid() };
    get_affinity(pid)
}

/// Per-thread context-switch counters via `getrusage(RUSAGE_THREAD)`.
///
/// `ru_nivcsw` (involuntary switches) is the signal CoreScout cares about: it
/// increments when the scheduler preempted us, which is exactly the event that
/// turns a clean benchmark iteration into an outlier. `RUSAGE_THREAD` is a
/// Linux extension; on other systems the equivalent is process-wide and far
/// less useful.
pub fn thread_switch_counters() -> Option<SwitchCounters> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    // SAFETY: getrusage fully initialises the struct on success.
    let rc = unsafe { libc::getrusage(libc::RUSAGE_THREAD, usage.as_mut_ptr()) };
    if rc != 0 {
        return None;
    }
    let usage = unsafe { usage.assume_init() };
    Some(SwitchCounters {
        voluntary: usage.ru_nvcsw.max(0) as u64,
        involuntary: usage.ru_nivcsw.max(0) as u64,
    })
}

/// Spawn a child process already confined to `cpus`.
///
/// The affinity is applied in the forked child *before* `execve`, so the target
/// program is on the intended CPUs from its very first instruction. Setting it
/// after spawn would leave the program's startup, its allocator arenas and any
/// threads it creates early on the wrong cores.
pub fn spawn_with_affinity(command: &mut Command, cpus: &CpuSet) -> Result<Child> {
    let mask = to_mask(cpus)?;
    let len = std::mem::size_of_val(&mask[..]);

    // SAFETY: `pre_exec` runs between fork and exec, where only
    // async-signal-safe work is permitted. `syscall()` is a bare syscall and
    // allocates nothing; `mask` was allocated before the fork and is inherited
    // by the child's address space unchanged.
    unsafe {
        command.pre_exec(move || {
            let rc = libc::syscall(
                libc::SYS_sched_setaffinity,
                0 as libc::c_long,
                len as libc::c_long,
                mask.as_ptr(),
            );
            if rc < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    command
        .spawn()
        .map_err(|e| Error::io(command.get_program(), e))
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_round_trips() {
        let set: CpuSet = [0u32, 5, 63, 64, 130].into_iter().collect();
        let mask = to_mask(&set).unwrap();
        assert_eq!(mask.len(), 3, "cpu 130 needs a third 64-bit word");
        assert_eq!(from_mask(&mask), set);
    }

    #[test]
    fn empty_mask_is_rejected() {
        // An empty affinity mask is EINVAL at the syscall; catching it here
        // gives a message that says what the user actually did wrong.
        assert!(to_mask(&CpuSet::new()).is_err());
    }

    #[test]
    fn word_sizing() {
        assert_eq!(words_for(0), 1);
        assert_eq!(words_for(63), 1);
        assert_eq!(words_for(64), 2);
        assert_eq!(words_for(255), 4);
    }

    #[test]
    fn reads_back_a_pin() {
        // The kernel is the test oracle here: pin to the first CPU we are
        // allowed on, then confirm the mask really narrowed.
        let original = current_thread_affinity().expect("get affinity");
        let first = original.iter().next().expect("at least one CPU");
        pin_current_thread(first).expect("pin");
        let now = current_thread_affinity().expect("get affinity");
        assert_eq!(now.to_vec(), vec![first]);
        set_current_thread_affinity(&original).expect("restore");
    }

    #[test]
    fn switch_counters_are_monotonic() {
        let a = thread_switch_counters().expect("RUSAGE_THREAD");
        let b = thread_switch_counters().expect("RUSAGE_THREAD");
        assert!(b.voluntary >= a.voluntary);
        assert!(b.involuntary >= a.involuntary);
        assert_eq!(b.delta(b), SwitchCounters::default());
    }
}
