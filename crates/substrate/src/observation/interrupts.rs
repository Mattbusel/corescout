//! Interrupt activity per CPU.
//!
//! | property | value |
//! |---|---|
//! | physical fact | how many hardware interrupts each CPU has serviced since boot |
//! | source | `/proc/interrupts` |
//! | sample rate | ~50 Hz, and the cost argues for less |
//! | cost | **the most expensive sensor here**: the kernel formats one row per IRQ source with one column per CPU |
//! | perturbation | **Negligible** to the observed CPUs; the cost lands entirely on the reader |
//! | uncertainty | exact counts, but per-CPU attribution follows IRQ affinity which can change between samples |
//!
//! # Why this is worth its cost
//!
//! Interrupt load is the single most under-appreciated source of tail latency,
//! and it is wildly unevenly distributed. CPU 0 is the default target for much
//! of the kernel's work, and network queues are pinned to particular CPUs by IRQ
//! affinity. A core carrying a busy NIC queue can look perfectly healthy on
//! every throughput measurement and still miss a deadline several times a
//! second.
//!
//! The original CoreScout inferred this indirectly, by noticing that a core's
//! benchmark samples were disturbed more often than its neighbours'. The mirror
//! observes it directly, and without perturbing anything.
//!
//! # The cost is real and is measured
//!
//! `/proc/interrupts` is generated on every read, and its size is
//! `O(irq_sources x cpus)`. On a 128-CPU server with a few hundred IRQ sources
//! that is a few hundred kilobytes of text formatted by the kernel, on demand,
//! for one caller. This is exactly the work the self-state plane exists to do
//! once instead of once per consumer, and exactly why the mirror publishes what
//! each observation cost.
//!
//! # What is published, and what is not
//!
//! Per-CPU totals, not per-source rows. A per-source breakdown would multiply
//! the entity count by the number of IRQ sources for information that only
//! matters when a consumer is already investigating one specific device. The
//! `InterruptSource` entity class exists for when that changes.

use crate::observation::source;
use crate::observation::{
    BindContext, Perturbation, Sensor, SensorDescriptor, SensorId, SensorOutcome, StateWriter,
    Uncertainty,
};
use corescout_core::error::{Error, Result};
use corescout_mirror::entity::keys;
use corescout_mirror::state::{ChannelId, Semantics, Unit};
use std::path::PathBuf;

pub struct InterruptSensor {
    path: PathBuf,
    /// Column position in `/proc/interrupts` to entity row.
    columns: Vec<(usize, u32)>,
    channel_total: Option<ChannelId>,
    channel_sources: Option<ChannelId>,
}

impl InterruptSensor {
    pub fn new() -> InterruptSensor {
        InterruptSensor {
            path: PathBuf::new(),
            columns: Vec::new(),
            channel_total: None,
            channel_sources: None,
        }
    }
}

impl Default for InterruptSensor {
    fn default() -> Self {
        Self::new()
    }
}

impl Sensor for InterruptSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: SensorId(6),
            key: "interrupts",
            physical_fact: "the number of hardware interrupts each CPU has serviced since boot",
            source: "/proc/interrupts",
            // Deliberately lower than the other sensors: the file is expensive
            // to generate and the counters do not need fine sampling to be
            // useful.
            max_rate_hz: 50.0,
            perturbation: Perturbation::Negligible,
            uncertainty: Uncertainty::unknown(
                "counts are exact; which CPU a future interrupt lands on depends on IRQ \
                 affinity and on irqbalance, either of which may change between samples",
            ),
            requires_privilege: false,
        }
    }

    fn bind(&mut self, ctx: &mut BindContext<'_>) -> Result<()> {
        self.path = ctx.substrate().roots.proc.join("interrupts");
        let Some(text) = source::string(&self.path) else {
            return Err(Error::unsupported("/proc/interrupts is not readable"));
        };

        // The header names the CPUs, in column order. Binding that mapping once
        // means the hot path never has to parse the header again, and it means a
        // machine whose CPU columns are sparse (after offlining) is handled
        // correctly rather than by assuming column N is CPU N.
        let cpus = parse_header(&text);
        if cpus.is_empty() {
            return Err(Error::unsupported(
                "/proc/interrupts has no per-CPU columns",
            ));
        }
        for (column, cpu) in cpus.iter().enumerate() {
            if let Some(row) = ctx.row_of(&keys::logical_cpu(*cpu)) {
                self.columns.push((column, row));
            }
        }

        self.channel_total =
            Some(ctx.declare_channel("cpu.interrupts.total", Unit::Count, Semantics::Cumulative));
        self.channel_sources = Some(ctx.declare_channel(
            "cpu.interrupts.active_sources",
            Unit::Count,
            Semantics::Instant,
        ));
        Ok(())
    }

    fn observe(&mut self, out: &mut StateWriter<'_>) -> SensorOutcome {
        let mut outcome = SensorOutcome::default();
        let Some(text) = source::string(&self.path) else {
            outcome.error();
            return outcome;
        };

        let width = self.columns.iter().map(|(c, _)| *c + 1).max().unwrap_or(0);
        let (totals, sources) = sum_columns(&text, width);

        for (column, row) in &self.columns {
            source::emit(
                out,
                &mut outcome,
                *row,
                self.channel_total,
                totals[*column] as f64,
            );
            source::emit(
                out,
                &mut outcome,
                *row,
                self.channel_sources,
                sources[*column] as f64,
            );
        }
        outcome
    }
}

