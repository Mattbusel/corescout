//! The watchdog: independent of the policy, and able to stop it.
//!
//! # Why it is separate
//!
//! A controller that can disable its own safety layer has no safety layer. The
//! watchdog holds no reference to the policy and the policy holds no mutable
//! reference to the watchdog; the loop asks the watchdog for a [`Verdict`]
//! before every action and obeys it.
//!
//! # What it watches
//!
//! | condition | why it matters |
//! |---|---|
//! | tick overrun | a stalled loop is holding a machine in whatever state it last chose |
//! | action budget | a policy acting every tick is thrashing, whatever it believes |
//! | failure rate | actions that keep failing mean the world is not as the model thinks |
//! | regression | the objective getting worse after acting is the signal to stop |
//! | model collapse | prediction skill falling to nothing means decisions rest on noise |
//!
//! # Failing safe means doing nothing
//!
//! Every trip leads to the same place: freeze actuation, revert what can be
//! reverted, and leave the machine to the operating system. That is a good
//! outcome. The Linux scheduler is a competent default, and the worst case for
//! this project is a machine left worse than it was found.

use serde::{Deserialize, Serialize};

/// What the watchdog permits right now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Verdict {
    /// Carry on.
    Proceed,
    /// Do not act this tick, for this reason. Observation continues.
    Pause(String),
    /// Stop permanently, revert what can be reverted, and hand the machine
    /// back. Requires an operator to clear.
    Stop(String),
}

impl Verdict {
    pub fn permits_action(&self) -> bool {
        matches!(self, Verdict::Proceed)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Verdict::Stop(_))
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Verdict::Proceed => None,
            Verdict::Pause(reason) | Verdict::Stop(reason) => Some(reason),
        }
    }
}

/// Limits the watchdog enforces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchdogConfig {
    /// A tick taking longer than this suggests the loop is stuck.
    pub max_tick_ns: u64,
    /// Consecutive overruns before stopping.
    pub max_overruns: u32,
    /// Actions per minute above which the controller is thrashing.
    pub max_actions_per_minute: u32,
    /// Consecutive failed actions before stopping.
    pub max_consecutive_failures: u32,
    /// Consecutive ticks where the objective got worse after acting.
    pub max_consecutive_regressions: u32,
    /// Prediction skill below which decisions rest on noise.
    pub min_model_skill: f64,
    /// Ticks to observe before the skill floor is enforced, so a cold model is
    /// not stopped for not yet having learned anything.
    pub warmup_ticks: u64,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        WatchdogConfig {
            max_tick_ns: 2_000_000_000,
            max_overruns: 3,
            max_actions_per_minute: 30,
            max_consecutive_failures: 3,
            max_consecutive_regressions: 5,
            // Deliberately just above zero rather than a demanding figure: the
            // floor exists to catch a model that has collapsed, not to enforce
            // a standard of excellence.
            min_model_skill: -0.05,
            warmup_ticks: 200,
        }
    }
}

/// Watches the control loop and can stop it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Watchdog {
    config: WatchdogConfig,
    ticks: u64,
    overruns: u32,
    consecutive_failures: u32,
    consecutive_regressions: u32,
    /// Monotonic timestamps of recent actions.
    recent_actions: Vec<u64>,
    stopped: Option<String>,
    /// Times a verdict other than Proceed was issued.
    interventions: u64,
}

impl Watchdog {
    pub fn new(config: WatchdogConfig) -> Watchdog {
        Watchdog {
            config,
            ticks: 0,
            overruns: 0,
            consecutive_failures: 0,
            consecutive_regressions: 0,
            recent_actions: Vec::new(),
            stopped: None,
            interventions: 0,
        }
    }

