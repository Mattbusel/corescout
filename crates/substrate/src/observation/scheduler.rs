//! Scheduler occupancy.
//!
//! | property | value |
//! |---|---|
//! | physical fact | how each CPU's time has been apportioned, how long runnable tasks waited for it, and how many tasks are runnable now |
//! | source | `/proc/stat`, `/proc/schedstat`, `/proc/loadavg` |
//! | sample rate | ~100 Hz; `/proc/stat` is quantised to `USER_HZ`, usually 10 ms |
//! | cost | three file reads, each generated on demand across all CPUs |
//! | perturbation | **Negligible**: the kernel maintains these counters regardless; generating the files costs the reading CPU only |
//! | uncertainty | `/proc/stat` is sampled by tick, so it misattributes work shorter than one tick |
//!
//! # Why the load average is deliberately absent
//!
//! `/proc/loadavg`'s first three numbers are exponentially weighted moving
//! averages over 1, 5 and 15 minutes. They are historical summaries, and by the
//! rule that keeps memory out of the mirror they cannot be published here. A
//! consumer wanting them can compute them from a series of snapshots, and will
//! then know exactly what window it used.
//!
//! The fourth field is a different kind of thing: `runnable/total` is an
//! instantaneous count of tasks in the runqueues right now. That *is* present
//! state, and it is published.
//!
//! Keeping these two apart while parsing the same line is a small thing that
//! makes the principle concrete: the test is not where a number came from, it is
//! whether the number is a fact about the present.
//!
//! # `/proc/schedstat` and its absence
//!
//! Per-CPU run and wait time come from `/proc/schedstat`, which requires
//! `CONFIG_SCHEDSTATS`. Several distributions ship it disabled because the
//! accounting costs a little on every context switch. Where it is missing, those
//! columns stay unobserved and the rest of the sensor still works. Wait time is
//! the single most informative scheduler quantity for a latency-sensitive
//! consumer, so its absence is worth noticing.

use crate::observation::source;
use crate::observation::{
    BindContext, Perturbation, Sensor, SensorDescriptor, SensorId, SensorOutcome, StateWriter,
    Uncertainty,
};
use corescout_core::error::{Error, Result};
use corescout_mirror::entity::keys;
use corescout_mirror::state::{ChannelId, Semantics, Unit};
use std::path::PathBuf;

/// The `/proc/stat` CPU time fields, in file order.
const TIME_FIELDS: [&str; 8] = [
    "user", "nice", "system", "idle", "iowait", "irq", "softirq", "steal",
];

pub struct SchedulerSensor {
    stat_path: PathBuf,
    schedstat_path: PathBuf,
    loadavg_path: PathBuf,
    /// CPU number to entity row.
    cpu_rows: Vec<(u32, u32)>,
    machine_row: Option<u32>,
    /// Nanoseconds per `USER_HZ` tick.
    ns_per_tick: f64,
    time_channels: Vec<ChannelId>,
    channel_run: Option<ChannelId>,
    channel_wait: Option<ChannelId>,
    channel_slices: Option<ChannelId>,
    channel_runnable: Option<ChannelId>,
    channel_tasks: Option<ChannelId>,
}

impl SchedulerSensor {
    pub fn new() -> SchedulerSensor {
        SchedulerSensor {
            stat_path: PathBuf::new(),
            schedstat_path: PathBuf::new(),
            loadavg_path: PathBuf::new(),
            cpu_rows: Vec::new(),
            machine_row: None,
            ns_per_tick: 0.0,
            time_channels: Vec::new(),
            channel_run: None,
            channel_wait: None,
            channel_slices: None,
            channel_runnable: None,
            channel_tasks: None,
        }
    }

    fn row_of_cpu(&self, cpu: u32) -> Option<u32> {
        self.cpu_rows
            .iter()
            .find(|(c, _)| *c == cpu)
            .map(|(_, r)| *r)
    }
}

impl Default for SchedulerSensor {
    fn default() -> Self {
        Self::new()
    }
}

