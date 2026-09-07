//! Per-CPU frequency on Windows, via `CallNtPowerInformation`.
//!
//! # What it reports
//!
//! `ProcessorInformation` returns one `PROCESSOR_POWER_INFORMATION` per logical
//! CPU: maximum MHz, current MHz, a current limit, and the idle state.
//!
//! # What "current MHz" actually means
//!
//! Not the clock rate. It is the frequency the power manager last *asked* for,
//! sampled at an unspecified moment, for a core whose actual clock may have
//! changed several times during the call. On many parts it is quantised to a
//! small number of values and on some it barely moves at all.
//!
//! That is the same caveat the Linux `scaling_cur_freq` carries, and it is
//! recorded here for the same reason: the mirror documents what a channel
//! approximates rather than pretending it is the thing itself.

use windows_sys::Win32::System::Power::{CallNtPowerInformation, ProcessorInformation};

use corescout_core::error::{Error, Result};

use crate::topology::Topology;

/// The layout `CallNtPowerInformation(ProcessorInformation)` fills.
///
/// Declared here because `windows-sys` does not expose it: it is documented in
/// the Windows SDK as `PROCESSOR_POWER_INFORMATION` and is a plain, stable,
/// fixed-size record.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessorPowerInformation {
    pub number: u32,
    pub max_mhz: u32,
    pub current_mhz: u32,
    pub mhz_limit: u32,
    pub max_idle_state: u32,
    pub current_idle_state: u32,
}

/// Read one record per logical CPU.
pub fn read(cpu_count: usize) -> Result<Vec<ProcessorPowerInformation>> {
    if cpu_count == 0 {
        return Ok(Vec::new());
    }
    let mut buffer = vec![ProcessorPowerInformation::default(); cpu_count];
    let bytes = std::mem::size_of_val(buffer.as_slice()) as u32;
    // SAFETY: the buffer is exactly `cpu_count` records of the documented
    // layout, and `bytes` is its true size. The call writes only into it.
    let status = unsafe {
        CallNtPowerInformation(
            ProcessorInformation,
            std::ptr::null(),
            0,
            buffer.as_mut_ptr().cast(),
            bytes,
        )
    };
    if status != 0 {
        return Err(Error::syscall(
            "CallNtPowerInformation(ProcessorInformation)",
            status,
        ));
    }
    Ok(buffer)
}

/// Fill a topology's per-CPU frequency fields.
///
/// Failure is not fatal. A machine that will not report its clocks still has a
/// topology, and the fields stay `None` so that a consumer can tell "not
/// measured" from "measured as zero".
pub fn fill(topology: &mut Topology) {
    let Ok(records) = read(topology.logical_cpus.len()) else {
        return;
    };
    for (cpu, record) in topology.logical_cpus.iter_mut().zip(records.iter()) {
        if record.max_mhz > 0 {
            cpu.frequency.max_khz = Some(record.max_mhz as u64 * 1000);
            // Windows calls this the maximum; on a part with turbo it is the
            // turbo ceiling rather than the base clock, so it is not recorded
            // as `base_khz`, which would be a different claim.
        }
        if record.current_mhz > 0 {
            cpu.frequency.current_khz = Some(record.current_mhz as u64 * 1000);
        }
        // No minimum is reported. Left absent rather than assumed to be zero.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_machine_reports_its_clocks() {
        // Live. If this fails the machine is not reporting power information,
        // which is a real finding rather than a broken test.
        let topology = super::super::topology::discover().expect("discoverable");
        let records = read(topology.logical_cpus.len()).expect("power information");
        assert_eq!(records.len(), topology.logical_cpus.len());
        assert!(
            records.iter().any(|r| r.max_mhz > 0),
            "no CPU reported a maximum frequency"
        );
        for record in &records {
            if record.max_mhz > 0 {
                // A plausible range for anything this code will run on.
                assert!(
                    (100..100_000).contains(&record.max_mhz),
                    "implausible max {} MHz",
                    record.max_mhz
                );
            }
        }
    }

    #[test]
    fn filling_a_topology_populates_frequencies() {
        let mut topology = super::super::topology::discover().expect("discoverable");
        for cpu in &mut topology.logical_cpus {
            cpu.frequency = Default::default();
        }
        fill(&mut topology);
        assert!(
            topology
                .logical_cpus
                .iter()
                .any(|cpu| cpu.frequency.max_khz.is_some()),
            "fill did not populate any maximum frequency"
        );
    }

    #[test]
    fn asking_for_no_cpus_is_not_an_error() {
        assert!(read(0).expect("empty is fine").is_empty());
    }
}
