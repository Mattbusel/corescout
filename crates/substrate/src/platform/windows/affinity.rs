//! Thread and process affinity on Windows.
//!
//! # There is no getter
//!
//! Windows exposes `SetThreadAffinityMask` and no corresponding get. The
//! documented way to read the current mask is to set it to something and use
//! the *previous* value the call returns, then set it back.
//!
//! That is a read implemented as two writes, and it is worth being explicit
//! about the consequence: reading a thread's affinity briefly widens it. On a
//! machine where something else is watching placement, the read is visible. The
//! mirror's rule is that observation should not perturb, and this observation
//! does, which is why [`current_thread_affinity`] is used only where a caller
//! has asked for it rather than on every pass.
//!
//! # Groups
//!
//! The classic mask APIs address one processor group of at most 64 CPUs. A
//! machine with more has several groups, and a plain mask cannot name a CPU
//! outside the calling thread's current group. Pinning across groups needs
//! `SetThreadGroupAffinity`, which is what [`pin_current_thread`] uses so that
//! the same code path works on a 16-CPU laptop and a 128-CPU server.

use windows_sys::Win32::System::Kernel::PROCESSOR_NUMBER;
use windows_sys::Win32::System::SystemInformation::GROUP_AFFINITY;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, GetProcessAffinityMask, GetThreadGroupAffinity,
    SetThreadAffinityMask, SetThreadGroupAffinity,
};

use corescout_core::cpuset::CpuSet;
use corescout_core::error::{Error, Result};
use corescout_core::LogicalId;

fn last_error(call: &'static str) -> Error {
    Error::syscall(
        call,
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
    )
}

/// Split a flat logical id into the group and bit Windows addresses it by.
fn split(cpu: LogicalId) -> (u16, u64) {
    ((cpu / 64) as u16, 1u64 << (cpu % 64))
}

/// The CPUs this process is permitted to use.
pub fn process_affinity() -> Result<CpuSet> {
    let mut process_mask: usize = 0;
    let mut system_mask: usize = 0;
    // SAFETY: both out-parameters are owned locals.
    let ok =
        unsafe { GetProcessAffinityMask(GetCurrentProcess(), &mut process_mask, &mut system_mask) };
    if ok == 0 {
        return Err(last_error("GetProcessAffinityMask"));
    }

    // The mask describes the process's group only. On a multi-group machine
    // this under-reports, and saying so is better than silently returning a
    // set that quietly excludes half the machine.
    let group = current_group().unwrap_or(0);
    let mut cpus = CpuSet::new();
    for bit in 0..64u32 {
        if process_mask & (1usize << bit) != 0 {
            cpus.insert(group as u32 * 64 + bit);
        }
    }
    Ok(cpus)
}

/// Which processor group the calling thread currently belongs to.
fn current_group() -> Option<u16> {
    let mut affinity = GROUP_AFFINITY {
        Mask: 0,
        Group: 0,
        Reserved: [0; 3],
    };
    // SAFETY: `affinity` is an owned local of the right type.
    let ok = unsafe { GetThreadGroupAffinity(GetCurrentThread(), &mut affinity) };
    (ok != 0).then_some(affinity.Group)
}

/// Confine the calling thread to exactly one CPU.
pub fn pin_current_thread(cpu: LogicalId) -> Result<()> {
    let (group, mask) = split(cpu);
    let affinity = GROUP_AFFINITY {
        Mask: mask as usize,
        Group: group,
        Reserved: [0; 3],
    };
    let mut previous = GROUP_AFFINITY {
        Mask: 0,
        Group: 0,
        Reserved: [0; 3],
    };
    // SAFETY: both structures are owned locals; the handle is a pseudo-handle
    // to the current thread and needs no closing.
    let ok = unsafe { SetThreadGroupAffinity(GetCurrentThread(), &affinity, &mut previous) };
    if ok == 0 {
        return Err(last_error("SetThreadGroupAffinity"));
    }
    Ok(())
}

/// Confine the calling thread to a set of CPUs.
pub fn set_current_thread_affinity(cpus: &CpuSet) -> Result<()> {
    if cpus.is_empty() {
        return Err(Error::invalid(
            "cannot confine a thread to an empty set of CPUs",
        ));
    }
    // A mask names CPUs within one group, so a set spanning groups cannot be
    // expressed. Refusing is better than silently pinning to the first group's
    // half of the request.
    let groups: Vec<u16> = {
        let mut seen: Vec<u16> = cpus.iter().map(|cpu| (cpu / 64) as u16).collect();
        seen.sort_unstable();
        seen.dedup();
        seen
    };
    if groups.len() > 1 {
        return Err(Error::unsupported(format!(
            "an affinity spanning {} processor groups cannot be set as one mask on Windows",
            groups.len()
        )));
    }
    let group = groups[0];
    let mut mask = 0usize;
    for cpu in cpus.iter() {
        mask |= 1usize << (cpu % 64);
    }
    let affinity = GROUP_AFFINITY {
        Mask: mask,
        Group: group,
        Reserved: [0; 3],
    };
    let mut previous = GROUP_AFFINITY {
        Mask: 0,
        Group: 0,
        Reserved: [0; 3],
    };
    // SAFETY: owned locals, pseudo-handle.
    let ok = unsafe { SetThreadGroupAffinity(GetCurrentThread(), &affinity, &mut previous) };
    if ok == 0 {
        return Err(last_error("SetThreadGroupAffinity"));
    }
    Ok(())
}

