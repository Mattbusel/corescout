//! Operations that go wrong here, counted.

use std::collections::BTreeMap;

use corescout_agent_observation::Action;
use serde::{Deserialize, Serialize};

/// One operation, in one place, and how it has gone.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FailureMode {
    /// Stable key: the fingerprint, and the workspace if there was one.
    pub key: String,
    /// The normalised operation.
    pub fingerprint: String,
    /// Where, if it was anywhere in particular.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Times it has been attempted.
    pub attempts: u64,
    /// Times it visibly failed.
    pub failures: u64,
    /// Times it claimed success and something proved otherwise.
    pub silent_failures: u64,
    /// Times it was a repeat of something that had just failed.
    pub retries: u64,
    /// Times nobody checked whether it worked.
    pub unverified: u64,
    /// The most common failure text, trimmed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub common_detail: Option<String>,
    /// Wall clock of the first attempt.
    pub first_ms: u64,
    /// Wall clock of the most recent attempt.
    pub last_ms: u64,
    /// Total milliseconds spent on it, successes and failures alike.
    pub spent_ms: u64,
    /// Failure texts and how often each was seen, capped.
    #[serde(default)]
    details: BTreeMap<String, u64>,
}

/// How many distinct failure texts are remembered per operation.
///
/// Enough to find the common one, few enough that a build printing a unique
/// error every time cannot grow this without bound.
const DETAIL_CEILING: usize = 16;

impl FailureMode {
    /// Start counting an operation.
    pub fn new(fingerprint: &str, workspace: Option<&str>) -> FailureMode {
        FailureMode {
            key: key_for(fingerprint, workspace),
            fingerprint: fingerprint.to_string(),
            workspace: workspace.map(str::to_string),
            ..FailureMode::default()
        }
    }

    /// Fold in one attempt.
    pub fn observe(&mut self, action: &Action) {
        if self.attempts == 0 {
            self.first_ms = action.started_ms;
        }
        self.attempts += 1;
        self.last_ms = action.started_ms;
        self.spent_ms += action.duration_ms;
        if action.visibly_failed() {
            self.failures += 1;
        }
        if action.is_silent_failure() {
            self.silent_failures += 1;
        }
        if action.retry_of.is_some() {
            self.retries += 1;
        }
        if action.unverified() {
            self.unverified += 1;
        }
        if let Some(detail) = action.reported.detail() {
            let short = shorten(detail);
            if !short.is_empty() {
                if self.details.len() < DETAIL_CEILING || self.details.contains_key(&short) {
                    *self.details.entry(short).or_insert(0) += 1;
                }
                self.common_detail = self
                    .details
                    .iter()
                    .max_by_key(|(_, count)| **count)
                    .map(|(text, _)| text.clone());
            }
        }
    }

    /// The share of attempts that visibly failed.
    ///
    /// `None` until anything has been attempted, so an operation nobody has
    /// run does not read as flawless.
    pub fn rate(&self) -> Option<f64> {
        (self.attempts > 0).then(|| self.failures as f64 / self.attempts as f64)
    }

    /// Mean milliseconds per attempt.
    pub fn mean_ms(&self) -> Option<f64> {
        (self.attempts > 0).then(|| self.spent_ms as f64 / self.attempts as f64)
    }

    /// Milliseconds spent on attempts that were retries of a failure.
    ///
    /// The closest thing to "time this cost you" that can be computed from
    /// what is actually observed. It is a floor, not a total: it does not
    /// count the thinking between attempts.
    pub fn wasted_ms(&self) -> u64 {
        match self.mean_ms() {
            Some(mean) => (mean * self.retries as f64) as u64,
            None => 0,
        }
    }

    /// Whether this has happened enough, and gone wrong enough, to be worth
    /// mentioning.
    pub fn is_recurring(&self, min_attempts: u64, min_failures: u64) -> bool {
        self.attempts >= min_attempts && self.failures >= min_failures
    }

    /// A line for the interface, written for someone who has not read any of
    /// this.
    pub fn headline(&self) -> String {
        let percent = (self.rate().unwrap_or(0.0) * 100.0).round() as u64;
        if self.silent_failures > 0 {
            format!(
                "{} reported success {} times when it had not worked",
                self.fingerprint, self.silent_failures
            )
        } else {
            format!(
                "{} fails about {percent}% of the time here ({} of {})",
                self.fingerprint, self.failures, self.attempts
            )
        }
    }
}

/// The storage key for an operation in a place.
pub fn key_for(fingerprint: &str, workspace: Option<&str>) -> String {
    match workspace {
        Some(workspace) => format!("{fingerprint}@{workspace}"),
        None => fingerprint.to_string(),
    }
}

