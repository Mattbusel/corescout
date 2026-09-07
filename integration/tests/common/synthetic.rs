//! A synthetic machine that produces a stream of mirror reflections.
//!
//! # What this is for, and what it is not evidence of
//!
//! The observer experiment needs a machine whose true structure is known, so
//! that "the observer discovered the SMT pairs" can be checked rather than
//! admired. On real hardware the ground truth is known too, but the experiment
//! cannot be run anywhere except on Linux, and cannot be run deterministically
//! at all.
//!
//! **The honest caveat, stated up front:** this simulator was written by the
//! same person as the observer, and it makes SMT siblings covary *by
//! construction*. Rediscovering that is therefore not evidence about real CPUs.
//! It is evidence about three other things, all of which are worth having:
//!
//! 1. The discovery pipeline works, and finds structure that is present.
//! 2. It does **not** find structure that is absent, which the frequency-domain
//!    edges below exist to check.
//! 3. The mirror's representation is *capable of carrying* this kind of signal
//!    from producer to consumer without loss.
//!
//! The real experiment needs an hour of `corescout mirror --record` on real
//! hardware. This is the harness for it, and the thing that will be run first
//! to check the harness is not broken.
//!
//! # The physics
//!
//! Deliberately crude, and shaped after the real couplings:
//!
//! - Each physical core has a latent activity level following an AR(1) walk.
//! - **Both SMT threads of a core are driven by that one activity**, plus small
//!   independent noise. This is the structure to be rediscovered.
//! - Frequency rises with activity and falls with temperature.
//! - Temperature follows activity through a first-order thermal lag.
//! - Busy time, idle time, cycles and interrupts accumulate.
//! - CPU 0 receives an order of magnitude more interrupts, as it does in life.
//! - The whole machine alternates between a quiet and a busy phase, so there
//!   are recurring whole-machine states to find.

#![allow(dead_code)]

use corescout_mirror::entity::keys;
use corescout_mirror::relation::{Relation, RelationKind};
use corescout_mirror::schema::{Availability, AvailabilityMatrix};
use corescout_mirror::state::{ChannelId, ChannelSpec, Semantics, StateMatrix, Unit};
use corescout_mirror::{Entity, MirrorSnapshot, FORMAT_VERSION};
use corescout_substrate::discovery::{Roots, Substrate};
use corescout_substrate::observation::{Perturbation, SensorId, SensorReport};
use corescout_substrate::platform::linux::sysfs::Sysfs;

use super::{FakeCpu, FakeMachine, TempTree};

/// What is actually true about the synthetic machine.
///
/// The observer never sees any of this.
#[derive(Debug, Clone)]
pub struct GroundTruth {
    /// Entity rows of the logical CPUs, in CPU order.
    pub cpu_rows: Vec<u32>,
    /// Entity row pairs that are SMT siblings of one physical core.
    pub smt_pairs: Vec<(u32, u32)>,
    /// Columns that are running totals.
    pub accumulator_columns: Vec<usize>,
    /// Column holding frequency.
    pub frequency_column: usize,
    /// Column holding temperature.
    pub temperature_column: usize,
    /// The CPU that carries the interrupt load.
    pub interrupt_heavy_row: u32,
    /// Edge type joining SMT siblings: behaviourally real.
    pub real_edge_kind: u16,
    /// Edge type joining every CPU to every other: behaviourally empty.
    pub vacuous_edge_kind: u16,
}

/// A machine that can be asked for its next reflection.
pub struct SyntheticMachine {
    _tree: TempTree,
    entities: Vec<Entity>,
    relations: Vec<Relation>,
    channels: Vec<ChannelSpec>,
    /// `(cpu index, entity row, physical core index)`.
    cpus: Vec<(usize, u32, usize)>,
    /// Latent activity per physical core.
    activity: Vec<f64>,
    /// Temperature per physical core.
    temperature: Vec<f64>,
    /// Accumulated totals, indexed by `[cpu][channel]`.
    totals: Vec<Vec<f64>>,
    rng: Rng,
    step: u64,
    truth: GroundTruth,
}

const CORES: usize = 4;
const THREADS_PER_CORE: usize = 2;

const COL_FREQUENCY: usize = 0;
const COL_TEMPERATURE: usize = 1;
const COL_BUSY: usize = 2;
const COL_IDLE: usize = 3;
const COL_INTERRUPTS: usize = 4;
const COL_CYCLES: usize = 5;
const COLUMNS: usize = 6;

/// Reflections per whole-machine phase. The machine alternates between a quiet
/// and a busy regime, so there is recurring structure at the machine level as
/// well as at the core level.
const PHASE_LENGTH: u64 = 60;

