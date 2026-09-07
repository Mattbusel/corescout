//! Rate limits, so an autonomous loop that goes wrong goes wrong slowly.
//!
//! # Why a budget rather than a cooldown
//!
//! A cooldown between actions permits an unbounded number of them given
//! enough time, which is exactly the failure mode: a controller that acts
//! every thirty seconds forever is worse than one that acts ten times in a
//! minute and then stops. So the limit is a budget over a window, and when it
//! is spent CoreScout stops until the window rolls.
//!
//! # The clock comes from outside
//!
//! Every method takes the current time rather than reading one. That makes the
//! whole thing testable without sleeping, and means the service can drive it
//! from the same monotonic clock the mirror uses instead of a second one that
//! disagrees.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// Nanoseconds in an hour.
const HOUR_NS: u64 = 3_600 * 1_000_000_000;
/// Nanoseconds in a day.
const DAY_NS: u64 = 24 * HOUR_NS;

/// How much CoreScout may do in a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// Actions permitted in any rolling hour.
    pub per_hour: u32,
    /// Actions permitted in any rolling day.
    pub per_day: u32,
    /// Consecutive failures before CoreScout stops acting and asks for help.
    pub failure_streak: u32,
}

impl Default for Limits {
    /// Deliberately small.
    ///
    /// These are not throughput numbers. An operational improvement worth
    /// having is not one that needs to be applied sixty times an hour, and a
    /// loop that wants to is malfunctioning.
    fn default() -> Limits {
        Limits {
            per_hour: 12,
            per_day: 60,
            failure_streak: 3,
        }
    }
}

/// Why the limiter said no.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exhausted {
    /// The hourly budget is spent.
    Hourly {
        /// How many are permitted per hour.
        limit: u32,
        /// Nanoseconds until one comes back.
        retry_in_ns: u64,
    },
    /// The daily budget is spent.
    Daily {
        /// How many are permitted per day.
        limit: u32,
        /// Nanoseconds until one comes back.
        retry_in_ns: u64,
    },
    /// Too many actions in a row failed.
    FailureStreak {
        /// How many failed consecutively.
        failures: u32,
    },
}

impl std::fmt::Display for Exhausted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Exhausted::Hourly { limit, retry_in_ns } => write!(
                f,
                "CoreScout has already made {limit} changes this hour; the next is available in {}",
                minutes(*retry_in_ns)
            ),
            Exhausted::Daily { limit, retry_in_ns } => write!(
                f,
                "CoreScout has already made {limit} changes today; the next is available in {}",
                minutes(*retry_in_ns)
            ),
            Exhausted::FailureStreak { failures } => write!(
                f,
                "{failures} changes in a row did not work, so CoreScout stopped and is waiting for you"
            ),
        }
    }
}

fn minutes(ns: u64) -> String {
    let total = ns / 60_000_000_000;
    match total {
        0 => "under a minute".into(),
        1 => "a minute".into(),
        n if n < 60 => format!("{n} minutes"),
        n => format!("{} hours", n / 60),
    }
}

/// A rolling budget over a window.
#[derive(Clone, Debug, Default)]
pub struct RateLimiter {
    limits: Limits,
    /// Times of past actions, newest last. Bounded by the daily limit, so this
    /// never grows.
    taken: VecDeque<u64>,
    failures: u32,
}

impl RateLimiter {
    /// A limiter with the given budget.
    pub fn new(limits: Limits) -> RateLimiter {
        RateLimiter {
            limits,
            taken: VecDeque::new(),
            failures: 0,
        }
    }

    /// The budget in force.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Change the budget, keeping the history.
    pub fn set_limits(&mut self, limits: Limits) {
        self.limits = limits;
    }

    /// Whether an action may be taken now.
    pub fn check(&self, now_ns: u64) -> Result<(), Exhausted> {
        if self.failures >= self.limits.failure_streak {
            return Err(Exhausted::FailureStreak {
                failures: self.failures,
            });
        }
        let in_hour = self.count_since(now_ns.saturating_sub(HOUR_NS));
        if in_hour >= self.limits.per_hour {
            return Err(Exhausted::Hourly {
                limit: self.limits.per_hour,
                retry_in_ns: self.retry_in(now_ns, HOUR_NS, self.limits.per_hour),
            });
        }
        let in_day = self.count_since(now_ns.saturating_sub(DAY_NS));
        if in_day >= self.limits.per_day {
            return Err(Exhausted::Daily {
                limit: self.limits.per_day,
                retry_in_ns: self.retry_in(now_ns, DAY_NS, self.limits.per_day),
            });
        }
        Ok(())
    }