impl Sensor for SchedulerSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: SensorId(5),
            key: "scheduler",
            physical_fact: "cumulative CPU time by class, runqueue wait time, and the number \
                            of tasks runnable right now",
            source: "/proc/stat, /proc/schedstat, /proc/loadavg",
            max_rate_hz: 100.0,
            perturbation: Perturbation::Negligible,
            uncertainty: Uncertainty::absolute(
                1.0,
                "/proc/stat is accounted per timer tick, so it attributes a whole tick to \
                 whatever was running when the tick fired; sub-tick work is misattributed",
            ),
            requires_privilege: false,
        }
    }

    fn bind(&mut self, ctx: &mut BindContext<'_>) -> Result<()> {
        let proc_root = ctx.substrate().roots.proc.clone();
        self.stat_path = proc_root.join("stat");
        self.schedstat_path = proc_root.join("schedstat");
        self.loadavg_path = proc_root.join("loadavg");

        if source::string(&self.stat_path).is_none() {
            return Err(Error::unsupported("/proc/stat is not readable"));
        }

        self.ns_per_tick = 1_000_000_000.0 / source::clock_ticks_per_second() as f64;
        self.machine_row = ctx.row_of("machine");
        for cpu in ctx
            .substrate()
            .topology
            .logical_cpus
            .iter()
            .filter(|c| c.online)
        {
            if let Some(row) = ctx.row_of(&keys::logical_cpu(cpu.id)) {
                self.cpu_rows.push((cpu.id, row));
            }
        }

        for field in TIME_FIELDS {
            self.time_channels.push(ctx.declare_channel(
                format!("cpu.time.{field}"),
                Unit::Nanosecond,
                Semantics::Cumulative,
            ));
        }
        self.channel_run = Some(ctx.declare_channel(
            "cpu.sched.run_time",
            Unit::Nanosecond,
            Semantics::Cumulative,
        ));
        self.channel_wait = Some(ctx.declare_channel(
            "cpu.sched.wait_time",
            Unit::Nanosecond,
            Semantics::Cumulative,
        ));
        self.channel_slices =
            Some(ctx.declare_channel("cpu.sched.timeslices", Unit::Count, Semantics::Cumulative));
        self.channel_runnable =
            Some(ctx.declare_channel("machine.tasks.runnable", Unit::Count, Semantics::Instant));
        self.channel_tasks =
            Some(ctx.declare_channel("machine.tasks.total", Unit::Count, Semantics::Instant));
        Ok(())
    }

    fn observe(&mut self, out: &mut StateWriter<'_>) -> SensorOutcome {
        let mut outcome = SensorOutcome::default();

        match source::string(&self.stat_path) {
            Some(text) => {
                for (cpu, times) in parse_proc_stat(&text) {
                    let Some(row) = self.row_of_cpu(cpu) else {
                        continue;
                    };
                    for (index, ticks) in times.iter().enumerate() {
                        // Ticks are converted to nanoseconds here, once, using
                        // the kernel's own USER_HZ. A consumer should never have
                        // to know what a jiffy is.
                        source::emit(
                            out,
                            &mut outcome,
                            row,
                            self.time_channels.get(index).copied(),
                            *ticks as f64 * self.ns_per_tick,
                        );
                    }
                }
            }
            None => outcome.error(),
        }

        // Optional: absent without CONFIG_SCHEDSTATS.
        if let Some(text) = source::string(&self.schedstat_path) {
            for (cpu, run_ns, wait_ns, slices) in parse_schedstat(&text) {
                let Some(row) = self.row_of_cpu(cpu) else {
                    continue;
                };
                source::emit(out, &mut outcome, row, self.channel_run, run_ns as f64);
                source::emit(out, &mut outcome, row, self.channel_wait, wait_ns as f64);
                source::emit(out, &mut outcome, row, self.channel_slices, slices as f64);
            }
        }

        if let (Some(machine), Some(text)) = (self.machine_row, source::string(&self.loadavg_path))
        {
            if let Some((runnable, total)) = parse_loadavg_tasks(&text) {
                source::emit(
                    out,
                    &mut outcome,
                    machine,
                    self.channel_runnable,
                    runnable as f64,
                );
                source::emit(out, &mut outcome, machine, self.channel_tasks, total as f64);
            }
        }

        outcome
    }
}

/// Parse the per-CPU lines of `/proc/stat` into `(cpu, [ticks; 8])`.
///
/// The aggregate `cpu` line is skipped: it is the sum of the others, so
/// publishing it would be publishing a derived quantity.
fn parse_proc_stat(text: &str) -> Vec<(u32, [u64; 8])> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("cpu") else {
            continue;
        };
        // The aggregate line is `cpu  ...` with no number attached. Splitting
        // on whitespace first would read its *first time field* as a CPU
        // number, inventing a CPU 100 that nothing else in the machine knows
        // about.
        if !rest.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let mut fields = rest.split_whitespace();
        let Some(index) = fields.next() else { continue };
        let Ok(cpu) = index.parse::<u32>() else {
            continue;
        };
        let mut times = [0u64; 8];
        for slot in times.iter_mut() {
            *slot = fields.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        }
        out.push((cpu, times));
    }
    out
}

