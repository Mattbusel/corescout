//! Sessions and tasks: the containers an action belongs to.

use serde::{Deserialize, Serialize};

/// One working session of one agent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// Unique across this machine.
    pub id: String,
    /// Which agent.
    pub agent: String,
    /// Where it was working, if it said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// When it began, Unix milliseconds.
    pub started_ms: u64,
    /// When it was last active.
    pub last_ms: u64,
    /// When it ended, if it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_ms: Option<u64>,
    /// Actions observed in it.
    pub actions: u64,
    /// Actions that visibly failed.
    pub failures: u64,
    /// Actions that were retries of an earlier one.
    pub retries: u64,
    /// Times CoreScout answered a question from this session.
    pub queries: u64,
    /// Times this session used a CoreScout capability.
    pub capability_uses: u64,
}

impl Session {
    /// Open a session.
    pub fn new(id: impl Into<String>, agent: impl Into<String>, now_ms: u64) -> Session {
        Session {
            id: id.into(),
            agent: agent.into(),
            workspace: None,
            started_ms: now_ms,
            last_ms: now_ms,
            ended_ms: None,
            actions: 0,
            failures: 0,
            retries: 0,
            queries: 0,
            capability_uses: 0,
        }
    }

    /// Whether this session is still open.
    pub fn is_open(&self) -> bool {
        self.ended_ms.is_none()
    }

    /// How long it has been going, in milliseconds.
    pub fn duration_ms(&self, now_ms: u64) -> u64 {
        self.ended_ms
            .unwrap_or(now_ms)
            .saturating_sub(self.started_ms)
    }

    /// The share of observed actions that visibly failed.
    ///
    /// `None` when nothing has been observed: a session with no actions has no
    /// failure rate, and reporting zero would make a fresh session look
    /// perfect.
    pub fn failure_rate(&self) -> Option<f64> {
        (self.actions > 0).then(|| self.failures as f64 / self.actions as f64)
    }
}

/// What became of a task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome", content = "detail")]
pub enum TaskOutcome {
    /// Still going.
    InProgress,
    /// The agent finished and said it worked.
    Completed,
    /// The agent gave up.
    Abandoned(String),
    /// A person stepped in and corrected it.
    ///
    /// The single most informative event this crate records: a human
    /// intervention is an unambiguous label that whatever the agent did was
    /// not right, produced by someone with more context than any heuristic.
    HumanCorrected(String),
    /// The agent noticed its own mistake and fixed it.
    SelfCorrected(String),
}

impl TaskOutcome {
    /// Whether this outcome says the task went wrong.
    pub fn went_wrong(&self) -> bool {
        matches!(
            self,
            TaskOutcome::Abandoned(_)
                | TaskOutcome::HumanCorrected(_)
                | TaskOutcome::SelfCorrected(_)
        )
    }

    /// Whether this outcome is settled.
    pub fn is_settled(&self) -> bool {
        !matches!(self, TaskOutcome::InProgress)
    }
}

/// A unit of work an agent attempted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    /// Unique across this machine.
    pub id: String,
    /// The session it belongs to.
    pub session: String,
    /// What the agent said it was doing, in its own words.
    pub description: String,
    /// A normalised form, so the same class of task can be counted.
    pub class: String,
    /// When it started, Unix milliseconds.
    pub started_ms: u64,
    /// When it ended, if it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_ms: Option<u64>,
    /// How it went.
    pub outcome: TaskOutcome,
    /// Actions taken within it.
    pub actions: u64,
    /// Actions that visibly failed.
    pub failures: u64,
}

impl Task {
    /// Begin a task.
    pub fn new(
        id: impl Into<String>,
        session: impl Into<String>,
        description: impl Into<String>,
        now_ms: u64,
    ) -> Task {
        let description = description.into();
        let class = crate::fingerprint::classify_task(&description);
        Task {
            id: id.into(),
            session: session.into(),
            description,
            class,
            started_ms: now_ms,
            ended_ms: None,
            outcome: TaskOutcome::InProgress,
            actions: 0,
            failures: 0,
        }
    }

    /// Close a task with an outcome.
    pub fn finish(&mut self, outcome: TaskOutcome, now_ms: u64) {
        self.outcome = outcome;
        self.ended_ms = Some(now_ms);
    }

    /// How long it took, or has taken so far.
    pub fn duration_ms(&self, now_ms: u64) -> u64 {
        self.ended_ms
            .unwrap_or(now_ms)
            .saturating_sub(self.started_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_with_no_actions_has_no_failure_rate() {
        // Reporting 0% would make a session that has done nothing look like a
        // perfect one, and the Home screen would say so.
        let session = Session::new("s1", "claude-code", 0);
        assert_eq!(session.failure_rate(), None);
    }

    #[test]
    fn a_failure_rate_is_reported_once_there_is_something_to_divide() {
        let mut session = Session::new("s1", "claude-code", 0);
        session.actions = 8;
        session.failures = 2;
        assert_eq!(session.failure_rate(), Some(0.25));
    }

    #[test]
    fn an_open_session_measures_its_duration_from_now() {
        let mut session = Session::new("s1", "claude-code", 1000);
        assert!(session.is_open());
        assert_eq!(session.duration_ms(5000), 4000);
        session.ended_ms = Some(3000);
        assert!(!session.is_open());
        assert_eq!(session.duration_ms(5000), 2000, "a closed session stops");
    }

    #[test]
    fn a_human_correction_counts_as_the_task_going_wrong() {
        // Somebody with full context said this was not right. There is no
        // stronger label available anywhere in this system.
        let correction = TaskOutcome::HumanCorrected("wrong file".into());
        assert!(correction.went_wrong());
        assert!(correction.is_settled());
    }

    #[test]
    fn completion_is_settled_and_did_not_go_wrong() {
        assert!(TaskOutcome::Completed.is_settled());
        assert!(!TaskOutcome::Completed.went_wrong());
        assert!(!TaskOutcome::InProgress.is_settled());
        assert!(!TaskOutcome::InProgress.went_wrong());
    }

    #[test]
    fn a_task_is_classified_when_it_is_created() {
        let task = Task::new("t1", "s1", "Fix the failing build in the API crate", 0);
        assert!(!task.class.is_empty());
        assert_eq!(task.outcome, TaskOutcome::InProgress);
        assert!(task.ended_ms.is_none());
    }

    #[test]
    fn finishing_a_task_stops_its_clock() {
        let mut task = Task::new("t1", "s1", "deploy", 1000);
        task.finish(TaskOutcome::Completed, 4000);
        assert_eq!(task.duration_ms(99_000), 3000);
    }

    #[test]
    fn sessions_and_tasks_survive_storage() {
        let mut session = Session::new("s1", "claude-code", 7);
        session.workspace = Some("w1".into());
        let json = serde_json::to_string(&session).expect("serialise");
        assert_eq!(
            serde_json::from_str::<Session>(&json).expect("deserialise"),
            session
        );

        let mut task = Task::new("t1", "s1", "run the tests", 7);
        task.finish(TaskOutcome::HumanCorrected("wrong branch".into()), 9);
        let json = serde_json::to_string(&task).expect("serialise");
        assert_eq!(
            serde_json::from_str::<Task>(&json).expect("deserialise"),
            task
        );
    }
}
