//! Inventing a move that was not in the action set.
//!
//! # The threshold
//!
//! Everything else in this crate is a primitive somebody wrote: set an
//! affinity, set a priority, set a NUMA preference. Choosing among them well is
//! control. It is not invention, because the space of possible moves was fixed
//! by us before the machine started.
//!
//! This module lets the action set itself change. When the machine finds a
//! target it wants to reach and no single primitive reaches it, it can compose
//! primitives into a [`Compound`] and, if that compound reliably works, add it
//! to what it can do.
//!
//! ```text
//! "I cannot produce concept_19 with any one action I have."
//!      -> search combinations of the actions I do have
//!      -> test the promising one
//!      -> it works 84% of the time
//!      -> concept_19 is now something I can bring about on purpose
//! ```
//!
//! # Why composition and not code generation
//!
//! Section 8 of the design imagines the machine writing a scheduling policy, a
//! memory allocator, or an execution shim. Those are real possibilities and this
//! is not them.
//!
//! Composition is the form of invention that can be made **safe and honest**
//! here. Every compound is built from primitives that are already scoped,
//! bounded, rate-limited, reversible and audited, so a compound inherits all of
//! that by construction: it cannot exceed the authority of its parts. A
//! generated allocator would inherit none of it, and the audit trail would be
//! describing code no reviewer had seen.
//!
//! So this is genuinely a new move, and genuinely a restricted kind of new move.
//! Both halves of that are worth being clear about, because "the machine wrote
//! its own scheduler" is a much more exciting sentence than the one that is
//! actually true.
//!
//! # A compound is a hypothesis until it is tested
//!
//! [`Inventory::propose`] returns a candidate. It is not usable until it has
//! been tried enough times to have a measured success rate, and one that fails
//! is discarded rather than kept in the hope that it was unlucky.

use serde::{Deserialize, Serialize};

use crate::ActionKind;

/// A move built from moves.
///
/// Ordered: the steps are applied in sequence, and the order is part of the
/// invention. Setting an affinity and then a priority is not always the same as
/// the reverse, because the first may change which CPU the second applies on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Compound {
    /// The machine's own name for it: `move_3`.
    pub name: String,
    /// The primitives, in order.
    pub steps: Vec<ActionKind>,
    /// What the machine built it to bring about, in its own vocabulary.
    pub intended: String,
    /// Why it was invented: what it could not do before.
    pub because: String,
    attempts: u32,
    successes: u32,
    pub invented_ns: u64,
    pub discarded: Option<String>,
}

impl Compound {
    /// How often it achieves what it was built for.
    ///
    /// `None` until there is enough evidence. A compound that worked twice has
    /// no success rate, and treating two successes as proof is how an invented
    /// action becomes a superstition.
    pub fn success_rate(&self) -> Option<f64> {
        const ENOUGH: u32 = 8;
        (self.attempts >= ENOUGH).then(|| self.successes as f64 / self.attempts as f64)
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Whether it may be used.
    ///
    /// Requires a measured rate that clears the bar. An untested compound is
    /// not usable, which is the difference between inventing a move and merely
    /// imagining one.
    pub fn is_usable(&self, min_rate: f64) -> bool {
        self.discarded.is_none() && self.success_rate().is_some_and(|rate| rate >= min_rate)
    }

    pub fn record(&mut self, succeeded: bool) {
        self.attempts += 1;
        if succeeded {
            self.successes += 1;
        }
    }

    /// The family name, so the effect model can learn about it as a unit.
    ///
    /// A compound is its own family, not the concatenation of its parts'
    /// families: the whole point is that it does something the parts do not do
    /// separately, so learning about it as `affinity+priority` would attribute
    /// its effects to the wrong things.
    pub fn family(&self) -> &str {
        &self.name
    }

    pub fn describe(&self) -> String {
        let steps = self
            .steps
            .iter()
            .map(|s| s.family())
            .collect::<Vec<_>>()
            .join(" then ");
        let record = match self.success_rate() {
            Some(rate) => format!("{:.0}% over {} attempts", rate * 100.0, self.attempts),
            None => format!("untested ({} attempts)", self.attempts),
        };
        let mut text = format!(
            "{}: {steps}, to bring about {} ({record}). Invented because {}",
            self.name, self.intended, self.because
        );
        if let Some(reason) = &self.discarded {
            text.push_str(&format!("  [discarded: {reason}]"));
        }
        text
    }
}

/// What the machine could not do, stated precisely enough to search for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gap {
    /// What it wants to bring about, in its own vocabulary.
    pub target: String,
    /// The primitive families already tried, and found insufficient.
    pub tried: Vec<String>,
    /// How close the best single primitive got, 0 to 1.
    pub best_single: f64,
}

