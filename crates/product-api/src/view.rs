//! What the screens are given.
//!
//! # Progressive disclosure is a data shape, not a CSS trick
//!
//! Every view here has a plain half and a technical half, and the plain half
//! is complete on its own: a sentence a person can read without knowing what a
//! latent state is. The technical half hangs off it. Doing this in the types
//! rather than in the interface means there is exactly one place where the
//! plain wording lives, and a screen cannot accidentally render the technical
//! half by default.
//!
//! # Numbers carry their own absence
//!
//! Every rate and reliability is an `Option`. Zero out of zero is not zero
//! percent, and a fresh install that reports 0% failures and 100% reliability
//! is lying about both.

use serde::{Deserialize, Serialize};

/// Whether the service is up, and what it is attached to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Status {
    /// Always true if this was answered at all.
    pub running: bool,
    /// The build.
    pub version: String,
    /// Milliseconds since the service started.
    pub uptime_ms: u64,
    /// Milliseconds since CoreScout first ran on this machine.
    pub learning_for_ms: u64,
    /// The autonomy mode, by name.
    pub autonomy: String,
    /// Whether the user has pressed pause.
    pub paused: bool,
    /// Whether the mirror is observing.
    pub observing: bool,
    /// Agents connected right now.
    pub connected: Vec<String>,
    /// Reflections taken this run.
    pub observations: u64,
    /// Recurring machine states discovered.
    pub states: usize,
    /// Actions observed from agents, ever.
    pub actions: u64,
}

/// The one thing on the Home screen that is not a number.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Home {
    /// One sentence: what CoreScout is doing right now.
    pub headline: String,
    /// Which AI, if any.
    pub connected: Vec<AgentSummary>,
    /// What happened today.
    pub today: Today,
    /// What the machine is doing, in plain words.
    pub machine: MachineMood,
    /// The most recent thing worth showing, if there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<LearnedCard>,
    /// What to say when there is nothing yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<Empty>,
}

/// The counts for today.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Today {
    /// Patterns that cleared the bars.
    pub learned: u64,
    /// Capabilities created.
    pub capabilities: u64,
    /// Beliefs withdrawn.
    pub retired: u64,
    /// Candidates that failed a threshold.
    pub rejected: u64,
}

impl Today {
    /// Whether anything happened.
    pub fn is_quiet(&self) -> bool {
        self.learned == 0 && self.capabilities == 0 && self.retired == 0
    }
}

/// The state of the machine, said in a way a person can act on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MachineMood {
    /// The plain sentence.
    pub plain: String,
    /// The technical name of the state, if one is recognised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<u32>,
    /// How many times this state has been seen before.
    pub seen: u64,
    /// How sure CoreScout is that it recognises this, if it can say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Whether this is unlike anything seen before.
    pub unfamiliar: bool,
}

/// What to say when a screen has nothing on it.
///
/// An empty dashboard reads as a broken product. An empty state that explains
/// why it is empty and what will fill it reads as a careful one, and the fact
/// that CoreScout has learned nothing yet is itself the honest answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Empty {
    /// The heading.
    pub title: String,
    /// The explanation.
    pub body: String,
}

/// One AI, on the AI screen and the Home screen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentSummary {
    /// The stable id.
    pub id: String,
    /// What to call it.
    pub name: String,
    /// Whether it is connected right now.
    pub connected: bool,
    /// One line.
    pub summary: String,
    /// Actions observed from it, ever.
    pub actions: u64,
    /// Sessions it has had.
    pub sessions: u64,
    /// Capabilities available to it.
    pub capabilities: usize,
    /// When it was last heard from, Unix milliseconds.
    pub last_seen_ms: u64,
}

/// One card on the Learned screen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LearnedCard {
    /// Stable id, for the detail view.
    pub id: String,
    /// What was learned, in one line.
    pub title: String,
    /// Why it matters.
    pub detail: String,
    /// "Seen together" or "Verified".
    pub basis: String,
    /// Whether this rests on randomised evidence.
    pub causal: bool,
    /// Zero to one.
    pub confidence: f64,
    /// The share of uses that worked, if it has been used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reliability: Option<f64>,
    /// Times it has been used.
    pub uses: u64,
    /// When it was last relevant, Unix milliseconds.
    pub last_ms: u64,
    /// Whether a connected AI can use this.
    pub usable_by_ai: bool,
    /// Whether this is a capability rather than an observation.
    pub is_capability: bool,
    /// Whether the user has approved it.
    pub approved: bool,
}