impl SyntheticMachine {
    pub fn new(seed: u64) -> SyntheticMachine {
        let machine = uniform_smt_machine();
        let tree = machine.write("synthetic");
        let topology = Sysfs::with_roots(tree.sys(), tree.proc())
            .read_topology(None)
            .expect("synthetic topology");
        let substrate = Substrate::new(topology, Roots::new(tree.sys(), tree.proc()));
        let (entities, mut relations) = substrate.structure();

        let mut cpus = Vec::new();
        for cpu in 0..(CORES * THREADS_PER_CORE) {
            let row = entities
                .iter()
                .position(|e| e.key == keys::logical_cpu(cpu as u32))
                .expect("cpu entity") as u32;
            // Layout: CPU n and CPU n + CORES are the two threads of core n.
            cpus.push((cpu, row, cpu % CORES));
        }

        // A frequency domain covering every CPU, exactly as the frequency
        // sensor emits for a package-wide cpufreq policy. It is a real edge
        // that carries no behavioural information here, and it is the control
        // in the experiment: an observer that reports lift for this has found
        // structure that is not there.
        for (_, a, _) in &cpus {
            for (_, b, _) in &cpus {
                if a != b {
                    relations.push(Relation::new(*a, *b, RelationKind::FrequencyDomain));
                }
            }
        }

        let channels = vec![
            channel(
                COL_FREQUENCY,
                "cpu.frequency.current",
                Unit::Kilohertz,
                Semantics::Instant,
            ),
            channel(
                COL_TEMPERATURE,
                "thermal.temperature",
                Unit::Celsius,
                Semantics::Instant,
            ),
            channel(
                COL_BUSY,
                "cpu.time.busy",
                Unit::Nanosecond,
                Semantics::Cumulative,
            ),
            channel(
                COL_IDLE,
                "cpu.time.idle",
                Unit::Nanosecond,
                Semantics::Cumulative,
            ),
            channel(
                COL_INTERRUPTS,
                "cpu.interrupts.total",
                Unit::Count,
                Semantics::Cumulative,
            ),
            channel(
                COL_CYCLES,
                "cpu.pmu.cycles",
                Unit::Count,
                Semantics::Cumulative,
            ),
        ];

        let mut smt_pairs = Vec::new();
        for core in 0..CORES {
            let a = cpus.iter().find(|(c, _, _)| *c == core).unwrap().1;
            let b = cpus.iter().find(|(c, _, _)| *c == core + CORES).unwrap().1;
            smt_pairs.push((a.min(b), a.max(b)));
        }

        let truth = GroundTruth {
            cpu_rows: cpus.iter().map(|(_, row, _)| *row).collect(),
            smt_pairs,
            accumulator_columns: vec![COL_BUSY, COL_IDLE, COL_INTERRUPTS, COL_CYCLES],
            frequency_column: COL_FREQUENCY,
            temperature_column: COL_TEMPERATURE,
            interrupt_heavy_row: cpus[0].1,
            real_edge_kind: RelationKind::SmtSibling.as_u16(),
            vacuous_edge_kind: RelationKind::FrequencyDomain.as_u16(),
        };

        SyntheticMachine {
            _tree: tree,
            entities,
            relations,
            channels,
            cpus,
            activity: vec![0.3; CORES],
            temperature: vec![45.0; CORES],
            totals: vec![vec![0.0; COLUMNS]; CORES * THREADS_PER_CORE],
            rng: Rng::new(seed),
            step: 0,
            truth,
        }
    }

