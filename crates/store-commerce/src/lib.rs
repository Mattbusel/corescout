//! What the Microsoft Store says about this copy of CoreScout.
//!
//! # The whole commercial design
//!
//! CoreScout is a paid app. You buy it in the Store, once, and you own it.
//! Microsoft takes the payment, handles tax and refunds, enforces the licence,
//! and lets you install it on your other machines under the same account.
//!
//! That is the entire model. There are no add-ons, no subscriptions, no
//! licence keys, no activation, no entitlement tiers and no paywall inside the
//! product, because the Store will not let an unlicensed copy run in the first
//! place. Everything this module does is ask one question so the interface can
//! say something true about the answer.
//!
//! An earlier design sold three add-ons and gated features behind them. It
//! worked, it was tested, and it was several hundred lines of licence
//! verification, entitlement checks and purchase flow to arrive where the
//! Store gets to for free. It is gone.
//!
//! # The trial
//!
//! Microsoft's, not ours. A paid app can offer a free trial period; the Store
//! grants it, counts it down, and stops the app when it ends. So the trial
//! needs no clock-rollback defence, no high-water mark and no stored state,
//! because the thing being defended is not on this machine.
//!
//! # Why the answer can be absent
//!
//! Every call here goes through WinRT, and the Store refuses to answer for a
//! process that is not in a Store-installed package: a side-loaded build, a
//! `cargo run`, or a package whose identity was never registered. That is not
//! a failure and not something to report to a user. It is
//! [`Unavailable::NotPackaged`], and it is what every development build sees.

#![cfg_attr(not(windows), allow(dead_code))]

use serde::{Deserialize, Serialize};
use std::fmt;

/// What the Store says about this copy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Licence {
    /// Whether this copy is licensed to run at all.
    ///
    /// The Store does not normally let an unlicensed copy start, so this is
    /// very nearly always true. It is read rather than assumed because a
    /// refund can revoke a licence while the app is open.
    pub active: bool,
    /// Whether this is the free trial rather than a purchase.
    pub trial: bool,
    /// Days left of the trial, rounded up so the first day reads as the full
    /// length and the last reads as one rather than zero. `None` when this is
    /// not a trial.
    pub trial_days_left: Option<u64>,
}

impl Licence {
    /// One line for the Settings screen.
    pub fn headline(&self) -> String {
        match (self.active, self.trial, self.trial_days_left) {
            (false, _, _) => "This copy of CoreScout is not licensed on this account.".into(),
            (true, false, _) => "You own CoreScout.".into(),
            (true, true, Some(0)) | (true, true, Some(1)) => {
                "Your CoreScout trial ends today.".into()
            }
            (true, true, Some(days)) => format!("{days} days left of your CoreScout trial."),
            (true, true, None) => "You are trying CoreScout.".into(),
        }
    }

    /// Whether to say anything at all about it on the way in.
    ///
    /// A trial in its last two days is worth one calm line. A purchase is not
    /// worth mentioning: somebody who bought this does not need reminding that
    /// they bought it.
    pub fn worth_mentioning(&self) -> bool {
        !self.active || (self.trial && self.trial_days_left.is_some_and(|days| days <= 2))
    }
}

/// Why there is no answer.
///
/// Deliberately not an error type that anything logs loudly. Two of these
/// three are the normal state of a development machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unavailable {
    /// Not running from an installed package, so there is no Store identity to
    /// ask about. Every build outside the Store is this.
    NotPackaged,
    /// Not built for Windows.
    NotWindows,
    /// The Store was asked and could not answer: no network, no signed-in
    /// account, or a service having a bad day.
    Unreachable(String),
}

impl fmt::Display for Unavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unavailable::NotPackaged => {
                write!(formatter, "not installed from the Microsoft Store")
            }
            Unavailable::NotWindows => write!(formatter, "not Windows"),
            Unavailable::Unreachable(why) => {
                write!(formatter, "the Microsoft Store did not answer: {why}")
            }
        }
    }
}