/// The full evidence behind one card.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Explanation {
    /// The card this explains.
    pub id: String,
    /// What was learned.
    pub title: String,
    /// The plain answer to "why did CoreScout do this?".
    pub simple: String,
    /// The evidence, in a sentence.
    pub evidence: String,
    /// Whether the evidence is randomised or merely observed.
    pub kind: String,
    /// Zero to one.
    pub confidence: f64,
    /// What CoreScout would otherwise have done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alternative: Option<String>,
    /// What happened when it was used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Everything underneath, for the technical view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technical: Option<serde_json::Value>,
}

/// One line on the Activity screen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Moment {
    /// Position in the log.
    pub sequence: u64,
    /// Unix milliseconds.
    pub at_ms: u64,
    /// What happened, in plain words.
    pub summary: String,
    /// Which kind of thing.
    pub kind: String,
    /// How loudly to show it.
    pub severity: String,
    /// What it was about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Why, if it was an action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// What happened, if it is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    /// Structured detail, for the technical view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

/// What the machine is, on the Computer screen.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    /// The processor, as it names itself.
    pub cpu: String,
    /// Physical cores.
    pub cores: usize,
    /// Logical processors.
    pub threads: usize,
    /// Whether the cores are of more than one kind.
    pub hybrid: bool,
    /// Entities in the mirror.
    pub entities: usize,
    /// Channels per entity.
    pub channels: usize,
    /// Cells carrying a reading rather than a reason there was not one.
    pub observed_cells: usize,
    /// Nanoseconds an observation pass takes.
    pub observe_ns: u64,
    /// The machine's own account of itself, one line per proposition.
    #[serde(default)]
    pub self_description: Vec<String>,
}

/// A recurring state, for the Computer screen and the mirror view.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateCard {
    /// The machine's own name for it.
    pub id: u32,
    /// A plain description.
    pub plain: String,
    /// Times it has been entered.
    pub entries: u64,
    /// Mean milliseconds spent in it.
    pub mean_dwell_ms: f64,
    /// The state most often entered next, and how often.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub then: Option<(u32, f64)>,
}

/// What CoreScout knows, on the "what my computer knows" page.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Knowledge {
    /// Milliseconds since CoreScout first ran here.
    pub learning_for_ms: u64,
    /// Recurring machine states discovered.
    pub machine_states: usize,
    /// Operational patterns found in agent activity.
    pub workflow_patterns: usize,
    /// Capabilities with randomised evidence behind them.
    pub verified_capabilities: usize,
    /// Procedures being tested right now.
    pub open_questions: usize,
    /// Candidates that failed a threshold.
    pub rejected: u64,
    /// Beliefs withdrawn.
    pub retired: u64,
    /// Grouped, for the four sections of that page.
    #[serde(default)]
    pub about_agents: Vec<String>,
    /// What is known about the places work happens.
    #[serde(default)]
    pub about_workspaces: Vec<String>,
    /// What is known about this machine.
    #[serde(default)]
    pub about_machine: Vec<String>,
    /// What is still unsettled.
    #[serde(default)]
    pub still_uncertain: Vec<String>,
}

/// What CoreScout observes, for the Privacy page.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Privacy {
    /// The one folder everything lives in.
    pub data_dir: String,
    /// Bytes on disk.
    pub bytes: u64,
    /// Whether anything is sent anywhere. Always false.
    pub telemetry: bool,
    /// One row per kind of thing stored.
    #[serde(default)]
    pub holdings: Vec<Holding>,
}

/// One kind of thing CoreScout stores.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Holding {
    /// The storage kind.
    pub kind: String,
    /// What it is, in plain words.
    pub describes: String,
    /// How many records.
    pub count: usize,
    /// Whether "forget my AI history" removes this.
    pub agent_activity: bool,
}

/// What the service costs, for Settings and Diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Diagnostics {
    /// Mean nanoseconds per observation pass.
    pub mean_observe_ns: u64,
    /// The worst pass seen.
    pub worst_observe_ns: u64,
    /// Reflections taken this run.
    pub observations: u64,
    /// Milliseconds between reflections.
    pub interval_ms: u64,
    /// The share of one core the observation itself uses.
    pub cpu_share: f64,
    /// Bytes the ring occupies. Fixed for the life of the file.
    pub ring_bytes: u64,
    /// Samples the ring holds.
    pub ring_samples: u64,
    /// Whether the ring has wrapped and older history is gone.
    pub ring_wrapped: bool,
    /// Records in the document store.
    pub documents: usize,
    /// Entries in the event log.
    pub events: u64,
    /// The storage schema version.
    pub schema: u32,
    /// Sensors that could not read, and why.
    #[serde(default)]
    pub gaps: Vec<String>,
}