fn shorten(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.len() > 160 {
        line.chars().take(160).collect()
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_agent_observation::{ActionKind, Report, Verification};

    fn action(reported: Report, retry: bool) -> Action {
        Action {
            id: "a".into(),
            session: "s".into(),
            task: None,
            workspace: Some("w".into()),
            started_ms: 1000,
            duration_ms: 3000,
            kind: ActionKind::Build,
            name: "cargo build".into(),
            fingerprint: "cargo build".into(),
            reported,
            verified: None,
            exit_code: None,
            files: Vec::new(),
            retry_of: retry.then(|| "a0".to_string()),
            machine_state: None,
        }
    }

    #[test]
    fn an_operation_nobody_has_run_has_no_failure_rate() {
        assert_eq!(FailureMode::new("cargo build", None).rate(), None);
    }

    #[test]
    fn failures_and_attempts_are_counted_separately() {
        let mut mode = FailureMode::new("cargo build", Some("w"));
        mode.observe(&action(Report::Failure("boom".into()), false));
        mode.observe(&action(Report::Success, false));
        mode.observe(&action(Report::Failure("boom".into()), true));
        assert_eq!(mode.attempts, 3);
        assert_eq!(mode.failures, 2);
        assert_eq!(mode.retries, 1);
        assert!((mode.rate().expect("a rate") - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_silent_failure_is_counted_as_its_own_thing() {
        // "Reported success and did not work" is a different fact from "failed",
        // and it is the more alarming one.
        let mut mode = FailureMode::new("deploy", None);
        mode.observe(&Action {
            verified: Some(Verification::Contradicted("still 502".into())),
            ..action(Report::Success, false)
        });
        assert_eq!(mode.failures, 1);
        assert_eq!(mode.silent_failures, 1);
        assert!(
            mode.headline().contains("reported success"),
            "{}",
            mode.headline()
        );
    }

    #[test]
    fn unverified_attempts_are_counted_so_the_gap_is_visible() {
        let mut mode = FailureMode::new("deploy", None);
        for _ in 0..5 {
            mode.observe(&action(Report::Success, false));
        }
        assert_eq!(mode.unverified, 5);
        assert_eq!(mode.failures, 0);
    }

    #[test]
    fn the_common_failure_text_is_the_one_seen_most() {
        let mut mode = FailureMode::new("cargo build", None);
        mode.observe(&action(Report::Failure("schema out of date".into()), false));
        mode.observe(&action(Report::Failure("schema out of date".into()), false));
        mode.observe(&action(Report::Failure("disk full".into()), false));
        assert_eq!(mode.common_detail.as_deref(), Some("schema out of date"));
    }

    #[test]
    fn a_build_with_a_unique_error_every_time_cannot_grow_this_forever() {
        let mut mode = FailureMode::new("cargo build", None);
        for n in 0..1000 {
            mode.observe(&action(Report::Failure(format!("error {n}")), false));
        }
        assert!(
            mode.details.len() <= DETAIL_CEILING,
            "{}",
            mode.details.len()
        );
        assert_eq!(mode.failures, 1000, "the count is still exact");
    }

    #[test]
    fn a_very_long_error_is_trimmed_before_it_is_stored() {
        let mut mode = FailureMode::new("cargo build", None);
        mode.observe(&action(Report::Failure("x".repeat(9000)), false));
        assert!(mode.common_detail.expect("a detail").len() <= 160);
    }

    #[test]
    fn wasted_time_counts_repeats_and_not_first_attempts() {
        let mut mode = FailureMode::new("cargo build", None);
        mode.observe(&action(Report::Failure("boom".into()), false));
        mode.observe(&action(Report::Failure("boom".into()), true));
        mode.observe(&action(Report::Success, true));
        // Three attempts of three seconds each, two of which were repeats.
        assert_eq!(mode.wasted_ms(), 6000);
    }

    #[test]
    fn recurrence_needs_both_repetition_and_failure() {
        let mut mode = FailureMode::new("cargo build", None);
        for _ in 0..20 {
            mode.observe(&action(Report::Success, false));
        }
        assert!(
            !mode.is_recurring(4, 2),
            "twenty clean runs is not a failure mode"
        );
        mode.observe(&action(Report::Failure("once".into()), false));
        assert!(!mode.is_recurring(4, 2), "one failure is not a pattern");
        mode.observe(&action(Report::Failure("twice".into()), false));
        assert!(mode.is_recurring(4, 2));
    }

    #[test]
    fn the_same_operation_in_two_places_is_two_modes() {
        // A build that fails in one repository and not another is the whole
        // point; merging them would average the fact away.
        assert_ne!(
            key_for("cargo build", Some("app")),
            key_for("cargo build", Some("web"))
        );
        assert_eq!(key_for("cargo build", None), "cargo build");
    }

    #[test]
    fn a_failure_mode_survives_storage() {
        let mut mode = FailureMode::new("cargo build", Some("w"));
        mode.observe(&action(Report::Failure("boom".into()), false));
        let json = serde_json::to_string(&mode).expect("serialise");
        assert_eq!(
            serde_json::from_str::<FailureMode>(&json).expect("deserialise"),
            mode
        );
    }
}
