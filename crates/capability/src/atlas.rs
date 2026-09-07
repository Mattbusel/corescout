//! The map: `G = (S, A, T)`.
//!
//! # What it is
//!
//! States the machine has found in itself, actions it can take, and transitions
//! it has measured between them. Every edge carries what it cost, how reliably
//! it worked, how long it took, and how it failed when it failed.
//!
//! ```text
//!                    affinity, 94%, cost 1.2
//!     concept_14  ------------------------->  concept_31
//!                    priority, 31%, cost 0.4
//!                 ------------------------->  concept_08
//! ```
//!
//! # Why the map and not the routes
//!
//! A capability is a route. A map is worth more than the routes drawn on it,
//! because it also says what is unreachable, what is reachable only expensively,
//! and where the unexplored edges are.
//!
//! A computer out of the box has an enormous space of physically possible
//! configurations and a small human-given vocabulary for talking about them. It
//! can be told about cores, threads, priorities and NUMA nodes. It has no
//! account at all of what *it specifically* can be made to become. The atlas is
//! the record of converting the first into the second, one measured transition
//! at a time.
//!
//! # Multi-hop
//!
//! [`Atlas::routes_from`] walks the graph, so a state two moves away counts as
//! reachable even when no single action gets there. This is where the map earns
//! its keep over a list of capabilities: the machine can discover that it can
//! get somewhere without ever having discovered a procedure that goes there
//! directly.
//!
//! # The number, and what is wrong with it
//!
//! [`Atlas::command`] is a single figure for *how much command does this machine
//! have over itself*. It is useful for watching one machine change over time and
//! is close to meaningless between machines, for reasons its own documentation
//! sets out at length. It is reported with those caveats attached or not at all.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::affordance::{Affordance, AffordanceId, Transition};
use crate::capability::{Capability, CapabilityId, Step};

/// A state the machine has found in itself, and what being there is worth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// The machine's own name for it.
    pub name: String,
    /// How valuable being here is, supplied by whatever measures value.
    ///
    /// **Not computed here.** An atlas that scored the worth of its own states
    /// would be marking its own homework, and every state would turn out to be
    /// valuable. Utility comes from outside, or is `None` and the state counts
    /// for nothing in [`Atlas::command`].
    pub utility: Option<f64>,
    pub visits: u64,
    pub first_seen_ns: u64,
    pub last_seen_ns: u64,
}

/// A way of getting somewhere, possibly in several moves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub to: String,
    /// The affordances traversed, in order.
    pub hops: Vec<AffordanceId>,
    /// Probability of arriving, over the whole route.
    ///
    /// The product of the hops, which assumes they are independent. That
    /// assumption is usually wrong and is flagged rather than hidden: see
    /// [`Route::is_measured`].
    pub reliability: f64,
    pub expected_cost: f64,
    /// True when the whole route has been run end to end and measured, rather
    /// than assembled from its parts.
    pub is_measured: bool,
}

impl Route {
    pub fn hops(&self) -> usize {
        self.hops.len()
    }

    pub fn describe(&self) -> String {
        let confidence = if self.is_measured {
            "measured end to end"
        } else {
            "assembled from single hops, never run as a whole"
        };
        format!(
            "{} in {} hop(s), {:.0}% reliable, expected cost {:.2} ({confidence})",
            self.to,
            self.hops.len(),
            self.reliability * 100.0,
            self.expected_cost
        )
    }
}

/// A summary of how much command a machine has over itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Command {
    /// States the machine knows exist.
    pub known_states: usize,
    /// States it can reliably reach from where it usually is.
    pub reachable_states: usize,
    /// Of those, how many need more than one move.
    pub multi_hop_states: usize,
    /// Measured edges.
    pub affordances: usize,
    /// Affordances promoted to invocable capabilities.
    pub capabilities: usize,
    /// The weighted figure. See [`Atlas::command`] for what is wrong with it.
    pub score: f64,
    /// States with no utility supplied, and therefore counting for nothing.
    pub unvalued_states: usize,
}

impl Command {
    pub fn describe(&self) -> String {
        format!(
            "{} states known, {} reachable ({} needing more than one move), \
             {} measured edges, {} invocable capabilities; command score {:.2}\n\
             {} states have no utility supplied and count for nothing in that score",
            self.known_states,
            self.reachable_states,
            self.multi_hop_states,
            self.affordances,
            self.capabilities,
            self.score,
            self.unvalued_states
        )
    }
}