/// One node of the live mirror visualisation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// Stable key.
    pub key: String,
    /// What to show.
    pub label: String,
    /// What kind of thing it is.
    pub class: String,
    /// Zero to one: how busy.
    pub activity: f64,
    /// Zero to one: how well CoreScout can see it.
    pub clarity: f64,
    /// Its parent, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// The live mirror, for the centrepiece view.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Live {
    /// The plain sentence.
    pub plain: String,
    /// The recognised state, if there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<u32>,
    /// Times that state has been entered.
    pub seen: u64,
    /// Nodes.
    #[serde(default)]
    pub nodes: Vec<Node>,
    /// Recent activity, oldest first, for the sparkline.
    #[serde(default)]
    pub trail: Vec<f32>,
    /// The states visited recently, oldest first.
    #[serde(default)]
    pub recent_states: Vec<u32>,
    /// Agent activity happening now, as short labels.
    #[serde(default)]
    pub agent_activity: Vec<String>,
}

/// Something CoreScout is still testing.
///
/// The honest half of the Learned screen. A product that shows only its
/// conclusions reads as more certain than it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Question {
    /// The procedure being tested.
    pub id: String,
    /// What it would be called if it works out.
    pub name: String,
    /// The operation it is about.
    pub about: String,
    /// Whether there is now enough randomised evidence to answer it.
    pub settled: bool,
    /// What is missing, in words.
    pub missing: String,
    /// Trials assigned by coin flip.
    pub randomised_trials: u32,
    /// Trials in total, however they were assigned.
    pub observed_trials: u32,
}

/// How to connect one AI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Setup {
    /// Which AI.
    pub agent: String,
    /// What to call it.
    pub title: String,
    /// Whether CoreScout can write the configuration itself.
    pub automatic: bool,
    /// Where the configuration lives, if it is a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    /// The configuration to paste.
    pub snippet: String,
    /// A command that does it, where one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// What to do, in words.
    pub instructions: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quiet_day_is_recognisable() {
        assert!(Today::default().is_quiet());
        assert!(!Today {
            learned: 1,
            ..Today::default()
        }
        .is_quiet());
    }

    #[test]
    fn a_rejected_count_alone_does_not_make_a_day_eventful() {
        // "CoreScout rejected 17 ideas today" is not something to celebrate on
        // the Home screen, though it belongs on the Knowledge page.
        assert!(Today {
            rejected: 17,
            ..Today::default()
        }
        .is_quiet());
    }

    #[test]
    fn every_view_survives_the_wire() {
        // These cross a process boundary to reach the interface, so a type
        // that cannot round-trip is a screen that cannot render.
        let status = Status {
            running: true,
            version: "0.3.0".into(),
            uptime_ms: 1,
            learning_for_ms: 2,
            autonomy: "suggest".into(),
            paused: false,
            observing: true,
            connected: vec!["claude-code".into()],
            observations: 900,
            states: 32,
            actions: 23,
        };
        let json = serde_json::to_string(&status).expect("serialise");
        assert_eq!(
            serde_json::from_str::<Status>(&json).expect("deserialise"),
            status
        );

        let card = LearnedCard {
            id: "c".into(),
            title: "t".into(),
            detail: "d".into(),
            basis: "Verified".into(),
            causal: true,
            confidence: 0.9,
            reliability: None,
            uses: 0,
            last_ms: 0,
            usable_by_ai: false,
            is_capability: true,
            approved: false,
        };
        let json = serde_json::to_string(&card).expect("serialise");
        assert_eq!(
            serde_json::from_str::<LearnedCard>(&json).expect("deserialise"),
            card
        );
    }

    #[test]
    fn a_card_that_has_never_been_used_has_no_reliability() {
        // The type makes the honest answer expressible; this is the test that
        // says the honest answer is what gets sent.
        let card = LearnedCard {
            id: "c".into(),
            title: "t".into(),
            detail: "d".into(),
            basis: "Seen together".into(),
            causal: false,
            confidence: 0.3,
            reliability: None,
            uses: 0,
            last_ms: 0,
            usable_by_ai: false,
            is_capability: false,
            approved: false,
        };
        let json = serde_json::to_string(&card).expect("serialise");
        assert!(
            !json.contains("reliability"),
            "an absent reliability is absent, not zero: {json}"
        );
    }
}