impl Gap {
    /// Whether this gap is worth trying to close by invention.
    ///
    /// Not worth it when a single primitive already nearly works: composing
    /// would add complexity, more failure modes and more audit surface to buy
    /// very little. Also not worth it when nothing was tried, because then the
    /// gap is untested rather than real.
    pub fn worth_inventing_for(&self) -> bool {
        !self.tried.is_empty() && self.best_single < 0.6
    }
}

/// How invention is governed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InventionRules {
    /// Longest compound that may be built.
    ///
    /// Short on purpose. A long sequence is hard to attribute, hard to revert
    /// cleanly, and its failures are hard to explain.
    pub max_steps: usize,
    /// Success rate a compound must reach to be usable.
    pub min_success_rate: f64,
    /// Attempts after which a failing compound is discarded.
    pub give_up_after: u32,
    /// Compounds to keep at once.
    pub max_compounds: usize,
}

impl Default for InventionRules {
    fn default() -> Self {
        InventionRules {
            max_steps: 3,
            min_success_rate: 0.7,
            give_up_after: 16,
            max_compounds: 16,
        }
    }
}

/// The moves the machine has invented for itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inventory {
    rules: InventionRules,
    compounds: Vec<Compound>,
    next: u32,
    proposed: u64,
    discarded: u64,
}

impl Default for Inventory {
    fn default() -> Self {
        Inventory::new(InventionRules::default())
    }
}

impl Inventory {
    pub fn new(rules: InventionRules) -> Inventory {
        Inventory {
            rules,
            compounds: Vec::new(),
            next: 0,
            proposed: 0,
            discarded: 0,
        }
    }

    pub fn rules(&self) -> &InventionRules {
        &self.rules
    }

    pub fn compounds(&self) -> &[Compound] {
        &self.compounds
    }

    /// Compounds that have earned the right to be used.
    pub fn usable(&self) -> Vec<&Compound> {
        self.compounds
            .iter()
            .filter(|c| c.is_usable(self.rules.min_success_rate))
            .collect()
    }

    /// Compounds still being tested.
    pub fn under_test(&self) -> Vec<&Compound> {
        self.compounds
            .iter()
            .filter(|c| c.discarded.is_none() && c.success_rate().is_none())
            .collect()
    }

