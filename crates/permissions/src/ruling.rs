//! The decision itself: may this happen, and if not, why not.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::authority::{Authority, Target};
use crate::autonomy::Autonomy;
use crate::limits::{Exhausted, Limits, RateLimiter};

/// How much a change could cost if it is wrong.
///
/// This is about consequence, not confidence. A change CoreScout is certain
/// about that would stop a service is still [`Risk::High`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Nothing outside CoreScout notices. Reading, recording, measuring.
    #[default]
    None,
    /// Affects how CoreScout's own work runs, and undoes itself.
    Low,
    /// Affects something the user is running.
    Moderate,
    /// Could stop something working, or is hard to undo.
    High,
}

impl Risk {
    /// The word shown in the interface.
    pub fn label(self) -> &'static str {
        match self {
            Risk::None => "no effect outside CoreScout",
            Risk::Low => "small and reversible",
            Risk::Moderate => "affects your work",
            Risk::High => "could break something",
        }
    }
}

/// A request to do something.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Which capability is asking. Grants are keyed on this.
    pub capability: String,
    /// What it would touch.
    pub targets: Vec<Target>,
    /// How much it could cost if wrong.
    pub risk: Risk,
    /// Whether CoreScout knows how to put it back.
    pub reversible: bool,
    /// Why, in one line, for the approval prompt and the audit log.
    pub reason: String,
}

impl Request {
    /// A request with the safe defaults: nothing risky, reversible.
    pub fn new(capability: impl Into<String>, reason: impl Into<String>) -> Request {
        Request {
            capability: capability.into(),
            targets: Vec::new(),
            risk: Risk::None,
            reversible: true,
            reason: reason.into(),
        }
    }

    /// Name something this would touch.
    pub fn touching(mut self, target: Target) -> Request {
        self.targets.push(target);
        self
    }

    /// Set how much it could cost.
    pub fn risking(mut self, risk: Risk) -> Request {
        self.risk = risk;
        self
    }

    /// Declare that CoreScout cannot put this back.
    pub fn irreversible(mut self) -> Request {
        self.reversible = false;
        self
    }
}

/// What the permission layer decided.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum Ruling {
    /// Go ahead.
    Allowed {
        /// Why it was allowed, for the audit log.
        because: String,
    },
    /// Ask the user first.
    NeedsApproval {
        /// What to tell them.
        because: String,
    },
    /// No, and this is why.
    Refused {
        /// What to tell them.
        because: String,
    },
}

impl Ruling {
    /// Whether the action may proceed right now.
    pub fn is_allowed(&self) -> bool {
        matches!(self, Ruling::Allowed { .. })
    }

    /// Whether a person could unblock this by saying yes.
    pub fn needs_approval(&self) -> bool {
        matches!(self, Ruling::NeedsApproval { .. })
    }

    /// The explanation, whichever way it went.
    pub fn because(&self) -> &str {
        match self {
            Ruling::Allowed { because }
            | Ruling::NeedsApproval { because }
            | Ruling::Refused { because } => because,
        }
    }
}

/// What the user decided about one capability.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Grant {
    /// Whether the user has approved this capability at all.
    pub approved: bool,
    /// Whether connected AI agents may invoke it without asking each time.
    pub auto_use: bool,
    /// Whether the user has switched it off. Beats `approved`.
    pub disabled: bool,
}

impl Grant {
    /// An approved capability that still asks before each use.
    pub fn approved() -> Grant {
        Grant {
            approved: true,
            auto_use: false,
            disabled: false,
        }
    }

    /// An approved capability an agent may use on its own.
    pub fn automatic() -> Grant {
        Grant {
            approved: true,
            auto_use: true,
            disabled: false,
        }
    }

    /// Whether this grant permits anything at all.
    pub fn is_usable(&self) -> bool {
        self.approved && !self.disabled
    }
}

/// Everything that decides whether CoreScout may act.
#[derive(Debug, Default)]
pub struct Permissions {
    autonomy: Autonomy,
    authority: Authority,
    grants: BTreeMap<String, Grant>,
    limiter: RateLimiter,
    paused: bool,
}

/// The stored form, so the whole state survives a restart.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Stored {
    /// The autonomy mode in force.
    pub autonomy: Autonomy,
    /// What may be touched.
    pub authority: Authority,
    /// Per-capability grants.
    pub grants: BTreeMap<String, Grant>,
    /// The rate budget.
    pub limits: Limits,
    /// Whether the user has pressed pause.
    pub paused: bool,
}

