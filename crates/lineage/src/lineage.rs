//! Generations, and whether the line is actually getting better.
//!
//! # What is being claimed
//!
//! ```text
//! Q(G0) < Q(G1) < Q(G2) < ...
//! ```
//!
//! on verified useful work per scarce physical input, net of what the search
//! cost. If that holds, the process is turning compute into computational
//! capital. If it does not, the process is an expensive way to keep a machine
//! busy.
//!
//! # Why the verdict can be negative
//!
//! [`Lineage::verdict`] returns [`Verdict::NotImproving`] and
//! [`Verdict::Untrustworthy`] as readily as it returns success, and a lineage
//! that cannot produce those answers is not measuring anything. The specific
//! failures it must be able to report:
//!
//! | failure | what it looks like |
//! |---|---|
//! | learned the benchmark | trained `Q` rises, held-out `Q` does not |
//! | search cost too much | gross `Q` rises, net `Q` does not |
//! | stopped doing the work | verification failures anywhere |
//! | the machine just got quieter | needs the control lineage; see [`Comparison`] |
//!
//! # The control
//!
//! A lineage that improves proves nothing on its own, because machines drift:
//! a quieter neighbour, a cooler room, a different kernel. The comparison that
//! means something is against a control lineage that spends **the same inputs**
//! and carries nothing forward. [`Comparison::attributable_gain`] is the
//! difference, and it is the only number here worth quoting to somebody
//! sceptical.

use serde::{Deserialize, Serialize};

use crate::productivity::{Inputs, Ledger, Pool, Weights};

/// One generation of a lineage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Generation {
    /// Zero-based. `G0` inherited nothing.
    pub index: u32,
    /// What it learned and passed on, as a count. The atlas itself lives in the
    /// capability crate; this records only its size, so the ledger stays a
    /// ledger.
    pub inherited_edges: usize,
    pub inherited_capabilities: usize,
    pub ledger: Ledger,
}

impl Generation {
    pub fn new(index: u32) -> Generation {
        Generation {
            index,
            inherited_edges: 0,
            inherited_capabilities: 0,
            ledger: Ledger::new(),
        }
    }

    pub fn name(&self) -> String {
        format!("G{}", self.index)
    }
}

/// What a lineage achieved, or failed to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Net productivity rose on held-out work across the line.
    Improving {
        from: f64,
        to: f64,
        /// Fractional gain.
        gain: f64,
        generations: u32,
    },
    /// It got better at what it practised and no better at anything else.
    LearnedTheBenchmark {
        trained_gain: f64,
        held_out_gain: f64,
    },
    /// The gains are real and cost more to find than they have returned.
    SearchCostTooHigh { gross_gain: f64, net_gain: f64 },
    /// Held-out productivity did not rise.
    NotImproving { from: f64, to: f64 },
    /// Some generation produced work that did not verify.
    Untrustworthy { generation: u32, failed: u64 },
    /// Not enough generations, or not enough work, to say.
    Inconclusive { reason: String },
}

impl Verdict {
    pub fn is_improving(&self) -> bool {
        matches!(self, Verdict::Improving { .. })
    }

    pub fn describe(&self) -> String {
        match self {
            Verdict::Improving {
                from,
                to,
                gain,
                generations,
            } => format!(
                "held-out net productivity rose from {from:.3e} to {to:.3e} over {generations} \
                 generations, a gain of {:.1}%",
                gain * 100.0
            ),
            Verdict::LearnedTheBenchmark {
                trained_gain,
                held_out_gain,
            } => format!(
                "trained work improved {:.1}% and held-out work {:.1}%: the line learned the \
                 workload it practised on, not the machine",
                trained_gain * 100.0,
                held_out_gain * 100.0
            ),
            Verdict::SearchCostTooHigh {
                gross_gain,
                net_gain,
            } => format!(
                "gross productivity rose {:.1}% but net rose {:.1}%: finding the improvement \
                 cost more than it has returned",
                gross_gain * 100.0,
                net_gain * 100.0
            ),
            Verdict::NotImproving { from, to } => format!(
                "held-out net productivity went from {from:.3e} to {to:.3e}: the line is not \
                 getting better"
            ),
            Verdict::Untrustworthy { generation, failed } => format!(
                "G{generation} produced {failed} units that did not verify; no productivity \
                 number from this lineage should be believed"
            ),
            Verdict::Inconclusive { reason } => format!("inconclusive: {reason}"),
        }
    }
}

/// A line of descent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lineage {
    pub name: String,
    pub weights: Weights,
    generations: Vec<Generation>,
    /// A gain smaller than this is not claimed.
    minimum_gain: f64,
}

impl Lineage {
    pub fn new(name: impl Into<String>, weights: Weights) -> Lineage {
        Lineage {
            name: name.into(),
            weights,
            generations: Vec::new(),
            // Five per cent. Smaller differences than this on a real machine are
            // usually the machine rather than the runtime.
            minimum_gain: 0.05,
        }
    }

