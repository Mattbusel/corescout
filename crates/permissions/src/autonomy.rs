//! The four autonomy modes, and what each one actually means.

use serde::{Deserialize, Serialize};

/// How much CoreScout may do without being asked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Autonomy {
    /// Watch and learn. Change nothing, ever.
    Observe,
    /// Recommend improvements. Every change waits for a person.
    #[default]
    Suggest,
    /// Carry out low-risk reversible changes; ask about anything else.
    Assist,
    /// Apply changes that have been validated, inside explicit boundaries.
    Autopilot,
}

impl Autonomy {
    /// The stable name used in storage and on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Autonomy::Observe => "observe",
            Autonomy::Suggest => "suggest",
            Autonomy::Assist => "assist",
            Autonomy::Autopilot => "autopilot",
        }
    }

    /// Parse a stored name.
    pub fn parse(name: &str) -> Option<Autonomy> {
        match name {
            "observe" => Some(Autonomy::Observe),
            "suggest" => Some(Autonomy::Suggest),
            "assist" => Some(Autonomy::Assist),
            "autopilot" => Some(Autonomy::Autopilot),
            _ => None,
        }
    }

    /// The name shown in the interface.
    pub fn title(self) -> &'static str {
        match self {
            Autonomy::Observe => "Observe",
            Autonomy::Suggest => "Suggest",
            Autonomy::Assist => "Assist",
            Autonomy::Autopilot => "Autopilot",
        }
    }

    /// One sentence, written for someone who has not read any documentation.
    pub fn summary(self) -> &'static str {
        match self {
            Autonomy::Observe => "CoreScout watches and learns. It changes nothing.",
            Autonomy::Suggest => "CoreScout suggests improvements. You approve every change.",
            Autonomy::Assist => {
                "CoreScout makes small reversible changes on its own. Anything bigger waits for you."
            }
            Autonomy::Autopilot => {
                "CoreScout applies changes it has verified, inside the limits you set."
            }
        }
    }

    /// Whether this mode permits any change at all.
    pub fn permits_change(self) -> bool {
        self != Autonomy::Observe
    }

    /// Whether this mode may act without asking first.
    pub fn may_act_unattended(self) -> bool {
        matches!(self, Autonomy::Assist | Autonomy::Autopilot)
    }

    /// Every mode, from least to most permissive.
    pub fn all() -> [Autonomy; 4] {
        [
            Autonomy::Observe,
            Autonomy::Suggest,
            Autonomy::Assist,
            Autonomy::Autopilot,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_asks_before_changing_anything() {
        // Shipping a product that changes a stranger's machine unattended on
        // first run is not a default, it is an incident.
        assert_eq!(Autonomy::default(), Autonomy::Suggest);
        assert!(!Autonomy::default().may_act_unattended());
    }

    #[test]
    fn modes_order_from_least_to_most_permissive() {
        assert!(Autonomy::Observe < Autonomy::Suggest);
        assert!(Autonomy::Suggest < Autonomy::Assist);
        assert!(Autonomy::Assist < Autonomy::Autopilot);
        assert_eq!(Autonomy::all().len(), 4);
    }

    #[test]
    fn every_mode_round_trips_and_has_words_for_a_person() {
        for mode in Autonomy::all() {
            assert_eq!(Autonomy::parse(mode.as_str()), Some(mode));
            assert!(!mode.title().is_empty());
            assert!(
                mode.summary().len() > 20,
                "{} needs a real sentence",
                mode.as_str()
            );
            assert!(
                !mode.summary().contains("latent"),
                "no jargon in mode summaries"
            );
        }
    }

    #[test]
    fn observe_changes_nothing() {
        assert!(!Autonomy::Observe.permits_change());
        assert!(!Autonomy::Observe.may_act_unattended());
    }

    #[test]
    fn an_unknown_mode_does_not_silently_become_autopilot() {
        assert_eq!(Autonomy::parse("full"), None);
        assert_eq!(Autonomy::parse(""), None);
    }
}