/// Everything the machine has learned about what it can become.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Atlas {
    /// Probability below which an edge is not counted as a way of getting
    /// anywhere.
    edge_threshold: f64,
    /// Longest route the atlas will assemble.
    max_hops: usize,
    #[serde(with = "corescout_core::serde_util::pairs")]
    nodes: BTreeMap<String, Node>,
    #[serde(with = "corescout_core::serde_util::pairs")]
    affordances: BTreeMap<String, Affordance>,
    capabilities: Vec<Capability>,
    next_affordance: u32,
    next_capability: u32,
}

impl Default for Atlas {
    fn default() -> Self {
        Atlas::new(0.7, 3)
    }
}

impl Atlas {
    pub fn new(edge_threshold: f64, max_hops: usize) -> Atlas {
        Atlas {
            edge_threshold: edge_threshold.clamp(0.0, 1.0),
            max_hops: max_hops.max(1),
            nodes: BTreeMap::new(),
            affordances: BTreeMap::new(),
            capabilities: Vec::new(),
            next_affordance: 0,
            next_capability: 0,
        }
    }

    pub fn edge_threshold(&self) -> f64 {
        self.edge_threshold
    }

    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    pub fn affordances(&self) -> impl Iterator<Item = &Affordance> {
        self.affordances.values()
    }

    pub fn capabilities(&self) -> &[Capability] {
        &self.capabilities
    }

    pub fn node(&self, name: &str) -> Option<&Node> {
        self.nodes.get(name)
    }

    pub fn known_states(&self) -> usize {
        self.nodes.len()
    }

    /// Note that the machine is in a state.
    pub fn visit(&mut self, state: &str, now_ns: u64) {
        let node = self.nodes.entry(state.to_string()).or_insert(Node {
            name: state.to_string(),
            utility: None,
            visits: 0,
            first_seen_ns: now_ns,
            last_seen_ns: now_ns,
        });
        node.visits += 1;
        node.last_seen_ns = now_ns;
    }

    /// Supply what being in a state is worth.
    ///
    /// From outside, always. See [`Node::utility`].
    pub fn value(&mut self, state: &str, utility: f64) {
        if let Some(node) = self.nodes.get_mut(state) {
            if utility.is_finite() {
                node.utility = Some(utility);
            }
        }
    }

    /// Record one transition.
    ///
    /// # Why this touches every edge from `(from, action)`
    ///
    /// An edge is the claim *from here, this action lands me there*. Testing it
    /// requires counting the times the action was taken from that state and the
    /// times it arrived, and those are different numbers.
    ///
    /// Keying the record by where the transition actually arrived would file
    /// every observation against the edge named by its own outcome, so every
    /// edge would report a hundred per cent and the whole atlas would be a lie.
    /// Instead the outcome is recorded against *all* known edges leaving
    /// `(from, action)`: an observation for each, an arrival for the one that
    /// matches.
    pub fn observe(&mut self, from: &str, action: &str, transition: Transition) -> AffordanceId {
        self.visit(from, transition.at_ns);
        self.visit(&transition.arrived, transition.at_ns);

        let arrived_key = format!("{from}--{action}-->{}", transition.arrived);
        if !self.affordances.contains_key(&arrived_key) {
            // How many times this action has been taken from this state before
            // anyone knew it could land here. Every one of those was a
            // non-arrival for this outcome.
            let prior = self
                .affordances
                .values()
                .filter(|a| a.from == from && a.action == action)
                .map(|a| a.observations())
                .max()
                .unwrap_or(0);
            self.next_affordance += 1;
            let id = AffordanceId(self.next_affordance);
            let mut fresh =
                Affordance::new(id, from, action, &transition.arrived, transition.at_ns);
            fresh.backfill(prior);
            self.affordances.insert(arrived_key.clone(), fresh);
        }

        // Every edge from this state under this action saw this attempt.
        let siblings: Vec<String> = self
            .affordances
            .iter()
            .filter(|(_, a)| a.from == from && a.action == action)
            .map(|(key, _)| key.clone())
            .collect();
        for key in siblings {
            if let Some(affordance) = self.affordances.get_mut(&key) {
                affordance.record(&transition);
            }
        }

        self.affordances
            .get(&arrived_key)
            .map(|a| a.id)
            .expect("just inserted")
    }

