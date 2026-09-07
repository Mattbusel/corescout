//! Compute workloads: integer ALU, unpredictable branches, scalar FP.
//!
//! These three are not interchangeable, and that is the point. A core that
//! wins on register-resident integer throughput can lose on branches, because
//! branch recovery cost depends on pipeline depth and on how much the branch
//! predictor's state has been polluted by whatever else runs on that core
//! (including its SMT sibling, which shares the predictor tables).

use std::hint::black_box;

use super::{Direction, Rng, Workload, WorkloadKind, WorkloadRun, WorkloadSpec};

// Iteration sizes are tuned for ~100-400 us on a ~3-5 GHz x86 core. See the
// module docs in `workloads/mod.rs` for why they are constants.
const ALU_ROUNDS: u64 = 500_000;
const BRANCH_ROUNDS: u64 = 50_000;
const BRANCH_TABLE: usize = 4096;
const FP_ROUNDS: u64 = 200_000;

/// Integer arithmetic with four independent dependency chains.
///
/// Four chains, because a single chain measures pure ALU *latency* while a
/// wide superscalar core is defined by how many chains it can retire at once.
/// Four is enough to saturate the integer ports of every current x86 core
/// without spilling the working set out of registers.
pub struct IntegerAlu;

impl Workload for IntegerAlu {
    fn spec(&self) -> WorkloadSpec {
        WorkloadSpec {
            id: "int-alu",
            name: "Integer ALU",
            description: "Four independent multiply/xor/rotate chains held in registers. \
                 Measures sustained integer throughput at whatever clock the core \
                 actually holds under load.",
            kind: WorkloadKind::Compute,
            direction: Direction::HigherIsBetter,
            ops_per_iteration: ALU_ROUNDS * 4 * 3,
            sample_multiplier: 1,
        }
    }

    fn instantiate(&self) -> Box<dyn WorkloadRun> {
        Box::new(IntegerAluRun {
            seeds: [
                0x1234_5678_9ABC_DEF0,
                0x0FED_CBA9_8765_4321,
                0xDEAD_BEEF_CAFE_F00D,
                0x5555_AAAA_3333_CCCC,
            ],
        })
    }
}

struct IntegerAluRun {
    seeds: [u64; 4],
}

impl WorkloadRun for IntegerAluRun {
    #[inline(never)]
    fn iterate(&mut self) -> u64 {
        // `black_box` on the inputs stops LLVM constant-folding the whole loop,
        // which it is entirely capable of doing here.
        let mut a = black_box(self.seeds[0]);
        let mut b = black_box(self.seeds[1]);
        let mut c = black_box(self.seeds[2]);
        let mut d = black_box(self.seeds[3]);

        for _ in 0..ALU_ROUNDS {
            a = a.wrapping_mul(6_364_136_223_846_793_005).rotate_left(13) ^ 0x9E37_79B9_7F4A_7C15;
            b = b.wrapping_mul(1_442_695_040_888_963_407).rotate_left(17) ^ 0xBF58_476D_1CE4_E5B9;
            c = c.wrapping_mul(6_364_136_223_846_793_005).rotate_left(23) ^ 0x94D0_49BB_1331_11EB;
            d = d.wrapping_mul(1_442_695_040_888_963_407).rotate_left(29) ^ 0x2545_F491_4F6C_DD1D;
        }

        // Feed the result forward so successive iterations are not identical
        // work on identical inputs, and return it so the caller can consume it.
        let checksum = a ^ b.rotate_left(1) ^ c.rotate_left(2) ^ d.rotate_left(3);
        self.seeds = [a | 1, b | 1, c | 1, d | 1];
        checksum | 1
    }
}

/// Data-dependent, deliberately unpredictable branches.
///
/// The comparison values come from a fixed-seed PRNG, so the branch direction
/// is uncorrelated with anything the predictor can learn but is identical
/// across runs. Expect roughly a 50% misprediction rate, which turns each
/// iteration into a pipeline-flush cost measurement.
pub struct BranchHeavy;

impl Workload for BranchHeavy {
    fn spec(&self) -> WorkloadSpec {
        WorkloadSpec {
            id: "branch",
            name: "Unpredictable branches",
            description: "Data-dependent branches over a pseudorandom table, mispredicting about \
                 half the time. Sensitive to pipeline depth and to branch-predictor state \
                 shared with an SMT sibling.",
            kind: WorkloadKind::Compute,
            direction: Direction::HigherIsBetter,
            ops_per_iteration: BRANCH_ROUNDS,
            sample_multiplier: 1,
        }
    }

    fn instantiate(&self) -> Box<dyn WorkloadRun> {
        let mut rng = Rng::new(0xB0A1_1CE5);
        let table: Vec<u8> = (0..BRANCH_TABLE).map(|_| rng.next_u64() as u8).collect();
        Box::new(BranchHeavyRun { table, acc: 1 })
    }
}

struct BranchHeavyRun {
    /// 4 KiB: comfortably L1-resident, so this measures branches, not memory.
    table: Vec<u8>,
    acc: u64,
}

