//! The syscalls behind the actuators.
//!
//! Separated from the actuator logic so that the bounds checking, scoping,
//! auditing and revert machinery can be read, reviewed and tested without
//! wading through `unsafe`, and so that the parts of this crate that are not
//! Linux-specific stay portable.
//!
//! As in the observation layer, affinity goes through `syscall(2)` rather than
//! glibc's `CPU_SET` macros: `cpu_set_t` is a fixed 1024-bit bitmap and large
//! machines exceed it.

use corescout_core::{CpuSet, Error, LogicalId, Result};

/// Number of `u64` words needed to hold `max_cpu`.
fn words_for(max_cpu: u32) -> usize {
    (max_cpu as usize / 64) + 1
}

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

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// `sched_setaffinity`. `pid == 0` means the calling thread.
pub fn set_affinity(pid: i32, cpus: &CpuSet) -> Result<()> {
    let mask = to_mask(cpus)?;
    let len = std::mem::size_of_val(&mask[..]);
    // SAFETY: `mask` is a live allocation of exactly `len` bytes, which the
    // kernel only reads.
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

/// `sched_getaffinity`, growing the buffer until the kernel is satisfied.
pub fn get_affinity(pid: i32) -> Result<CpuSet> {
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
            return Ok(from_mask(&mask));
        }
        let err = errno();
        if err == libc::EINVAL && words < 1024 {
            words *= 2;
            continue;
        }
        return Err(Error::Syscall {
            call: "sched_getaffinity",
            errno: err,
        });
    }
}

/// This process's own affinity mask.
pub fn process_affinity() -> Result<CpuSet> {
    // SAFETY: getpid cannot fail.
    let pid = unsafe { libc::getpid() };
    get_affinity(pid)
}

/// `setpriority(PRIO_PROCESS, pid, nice)`.
pub fn set_priority(pid: i32, nice: i32) -> Result<()> {
    // SAFETY: a plain syscall with scalar arguments.
    let rc = unsafe { libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, nice) };
    if rc != 0 {
        return Err(Error::Syscall {
            call: "setpriority",
            errno: errno(),
        });
    }
    Ok(())
}

/// `getpriority(PRIO_PROCESS, pid)`.
///
/// `-1` is a legal niceness *and* the error return, so `errno` must be cleared
/// first and checked after. Getting this wrong reports a valid priority as a
/// failure roughly once in forty.
pub fn get_priority(pid: i32) -> Result<i32> {
    // SAFETY: writing through the libc errno location, then reading it back.
    unsafe {
        *libc::__errno_location() = 0;
    }
    let value = unsafe { libc::getpriority(libc::PRIO_PROCESS, pid as libc::id_t) };
    let err = unsafe { *libc::__errno_location() };
    if value == -1 && err != 0 {
        return Err(Error::Syscall {
            call: "getpriority",
            errno: err,
        });
    }
    Ok(value)
}

/// `set_mempolicy(MPOL_PREFERRED, ...)` for the calling thread.
///
/// Only the calling thread: Linux has no syscall to set another process's NUMA
/// policy, which is why [`crate::actuators::numa`] declines rather than
/// pretending. `numactl` works by starting the process itself, which is the
/// same limitation wearing a hat.
pub fn set_numa_preference(node: Option<u32>) -> Result<()> {
    const MPOL_DEFAULT: libc::c_long = 0;
    const MPOL_PREFERRED: libc::c_long = 1;
    const SYS_SET_MEMPOLICY: libc::c_long = 238; // x86-64

    let (mode, mask, maxnode): (libc::c_long, Vec<u64>, libc::c_long) = match node {
        Some(node) => {
            let words = (node as usize / 64) + 1;
            let mut mask = vec![0u64; words];
            mask[node as usize / 64] |= 1u64 << (node % 64);
            (MPOL_PREFERRED, mask, (words * 64) as libc::c_long)
        }
        // Clearing the policy takes a null mask.
        None => (MPOL_DEFAULT, Vec::new(), 0),
    };

    let ptr = if mask.is_empty() {
        std::ptr::null()
    } else {
        mask.as_ptr()
    };
    // SAFETY: `mask` lives across the call and `maxnode` describes its extent.
    let rc = unsafe { libc::syscall(SYS_SET_MEMPOLICY, mode, ptr, maxnode) };
    if rc < 0 {
        return Err(Error::Syscall {
            call: "set_mempolicy",
            errno: errno(),
        });
    }
    Ok(())
}

/// `get_mempolicy`, returning the preferred node when one is set.
pub fn get_numa_preference() -> Result<Option<u32>> {
    const SYS_GET_MEMPOLICY: libc::c_long = 239; // x86-64
    const MPOL_PREFERRED: i32 = 1;

    let mut mode: i32 = 0;
    let mut mask = vec![0u64; 16];
    let maxnode = (mask.len() * 64) as libc::c_long;
    // SAFETY: both out-parameters are live for the duration of the call.
    let rc = unsafe {
        libc::syscall(
            SYS_GET_MEMPOLICY,
            &mut mode as *mut i32,
            mask.as_mut_ptr(),
            maxnode,
            0usize,
            0usize,
        )
    };
    if rc < 0 {
        return Err(Error::Syscall {
            call: "get_mempolicy",
            errno: errno(),
        });
    }
    if mode != MPOL_PREFERRED {
        return Ok(None);
    }
    for (index, word) in mask.iter().enumerate() {
        if *word != 0 {
            return Ok(Some((index * 64 + word.trailing_zeros() as usize) as u32));
        }
    }
    Ok(None)
}

/// Whether a process exists and can be signalled by this user.
///
/// Used by the scope layer before it will accept a pid: a controller must not
/// be able to register a process it has no business touching.
pub fn process_is_reachable(pid: i32) -> bool {
    // SAFETY: signal 0 performs the permission check without delivering
    // anything, which is exactly what is wanted.
    unsafe { libc::kill(pid, 0) == 0 }
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
    fn an_empty_mask_is_refused_before_the_syscall() {
        assert!(to_mask(&CpuSet::new()).is_err());
    }

    #[test]
    fn word_sizing() {
        assert_eq!(words_for(0), 1);
        assert_eq!(words_for(63), 1);
        assert_eq!(words_for(64), 2);
    }

    #[test]
    fn reading_our_own_affinity_works() {
        let cpus = process_affinity().expect("this process has an affinity mask");
        assert!(!cpus.is_empty());
    }

    #[test]
    fn reading_our_own_priority_works() {
        // The -1 ambiguity: this must not report an error for a real value.
        let nice = get_priority(0).expect("getpriority");
        assert!((-20..=19).contains(&nice), "implausible niceness {nice}");
    }
}
