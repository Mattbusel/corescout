//! Launching a process already confined to a set of CPUs.
//!
//! # Why this is separate from the affinity actuator
//!
//! Setting affinity after `spawn` leaves the program's startup, its allocator
//! arenas and any threads it creates early on the wrong CPUs. The mask has to
//! be in place between `fork` and `execve`, which is a different mechanism from
//! changing a running process, and a different kind of decision: this is where
//! a workload *enters* the system rather than being moved within it.
//!
//! It is also the one actuator that does not need scope, because the process
//! did not exist until it was created here. Creating a process you then control
//! is the strongest possible form of opt-in.

use std::process::{Child, Command};

use corescout_core::{CpuSet, Error, Result};

/// Spawn a child confined to `cpus`.
pub fn spawn_with_affinity(command: &mut Command, cpus: &CpuSet) -> Result<Child> {
    if cpus.is_empty() {
        return Err(Error::invalid(
            "cannot launch a process with an empty CPU set",
        ));
    }
    #[cfg(target_os = "linux")]
    {
        linux_spawn(command, cpus)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = command;
        Err(Error::unsupported(
            "launching with an affinity mask (implemented for Linux only)",
        ))
    }
}

#[cfg(target_os = "linux")]
fn linux_spawn(command: &mut Command, cpus: &CpuSet) -> Result<Child> {
    use std::os::unix::process::CommandExt;

    let max = cpus.max().expect("non-empty");
    let words = (max as usize / 64) + 1;
    let mut mask = vec![0u64; words];
    for cpu in cpus.iter() {
        mask[cpu as usize / 64] |= 1u64 << (cpu % 64);
    }
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

/// Wait for a child and return the code a shell would report.
pub fn wait(child: &mut Child, program: &str) -> Result<i32> {
    let status = child.wait().map_err(|e| Error::io(program, e))?;
    Ok(exit_code(&status))
}

/// The exit code to propagate for a finished child.
///
/// A process killed by a signal has no exit code of its own; `128 + signal` is
/// the shell convention, and is what the script this was dropped into expects.
pub fn exit_code(status: &std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal().map(|s| 128 + s).unwrap_or(1)
    }
    #[cfg(not(unix))]
    {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_mask_is_refused_before_forking() {
        let mut command = Command::new("/bin/true");
        let error = spawn_with_affinity(&mut command, &CpuSet::new()).unwrap_err();
        assert!(error.to_string().contains("empty CPU set"));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn a_child_inherits_the_mask_it_was_given() {
        let cpu = crate::permitted_cpus()
            .iter()
            .next()
            .expect("at least one CPU");
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("grep Cpus_allowed_list /proc/self/status")
            .stdout(std::process::Stdio::piped());

        let child = spawn_with_affinity(&mut command, &[cpu].into_iter().collect()).expect("spawn");
        let output = child.wait_with_output().expect("wait");
        let text = String::from_utf8_lossy(&output.stdout);
        let reported = text.split(':').nth(1).expect("Cpus_allowed_list").trim();
        assert_eq!(reported, cpu.to_string());
    }
}
