//! Useful work per scarce physical input, measured so that cheating is
//! expensive.
//!
//! # The quantity
//!
//! ```text
//!         verified useful work
//! Q  =  ------------------------
//!        scarce physical inputs
//! ```
//!
//! A lineage of self-modified runtimes is only interesting if `Q` rises. This
//! module exists to make that number hard to inflate, because it is the number
//! everything else in the project would be judged by, and it is trivially
//! faked by a system that measures its own success.
//!
//! # The five ways this gets faked, and what stops each
//!
//! **Skip the work and report the throughput.** Every unit of work carries a
//! result that must match an expected value the runtime never sees. Unverified
//! output counts as **zero work**, and its inputs are still charged. Skipping
//! work is therefore strictly worse than doing it, which is the property that
//! has to hold and the one a naive counter does not have.
//!
//! **Ignore what the search cost.** Compute spent exploring is capitalised and
//! amortised into the descendant that inherits the result, so a gain that cost
//! more to find than it will ever return shows up as negative.
//! [`Ledger::net_productivity`] is the number to quote; [`Ledger::productivity`]
//! is the flattering one.
//!
//! **Optimise the measured workload.** Held-out work is recorded separately and
//! never explored against. A lineage whose gross `Q` rises while its held-out
//! `Q` does not has learned the benchmark.
//!
//! **Take credit for a quieter machine.** Not solvable here; it needs a control
//! lineage spending identical inputs and learning nothing. This module provides
//! the ledger such a comparison is made with.
//!
//! **Define "useful" yourself.** Work is verified against an external
//! expectation. A runtime that could choose what counted as success would find
//! that it was succeeding.

use serde::{Deserialize, Serialize};

/// The scarce things consumed to do work.
///
/// Kept as separate quantities rather than one number, because collapsing them
/// early hides the trade a runtime is most likely to make: buying throughput
/// with a great deal more memory, or with energy, and calling it an improvement.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Inputs {
    /// Silicon time: nanoseconds of a core actually occupied.
    pub silicon_ns: u64,
    /// Energy, where the machine can measure it.
    pub energy_uj: u64,
    /// Memory held, integrated over time: byte-nanoseconds.
    ///
    /// A runtime that holds twice the memory for the same wall time has
    /// consumed twice as much of a scarce thing, and a metric that ignores it
    /// will reward doing so.
    pub memory_byte_ns: u128,
}

impl Inputs {
    pub fn silicon(ns: u64) -> Inputs {
        Inputs {
            silicon_ns: ns,
            ..Inputs::default()
        }
    }

    pub fn add(&mut self, other: &Inputs) {
        self.silicon_ns = self.silicon_ns.saturating_add(other.silicon_ns);
        self.energy_uj = self.energy_uj.saturating_add(other.energy_uj);
        self.memory_byte_ns = self.memory_byte_ns.saturating_add(other.memory_byte_ns);
    }

    pub fn is_zero(&self) -> bool {
        self.silicon_ns == 0 && self.energy_uj == 0 && self.memory_byte_ns == 0
    }

    /// The inputs as one figure, for a ratio.
    ///
    /// `weights` says what a unit of each is worth relative to the others, and
    /// **must come from outside**. A runtime that chose its own weights would
    /// discover that whatever it consumes most of is cheap.
    pub fn scalar(&self, weights: &Weights) -> f64 {
        self.silicon_ns as f64 * weights.silicon
            + self.energy_uj as f64 * weights.energy
            + self.memory_byte_ns as f64 * weights.memory
    }
}

/// What each scarce input is worth, relative to the others.
///
/// Supplied by whoever is paying for the hardware. There is no principled way
/// for the machine to derive these, and letting it try is how a metric becomes
/// a mirror.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Weights {
    pub silicon: f64,
    pub energy: f64,
    pub memory: f64,
}

impl Default for Weights {
    fn default() -> Self {
        // Silicon time only. The honest default on a machine that cannot
        // measure its own power, which is most of them: weighting a quantity
        // that is always zero would silently make it free.
        Weights {
            silicon: 1.0,
            energy: 0.0,
            memory: 0.0,
        }
    }
}

impl Weights {
    pub fn is_usable(&self) -> bool {
        let all = [self.silicon, self.energy, self.memory];
        all.iter().all(|w| w.is_finite() && *w >= 0.0) && all.iter().any(|w| *w > 0.0)
    }
}