    pub fn generations(&self) -> &[Generation] {
        &self.generations
    }

    pub fn len(&self) -> usize {
        self.generations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.generations.is_empty()
    }

    pub fn latest(&self) -> Option<&Generation> {
        self.generations.last()
    }

    /// Begin a generation, charging it the search cost of everything before it.
    ///
    /// The inheritance is cumulative on purpose. A line that keeps searching
    /// keeps owing, and a descendant twelve generations down is still paying
    /// for what its ancestors spent finding the advantage it was born with.
    pub fn begin(&mut self, edges: usize, capabilities: usize) -> &mut Generation {
        let index = self.generations.len() as u32;
        let mut generation = Generation::new(index);
        generation.inherited_edges = edges;
        generation.inherited_capabilities = capabilities;

        // Each ancestor's own search, counted once. Adding their `inherited`
        // too would charge the earliest generations again at every step, so a
        // long line would look worse and worse for no reason.
        let mut owed = Inputs::default();
        for ancestor in &self.generations {
            owed.add(&ancestor.ledger.exploration());
        }
        generation.ledger.inherit_exploration(owed);

        self.generations.push(generation);
        self.generations.last_mut().expect("just pushed")
    }

    /// The current generation, for recording into.
    pub fn current(&mut self) -> Option<&mut Generation> {
        self.generations.last_mut()
    }

    /// Net productivity of one generation on one pool.
    pub fn productivity(&self, index: usize, pool: Pool) -> Option<f64> {
        self.generations
            .get(index)?
            .ledger
            .net_productivity(pool, &self.weights)
    }

    /// Where the line stands.
    pub fn verdict(&self) -> Verdict {
        if self.generations.len() < 2 {
            return Verdict::Inconclusive {
                reason: format!(
                    "{} generation(s); a lineage needs at least two to have a direction",
                    self.generations.len()
                ),
            };
        }

        // Trust before performance. A faster wrong answer is not a result.
        for generation in &self.generations {
            let failed = generation.ledger.work(Pool::Trained).failed
                + generation.ledger.work(Pool::HeldOut).failed;
            if failed > 0 {
                return Verdict::Untrustworthy {
                    generation: generation.index,
                    failed,
                };
            }
        }

        let first = self.generations.first().expect("checked");
        let last = self.generations.last().expect("checked");
        let weights = &self.weights;

        let (Some(from), Some(to)) = (
            first.ledger.net_productivity(Pool::HeldOut, weights),
            last.ledger.net_productivity(Pool::HeldOut, weights),
        ) else {
            return Verdict::Inconclusive {
                reason: "no held-out work was measured; a lineage measured only on what it \
                         practised proves nothing"
                    .into(),
            };
        };
        if from <= 0.0 {
            return Verdict::Inconclusive {
                reason: "the first generation produced no verified work".into(),
            };
        }

        let held_out_gain = (to - from) / from;

        // Did it learn the benchmark?
        if let (Some(trained_from), Some(trained_to)) = (
            first.ledger.net_productivity(Pool::Trained, weights),
            last.ledger.net_productivity(Pool::Trained, weights),
        ) {
            if trained_from > 0.0 {
                let trained_gain = (trained_to - trained_from) / trained_from;
                if trained_gain > self.minimum_gain && held_out_gain < self.minimum_gain {
                    return Verdict::LearnedTheBenchmark {
                        trained_gain,
                        held_out_gain,
                    };
                }
            }
        }

        // Did the search cost more than it returned?
        if let (Some(gross_from), Some(gross_to)) = (
            first.ledger.productivity(Pool::HeldOut, weights),
            last.ledger.productivity(Pool::HeldOut, weights),
        ) {
            if gross_from > 0.0 {
                let gross_gain = (gross_to - gross_from) / gross_from;
                if gross_gain > self.minimum_gain && held_out_gain < self.minimum_gain {
                    return Verdict::SearchCostTooHigh {
                        gross_gain,
                        net_gain: held_out_gain,
                    };
                }
            }
        }

        if held_out_gain > self.minimum_gain {
            Verdict::Improving {
                from,
                to,
                gain: held_out_gain,
                generations: self.generations.len() as u32,
            }
        } else {
            Verdict::NotImproving { from, to }
        }
    }

    pub fn render(&self) -> String {
        let mut out = format!("{}: {} generations\n", self.name, self.generations.len());
        for generation in &self.generations {
            let held = generation
                .ledger
                .net_productivity(Pool::HeldOut, &self.weights);
            let trained = generation
                .ledger
                .net_productivity(Pool::Trained, &self.weights);
            out.push_str(&format!(
                "  {}: inherited {} edges / {} capabilities; net Q trained {} held-out {}\n",
                generation.name(),
                generation.inherited_edges,
                generation.inherited_capabilities,
                trained.map(|q| format!("{q:.3e}")).unwrap_or("n/a".into()),
                held.map(|q| format!("{q:.3e}")).unwrap_or("n/a".into()),
            ));
        }
        out.push_str(&format!("\n{}\n", self.verdict().describe()));
        out
    }
}