    pub fn get(&self, name: &str) -> Option<&Compound> {
        self.compounds.iter().find(|c| c.name == name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Compound> {
        self.compounds.iter_mut().find(|c| c.name == name)
    }

    pub fn proposed(&self) -> u64 {
        self.proposed
    }

    pub fn discarded_count(&self) -> u64 {
        self.discarded
    }

    /// Invent a move to close a gap.
    ///
    /// `available` is the primitive actions the machine may use, which is
    /// already bounded by scope and policy. A compound cannot contain anything
    /// that is not in it, so invention cannot widen the machine's authority: it
    /// can only rearrange what it was already allowed to do.
    ///
    /// Returns `None` when the gap is not worth closing, when there is nothing
    /// to build from, or when the same compound already exists.
    pub fn propose(
        &mut self,
        gap: &Gap,
        available: &[ActionKind],
        now_ns: u64,
    ) -> Option<&Compound> {
        if !gap.worth_inventing_for() {
            return None;
        }
        let usable: Vec<&ActionKind> = available
            .iter()
            .filter(|a| !matches!(a, ActionKind::Hold))
            .collect();
        if usable.len() < 2 {
            // Nothing to compose. One primitive is not a compound.
            return None;
        }
        if self
            .compounds
            .iter()
            .filter(|c| c.discarded.is_none())
            .count()
            >= self.rules.max_compounds
        {
            return None;
        }

        // Take primitives from distinct families. Two affinity changes in a row
        // is not a new move, it is the second one; the interesting compounds
        // combine mechanisms that act on different things.
        let mut steps: Vec<ActionKind> = Vec::new();
        let mut families: Vec<&str> = Vec::new();
        for action in usable {
            if families.contains(&action.family()) {
                continue;
            }
            families.push(action.family());
            steps.push(action.clone());
            if steps.len() >= self.rules.max_steps {
                break;
            }
        }
        if steps.len() < 2 {
            return None;
        }

        // The same combination is not invented twice.
        if self
            .compounds
            .iter()
            .any(|c| c.discarded.is_none() && c.steps == steps)
        {
            return None;
        }

        self.next += 1;
        self.proposed += 1;
        let name = format!("move_{}", self.next);
        self.compounds.push(Compound {
            name: name.clone(),
            steps,
            intended: gap.target.clone(),
            because: format!(
                "no single action reached {} (best got {:.0}% of the way, over {} \
                 families tried)",
                gap.target,
                gap.best_single * 100.0,
                gap.tried.len()
            ),
            attempts: 0,
            successes: 0,
            invented_ns: now_ns,
            discarded: None,
        });
        self.compounds.last()
    }

    /// Record how an invented move performed.
    pub fn record(&mut self, name: &str, succeeded: bool) -> bool {
        let give_up_after = self.rules.give_up_after;
        let min_rate = self.rules.min_success_rate;
        let Some(compound) = self.get_mut(name) else {
            return false;
        };
        if compound.discarded.is_some() {
            return false;
        }
        compound.record(succeeded);
        if compound.attempts >= give_up_after {
            if let Some(rate) = compound.success_rate() {
                if rate < min_rate {
                    compound.discarded = Some(format!(
                        "worked {:.0}% of {} attempts, below the {:.0}% required",
                        rate * 100.0,
                        compound.attempts,
                        min_rate * 100.0
                    ));
                    self.discarded += 1;
                }
            }
        }
        true
    }

    /// A report a person can read.
    pub fn render(&self) -> String {
        if self.compounds.is_empty() {
            return "the machine has invented no new moves: every target it wanted was \
                    reachable with an action it already had\n"
                .into();
        }
        let mut out = format!(
            "{} moves invented, {} discarded\n",
            self.proposed, self.discarded
        );
        for compound in &self.compounds {
            out.push_str(&format!("  {}\n", compound.describe()));
        }
        out
    }
}

/// Turn a compound into the actions to perform, in order.
///
/// A plain accessor by design. Nothing here decides *whether* to perform them:
/// that goes through the same scope, bounds, rate limit, confirmation and audit
/// path as any primitive, which is what keeps a compound within the authority of
/// its parts.
pub fn steps_of(compound: &Compound) -> &[ActionKind] {
    &compound.steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Target;
    use corescout_core::CpuSet;

    fn primitives() -> Vec<ActionKind> {
        vec![
            ActionKind::Hold,
            ActionKind::SetAffinity {
                target: Target::CurrentThread,
                cpus: [1u32].into_iter().collect::<CpuSet>(),
            },
            ActionKind::SetPriority {
                target: Target::CurrentThread,
                nice: 5,
            },
            ActionKind::SetNumaPreference {
                target: Target::CurrentThread,
                node: Some(0),
            },
        ]
    }

    fn gap() -> Gap {
        Gap {
            target: "concept_19".into(),
            tried: vec!["affinity".into(), "priority".into()],
            best_single: 0.2,
        }
    }

    #[test]
    fn a_gap_a_single_action_nearly_closes_is_not_worth_inventing_for() {
        // Composing to buy 5% is more failure modes and more audit surface for
        // very little.
        let nearly = Gap {
            best_single: 0.95,
            ..gap()
        };
        assert!(!nearly.worth_inventing_for());
        let mut inventory = Inventory::default();
        assert!(inventory.propose(&nearly, &primitives(), 0).is_none());
    }

    #[test]
    fn an_untested_gap_is_not_worth_inventing_for() {
        // Nothing tried means the gap is unmeasured, not real.
        let untested = Gap {
            tried: Vec::new(),
            ..gap()
        };
        assert!(!untested.worth_inventing_for());
    }

    #[test]
    fn a_real_gap_produces_a_compound_with_a_machine_name() {
        let mut inventory = Inventory::default();
        let compound = inventory
            .propose(&gap(), &primitives(), 100)
            .expect("invented");
        assert_eq!(compound.name, "move_1");
        assert_eq!(compound.intended, "concept_19");
        assert!(compound.steps.len() >= 2);
        assert!(compound.because.contains("no single action"));
    }

    #[test]
    fn an_invented_move_cannot_contain_anything_it_was_not_offered() {
        // The property that keeps invention inside the machine's existing
        // authority: a compound rearranges what it was already allowed to do.
        let offered = vec![
            ActionKind::SetAffinity {
                target: Target::CurrentThread,
                cpus: [1u32].into_iter().collect::<CpuSet>(),
            },
            ActionKind::SetPriority {
                target: Target::CurrentThread,
                nice: 5,
            },
        ];
        let mut inventory = Inventory::default();
        let compound = inventory.propose(&gap(), &offered, 0).expect("invented");
        for step in &compound.steps {
            assert!(offered.contains(step), "{step:?} was never offered");
        }
    }

    #[test]
    fn hold_is_never_a_step_in_an_invented_move() {
        let mut inventory = Inventory::default();
        let compound = inventory
            .propose(&gap(), &primitives(), 0)
            .expect("invented");
        assert!(!compound.steps.iter().any(|s| matches!(s, ActionKind::Hold)));
    }

    #[test]
    fn a_compound_combines_distinct_families() {
        // Two affinity changes in a row is not a new move, it is the second one.
        let mut inventory = Inventory::default();
        let compound = inventory
            .propose(&gap(), &primitives(), 0)
            .expect("invented");
        let mut families: Vec<&str> = compound.steps.iter().map(|s| s.family()).collect();
        let count = families.len();
        families.sort_unstable();
        families.dedup();
        assert_eq!(families.len(), count, "a family repeated in one compound");
    }

    #[test]
    fn nothing_is_invented_from_a_single_primitive() {
        let mut inventory = Inventory::default();
        let one = vec![ActionKind::SetPriority {
            target: Target::CurrentThread,
            nice: 5,
        }];
        assert!(inventory.propose(&gap(), &one, 0).is_none());
    }

    #[test]
    fn an_invented_move_is_not_usable_until_it_has_been_tested() {
        // The difference between inventing a move and imagining one.
        let mut inventory = Inventory::default();
        inventory
            .propose(&gap(), &primitives(), 0)
            .expect("invented");
        assert!(inventory.usable().is_empty());
        assert_eq!(inventory.under_test().len(), 1);

        for _ in 0..2 {
            inventory.record("move_1", true);
        }
        assert!(
            inventory.usable().is_empty(),
            "two successes is not a success rate"
        );
    }

    #[test]
    fn a_move_that_works_becomes_usable() {
        let mut inventory = Inventory::default();
        inventory
            .propose(&gap(), &primitives(), 0)
            .expect("invented");
        for _ in 0..10 {
            inventory.record("move_1", true);
        }
        assert_eq!(inventory.usable().len(), 1);
        assert!(inventory.get("move_1").unwrap().describe().contains("100%"));
    }

    #[test]
    fn a_move_that_does_not_work_is_discarded_rather_than_kept_hopefully() {
        let mut inventory = Inventory::default();
        inventory
            .propose(&gap(), &primitives(), 0)
            .expect("invented");
        for i in 0..20 {
            inventory.record("move_1", i % 5 == 0);
        }
        let compound = inventory.get("move_1").expect("present");
        assert!(compound.discarded.is_some(), "{}", compound.describe());
        assert!(inventory.usable().is_empty());
        assert_eq!(inventory.discarded_count(), 1);
    }

    #[test]
    fn a_discarded_move_accepts_no_further_evidence() {
        let mut inventory = Inventory::default();
        inventory
            .propose(&gap(), &primitives(), 0)
            .expect("invented");
        for _ in 0..20 {
            inventory.record("move_1", false);
        }
        let before = inventory.get("move_1").unwrap().attempts();
        assert!(!inventory.record("move_1", true));
        assert_eq!(inventory.get("move_1").unwrap().attempts(), before);
    }

    #[test]
    fn the_same_combination_is_not_invented_twice() {
        let mut inventory = Inventory::default();
        inventory
            .propose(&gap(), &primitives(), 0)
            .expect("invented");
        assert!(inventory.propose(&gap(), &primitives(), 0).is_none());
    }

    #[test]
    fn a_compound_is_its_own_family_not_its_parts_concatenated() {
        // The point of a compound is that it does something its parts do not do
        // separately; attributing its effects to `affinity` would credit the
        // wrong mechanism.
        let mut inventory = Inventory::default();
        let compound = inventory
            .propose(&gap(), &primitives(), 0)
            .expect("invented");
        assert_eq!(compound.family(), "move_1");
        for step in &compound.steps {
            assert_ne!(compound.family(), step.family());
        }
    }

    #[test]
    fn compounds_are_capped() {
        let mut inventory = Inventory::new(InventionRules {
            max_compounds: 1,
            ..InventionRules::default()
        });
        inventory
            .propose(&gap(), &primitives(), 0)
            .expect("invented");
        let other = Gap {
            target: "concept_20".into(),
            ..gap()
        };
        assert!(inventory.propose(&other, &primitives(), 0).is_none());
    }

    #[test]
    fn compounds_respect_the_length_limit() {
        let mut inventory = Inventory::new(InventionRules {
            max_steps: 2,
            ..InventionRules::default()
        });
        let compound = inventory
            .propose(&gap(), &primitives(), 0)
            .expect("invented");
        assert_eq!(compound.steps.len(), 2);
    }

    #[test]
    fn an_inventory_that_never_needed_a_new_move_says_so() {
        assert!(Inventory::default()
            .render()
            .contains("reachable with an action it already had"));
    }

    #[test]
    fn an_inventory_round_trips_through_json() {
        let mut inventory = Inventory::default();
        inventory
            .propose(&gap(), &primitives(), 0)
            .expect("invented");
        inventory.record("move_1", true);
        let text = serde_json::to_string(&inventory).expect("serialises");
        let back: Inventory = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back, inventory);
    }
}