    /// Record that an action was taken, and whether it worked.
    ///
    /// Both halves matter: the budget counts attempts, and the streak counts
    /// consecutive failures. An action that succeeds clears the streak, which
    /// is why this is one call and not two.
    pub fn record(&mut self, now_ns: u64, succeeded: bool) {
        self.taken.push_back(now_ns);
        while self.taken.len() > self.limits.per_day.max(1) as usize * 2 {
            self.taken.pop_front();
        }
        if succeeded {
            self.failures = 0;
        } else {
            self.failures += 1;
        }
    }

    /// Consecutive failures.
    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// Actions taken in the last hour.
    pub fn in_last_hour(&self, now_ns: u64) -> u32 {
        self.count_since(now_ns.saturating_sub(HOUR_NS))
    }

    /// Actions taken in the last day.
    pub fn in_last_day(&self, now_ns: u64) -> u32 {
        self.count_since(now_ns.saturating_sub(DAY_NS))
    }

    /// Clear the failure streak. What the "resume" button does.
    pub fn forgive(&mut self) {
        self.failures = 0;
    }

    /// Forget everything, budget and streak alike.
    pub fn reset(&mut self) {
        self.taken.clear();
        self.failures = 0;
    }

    fn count_since(&self, floor_ns: u64) -> u32 {
        self.taken.iter().filter(|at| **at >= floor_ns).count() as u32
    }