impl Permissions {
    /// The safe starting position: Suggest, own process only, nothing granted.
    pub fn new() -> Permissions {
        Permissions {
            autonomy: Autonomy::default(),
            authority: Authority::own_process_only(),
            grants: BTreeMap::new(),
            limiter: RateLimiter::new(Limits::default()),
            paused: false,
        }
    }

    /// Rebuild from storage.
    pub fn from_stored(stored: Stored) -> Permissions {
        Permissions {
            autonomy: stored.autonomy,
            authority: stored.authority,
            grants: stored.grants,
            limiter: RateLimiter::new(stored.limits),
            paused: stored.paused,
        }
    }

    /// The form to store. The rate history is not kept: after a restart the
    /// budget starts fresh, which is the forgiving direction and matters less
    /// than the alternative of a stale budget blocking a user who just
    /// restarted to unblock themselves.
    pub fn to_stored(&self) -> Stored {
        Stored {
            autonomy: self.autonomy,
            authority: self.authority.clone(),
            grants: self.grants.clone(),
            limits: self.limiter.limits(),
            paused: self.paused,
        }
    }

    /// The mode in force.
    pub fn autonomy(&self) -> Autonomy {
        self.autonomy
    }

    /// Change the mode.
    pub fn set_autonomy(&mut self, autonomy: Autonomy) {
        self.autonomy = autonomy;
    }

    /// What may be touched.
    pub fn authority(&self) -> &Authority {
        &self.authority
    }

    /// Change what may be touched.
    pub fn authority_mut(&mut self) -> &mut Authority {
        &mut self.authority
    }

    /// The rate budget and its history.
    pub fn limiter(&self) -> &RateLimiter {
        &self.limiter
    }

    /// The rate budget and its history.
    pub fn limiter_mut(&mut self) -> &mut RateLimiter {
        &mut self.limiter
    }

    /// Stop everything now.
    ///
    /// This is the button. It is a single boolean checked before anything
    /// else, so there is no path through this crate that acts while it is set.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Start again.
    pub fn resume(&mut self) {
        self.paused = false;
    }

    /// Whether everything is stopped.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// What the user decided about a capability.
    pub fn grant(&self, capability: &str) -> Grant {
        self.grants.get(capability).cloned().unwrap_or_default()
    }

    /// Record a decision about a capability.
    pub fn set_grant(&mut self, capability: impl Into<String>, grant: Grant) {
        self.grants.insert(capability.into(), grant);
    }

    /// Forget a decision about a capability.
    pub fn forget_grant(&mut self, capability: &str) {
        self.grants.remove(capability);
    }

    /// Every grant, for the settings screen.
    pub fn grants(&self) -> &BTreeMap<String, Grant> {
        &self.grants
    }

    /// Decide whether a request may proceed.
    ///
    /// The order is pause, authority, autonomy, grant, rate limit. Authority
    /// before autonomy is the load-bearing part: switching on Autopilot must
    /// not widen reach.
    pub fn rule(&self, request: &Request, now_ns: u64) -> Ruling {
        if self.paused {
            return Ruling::Refused {
                because: "CoreScout is paused".into(),
            };
        }

        if let Err(denial) = self.authority.permits(&request.targets) {
            return Ruling::Refused {
                because: denial.to_string(),
            };
        }

        let grant = self.grant(&request.capability);
        if grant.disabled {
            return Ruling::Refused {
                because: format!("you switched {} off", request.capability),
            };
        }

        // Reading and measuring is what Observe mode is *for*, so a request
        // that changes nothing outside CoreScout is not gated on mode at all.
        if request.risk == Risk::None {
            return self.against_the_budget(now_ns, "it changes nothing outside CoreScout");
        }

        match self.autonomy {
            Autonomy::Observe => Ruling::Refused {
                because: "CoreScout is in Observe mode and makes no changes".into(),
            },
            Autonomy::Suggest => Ruling::NeedsApproval {
                because: "CoreScout is in Suggest mode, so every change waits for you".into(),
            },
            Autonomy::Assist => {
                if request.risk <= Risk::Low && request.reversible {
                    self.against_the_budget(now_ns, "it is small and CoreScout can undo it")
                } else {
                    Ruling::NeedsApproval {
                        because: format!(
                            "this change is {} and Assist mode asks first",
                            request.risk.label()
                        ),
                    }
                }
            }
            Autonomy::Autopilot => {
                if request.risk >= Risk::High {
                    return Ruling::NeedsApproval {
                        because: "this change could break something, so it waits for you even on \
                                  Autopilot"
                            .into(),
                    };
                }
                if !request.reversible {
                    return Ruling::NeedsApproval {
                        because: "CoreScout cannot undo this change, so it waits for you".into(),
                    };
                }
                if !grant.is_usable() {
                    return Ruling::NeedsApproval {
                        because: format!("you have not approved {} yet", request.capability),
                    };
                }
                self.against_the_budget(now_ns, "you approved this and it is within your limits")
            }
        }
    }