/// A lineage against its control.
///
/// The only comparison worth quoting. A lineage that improves proves nothing
/// alone, because machines drift; the control spends the same inputs and
/// carries nothing forward, so whatever it also gained was not the learning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    pub learned: Verdict,
    pub control: Verdict,
    /// Held-out gain of the learning line minus that of the control.
    pub attributable_gain: Option<f64>,
}

impl Comparison {
    pub fn of(learned: &Lineage, control: &Lineage) -> Comparison {
        let gain = |lineage: &Lineage| -> Option<f64> {
            match lineage.verdict() {
                Verdict::Improving { gain, .. } => Some(gain),
                Verdict::NotImproving { from, to } if from > 0.0 => Some((to - from) / from),
                Verdict::SearchCostTooHigh { net_gain, .. } => Some(net_gain),
                Verdict::LearnedTheBenchmark { held_out_gain, .. } => Some(held_out_gain),
                _ => None,
            }
        };
        Comparison {
            attributable_gain: match (gain(learned), gain(control)) {
                (Some(a), Some(b)) => Some(a - b),
                _ => None,
            },
            learned: learned.verdict(),
            control: control.verdict(),
        }
    }

    /// Whether the gain can be credited to the learning rather than to drift.
    pub fn is_attributable(&self, minimum: f64) -> bool {
        self.learned.is_improving() && self.attributable_gain.is_some_and(|g| g > minimum)
    }