/// Work done, and whether it was actually done.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Work {
    /// Units whose result matched what was expected.
    pub verified: u64,
    /// Units whose result did not match. Counted, and worth nothing.
    pub failed: u64,
}

impl Work {
    pub fn attempted(&self) -> u64 {
        self.verified + self.failed
    }

    /// Fraction of attempted work that was actually correct.
    pub fn correctness(&self) -> Option<f64> {
        (self.attempted() > 0).then(|| self.verified as f64 / self.attempted() as f64)
    }

    pub fn add(&mut self, other: &Work) {
        self.verified += other.verified;
        self.failed += other.failed;
    }
}

/// Which pool a measurement belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pool {
    /// Workloads the lineage was allowed to explore against.
    Trained,
    /// Workloads it has never seen. The number that matters.
    HeldOut,
}

/// The record of what one generation produced and what it consumed.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Ledger {
    /// Work and inputs on workloads the lineage explored against.
    trained_work: Work,
    trained_inputs: Inputs,
    /// Work and inputs on workloads it has never seen.
    held_out_work: Work,
    held_out_inputs: Inputs,
    /// Compute spent searching, by this generation, for the benefit of the next.
    ///
    /// Not charged here. Charged to whoever inherits the result, via
    /// [`Ledger::inherit_exploration`].
    exploration_inputs: Inputs,
    /// Exploration inherited from ancestors and still being paid off.
    inherited_exploration: Inputs,
}

impl Ledger {
    pub fn new() -> Ledger {
        Ledger::default()
    }

    /// Record a unit of work whose result is checked against an expectation.
    ///
    /// The expectation comes from the caller and is never shown to the runtime.
    /// A unit that does not verify contributes no work and its inputs are still
    /// charged, so skipping work is strictly worse than doing it.
    pub fn record(&mut self, pool: Pool, result: u64, expected: u64, inputs: Inputs) {
        let verified = result == expected;
        let (work, spent) = match pool {
            Pool::Trained => (&mut self.trained_work, &mut self.trained_inputs),
            Pool::HeldOut => (&mut self.held_out_work, &mut self.held_out_inputs),
        };
        if verified {
            work.verified += 1;
        } else {
            work.failed += 1;
        }
        spent.add(&inputs);
    }

    /// Record compute spent searching rather than producing.
    pub fn explore(&mut self, inputs: Inputs) {
        self.exploration_inputs.add(&inputs);
    }

    /// Take on an ancestor's search cost.
    ///
    /// A descendant that inherits an advantage inherits the bill for finding it.
    /// Without this the lineage reports free improvements, and the search could
    /// cost more than it will ever return while every generation still looked
    /// profitable.
    pub fn inherit_exploration(&mut self, inputs: Inputs) {
        self.inherited_exploration.add(&inputs);
    }

    pub fn work(&self, pool: Pool) -> Work {
        match pool {
            Pool::Trained => self.trained_work,
            Pool::HeldOut => self.held_out_work,
        }
    }

    pub fn inputs(&self, pool: Pool) -> Inputs {
        match pool {
            Pool::Trained => self.trained_inputs,
            Pool::HeldOut => self.held_out_inputs,
        }
    }

    pub fn exploration(&self) -> Inputs {
        self.exploration_inputs
    }

    pub fn inherited(&self) -> Inputs {
        self.inherited_exploration
    }

    /// Verified work per scarce input, on one pool, ignoring what it cost to
    /// learn.
    ///
    /// The flattering number. Use [`Ledger::net_productivity`] to quote.
    pub fn productivity(&self, pool: Pool, weights: &Weights) -> Option<f64> {
        if !weights.is_usable() {
            return None;
        }
        let work = self.work(pool);
        let inputs = self.inputs(pool);
        let denominator = inputs.scalar(weights);
        if denominator <= 0.0 {
            return None;
        }
        Some(work.verified as f64 / denominator)
    }

    /// The same, with inherited search cost charged in.
    ///
    /// The number that decides whether the lineage was worth running.
    pub fn net_productivity(&self, pool: Pool, weights: &Weights) -> Option<f64> {
        if !weights.is_usable() {
            return None;
        }
        let work = self.work(pool);
        let mut inputs = self.inputs(pool);
        inputs.add(&self.inherited_exploration);
        let denominator = inputs.scalar(weights);
        if denominator <= 0.0 {
            return None;
        }
        Some(work.verified as f64 / denominator)
    }