    pub fn truth(&self) -> &GroundTruth {
        &self.truth
    }

    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// Advance the machine and produce its next reflection.
    pub fn tick(&mut self) -> MirrorSnapshot {
        self.step += 1;
        // 100 ms per step, matching the mirror's default cadence.
        let dt_ns = 100_000_000.0;

        // The machine as a whole alternates between quiet and busy.
        let busy_phase = (self.step / PHASE_LENGTH) % 2 == 1;
        let target = if busy_phase { 0.75 } else { 0.15 };

        for core in 0..CORES {
            // AR(1) walk towards the phase target: momentum, which is what
            // makes the series predictable at all.
            let noise = (self.rng.next_unit() - 0.5) * 0.12;
            self.activity[core] =
                (0.82 * self.activity[core] + 0.18 * target + noise).clamp(0.0, 1.0);

            // First-order thermal lag driven by activity.
            let equilibrium = 40.0 + 35.0 * self.activity[core];
            self.temperature[core] += 0.12 * (equilibrium - self.temperature[core]);
        }

        for (cpu, _, core) in self.cpus.clone() {
            let activity = self.activity[core];
            let temperature = self.temperature[core];

            // Small per-thread independent noise: siblings are strongly
            // coupled, not identical. A correlation of exactly 1.0 would be a
            // simulator artefact rather than a physical claim.
            let jitter = (self.rng.next_unit() - 0.5) * 0.04;
            let local = (activity + jitter).clamp(0.0, 1.0);

            let frequency = 800_000.0 + 4_200_000.0 * local - 12_000.0 * (temperature - 45.0);
            self.totals[cpu][COL_BUSY] += local * dt_ns;
            self.totals[cpu][COL_IDLE] += (1.0 - local) * dt_ns;
            self.totals[cpu][COL_CYCLES] += frequency * 1000.0 * local * dt_ns / 1e9;
            // CPU 0 carries the machine's interrupt load.
            let interrupt_rate = if cpu == 0 { 9_000.0 } else { 700.0 };
            self.totals[cpu][COL_INTERRUPTS] += interrupt_rate * (0.5 + local) * (dt_ns / 1e9);
        }

        let mut state = StateMatrix::new(self.entities.len(), self.channels.len());
        // Availability travels with the state: every cell this fixture fills is
        // marked Observed, so a consumer sees the same shape it would from a
        // real machine rather than a matrix with no reasons attached.
        let mut availability = AvailabilityMatrix::new(self.entities.len(), self.channels.len());
        for (cpu, row, core) in &self.cpus {
            let row = *row as usize;
            let local_frequency = 800_000.0 + 4_200_000.0 * self.activity[*core]
                - 12_000.0 * (self.temperature[*core] - 45.0);
            state.set(row, COL_FREQUENCY, local_frequency);
            state.set(row, COL_TEMPERATURE, self.temperature[*core]);
            for col in [COL_BUSY, COL_IDLE, COL_INTERRUPTS, COL_CYCLES] {
                state.set(row, col, self.totals[*cpu][col]);
            }
            for col in 0..self.channels.len() {
                availability.set(row, col, Availability::Observed);
            }
        }

        MirrorSnapshot {
            format_version: FORMAT_VERSION,
            epoch: 1,
            sequence: self.step,
            monotonic_ns: self.step * dt_ns as u64,
            realtime_ns: 1_700_000_000_000_000_000 + self.step * dt_ns as u64,
            entities: self.entities.clone(),
            channels: self.channels.clone(),
            relations: self.relations.clone(),
            state,
            availability,
            sensors: vec![SensorReport {
                id: SensorId(1),
                key: "synthetic".to_string(),
                perturbation: Perturbation::Negligible,
                last_cost_ns: 1000,
                sampling_latency_ns: 0,
                sample_age_ns: 0,
                samples: (self.cpus.len() * COLUMNS) as u32,
                errors: 0,
                confidence: 1.0,
                availability: Availability::Observed,
                inactive: false,
            }],
        }
    }

    /// A whole run of reflections.
    pub fn run(&mut self, frames: usize) -> Vec<MirrorSnapshot> {
        (0..frames).map(|_| self.tick()).collect()
    }
}

fn channel(index: usize, key: &str, unit: Unit, semantics: Semantics) -> ChannelSpec {
    ChannelSpec {
        id: ChannelId(index as u16),
        key: key.to_string(),
        unit,
        semantics,
        sensor: SensorId(1),
    }
}

/// Four physical cores, two threads each, all online, one package, one node.
///
/// CPU `n` and CPU `n + 4` are the two threads of core `n`, which is the usual
/// Linux enumeration and the reason SMT siblings are so easy to pin by accident.
fn uniform_smt_machine() -> FakeMachine {
    let mut cpus = Vec::new();
    for id in 0..(CORES * THREADS_PER_CORE) as u32 {
        let core_id = id % CORES as u32;
        let siblings = format!("{core_id},{}", core_id + CORES as u32);
        cpus.push(FakeCpu {
            id,
            package: 0,
            core_id,
            siblings: siblings.clone(),
            caches: vec![
                (0, 1, "Data", "32K", siblings.clone()),
                (2, 2, "Unified", "512K", siblings),
                (3, 3, "Unified", "16M", "0-7".to_string()),
            ],
            freq: Some((3_600_000, 800_000, 5_000_000)),
            highest_perf: None,
        });
    }
    FakeMachine {
        model: "Synthetic Observer Test CPU".to_string(),
        vendor: "GenuineTest".to_string(),
        cpus,
        present: "0-7".to_string(),
        online: "0-7".to_string(),
        offline: String::new(),
        nodes: vec![(0, "0-7".to_string(), 16_000_000)],
        p_cores: None,
        e_cores: None,
        // The simulator supplies the state itself; it does not want sensors
        // reading files.
        bare: true,
    }
}

/// Deterministic xorshift, so a run is reproducible.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn next_unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}