impl WorkloadRun for BranchHeavyRun {
    #[inline(never)]
    fn iterate(&mut self) -> u64 {
        let table = black_box(&self.table);
        let mut acc = black_box(self.acc);
        let mut idx = 0usize;

        for i in 0..BRANCH_ROUNDS {
            let v = table[idx];
            // A chain of unrelated conditions: a conditional-move would be a
            // valid compilation of any one of these, but the nested structure
            // and the differing bodies keep real branches in the output.
            if v & 1 != 0 {
                acc = acc.wrapping_add(v as u64);
            } else if v & 2 != 0 {
                acc ^= (v as u64) << 3;
            } else if v > 200 {
                acc = acc.rotate_left(7);
            } else {
                acc = acc.wrapping_mul(3).wrapping_add(1);
            }
            // Index walk that the prefetcher can follow (we are not measuring
            // memory here) but whose *values* the predictor cannot.
            idx = (idx + 1 + (acc as usize & 1)) & (BRANCH_TABLE - 1);
            acc ^= i;
        }

        self.acc = acc | 1;
        acc | 1
    }
}

/// Small scalar floating-point dependency chains.
///
/// Deliberately scalar and dependent rather than vectorised: the goal is to
/// expose FP unit latency and clock behaviour, not to measure how far the core
/// downclocks under AVX-512, which is a different question and would need a
/// different workload to answer honestly.
pub struct FloatScalar;

impl Workload for FloatScalar {
    fn spec(&self) -> WorkloadSpec {
        WorkloadSpec {
            id: "fp-scalar",
            name: "Scalar floating point",
            description: "Four dependent multiply-add chains in double precision. Exposes FP unit \
                 latency and the clock the core sustains on FP work.",
            kind: WorkloadKind::Compute,
            direction: Direction::HigherIsBetter,
            ops_per_iteration: FP_ROUNDS * 4 * 2,
            sample_multiplier: 1,
        }
    }

    fn instantiate(&self) -> Box<dyn WorkloadRun> {
        Box::new(FloatScalarRun {
            state: [1.000_000_1, 0.999_999_9, 1.000_000_3, 0.999_999_7],
        })
    }
}

struct FloatScalarRun {
    state: [f64; 4],
}

impl WorkloadRun for FloatScalarRun {
    #[inline(never)]
    fn iterate(&mut self) -> u64 {
        let mut a = black_box(self.state[0]);
        let mut b = black_box(self.state[1]);
        let mut c = black_box(self.state[2]);
        let mut d = black_box(self.state[3]);
        // Multipliers chosen so the values orbit 1.0 instead of drifting into
        // denormals, which would measure microcode assists rather than the FP
        // pipeline.
        let (m1, m2) = (black_box(1.000_000_2f64), black_box(0.999_999_8f64));

        for _ in 0..FP_ROUNDS {
            a = a * m1 + 0.000_000_1;
            b = b * m2 + 0.000_000_1;
            c = c * m1 - 0.000_000_1;
            d = d * m2 - 0.000_000_1;
        }

        let sum = a + b + c + d;
        // Renormalise so the next iteration starts from the same regime.
        self.state = [
            renormalise(a),
            renormalise(b),
            renormalise(c),
            renormalise(d),
        ];
        // Fold the FP result into an integer checksum without letting the
        // optimiser discard it.
        sum.to_bits() | 1
    }
}

/// Pull a value back towards 1.0 while keeping it input-dependent.
#[inline]
fn renormalise(v: f64) -> f64 {
    if v.is_finite() && v > 0.5 && v < 2.0 {
        v
    } else {
        1.000_000_1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_alu_state_evolves() {
        let mut run = IntegerAlu.instantiate();
        let a = run.iterate();
        let b = run.iterate();
        assert_ne!(a, 0);
        assert_ne!(
            a, b,
            "iterations returned identical checksums; state is not feeding forward"
        );
    }

    #[test]
    fn branch_workload_is_reproducible_across_instances() {
        // Same seed, same table, same branch sequence: two fresh instances
        // must agree, or the measurement is not repeatable.
        let a = BranchHeavy.instantiate().iterate();
        let b = BranchHeavy.instantiate().iterate();
        assert_eq!(a, b);
    }

    #[test]
    fn float_workload_stays_in_a_sane_numeric_range() {
        let mut run = FloatScalar.instantiate();
        for _ in 0..5 {
            let bits = run.iterate();
            let value = f64::from_bits(bits & !1);
            assert!(
                value.is_finite(),
                "FP workload escaped into a non-finite value"
            );
        }
    }

    #[test]
    fn ops_counts_are_declared() {
        for spec in [IntegerAlu.spec(), BranchHeavy.spec(), FloatScalar.spec()] {
            assert_eq!(spec.kind, WorkloadKind::Compute);
            assert_eq!(spec.direction, Direction::HigherIsBetter);
            assert!(spec.ops_per_iteration > 1000);
        }
    }
}