/// Ask the Store about this copy.
#[cfg(windows)]
pub fn licence() -> Result<Licence, Unavailable> {
    use windows::Services::Store::StoreContext;

    if corescout_storage::packaged::family_name().is_none() {
        return Err(Unavailable::NotPackaged);
    }

    let reach = |error: windows::core::Error| Unavailable::Unreachable(error.message().to_string());

    let app = StoreContext::GetDefault()
        .map_err(reach)?
        .GetAppLicenseAsync()
        .map_err(reach)?
        .get()
        .map_err(reach)?;

    let trial = app.IsTrial().map_err(reach)?;
    Ok(Licence {
        active: app.IsActive().map_err(reach)?,
        trial,
        trial_days_left: if trial {
            // A WinRT TimeSpan is 100-nanosecond intervals. Rounded up, so a
            // trial with four hours left says one day rather than zero.
            let remaining = app.TrialTimeRemaining().map_err(reach)?.Duration;
            Some(if remaining <= 0 {
                0
            } else {
                (remaining as u64).div_ceil(24 * 60 * 60 * 10_000_000)
            })
        } else {
            None
        },
    })
}

/// Ask the Store about this copy.
#[cfg(not(windows))]
pub fn licence() -> Result<Licence, Unavailable> {
    Err(Unavailable::NotWindows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owned() -> Licence {
        Licence {
            active: true,
            trial: false,
            trial_days_left: None,
        }
    }

    fn trial(days: u64) -> Licence {
        Licence {
            active: true,
            trial: true,
            trial_days_left: Some(days),
        }
    }

    /// Whatever the Store does, the answer for a developer build is the quiet
    /// one. A build that reported "the Microsoft Store did not answer" on
    /// every `cargo run` would train everyone to ignore it.
    #[test]
    fn an_ordinary_build_is_not_an_error_worth_showing() {
        match licence() {
            Err(Unavailable::NotPackaged) | Err(Unavailable::NotWindows) => {}
            Err(Unavailable::Unreachable(why)) => {
                panic!("an unpackaged build should not reach the Store at all: {why}")
            }
            Ok(found) => panic!("an unpackaged build has no licence, got {found:?}"),
        }
    }

    #[test]
    fn every_reason_can_be_shown_to_somebody() {
        for reason in [
            Unavailable::NotPackaged,
            Unavailable::NotWindows,
            Unavailable::Unreachable("no network".into()),
        ] {
            let said = reason.to_string();
            assert!(!said.is_empty());
            assert!(!said.contains("Unavailable"), "{said}");
        }
    }

    /// Somebody who paid is not told about it every time they open Settings.
    #[test]
    fn owning_it_is_not_news() {
        assert_eq!(owned().headline(), "You own CoreScout.");
        assert!(!owned().worth_mentioning());
    }

    #[test]
    fn a_trial_counts_down_and_says_so_only_near_the_end() {
        assert_eq!(trial(7).headline(), "7 days left of your CoreScout trial.");
        assert!(!trial(7).worth_mentioning(), "day one should be quiet");
        assert!(!trial(3).worth_mentioning());
        assert!(trial(2).worth_mentioning());
        assert!(trial(1).worth_mentioning());
    }

    /// The last day says so rather than counting, because "1 days" is wrong
    /// and "0 days left" is worse.
    #[test]
    fn the_last_day_says_so_rather_than_counting() {
        assert_eq!(trial(1).headline(), "Your CoreScout trial ends today.");
        assert_eq!(trial(0).headline(), "Your CoreScout trial ends today.");
    }

    /// A revoked licence is the one case worth interrupting somebody about,
    /// and it must not be described as a trial.
    #[test]
    fn a_revoked_licence_says_so_plainly() {
        let revoked = Licence {
            active: false,
            trial: false,
            trial_days_left: None,
        };
        assert!(revoked.worth_mentioning());
        assert!(revoked.headline().contains("not licensed"), "{revoked:?}");
    }
}