/// Parse the `CPU0 CPU1 ...` header into the CPU numbers, in column order.
fn parse_header(text: &str) -> Vec<u32> {
    let Some(header) = text.lines().next() else {
        return Vec::new();
    };
    header
        .split_whitespace()
        .filter_map(|token| token.strip_prefix("CPU")?.parse::<u32>().ok())
        .collect()
}

/// Sum every IRQ row per column, and count how many sources have fired at all.
///
/// Returns `(totals, active_sources)`, both indexed by column.
fn sum_columns(text: &str, width: usize) -> (Vec<u64>, Vec<u32>) {
    let mut totals = vec![0u64; width];
    let mut sources = vec![0u32; width];

    for line in text.lines().skip(1) {
        // Rows are `LABEL:  n n n ...  description`. The label may be a number
        // (a hardware IRQ) or letters (NMI, LOC, TLB, ...); both count.
        let Some((_, rest)) = line.split_once(':') else {
            continue;
        };
        for (column, token) in rest.split_whitespace().take(width).enumerate() {
            // The trailing description columns are not numbers, and `take`
            // alone does not exclude them on a machine with few CPUs, so a
            // failed parse ends the row.
            let Ok(count) = token.parse::<u64>() else {
                break;
            };
            totals[column] = totals[column].saturating_add(count);
            if count > 0 {
                sources[column] += 1;
            }
        }
    }
    (totals, sources)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INTERRUPTS: &str = "\
           CPU0       CPU1       CPU2       CPU3
  0:         31          0          0          0   IO-APIC   2-edge      timer
  8:          1          0          0          0   IO-APIC   8-edge      rtc0
  9:          0          0          0          0   IO-APIC   9-fasteoi   acpi
 16:       1523          0          0          0   IO-APIC  16-fasteoi   ehci_hcd:usb1
124:          0      88192          0          0   PCI-MSI 524288-edge   eth0-rx-0
NMI:          2          2          2          2   Non-maskable interrupts
LOC:    9812345    8712345    7612345    6512345   Local timer interrupts
TLB:        104        233        311        498   TLB shootdowns
";

    #[test]
    fn the_header_names_the_cpu_columns() {
        assert_eq!(parse_header(INTERRUPTS), vec![0, 1, 2, 3]);
    }

    #[test]
    fn a_sparse_cpu_set_is_read_from_the_header_not_assumed() {
        // After offlining, the columns are the online CPUs and their numbers
        // are not contiguous. Assuming column N is CPU N would silently
        // attribute one CPU's interrupts to another.
        let text = "  CPU0  CPU3\n  0:  5  7  IO-APIC  timer\n";
        assert_eq!(parse_header(text), vec![0, 3]);
    }

    #[test]
    fn totals_are_summed_across_every_source() {
        let (totals, _) = sum_columns(INTERRUPTS, 4);
        // CPU0: 31 + 1 + 0 + 1523 + 0 + 2 + 9812345 + 104
        assert_eq!(totals[0], 31 + 1 + 1523 + 2 + 9_812_345 + 104);
        // CPU1 carries the NIC queue: the asymmetry this sensor exists to show.
        assert_eq!(totals[1], 88_192 + 2 + 8_712_345 + 233);
        assert!(totals[1] < totals[0]);
    }

    #[test]
    fn active_sources_counts_only_sources_that_have_fired() {
        let (_, sources) = sum_columns(INTERRUPTS, 4);
        // CPU0: timer, rtc0, ehci, NMI, LOC, TLB. Not acpi and not eth0-rx-0.
        assert_eq!(sources[0], 6);
        // CPU2: NMI, LOC, TLB only.
        assert_eq!(sources[2], 3);
    }

    #[test]
    fn trailing_description_columns_are_not_counted_as_interrupts() {
        // With few CPUs, the description text sits within `width` columns and
        // would otherwise be parsed as counts.
        let text = "  CPU0\n  0:  31  IO-APIC  2-edge  timer\n";
        let (totals, _) = sum_columns(text, 1);
        assert_eq!(totals[0], 31);
    }

    #[test]
    fn malformed_input_is_survivable() {
        assert!(parse_header("").is_empty());
        let (totals, sources) = sum_columns("garbage with no colon\n", 2);
        assert_eq!(totals, vec![0, 0]);
        assert_eq!(sources, vec![0, 0]);
    }

    #[test]
    fn a_row_shorter_than_the_cpu_count_does_not_panic() {
        // Some rows, such as ERR and MIS, carry a single total rather than a
        // per-CPU breakdown.
        let text = "  CPU0  CPU1\n  0:  5  7  IO-APIC timer\nERR:  3\n";
        let (totals, _) = sum_columns(text, 2);
        assert_eq!(totals[0], 8);
        assert_eq!(totals[1], 7);
    }
}