    /// Edges leaving a state that are reliable enough to count.
    pub fn edges_from(&self, state: &str) -> Vec<&Affordance> {
        self.affordances
            .values()
            .filter(|a| a.from == state && a.is_edge(self.edge_threshold))
            .collect()
    }

    /// Everywhere the machine can get from here, and how.
    ///
    /// Breadth-first over the edges, keeping the cheapest route to each state.
    /// A route of more than one hop is marked as assembled rather than measured,
    /// because the hops were each measured separately and their product assumes
    /// an independence nobody has checked.
    pub fn routes_from(&self, start: &str) -> BTreeMap<String, Route> {
        let mut best: BTreeMap<String, Route> = BTreeMap::new();
        let mut queue: VecDeque<(String, Vec<AffordanceId>, f64, f64)> = VecDeque::new();
        queue.push_back((start.to_string(), Vec::new(), 1.0, 0.0));
        let mut seen: BTreeSet<String> = BTreeSet::new();
        seen.insert(start.to_string());

        while let Some((here, hops, reliability, cost)) = queue.pop_front() {
            if hops.len() >= self.max_hops {
                continue;
            }
            for edge in self.edges_from(&here) {
                let (Some(probability), Some(step_cost)) =
                    (edge.probability(), edge.expected_cost())
                else {
                    continue;
                };
                if edge.to == start {
                    continue;
                }
                let mut route_hops = hops.clone();
                route_hops.push(edge.id);
                let route = Route {
                    to: edge.to.clone(),
                    reliability: reliability * probability,
                    expected_cost: cost + step_cost,
                    is_measured: route_hops.len() == 1,
                    hops: route_hops.clone(),
                };
                let better = match best.get(&edge.to) {
                    None => true,
                    Some(existing) => route.expected_cost < existing.expected_cost,
                };
                if better {
                    best.insert(edge.to.clone(), route.clone());
                }
                if seen.insert(edge.to.clone()) {
                    queue.push_back((
                        edge.to.clone(),
                        route_hops,
                        route.reliability,
                        route.expected_cost,
                    ));
                }
            }
        }
        best
    }

    /// Whether the machine can get from one state to another at all.
    pub fn can_reach(&self, from: &str, to: &str) -> bool {
        self.routes_from(from).contains_key(to)
    }

    /// Promote an affordance to an invocable capability.
    ///
    /// Refuses unless the affordance's own [`crate::affordance::Affordance::promotable`]
    /// check passes, which requires randomised evidence. Watching a transition
    /// happen is not evidence of being able to cause it, and this is the point
    /// at which the machine would start acting on the difference.
    pub fn promote(&mut self, id: AffordanceId, now_ns: u64) -> Result<CapabilityId, String> {
        let affordance = self
            .affordances
            .values()
            .find(|a| a.id == id)
            .ok_or_else(|| format!("{id} is not in the atlas"))?;
        affordance
            .promotable(self.edge_threshold)
            .map_err(|why| why.describe())?;
        if self.capabilities.iter().any(|c| {
            c.target == affordance.to && c.precondition.as_deref() == Some(&affordance.from)
        }) {
            return Err(format!(
                "{} is already reachable from {} by an existing capability",
                affordance.to, affordance.from
            ));
        }

        self.next_capability += 1;
        let capability_id = CapabilityId(self.next_capability);
        let capability = Capability::new(
            capability_id,
            affordance.to.clone(),
            Some(affordance.from.clone()),
            vec![Step::new(affordance.action.clone(), None)],
            now_ns,
        );
        self.capabilities.push(capability);
        Ok(capability_id)
    }

    pub fn capability_mut(&mut self, id: CapabilityId) -> Option<&mut Capability> {
        self.capabilities.iter_mut().find(|c| c.id == id)
    }

    /// Capabilities that are still delivering.
    pub fn usable_capabilities(&self, min_reliability: f64) -> Vec<&Capability> {
        self.capabilities
            .iter()
            .filter(|c| c.is_usable(min_reliability))
            .collect()
    }