/// Read the calling thread's affinity.
///
/// See the module note: this is a read implemented as two writes, and it
/// briefly widens the thread's affinity.
pub fn current_thread_affinity() -> Result<CpuSet> {
    let mut affinity = GROUP_AFFINITY {
        Mask: 0,
        Group: 0,
        Reserved: [0; 3],
    };
    // SAFETY: owned local.
    let ok = unsafe { GetThreadGroupAffinity(GetCurrentThread(), &mut affinity) };
    if ok != 0 && affinity.Mask != 0 {
        let mut cpus = CpuSet::new();
        for bit in 0..64u32 {
            if affinity.Mask & (1usize << bit) != 0 {
                cpus.insert(affinity.Group as u32 * 64 + bit);
            }
        }
        return Ok(cpus);
    }

    // Fall back to the set-and-restore trick.
    let permitted = process_affinity()?;
    let mut wide = 0usize;
    for cpu in permitted.iter() {
        wide |= 1usize << (cpu % 64);
    }
    // SAFETY: pseudo-handle; the returned value is the previous mask, and zero
    // means the call failed.
    let previous = unsafe { SetThreadAffinityMask(GetCurrentThread(), wide) };
    if previous == 0 {
        return Err(last_error("SetThreadAffinityMask"));
    }
    // Put it back before doing anything else, including reporting an error.
    // SAFETY: restoring a mask the OS just gave us.
    let restored = unsafe { SetThreadAffinityMask(GetCurrentThread(), previous) };
    if restored == 0 {
        return Err(last_error("SetThreadAffinityMask (restoring)"));
    }

    let group = current_group().unwrap_or(0);
    let mut cpus = CpuSet::new();
    for bit in 0..64u32 {
        if previous & (1usize << bit) != 0 {
            cpus.insert(group as u32 * 64 + bit);
        }
    }
    Ok(cpus)
}

/// The CPU the calling thread is running on right now.
///
/// Advisory: it can be stale before it is returned, because the scheduler is
/// free to move the thread between the read and the use. Reported anyway,
/// because "which CPU am I on" is a real question and a stale answer is more
/// useful than none as long as nobody pretends otherwise.
pub fn current_cpu() -> Option<LogicalId> {
    let mut number = PROCESSOR_NUMBER {
        Group: 0,
        Number: 0,
        Reserved: 0,
    };
    // SAFETY: owned local.
    unsafe {
        windows_sys::Win32::System::Threading::GetCurrentProcessorNumberEx(&mut number);
    }
    Some(number.Group as u32 * 64 + number.Number as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_permitted_some_cpus() {
        let permitted = process_affinity().expect("Windows knows what we may use");
        assert!(!permitted.is_empty());
    }

    #[test]
    fn pinning_this_thread_actually_moves_it() {
        // A live test on the machine running it. If pinning were a no-op the
        // whole benchmark layer would be measuring nothing.
        let permitted = process_affinity().expect("permitted");
        let original = current_thread_affinity().expect("readable");

        for cpu in permitted.iter().take(4) {
            pin_current_thread(cpu).expect("pinnable");
            let now = current_thread_affinity().expect("readable");
            assert_eq!(
                now.to_vec(),
                vec![cpu],
                "after pinning to {cpu} the thread should be confined to it"
            );
            // And it should actually be running there.
            if let Some(running_on) = current_cpu() {
                assert_eq!(
                    running_on, cpu,
                    "pinned to {cpu} but running on {running_on}"
                );
            }
        }

        set_current_thread_affinity(&original).expect("restorable");
    }

    #[test]
    fn an_empty_affinity_is_refused() {
        assert!(set_current_thread_affinity(&CpuSet::new()).is_err());
    }

    #[test]
    fn a_narrowed_affinity_can_be_widened_again() {
        let permitted = process_affinity().expect("permitted");
        let original = current_thread_affinity().expect("readable");
        let first = permitted.iter().next().expect("at least one CPU");

        pin_current_thread(first).expect("pinnable");
        assert_eq!(current_thread_affinity().unwrap().len(), 1);

        set_current_thread_affinity(&permitted).expect("wideable");
        assert!(!current_thread_affinity().unwrap().is_empty());

        set_current_thread_affinity(&original).expect("restorable");
    }

    #[test]
    fn splitting_a_logical_id_addresses_the_right_group() {
        assert_eq!(split(0), (0, 1));
        assert_eq!(split(63), (0, 1 << 63));
        assert_eq!(split(64), (1, 1));
        assert_eq!(split(65), (1, 2));
    }
}