    fn against_the_budget(&self, now_ns: u64, because: &str) -> Ruling {
        match self.limiter.check(now_ns) {
            Ok(()) => Ruling::Allowed {
                because: because.into(),
            },
            Err(exhausted) => refusal(exhausted),
        }
    }
}

fn refusal(exhausted: Exhausted) -> Ruling {
    match exhausted {
        // A failure streak is something a person clears, so it is a question
        // rather than a wall.
        Exhausted::FailureStreak { .. } => Ruling::NeedsApproval {
            because: exhausted.to_string(),
        },
        _ => Ruling::Refused {
            because: exhausted.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn change(risk: Risk) -> Request {
        Request::new("pin_build", "builds here run on the slow cores").risking(risk)
    }

    #[test]
    fn pause_beats_everything() {
        // The emergency stop is the one promise that cannot have an exception,
        // so it is tested against the most permissive configuration there is.
        let mut permissions = Permissions::new();
        permissions.set_autonomy(Autonomy::Autopilot);
        permissions.set_grant("pin_build", Grant::automatic());
        assert!(permissions.rule(&change(Risk::Low), 0).is_allowed());
        permissions.pause();
        for risk in [Risk::None, Risk::Low, Risk::Moderate, Risk::High] {
            let ruling = permissions.rule(&change(risk), 0);
            assert_eq!(
                ruling,
                Ruling::Refused {
                    because: "CoreScout is paused".into()
                }
            );
        }
    }

    #[test]
    fn autopilot_cannot_widen_reach() {
        // The central rule of this crate: autonomy decides whether a person is
        // asked, never what may be touched.
        let mut permissions = Permissions::new();
        permissions.set_autonomy(Autonomy::Autopilot);
        permissions.set_grant("pin_build", Grant::automatic());
        let request = change(Risk::Low).touching(Target::Path(PathBuf::from("C:\\Projects\\app")));
        let ruling = permissions.rule(&request, 0);
        assert!(!ruling.is_allowed(), "{ruling:?}");
        assert!(
            ruling.because().contains("outside the folders"),
            "{ruling:?}"
        );
    }

    #[test]
    fn a_forbidden_path_is_refused_at_every_mode() {
        let key = Target::Path(PathBuf::from("C:\\Users\\someone\\.ssh\\id_rsa"));
        for mode in Autonomy::all() {
            let mut permissions = Permissions::new();
            permissions.set_autonomy(mode);
            permissions.set_grant("pin_build", Grant::automatic());
            permissions.authority_mut().grant_root("C:\\Users\\someone");
            let ruling = permissions.rule(&change(Risk::Low).touching(key.clone()), 0);
            assert!(
                !ruling.is_allowed(),
                "{} allowed it: {ruling:?}",
                mode.as_str()
            );
        }
    }

    #[test]
    fn observe_refuses_every_change_and_permits_every_measurement() {
        let mut permissions = Permissions::new();
        permissions.set_autonomy(Autonomy::Observe);
        assert!(permissions.rule(&change(Risk::None), 0).is_allowed());
        for risk in [Risk::Low, Risk::Moderate, Risk::High] {
            let ruling = permissions.rule(&change(risk), 0);
            assert!(matches!(ruling, Ruling::Refused { .. }));
            assert!(ruling.because().contains("Observe"));
        }
    }

    #[test]
    fn suggest_asks_about_everything_that_changes_anything() {
        let permissions = Permissions::new();
        assert_eq!(permissions.autonomy(), Autonomy::Suggest);
        for risk in [Risk::Low, Risk::Moderate, Risk::High] {
            assert!(permissions.rule(&change(risk), 0).needs_approval());
        }
        assert!(permissions.rule(&change(Risk::None), 0).is_allowed());
    }

    #[test]
    fn assist_acts_on_small_reversible_changes_only() {
        let mut permissions = Permissions::new();
        permissions.set_autonomy(Autonomy::Assist);
        assert!(permissions.rule(&change(Risk::Low), 0).is_allowed());
        assert!(permissions
            .rule(&change(Risk::Moderate), 0)
            .needs_approval());
        assert!(permissions
            .rule(&change(Risk::Low).irreversible(), 0)
            .needs_approval());
    }

    #[test]
    fn autopilot_still_asks_about_anything_that_could_break_something() {
        // "Autopilot" is not "unrestricted". If this test ever goes green with
        // an Allowed, the mode has stopped meaning what the Settings page says.
        let mut permissions = Permissions::new();
        permissions.set_autonomy(Autonomy::Autopilot);
        permissions.set_grant("pin_build", Grant::automatic());
        assert!(permissions.rule(&change(Risk::High), 0).needs_approval());
        assert!(permissions
            .rule(&change(Risk::Moderate).irreversible(), 0)
            .needs_approval());
        assert!(permissions.rule(&change(Risk::Moderate), 0).is_allowed());
    }

    #[test]
    fn autopilot_will_not_use_a_capability_the_user_never_approved() {
        let mut permissions = Permissions::new();
        permissions.set_autonomy(Autonomy::Autopilot);
        let ruling = permissions.rule(&change(Risk::Low), 0);
        assert!(ruling.needs_approval());
        assert!(ruling.because().contains("not approved"), "{ruling:?}");
    }

    #[test]
    fn switching_a_capability_off_refuses_it_outright() {
        let mut permissions = Permissions::new();
        permissions.set_autonomy(Autonomy::Autopilot);
        permissions.set_grant(
            "pin_build",
            Grant {
                approved: true,
                auto_use: true,
                disabled: true,
            },
        );
        let ruling = permissions.rule(&change(Risk::Low), 0);
        assert!(matches!(ruling, Ruling::Refused { .. }));
        assert!(ruling.because().contains("switched"), "{ruling:?}");
    }

    #[test]
    fn a_spent_budget_stops_an_autopilot_that_is_otherwise_entitled() {
        let mut permissions = Permissions::new();
        permissions.set_autonomy(Autonomy::Autopilot);
        permissions.set_grant("pin_build", Grant::automatic());
        permissions.limiter_mut().set_limits(Limits {
            per_hour: 2,
            per_day: 10,
            failure_streak: 9,
        });
        for _ in 0..2 {
            assert!(permissions.rule(&change(Risk::Low), 0).is_allowed());
            permissions.limiter_mut().record(0, true);
        }
        let ruling = permissions.rule(&change(Risk::Low), 0);
        assert!(matches!(ruling, Ruling::Refused { .. }));
        assert!(ruling.because().contains("this hour"), "{ruling:?}");
    }

    #[test]
    fn a_failure_streak_becomes_a_question_rather_than_a_wall() {
        // The user can clear it, so refusing outright would leave them with no
        // way forward and no idea why.
        let mut permissions = Permissions::new();
        permissions.set_autonomy(Autonomy::Assist);
        permissions.limiter_mut().set_limits(Limits {
            per_hour: 50,
            per_day: 50,
            failure_streak: 2,
        });
        permissions.limiter_mut().record(0, false);
        permissions.limiter_mut().record(0, false);
        assert!(permissions.rule(&change(Risk::Low), 0).needs_approval());
    }

    #[test]
    fn every_ruling_carries_a_reason_worth_showing() {
        let mut permissions = Permissions::new();
        permissions.set_autonomy(Autonomy::Assist);
        for risk in [Risk::None, Risk::Low, Risk::Moderate, Risk::High] {
            let ruling = permissions.rule(&change(risk), 0);
            assert!(ruling.because().len() > 15, "{ruling:?}");
            assert!(
                !ruling.because().contains('_'),
                "no identifiers: {ruling:?}"
            );
        }
    }

    #[test]
    fn the_whole_decision_survives_a_restart() {
        let mut permissions = Permissions::new();
        permissions.set_autonomy(Autonomy::Assist);
        permissions.authority_mut().grant_root("C:\\Projects\\app");
        permissions.set_grant("pin_build", Grant::automatic());
        permissions.pause();

        let json = serde_json::to_string(&permissions.to_stored()).expect("serialise");
        let stored: Stored = serde_json::from_str(&json).expect("deserialise");
        let back = Permissions::from_stored(stored);

        assert_eq!(back.autonomy(), Autonomy::Assist);
        assert!(back.is_paused(), "a pause must survive a restart");
        assert_eq!(back.grant("pin_build"), Grant::automatic());
        assert_eq!(back.authority().roots().len(), 1);
    }

    #[test]
    fn a_capability_nobody_has_decided_about_is_not_approved() {
        let permissions = Permissions::new();
        assert_eq!(permissions.grant("never_seen"), Grant::default());
        assert!(!permissions.grant("never_seen").is_usable());
    }
}