    /// Whether this generation did what it claimed.
    ///
    /// A generation with failed verifications is not slightly worse, it is
    /// suspect: the most likely cause of a runtime producing wrong answers
    /// faster is that it stopped doing the work.
    pub fn is_trustworthy(&self) -> bool {
        self.trained_work.failed == 0 && self.held_out_work.failed == 0
    }

    pub fn describe(&self, weights: &Weights) -> String {
        let mut out = String::new();
        for (label, pool) in [("trained", Pool::Trained), ("held out", Pool::HeldOut)] {
            let work = self.work(pool);
            if work.attempted() == 0 {
                continue;
            }
            let gross = self.productivity(pool, weights);
            let net = self.net_productivity(pool, weights);
            out.push_str(&format!(
                "{label}: {} verified, {} failed; Q {} gross, {} net\n",
                work.verified,
                work.failed,
                gross.map(|q| format!("{q:.6}")).unwrap_or("n/a".into()),
                net.map(|q| format!("{q:.6}")).unwrap_or("n/a".into()),
            ));
        }
        if !self.is_trustworthy() {
            out.push_str(
                "WORK FAILED VERIFICATION: this generation's numbers should not be believed, \
                 because the cheapest way to look fast is to stop doing the work\n",
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weights() -> Weights {
        Weights::default()
    }

    #[test]
    fn skipping_work_is_strictly_worse_than_doing_it() {
        // The anti-fraud property everything else rests on. A runtime that
        // returns wrong answers quickly must score lower than one that returns
        // right answers slowly.
        let mut honest = Ledger::new();
        for _ in 0..100 {
            honest.record(Pool::Trained, 42, 42, Inputs::silicon(1000));
        }
        let mut cheat = Ledger::new();
        for _ in 0..100 {
            // Ten times faster, and wrong.
            cheat.record(Pool::Trained, 0, 42, Inputs::silicon(100));
        }
        let honest_q = honest.productivity(Pool::Trained, &weights()).unwrap();
        let cheat_q = cheat.productivity(Pool::Trained, &weights()).unwrap();
        assert!(
            cheat_q < honest_q,
            "cheating scored {cheat_q} against an honest {honest_q}"
        );
        assert_eq!(cheat_q, 0.0, "unverified work is worth nothing");
        assert!(!cheat.is_trustworthy());
        assert!(cheat
            .describe(&weights())
            .contains("WORK FAILED VERIFICATION"));
    }

    #[test]
    fn the_inputs_of_failed_work_are_still_charged() {
        // Otherwise failing is free, and a runtime that fails fast on the hard
        // units and succeeds on the easy ones scores well.
        let mut ledger = Ledger::new();
        ledger.record(Pool::Trained, 1, 1, Inputs::silicon(1000));
        ledger.record(Pool::Trained, 0, 1, Inputs::silicon(1000));
        let q = ledger.productivity(Pool::Trained, &weights()).unwrap();
        assert!((q - 1.0 / 2000.0).abs() < 1e-12, "got {q}");
    }

    #[test]
    fn exploration_is_charged_to_whoever_inherits_the_gain() {
        let mut parent = Ledger::new();
        parent.explore(Inputs::silicon(50_000));
        for _ in 0..100 {
            parent.record(Pool::Trained, 1, 1, Inputs::silicon(1000));
        }

        let mut child = Ledger::new();
        child.inherit_exploration(parent.exploration());
        for _ in 0..100 {
            // The child is genuinely faster.
            child.record(Pool::Trained, 1, 1, Inputs::silicon(800));
        }

        let gross = child.productivity(Pool::Trained, &weights()).unwrap();
        let net = child.net_productivity(Pool::Trained, &weights()).unwrap();
        assert!(net < gross, "the search has to be paid for");

        // Gross says the child improved; net says the search cost more than it
        // has yet returned. Both are true and only one is honest to quote.
        let parent_q = parent.productivity(Pool::Trained, &weights()).unwrap();
        assert!(gross > parent_q);
        assert!(net < parent_q, "net {net} against a parent {parent_q}");
    }

    #[test]
    fn a_gain_large_enough_eventually_repays_the_search() {
        let mut child = Ledger::new();
        child.inherit_exploration(Inputs::silicon(50_000));
        // Enough work that the fixed search cost is amortised away.
        for _ in 0..10_000 {
            child.record(Pool::Trained, 1, 1, Inputs::silicon(800));
        }
        let net = child.net_productivity(Pool::Trained, &weights()).unwrap();
        let baseline = 1.0 / 1000.0;
        assert!(
            net > baseline,
            "net {net} should beat a baseline {baseline}"
        );
    }

    #[test]
    fn held_out_work_is_accounted_separately() {
        // A lineage whose trained Q rises while held-out Q does not has learned
        // the benchmark rather than the machine.
        let mut ledger = Ledger::new();
        for _ in 0..100 {
            ledger.record(Pool::Trained, 1, 1, Inputs::silicon(500));
            ledger.record(Pool::HeldOut, 1, 1, Inputs::silicon(1000));
        }
        let trained = ledger.productivity(Pool::Trained, &weights()).unwrap();
        let held_out = ledger.productivity(Pool::HeldOut, &weights()).unwrap();
        assert!(trained > held_out);
        assert!(ledger.describe(&weights()).contains("held out"));
    }

    #[test]
    fn memory_is_a_scarce_input_and_cannot_be_spent_freely() {
        // Buying throughput with a great deal more memory is the trade a
        // runtime is most likely to make and call an improvement.
        let costed = Weights {
            silicon: 1.0,
            energy: 0.0,
            memory: 1e-6,
        };
        let mut lean = Ledger::new();
        let mut fat = Ledger::new();
        for _ in 0..100 {
            lean.record(Pool::Trained, 1, 1, Inputs::silicon(1000));
            fat.record(
                Pool::Trained,
                1,
                1,
                Inputs {
                    silicon_ns: 800,
                    energy_uj: 0,
                    memory_byte_ns: 1_000_000_000,
                },
            );
        }
        // Faster on silicon alone.
        assert!(
            fat.productivity(Pool::Trained, &Weights::default())
                .unwrap()
                > lean
                    .productivity(Pool::Trained, &Weights::default())
                    .unwrap()
        );
        // Worse once memory is priced.
        assert!(
            fat.productivity(Pool::Trained, &costed).unwrap()
                < lean.productivity(Pool::Trained, &costed).unwrap()
        );
    }

    #[test]
    fn weights_must_come_from_outside_and_must_price_something() {
        // A runtime choosing its own weights would find that whatever it
        // consumes most of is free.
        let nothing = Weights {
            silicon: 0.0,
            energy: 0.0,
            memory: 0.0,
        };
        assert!(!nothing.is_usable());
        let mut ledger = Ledger::new();
        ledger.record(Pool::Trained, 1, 1, Inputs::silicon(1000));
        assert_eq!(ledger.productivity(Pool::Trained, &nothing), None);

        let broken = Weights {
            silicon: f64::NAN,
            ..Weights::default()
        };
        assert!(!broken.is_usable());
    }

    #[test]
    fn a_ledger_with_no_work_has_no_productivity() {
        let ledger = Ledger::new();
        assert_eq!(ledger.productivity(Pool::Trained, &weights()), None);
        assert_eq!(ledger.net_productivity(Pool::Trained, &weights()), None);
    }

    #[test]
    fn correctness_is_reported_alongside_the_ratio() {
        let mut ledger = Ledger::new();
        for i in 0..100 {
            ledger.record(Pool::Trained, i % 10, 0, Inputs::silicon(100));
        }
        let work = ledger.work(Pool::Trained);
        assert_eq!(work.verified, 10);
        assert_eq!(work.failed, 90);
        assert!((work.correctness().unwrap() - 0.1).abs() < 1e-9);
    }

    #[test]
    fn a_ledger_round_trips_through_json() {
        let mut ledger = Ledger::new();
        ledger.record(Pool::Trained, 1, 1, Inputs::silicon(1000));
        ledger.explore(Inputs::silicon(50));
        ledger.inherit_exploration(Inputs::silicon(20));
        let text = serde_json::to_string(&ledger).expect("serialises");
        let back: Ledger = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back, ledger);
    }
}