    /// When the oldest action inside the window falls out of it.
    fn retry_in(&self, now_ns: u64, window_ns: u64, limit: u32) -> u64 {
        let floor = now_ns.saturating_sub(window_ns);
        let mut inside: Vec<u64> = self
            .taken
            .iter()
            .copied()
            .filter(|at| *at >= floor)
            .collect();
        inside.sort_unstable();
        // The action that has to expire is the one `limit` places from the
        // newest, not simply the oldest: with a limit of 12 and 30 recorded,
        // waiting for the oldest to expire still leaves 29 inside the window.
        let index = inside.len().saturating_sub(limit as usize);
        match inside.get(index) {
            Some(at) => (at + window_ns).saturating_sub(now_ns),
            None => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: u64 = 60_000_000_000;

    #[test]
    fn the_default_budget_is_small() {
        // If these ever look like throughput numbers, something has gone
        // wrong with what this product is for.
        let limits = Limits::default();
        assert!(limits.per_hour <= 20, "{} is too many", limits.per_hour);
        assert!(limits.per_day <= 100);
        assert!(limits.failure_streak <= 5);
    }

    #[test]
    fn actions_are_permitted_up_to_the_hourly_budget_and_then_not() {
        let mut limiter = RateLimiter::new(Limits {
            per_hour: 3,
            per_day: 100,
            failure_streak: 99,
        });
        let mut now = 10 * HOUR_NS;
        for _ in 0..3 {
            assert!(limiter.check(now).is_ok());
            limiter.record(now, true);
            now += MINUTE;
        }
        match limiter.check(now) {
            Err(Exhausted::Hourly { limit, .. }) => assert_eq!(limit, 3),
            other => panic!("expected the hourly budget to be spent, got {other:?}"),
        }
    }

    #[test]
    fn the_budget_comes_back_as_the_window_rolls() {
        let mut limiter = RateLimiter::new(Limits {
            per_hour: 2,
            per_day: 100,
            failure_streak: 99,
        });
        let start = 5 * HOUR_NS;
        limiter.record(start, true);
        limiter.record(start + MINUTE, true);
        assert!(limiter.check(start + 2 * MINUTE).is_err());
        // An hour after the first action, one slot is free again.
        assert!(limiter.check(start + HOUR_NS + 1).is_ok());
    }

    #[test]
    fn the_retry_hint_points_at_when_a_slot_actually_frees() {
        let mut limiter = RateLimiter::new(Limits {
            per_hour: 2,
            per_day: 100,
            failure_streak: 99,
        });
        let start = 5 * HOUR_NS;
        limiter.record(start, true);
        limiter.record(start + 10 * MINUTE, true);
        let now = start + 20 * MINUTE;
        match limiter.check(now) {
            Err(Exhausted::Hourly { retry_in_ns, .. }) => {
                // The oldest action expires 40 minutes from now.
                assert_eq!(retry_in_ns, 40 * MINUTE);
            }
            other => panic!("expected an hourly refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_retry_hint_is_right_when_more_are_recorded_than_the_limit() {
        // Limits can be lowered while history exists. Waiting for the single
        // oldest action then leaves the window still over budget.
        let mut limiter = RateLimiter::new(Limits {
            per_hour: 10,
            per_day: 100,
            failure_streak: 99,
        });
        let start = 5 * HOUR_NS;
        for n in 0..10 {
            limiter.record(start + n * MINUTE, true);
        }
        limiter.set_limits(Limits {
            per_hour: 2,
            per_day: 100,
            failure_streak: 99,
        });
        let now = start + 20 * MINUTE;
        match limiter.check(now) {
            Err(Exhausted::Hourly { retry_in_ns, .. }) => {
                // The eighth action is the one that has to fall out.
                assert_eq!(retry_in_ns, (HOUR_NS + 8 * MINUTE) - 20 * MINUTE);
            }
            other => panic!("expected an hourly refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_run_of_failures_stops_everything() {
        // An autonomous loop that keeps failing is not making progress, it is
        // repeating a mistake. Stopping is the correct response.
        let mut limiter = RateLimiter::new(Limits {
            per_hour: 100,
            per_day: 100,
            failure_streak: 3,
        });
        let now = HOUR_NS;
        for _ in 0..3 {
            limiter.record(now, false);
        }
        match limiter.check(now) {
            Err(Exhausted::FailureStreak { failures }) => assert_eq!(failures, 3),
            other => panic!("expected the streak to stop it, got {other:?}"),
        }
    }

    #[test]
    fn one_success_clears_the_streak() {
        let mut limiter = RateLimiter::new(Limits {
            per_hour: 100,
            per_day: 100,
            failure_streak: 3,
        });
        let now = HOUR_NS;
        limiter.record(now, false);
        limiter.record(now, false);
        limiter.record(now, true);
        assert_eq!(limiter.failures(), 0);
        assert!(limiter.check(now).is_ok());
    }

    #[test]
    fn forgiving_lets_it_try_again_without_forgetting_the_budget() {
        let mut limiter = RateLimiter::new(Limits {
            per_hour: 10,
            per_day: 100,
            failure_streak: 2,
        });
        let now = HOUR_NS;
        limiter.record(now, false);
        limiter.record(now, false);
        assert!(limiter.check(now).is_err());
        limiter.forgive();
        assert!(limiter.check(now).is_ok());
        assert_eq!(limiter.in_last_hour(now), 2, "the budget was still spent");
    }

    #[test]
    fn the_daily_budget_binds_even_when_the_hourly_one_does_not() {
        let mut limiter = RateLimiter::new(Limits {
            per_hour: 100,
            per_day: 4,
            failure_streak: 99,
        });
        let mut now = DAY_NS;
        for _ in 0..4 {
            limiter.record(now, true);
            now += 2 * HOUR_NS;
        }
        assert!(matches!(limiter.check(now), Err(Exhausted::Daily { .. })));
        assert_eq!(limiter.in_last_hour(now), 0, "nothing in the last hour");
    }

    #[test]
    fn history_never_grows_without_bound() {
        let mut limiter = RateLimiter::new(Limits {
            per_hour: 5,
            per_day: 10,
            failure_streak: 99,
        });
        for n in 0..100_000u64 {
            limiter.record(n * 1_000_000, true);
        }
        assert!(limiter.taken.len() <= 20, "{}", limiter.taken.len());
    }

    #[test]
    fn every_refusal_explains_itself_in_words() {
        let refusals = [
            Exhausted::Hourly {
                limit: 12,
                retry_in_ns: 25 * MINUTE,
            },
            Exhausted::Daily {
                limit: 60,
                retry_in_ns: 3 * HOUR_NS,
            },
            Exhausted::FailureStreak { failures: 3 },
        ];
        for refusal in refusals {
            let text = refusal.to_string();
            assert!(text.len() > 30, "{text}");
            assert!(
                text.contains("CoreScout") || text.contains("changes"),
                "{text}"
            );
        }
        assert!(Exhausted::Hourly {
            limit: 1,
            retry_in_ns: 25 * MINUTE
        }
        .to_string()
        .contains("25 minutes"));
    }
}