    pub fn describe(&self) -> String {
        let mut out = format!(
            "learned line: {}\ncontrol line: {}\n",
            self.learned.describe(),
            self.control.describe()
        );
        match self.attributable_gain {
            Some(gain) => out.push_str(&format!(
                "\nattributable to learning: {:.1}%. Anything the control also gained was \
                 the machine, not the runtime.\n",
                gain * 100.0
            )),
            None => out.push_str(
                "\nno attributable gain can be computed; one of the lines produced no \
                 comparable number\n",
            ),
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

    /// A generation that does `units` of work at `ns` each, correctly.
    fn produce(lineage: &mut Lineage, units: u64, trained_ns: u64, held_out_ns: u64) {
        let generation = lineage.current().expect("a generation");
        for _ in 0..units {
            generation
                .ledger
                .record(Pool::Trained, 1, 1, Inputs::silicon(trained_ns));
            generation
                .ledger
                .record(Pool::HeldOut, 1, 1, Inputs::silicon(held_out_ns));
        }
    }

    #[test]
    fn one_generation_has_no_direction() {
        let mut lineage = Lineage::new("test", weights());
        lineage.begin(0, 0);
        produce(&mut lineage, 100, 1000, 1000);
        assert!(matches!(lineage.verdict(), Verdict::Inconclusive { .. }));
    }

    #[test]
    fn a_line_that_gets_faster_on_held_out_work_is_improving() {
        let mut lineage = Lineage::new("test", weights());
        lineage.begin(0, 0);
        produce(&mut lineage, 1000, 1000, 1000);
        lineage.begin(40, 3);
        produce(&mut lineage, 1000, 800, 800);
        let verdict = lineage.verdict();
        assert!(verdict.is_improving(), "{}", verdict.describe());
        match verdict {
            Verdict::Improving { gain, .. } => assert!(gain > 0.2, "gain {gain}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_line_that_only_improves_on_what_it_practised_is_caught() {
        // The most likely false positive: the descendant is tuned to the
        // workload it explored against and no better at anything else.
        let mut lineage = Lineage::new("test", weights());
        lineage.begin(0, 0);
        produce(&mut lineage, 1000, 1000, 1000);
        lineage.begin(40, 3);
        produce(&mut lineage, 1000, 500, 1000);
        let verdict = lineage.verdict();
        assert!(!verdict.is_improving());
        assert!(matches!(verdict, Verdict::LearnedTheBenchmark { .. }));
        assert!(verdict.describe().contains("not the machine"));
    }

    #[test]
    fn a_search_that_costs_more_than_it_returns_is_caught() {
        let mut lineage = Lineage::new("test", weights());
        lineage.begin(0, 0);
        produce(&mut lineage, 1000, 1000, 1000);
        // An enormous search for a small gain.
        lineage
            .current()
            .expect("a generation")
            .ledger
            .explore(Inputs::silicon(400_000));
        lineage.begin(40, 3);
        produce(&mut lineage, 1000, 900, 900);
        let verdict = lineage.verdict();
        assert!(!verdict.is_improving(), "{}", verdict.describe());
        assert!(matches!(verdict, Verdict::SearchCostTooHigh { .. }));
        assert!(verdict
            .describe()
            .contains("cost more than it has returned"));
    }

    #[test]
    fn work_that_does_not_verify_invalidates_the_whole_line() {
        // Trust before performance: a faster wrong answer is not a result, and
        // the generation that produced it contaminates the comparison.
        let mut lineage = Lineage::new("test", weights());
        lineage.begin(0, 0);
        produce(&mut lineage, 1000, 1000, 1000);
        lineage.begin(40, 3);
        produce(&mut lineage, 1000, 100, 100);
        lineage.current().expect("a generation").ledger.record(
            Pool::HeldOut,
            0,
            1,
            Inputs::silicon(1),
        );
        let verdict = lineage.verdict();
        assert!(matches!(verdict, Verdict::Untrustworthy { .. }));
        assert!(verdict.describe().contains("should be believed"));
    }

    #[test]
    fn a_line_measured_only_on_what_it_practised_is_inconclusive() {
        let mut lineage = Lineage::new("test", weights());
        for _ in 0..2 {
            lineage.begin(0, 0);
            let generation = lineage.current().expect("a generation");
            for _ in 0..100 {
                generation
                    .ledger
                    .record(Pool::Trained, 1, 1, Inputs::silicon(1000));
            }
        }
        let verdict = lineage.verdict();
        assert!(matches!(verdict, Verdict::Inconclusive { .. }));
        assert!(verdict.describe().contains("proves nothing"));
    }

    #[test]
    fn a_flat_line_is_reported_as_not_improving() {
        let mut lineage = Lineage::new("test", weights());
        for _ in 0..3 {
            lineage.begin(0, 0);
            produce(&mut lineage, 500, 1000, 1000);
        }
        let verdict = lineage.verdict();
        assert!(matches!(verdict, Verdict::NotImproving { .. }));
        assert!(verdict.describe().contains("not getting better"));
    }

    #[test]
    fn search_cost_is_charged_once_however_deep_the_line() {
        // Charging cumulative inheritance again at each step would make a long
        // line look worse and worse for no reason.
        let mut lineage = Lineage::new("test", weights());
        for _ in 0..4 {
            lineage.begin(0, 0);
            produce(&mut lineage, 100, 1000, 1000);
            lineage
                .current()
                .expect("a generation")
                .ledger
                .explore(Inputs::silicon(1000));
        }
        let last = lineage.latest().expect("a generation");
        // Three ancestors, a thousand each.
        assert_eq!(last.ledger.inherited().silicon_ns, 3000);
    }

    #[test]
    fn a_gain_the_control_also_shows_is_not_attributable_to_learning() {
        // The comparison that means something. Both lines got faster; only one
        // was learning, so the difference is what the learning bought.
        let mut learned = Lineage::new("learned", weights());
        learned.begin(0, 0);
        produce(&mut learned, 1000, 1000, 1000);
        learned.begin(40, 3);
        produce(&mut learned, 1000, 700, 700);

        let mut control = Lineage::new("control", weights());
        control.begin(0, 0);
        produce(&mut control, 1000, 1000, 1000);
        control.begin(0, 0);
        // The machine simply got quieter: the control improved too.
        produce(&mut control, 1000, 720, 720);

        let comparison = Comparison::of(&learned, &control);
        assert!(learned.verdict().is_improving());
        assert!(control.verdict().is_improving());
        let gain = comparison.attributable_gain.expect("both comparable");
        assert!(gain.abs() < 0.1, "attributable gain was {gain}");
        assert!(
            !comparison.is_attributable(0.1),
            "nearly all of it was drift: {}",
            comparison.describe()
        );
    }

    #[test]
    fn a_gain_the_control_does_not_show_is_attributable() {
        let mut learned = Lineage::new("learned", weights());
        learned.begin(0, 0);
        produce(&mut learned, 1000, 1000, 1000);
        learned.begin(40, 3);
        produce(&mut learned, 1000, 600, 600);

        let mut control = Lineage::new("control", weights());
        control.begin(0, 0);
        produce(&mut control, 1000, 1000, 1000);
        control.begin(0, 0);
        produce(&mut control, 1000, 1000, 1000);

        let comparison = Comparison::of(&learned, &control);
        assert!(comparison.is_attributable(0.2), "{}", comparison.describe());
        assert!(comparison.describe().contains("attributable to learning"));
    }

    #[test]
    fn a_lineage_round_trips_through_json() {
        let mut lineage = Lineage::new("test", weights());
        lineage.begin(0, 0);
        produce(&mut lineage, 10, 1000, 1000);
        let text = serde_json::to_string(&lineage).expect("serialises");
        let back: Lineage = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back, lineage);
    }
}