/// Parse the per-CPU lines of `/proc/schedstat` into
/// `(cpu, run_ns, wait_ns, timeslices)`.
///
/// The last three fields of a `cpuN` line are `rq_cpu_time`, `run_delay` and
/// `pcount`. Taking them from the end rather than by fixed position makes this
/// robust across schedstat versions, which have added fields to the front.
fn parse_schedstat(text: &str) -> Vec<(u32, u64, u64, u64)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("cpu") else {
            continue;
        };
        if !rest.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let mut fields = rest.split_whitespace();
        let Some(index) = fields.next() else { continue };
        let Ok(cpu) = index.parse::<u32>() else {
            continue;
        };
        let values: Vec<u64> = fields.filter_map(|v| v.parse().ok()).collect();
        if values.len() < 3 {
            continue;
        }
        let tail = &values[values.len() - 3..];
        out.push((cpu, tail[0], tail[1], tail[2]));
    }
    out
}

/// Extract `runnable/total` from `/proc/loadavg`.
///
/// The three load averages on the same line are deliberately not returned; see
/// the module documentation.
fn parse_loadavg_tasks(text: &str) -> Option<(u64, u64)> {
    let field = text.split_whitespace().nth(3)?;
    let (runnable, total) = field.split_once('/')?;
    Some((runnable.parse().ok()?, total.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROC_STAT: &str = "\
cpu  100 200 300 400 500 600 700 800 0 0
cpu0 1 2 3 4 5 6 7 8 0 0
cpu1 10 20 30 40 50 60 70 80 0 0
intr 12345 1 2 3
ctxt 987654
btime 1700000000
processes 4242
procs_running 3
procs_blocked 0
";

    const SCHEDSTAT: &str = "\
version 15
timestamp 4294900000
cpu0 0 0 0 0 0 0 123456789 987654321 4242
domain0 00000000,00000003 0 0 0
cpu1 0 0 0 0 0 0 111 222 333
";

    #[test]
    fn proc_stat_yields_per_cpu_times_and_skips_the_aggregate() {
        let parsed = parse_proc_stat(PROC_STAT);
        assert_eq!(parsed.len(), 2, "the aggregate `cpu` line must be skipped");
        assert!(
            !parsed.iter().any(|(cpu, _)| *cpu == 100),
            "the aggregate first time field must not be read as a CPU number"
        );
        assert_eq!(parsed[0].0, 0);
        assert_eq!(parsed[0].1, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(parsed[1].0, 1);
        assert_eq!(parsed[1].1[3], 40, "idle time");
    }

    #[test]
    fn proc_stat_tolerates_a_short_line() {
        // Older kernels omit the guest fields; a truncated line must not
        // produce garbage.
        let parsed = parse_proc_stat("cpu0 1 2 3 4\n");
        assert_eq!(parsed[0].1, [1, 2, 3, 4, 0, 0, 0, 0]);
    }

    #[test]
    fn schedstat_takes_the_last_three_fields() {
        let parsed = parse_schedstat(SCHEDSTAT);
        assert_eq!(parsed.len(), 2, "domain lines must be ignored");
        assert_eq!(parsed[0], (0, 123_456_789, 987_654_321, 4242));
        assert_eq!(parsed[1], (1, 111, 222, 333));
    }

    #[test]
    fn schedstat_survives_a_version_with_extra_leading_fields() {
        // The reason fields are taken from the end.
        let text = "cpu0 9 9 9 9 9 9 9 9 9 7 8 9\n";
        assert_eq!(parse_schedstat(text), vec![(0, 7, 8, 9)]);
    }

    #[test]
    fn loadavg_gives_the_instantaneous_counts_only() {
        let parsed = parse_loadavg_tasks("0.52 0.61 0.70 3/1234 5678\n");
        assert_eq!(parsed, Some((3, 1234)));
    }

    #[test]
    fn loadavg_averages_are_not_extracted() {
        // Asserting an absence, because it is a design rule rather than an
        // oversight: those three numbers are memory, not mirror.
        let text = "0.52 0.61 0.70 3/1234 5678\n";
        let parsed = parse_loadavg_tasks(text).unwrap();
        assert_ne!(parsed.0, 0, "runnable count is present");
        assert!(
            !TIME_FIELDS.iter().any(|f| f.contains("load")),
            "no channel should carry a load average"
        );
    }

    #[test]
    fn malformed_input_yields_nothing_rather_than_panicking() {
        assert!(parse_proc_stat("").is_empty());
        assert!(parse_schedstat("cpu\n").is_empty());
        assert_eq!(parse_loadavg_tasks("garbage"), None);
        assert_eq!(parse_loadavg_tasks("1 2 3 notafraction 5"), None);
    }
}