    pub fn config(&self) -> &WatchdogConfig {
        &self.config
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn interventions(&self) -> u64 {
        self.interventions
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.is_some()
    }

    /// Clear a stop. Only an operator should call this, which is why the loop
    /// never does.
    pub fn reset(&mut self) {
        self.stopped = None;
        self.overruns = 0;
        self.consecutive_failures = 0;
        self.consecutive_regressions = 0;
    }

    /// Record how long a tick took.
    pub fn tick_completed(&mut self, duration_ns: u64) {
        self.ticks += 1;
        if duration_ns > self.config.max_tick_ns {
            self.overruns += 1;
            if self.overruns >= self.config.max_overruns {
                self.stopped = Some(format!(
                    "{} consecutive ticks took longer than {} ms; the loop is not keeping up",
                    self.overruns,
                    self.config.max_tick_ns / 1_000_000
                ));
            }
        } else {
            self.overruns = 0;
        }
    }

    /// Record that an action was taken.
    pub fn action_taken(&mut self, monotonic_ns: u64, succeeded: bool) {
        self.recent_actions.push(monotonic_ns);
        let cutoff = monotonic_ns.saturating_sub(60_000_000_000);
        self.recent_actions.retain(|t| *t >= cutoff);

        if succeeded {
            self.consecutive_failures = 0;
        } else {
            self.consecutive_failures += 1;
            if self.consecutive_failures >= self.config.max_consecutive_failures {
                self.stopped = Some(format!(
                    "{} actions in a row failed; the machine is not behaving as the model \
                     expects",
                    self.consecutive_failures
                ));
            }
        }
    }

    /// Record whether the objective improved after the last action.
    pub fn outcome_observed(&mut self, improved: bool) {
        if improved {
            self.consecutive_regressions = 0;
        } else {
            self.consecutive_regressions += 1;
            if self.consecutive_regressions >= self.config.max_consecutive_regressions {
                self.stopped = Some(format!(
                    "the objective got worse {} times in a row after acting; the controller \
                     is making things worse",
                    self.consecutive_regressions
                ));
            }
        }
    }

    /// Decide whether the loop may act.
    pub fn assess(&mut self, monotonic_ns: u64, model_skill: f64) -> Verdict {
        if let Some(reason) = &self.stopped {
            self.interventions += 1;
            return Verdict::Stop(reason.clone());
        }

        // Greater-or-equal, so that a warmup of zero means "enforce from the
        // first assessment" rather than "from the second".
        if self.ticks >= self.config.warmup_ticks && model_skill < self.config.min_model_skill {
            self.stopped = Some(format!(
                "prediction skill fell to {model_skill:.3}, below the floor of {:.3}; \
                 decisions would rest on noise",
                self.config.min_model_skill
            ));
            self.interventions += 1;
            return Verdict::Stop(self.stopped.clone().unwrap_or_default());
        }

        let cutoff = monotonic_ns.saturating_sub(60_000_000_000);
        let recent = self.recent_actions.iter().filter(|t| **t >= cutoff).count();
        if recent >= self.config.max_actions_per_minute as usize {
            self.interventions += 1;
            // A pause, not a stop: thrashing is usually a phase, and the right
            // response is to wait rather than to give up on the machine.
            return Verdict::Pause(format!(
                "{recent} actions in the last minute reaches the ceiling of {}",
                self.config.max_actions_per_minute
            ));
        }

        Verdict::Proceed
    }
}

impl Default for Watchdog {
    fn default() -> Self {
        Watchdog::new(WatchdogConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watchdog() -> Watchdog {
        Watchdog::new(WatchdogConfig {
            warmup_ticks: 0,
            ..WatchdogConfig::default()
        })
    }

    #[test]
    fn a_healthy_loop_may_proceed() {
        let mut watchdog = watchdog();
        assert_eq!(watchdog.assess(1_000, 0.4), Verdict::Proceed);
        assert!(watchdog.assess(1_000, 0.4).permits_action());
    }

    #[test]
    fn repeated_overruns_stop_the_loop() {
        let mut watchdog = watchdog();
        for _ in 0..3 {
            watchdog.tick_completed(5_000_000_000);
        }
        let verdict = watchdog.assess(1_000, 0.4);
        assert!(verdict.is_terminal());
        assert!(verdict.reason().unwrap().contains("not keeping up"));
    }

    #[test]
    fn a_single_slow_tick_is_forgiven() {
        let mut watchdog = watchdog();
        watchdog.tick_completed(5_000_000_000);
        watchdog.tick_completed(1_000);
        watchdog.tick_completed(1_000);
        assert_eq!(watchdog.assess(1_000, 0.4), Verdict::Proceed);
    }

    #[test]
    fn repeated_failures_stop_the_loop() {
        let mut watchdog = watchdog();
        for i in 0..3 {
            watchdog.action_taken(i * 1_000_000, false);
        }
        assert!(watchdog.assess(3_000_000, 0.4).is_terminal());
    }

    #[test]
    fn making_things_worse_repeatedly_stops_the_loop() {
        // The most important trip: a controller that is confidently harming the
        // machine.
        let mut watchdog = watchdog();
        for _ in 0..5 {
            watchdog.outcome_observed(false);
        }
        let verdict = watchdog.assess(1_000, 0.4);
        assert!(verdict.is_terminal());
        assert!(verdict.reason().unwrap().contains("making things worse"));
    }

    #[test]
    fn an_improvement_resets_the_regression_count() {
        let mut watchdog = watchdog();
        for _ in 0..4 {
            watchdog.outcome_observed(false);
        }
        watchdog.outcome_observed(true);
        for _ in 0..4 {
            watchdog.outcome_observed(false);
        }
        assert!(!watchdog.assess(1_000, 0.4).is_terminal());
    }

    #[test]
    fn thrashing_pauses_rather_than_stopping() {
        // Acting too often is usually a phase; the right response is to wait.
        let mut watchdog = Watchdog::new(WatchdogConfig {
            max_actions_per_minute: 3,
            warmup_ticks: 0,
            ..WatchdogConfig::default()
        });
        for i in 0..3 {
            watchdog.action_taken(i * 1_000_000, true);
        }
        let verdict = watchdog.assess(3_000_000, 0.4);
        assert!(!verdict.permits_action());
        assert!(!verdict.is_terminal());
    }

    #[test]
    fn the_action_window_rolls_forward() {
        let mut watchdog = Watchdog::new(WatchdogConfig {
            max_actions_per_minute: 3,
            warmup_ticks: 0,
            ..WatchdogConfig::default()
        });
        for i in 0..3 {
            watchdog.action_taken(i * 1_000_000, true);
        }
        // A minute later those actions no longer count.
        assert_eq!(watchdog.assess(120_000_000_000, 0.4), Verdict::Proceed);
    }

    #[test]
    fn a_collapsed_model_stops_the_loop_but_only_after_warmup() {
        let mut cold = Watchdog::new(WatchdogConfig::default());
        cold.tick_completed(1_000);
        assert_eq!(
            cold.assess(1_000, -0.9),
            Verdict::Proceed,
            "a model that has not learned yet must not trip the floor"
        );

        let mut warm = Watchdog::new(WatchdogConfig {
            warmup_ticks: 1,
            ..WatchdogConfig::default()
        });
        warm.tick_completed(1_000);
        warm.tick_completed(1_000);
        assert!(warm.assess(1_000, -0.9).is_terminal());
    }

    #[test]
    fn a_stop_persists_until_an_operator_clears_it() {
        let mut watchdog = watchdog();
        for _ in 0..5 {
            watchdog.outcome_observed(false);
        }
        assert!(watchdog.assess(1_000, 0.4).is_terminal());
        assert!(watchdog.is_stopped());
        // Later good behaviour does not clear it.
        watchdog.outcome_observed(true);
        assert!(watchdog.assess(2_000, 0.9).is_terminal());
        watchdog.reset();
        assert_eq!(watchdog.assess(3_000, 0.9), Verdict::Proceed);
    }
}