    /// How much command the machine has over itself.
    ///
    /// ```text
    /// score = sum over reachable s of  utility(s) * reliability(s) / cost(s)
    /// ```
    ///
    /// # What is wrong with this number
    ///
    /// **It is not comparable between machines** unless they share a utility
    /// function, and utility is supplied from outside precisely because the
    /// atlas must not invent it. Two machines with different notions of what is
    /// worth reaching will produce incomparable scores.
    ///
    /// **It rewards knowing about many cheap states** over being able to reach
    /// one important one. A machine that discovers fifty trivial states can
    /// score higher than one that discovered a single valuable one.
    ///
    /// **It ignores contention.** Two states may each be reachable and
    /// impossible to hold at once, and nothing here notices, so the score is an
    /// upper bound on what the machine can do simultaneously.
    ///
    /// **It says nothing about whether the states are worth wanting.** That is
    /// what utility is for, and unvalued states are excluded rather than
    /// assumed to be worth something, which is why [`Command::unvalued_states`]
    /// is reported alongside.
    ///
    /// It is useful for one thing: watching a single machine's figure move as it
    /// accumulates experience.
    pub fn command(&self, from: &str) -> Command {
        let routes = self.routes_from(from);
        let mut score = 0.0;
        let mut unvalued = 0;
        for (state, route) in &routes {
            match self.nodes.get(state).and_then(|node| node.utility) {
                Some(utility) if route.expected_cost > 0.0 => {
                    score += utility * route.reliability / route.expected_cost;
                }
                Some(_) => {}
                None => unvalued += 1,
            }
        }
        Command {
            known_states: self.nodes.len(),
            reachable_states: routes.len(),
            multi_hop_states: routes.values().filter(|r| r.hops() > 1).count(),
            affordances: self
                .affordances
                .values()
                .filter(|a| a.is_edge(self.edge_threshold))
                .count(),
            capabilities: self.usable_capabilities(self.edge_threshold).len(),
            score,
            unvalued_states: unvalued,
        }
    }

    pub fn render(&self, from: &str) -> String {
        let command = self.command(from);
        let mut out = format!("{}\n\n", command.describe());
        if self.affordances.is_empty() {
            out.push_str("no transitions have been measured\n");
            return out;
        }
        out.push_str("edges:\n");
        for affordance in self.affordances.values() {
            if affordance.is_edge(self.edge_threshold) {
                out.push_str(&format!("  {}\n", affordance.describe()));
            }
        }
        let routes = self.routes_from(from);
        if !routes.is_empty() {
            out.push_str(&format!("\nfrom {from}:\n"));
            for route in routes.values() {
                out.push_str(&format!("  {}\n", route.describe()));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Failure;

    fn arrive(atlas: &mut Atlas, from: &str, action: &str, to: &str, times: u32, randomised: bool) {
        for i in 0..times {
            atlas.observe(
                from,
                action,
                Transition {
                    at_ns: i as u64 * 1_000_000,
                    arrived: to.to_string(),
                    cost: 1.0,
                    latency_ns: 500_000,
                    randomised,
                    failure: None,
                },
            );
        }
    }

    /// The action was taken and the machine ended up somewhere else.
    fn miss(atlas: &mut Atlas, from: &str, action: &str, times: u32) {
        for i in 0..times {
            atlas.observe(
                from,
                action,
                Transition {
                    at_ns: i as u64,
                    arrived: "elsewhere".to_string(),
                    cost: 1.0,
                    latency_ns: 0,
                    randomised: true,
                    failure: Some(Failure::DidNotArrive),
                },
            );
        }
    }

    #[test]
    fn an_empty_atlas_knows_nothing() {
        let atlas = Atlas::default();
        assert_eq!(atlas.known_states(), 0);
        assert!(atlas.render("anywhere").contains("no transitions"));
    }

    #[test]
    fn observing_transitions_builds_the_graph() {
        let mut atlas = Atlas::default();
        arrive(&mut atlas, "s1", "affinity", "s2", 20, false);
        assert_eq!(atlas.known_states(), 2);
        assert_eq!(atlas.edges_from("s1").len(), 1);
        assert!(atlas.can_reach("s1", "s2"));
        assert!(!atlas.can_reach("s2", "s1"));
    }

    #[test]
    fn an_unreliable_edge_is_not_a_way_of_getting_anywhere() {
        let mut atlas = Atlas::default();
        arrive(&mut atlas, "s1", "affinity", "s2", 5, false);
        miss(&mut atlas, "s1", "affinity", 15);
        // s2 is reached a quarter of the time, so it is not reachable.
        assert!(!atlas.can_reach("s1", "s2"));
        // "elsewhere" is reached three quarters of the time, so it *is*: an
        // action that reliably takes the machine somewhere it did not intend is
        // still a measured edge, and the atlas records what happens rather than
        // what was wanted.
        assert!(atlas.can_reach("s1", "elsewhere"));
        assert_eq!(atlas.edges_from("s1").len(), 1);
    }

    #[test]
    fn every_edge_from_one_action_sees_every_attempt() {
        // The accounting that makes a probability mean anything. Filing an
        // observation only against the edge it happened to arrive at would give
        // every edge a hundred per cent by construction.
        let mut atlas = Atlas::default();
        arrive(&mut atlas, "s1", "affinity", "s2", 15, false);
        miss(&mut atlas, "s1", "affinity", 5);
        let to_s2 = atlas
            .affordances()
            .find(|a| a.to == "s2")
            .expect("the intended edge");
        let to_elsewhere = atlas
            .affordances()
            .find(|a| a.to == "elsewhere")
            .expect("the other outcome");
        assert_eq!(to_s2.observations(), 20);
        assert_eq!(to_elsewhere.observations(), 20);
        assert!((to_s2.probability().unwrap() - 0.75).abs() < 1e-9);
        assert!((to_elsewhere.probability().unwrap() - 0.25).abs() < 1e-9);
        let total: f64 = atlas
            .affordances()
            .filter(|a| a.from == "s1" && a.action == "affinity")
            .filter_map(|a| a.probability())
            .sum();
        assert!(
            (total - 1.0).abs() < 1e-9,
            "outcomes should partition, got {total}"
        );
    }

    #[test]
    fn the_atlas_finds_a_state_two_moves_away() {
        // Where a map earns its keep over a list of routes: the machine can get
        // somewhere it never discovered a direct procedure for.
        let mut atlas = Atlas::default();
        arrive(&mut atlas, "s1", "affinity", "s2", 20, false);
        arrive(&mut atlas, "s2", "priority", "s3", 20, false);
        assert!(atlas.can_reach("s1", "s3"));
        let routes = atlas.routes_from("s1");
        let route = routes.get("s3").expect("a two-hop route");
        assert_eq!(route.hops(), 2);
    }

    #[test]
    fn an_assembled_route_says_it_has_never_been_run_as_a_whole() {
        // Its reliability is the product of hops measured separately, which
        // assumes an independence nobody checked.
        let mut atlas = Atlas::default();
        arrive(&mut atlas, "s1", "affinity", "s2", 18, false);
        miss(&mut atlas, "s1", "affinity", 2);
        arrive(&mut atlas, "s2", "priority", "s3", 18, false);
        miss(&mut atlas, "s2", "priority", 2);
        let routes = atlas.routes_from("s1");
        let one_hop = routes.get("s2").expect("direct");
        assert!(one_hop.is_measured);
        let two_hop = routes.get("s3").expect("indirect");
        assert!(!two_hop.is_measured);
        assert!(two_hop.describe().contains("never run as a whole"));
        assert!(two_hop.reliability < one_hop.reliability);
    }

    #[test]
    fn routes_respect_the_hop_limit() {
        let mut atlas = Atlas::new(0.7, 1);
        arrive(&mut atlas, "s1", "affinity", "s2", 20, false);
        arrive(&mut atlas, "s2", "priority", "s3", 20, false);
        assert!(atlas.can_reach("s1", "s2"));
        assert!(
            !atlas.can_reach("s1", "s3"),
            "s3 is two hops and the limit is one"
        );
    }

    #[test]
    fn watching_a_transition_does_not_make_it_promotable() {
        // The rule that separates the middle layer from the top one.
        let mut atlas = Atlas::default();
        arrive(&mut atlas, "s1", "affinity", "s2", 100, false);
        let id = atlas.edges_from("s1")[0].id;
        let error = atlas.promote(id, 0).unwrap_err();
        assert!(error.contains("not evidence of being able"), "{error}");
        assert!(atlas.capabilities().is_empty());
    }

    #[test]
    fn intervening_makes_it_promotable() {
        let mut atlas = Atlas::default();
        arrive(&mut atlas, "s1", "affinity", "s2", 20, true);
        let id = atlas.edges_from("s1")[0].id;
        let capability = atlas.promote(id, 0).expect("promotable");
        assert_eq!(capability.to_string(), "capability_1");
        assert_eq!(atlas.capabilities().len(), 1);
        assert_eq!(
            atlas.capabilities()[0].precondition.as_deref(),
            Some("s1"),
            "a promoted capability knows where it works from"
        );
    }

    #[test]
    fn the_same_route_is_not_promoted_twice() {
        let mut atlas = Atlas::default();
        arrive(&mut atlas, "s1", "affinity", "s2", 20, true);
        arrive(&mut atlas, "s1", "priority", "s2", 20, true);
        let ids: Vec<AffordanceId> = atlas.edges_from("s1").iter().map(|a| a.id).collect();
        assert!(atlas.promote(ids[0], 0).is_ok());
        let second = atlas.promote(ids[1], 0).unwrap_err();
        assert!(second.contains("already reachable"), "{second}");
    }

    #[test]
    fn promoting_something_not_in_the_atlas_fails() {
        let mut atlas = Atlas::default();
        assert!(atlas.promote(AffordanceId(99), 0).is_err());
    }

    #[test]
    fn command_counts_only_states_whose_worth_was_supplied() {
        // An atlas that scored its own states would find them all valuable.
        let mut atlas = Atlas::default();
        arrive(&mut atlas, "s1", "affinity", "s2", 20, false);
        arrive(&mut atlas, "s1", "priority", "s3", 20, false);
        let before = atlas.command("s1");
        assert_eq!(before.score, 0.0);
        assert_eq!(before.unvalued_states, 2);

        atlas.value("s2", 10.0);
        let after = atlas.command("s1");
        assert!(after.score > 0.0);
        assert_eq!(after.unvalued_states, 1);
    }

    #[test]
    fn command_rises_as_the_machine_learns_more_about_itself() {
        // The quantity worth watching: one machine's figure over time.
        let mut atlas = Atlas::default();
        arrive(&mut atlas, "s1", "affinity", "s2", 20, false);
        atlas.value("s2", 10.0);
        let early = atlas.command("s1");

        arrive(&mut atlas, "s1", "priority", "s3", 20, false);
        atlas.value("s3", 10.0);
        arrive(&mut atlas, "s2", "numa", "s4", 20, false);
        atlas.value("s4", 10.0);
        let later = atlas.command("s1");

        assert!(later.reachable_states > early.reachable_states);
        assert!(later.score > early.score);
        assert!(later.multi_hop_states > 0);
    }

    #[test]
    fn a_cheaper_route_is_preferred() {
        let mut atlas = Atlas::default();
        for i in 0..20 {
            atlas.observe(
                "s1",
                "expensive",
                Transition {
                    at_ns: i,
                    arrived: "s2".into(),
                    cost: 10.0,
                    latency_ns: 0,
                    randomised: false,
                    failure: None,
                },
            );
            atlas.observe(
                "s1",
                "cheap",
                Transition {
                    at_ns: i,
                    arrived: "s2".into(),
                    cost: 1.0,
                    latency_ns: 0,
                    randomised: false,
                    failure: None,
                },
            );
        }
        let routes = atlas.routes_from("s1");
        assert!((routes["s2"].expected_cost - 1.0).abs() < 1e-9);
    }

    #[test]
    fn an_atlas_round_trips_through_json() {
        let mut atlas = Atlas::default();
        arrive(&mut atlas, "s1", "affinity", "s2", 20, true);
        atlas.value("s2", 3.0);
        let id = atlas.edges_from("s1")[0].id;
        atlas.promote(id, 0).expect("promotable");
        let text = serde_json::to_string(&atlas).expect("serialises");
        let back: Atlas = serde_json::from_str(&text).expect("deserialises");
        assert_eq!(back, atlas);
    }
}
