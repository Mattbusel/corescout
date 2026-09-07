//! What was noticed, and on what grounds.
//!
//! [`Basis`] is the load-bearing type in this crate. Every learned item the
//! interface shows carries one, and the two variants are shown differently,
//! worded differently, and trusted differently.

use serde::{Deserialize, Serialize};

/// What a pattern is about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "about", content = "id")]
pub enum Subject {
    /// An operation, wherever it happens.
    Operation(String),
    /// An operation in one place.
    OperationHere {
        /// The normalised operation.
        operation: String,
        /// The workspace it is about.
        workspace: String,
    },
    /// A repository or folder.
    Workspace(String),
    /// One AI system.
    Agent(String),
    /// A recurring state of the machine itself.
    MachineState(u32),
}

impl Subject {
    /// A stable key.
    pub fn key(&self) -> String {
        match self {
            Subject::Operation(op) => format!("op:{op}"),
            Subject::OperationHere {
                operation,
                workspace,
            } => {
                format!("op:{operation}@{workspace}")
            }
            Subject::Workspace(id) => format!("ws:{id}"),
            Subject::Agent(id) => format!("agent:{id}"),
            Subject::MachineState(id) => format!("state:{id}"),
        }
    }
}

/// On what grounds CoreScout believes something.
///
/// The distinction is not decoration. An association says two things were seen
/// together; a causal basis says CoreScout deliberately varied one of them and
/// measured what happened. Only the second supports "do this instead".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "basis")]
pub enum Basis {
    /// Seen together. Might be the procedure; might be the kind of run that
    /// tends to use it.
    Association {
        /// Times the two were seen together.
        together: u64,
        /// Times the subject occurred at all.
        occurrences: u64,
        /// Failure rate when the condition held.
        rate_with: f64,
        /// Failure rate when it did not.
        rate_without: f64,
    },
    /// Measured under randomised assignment.
    Causal {
        /// The difference, in the units of the outcome.
        delta: f64,
        /// The standard error of that difference.
        standard_error: f64,
        /// Randomised trials on the treatment arm.
        treatment_trials: u32,
        /// Randomised trials on the control arm.
        control_trials: u32,
        /// Failure rate when the procedure was applied.
        rate_with: f64,
        /// Failure rate when it was not.
        rate_without: f64,
    },
}

impl Basis {
    /// Whether this rests on randomised evidence.
    pub fn is_causal(&self) -> bool {
        matches!(self, Basis::Causal { .. })
    }

    /// The word the interface shows on the card.
    pub fn label(&self) -> &'static str {
        match self {
            Basis::Association { .. } => "Seen together",
            Basis::Causal { .. } => "Verified",
        }
    }

    /// How strongly this is believed, from zero to one.
    ///
    /// For an association it is repetition and lift; for a causal basis it is
    /// how many standard errors the difference is. The two are not on the same
    /// scale and the interface never puts them side by side without the label.
    pub fn confidence(&self) -> f64 {
        match self {
            Basis::Association {
                together,
                rate_with,
                rate_without,
                ..
            } => {
                let lift = (rate_without - rate_with).abs().min(1.0);
                let weight = (*together as f64 / 12.0).min(1.0);
                // Capped below one on purpose. An association is never
                // certain, however many times it has been seen, and a number
                // that can reach 100% invites the interface to say so.
                (lift * weight * 0.75).clamp(0.0, 0.75)
            }
            Basis::Causal {
                delta,
                standard_error,
                ..
            } => {
                if *standard_error <= 0.0 || !standard_error.is_finite() {
                    return 0.0;
                }
                let sigmas = (delta / standard_error).abs();
                (sigmas / 4.0).clamp(0.0, 0.99)
            }
        }
    }

    /// One sentence explaining the grounds, for the "Why?" panel.
    pub fn explain(&self) -> String {
        match self {
            Basis::Association {
                together,
                occurrences,
                rate_with,
                rate_without,
            } => format!(
                "Seen together {together} times out of {occurrences}. Failures ran at {}% when \
                 it held and {}% when it did not. CoreScout has not tested whether one causes \
                 the other.",
                percent(*rate_with),
                percent(*rate_without)
            ),
            Basis::Causal {
                treatment_trials,
                control_trials,
                rate_with,
                rate_without,
                ..
            } => format!(
                "In {} randomised trials, failures ran at {}% with this procedure and {}% \
                 without it ({treatment_trials} with, {control_trials} without).",
                treatment_trials + control_trials,
                percent(*rate_with),
                percent(*rate_without)
            ),
        }
    }
}

fn percent(rate: f64) -> u64 {
    (rate.clamp(0.0, 1.0) * 100.0).round() as u64
}

/// Something CoreScout has noticed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    /// Stable across restarts.
    pub id: String,
    /// What it is about.
    pub subject: Subject,
    /// One line, for someone who has read nothing.
    pub headline: String,
    /// Why it matters, in a sentence.
    pub detail: String,
    /// On what grounds.
    pub basis: Basis,
    /// Wall clock of the first supporting observation.
    pub first_ms: u64,
    /// Wall clock of the most recent one.
    pub last_ms: u64,
    /// Whether this has been withdrawn, and why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retired: Option<String>,
}

impl Pattern {
    /// Whether this is still believed.
    pub fn is_live(&self) -> bool {
        self.retired.is_none()
    }

    /// Withdraw it.
    pub fn retire(&mut self, reason: impl Into<String>) {
        self.retired = Some(reason.into());
    }

    /// How strongly it is believed.
    pub fn confidence(&self) -> f64 {
        if self.is_live() {
            self.basis.confidence()
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn association(together: u64, with: f64, without: f64) -> Basis {
        Basis::Association {
            together,
            occurrences: together * 2,
            rate_with: with,
            rate_without: without,
        }
    }

    #[test]
    fn an_association_never_reaches_certainty() {
        // However many times two things have been seen together, they have
        // still only been seen together. A confidence that can reach 100%
        // invites the interface to print it.
        let overwhelming = association(100_000, 0.0, 1.0);
        assert!(
            overwhelming.confidence() <= 0.75,
            "{}",
            overwhelming.confidence()
        );
        assert!(!overwhelming.is_causal());
        assert_eq!(overwhelming.label(), "Seen together");
    }

    #[test]
    fn a_causal_basis_says_so() {
        let measured = Basis::Causal {
            delta: -0.36,
            standard_error: 0.06,
            treatment_trials: 14,
            control_trials: 15,
            rate_with: 0.07,
            rate_without: 0.43,
        };
        assert!(measured.is_causal());
        assert_eq!(measured.label(), "Verified");
        assert!(measured.confidence() > 0.75);
    }

    #[test]
    fn a_causal_estimate_with_no_spread_is_not_infinitely_confident() {
        // A standard error of zero is a degenerate sample, not certainty.
        let degenerate = Basis::Causal {
            delta: -0.5,
            standard_error: 0.0,
            treatment_trials: 2,
            control_trials: 2,
            rate_with: 0.0,
            rate_without: 0.5,
        };
        assert_eq!(degenerate.confidence(), 0.0);
    }

    #[test]
    fn a_weak_association_is_weakly_believed() {
        let barely = association(3, 0.40, 0.44);
        assert!(barely.confidence() < 0.05, "{}", barely.confidence());
    }

    #[test]
    fn repetition_alone_does_not_make_an_association_strong() {
        // Seen together a thousand times with no difference in outcome is a
        // thousand observations of nothing.
        let pointless = association(1000, 0.5, 0.5);
        assert_eq!(pointless.confidence(), 0.0);
    }

    #[test]
    fn an_association_explains_that_it_has_not_been_tested() {
        let text = association(7, 0.1, 0.5).explain();
        assert!(text.contains("has not tested"), "{text}");
        assert!(text.contains("10%") && text.contains("50%"), "{text}");
    }

    #[test]
    fn a_causal_explanation_names_the_randomised_trials() {
        let text = Basis::Causal {
            delta: -0.36,
            standard_error: 0.06,
            treatment_trials: 14,
            control_trials: 15,
            rate_with: 0.07,
            rate_without: 0.43,
        }
        .explain();
        assert!(text.contains("29 randomised trials"), "{text}");
        assert!(text.contains("7%") && text.contains("43%"), "{text}");
    }

    #[test]
    fn a_retired_pattern_is_believed_at_zero() {
        let mut pattern = Pattern {
            id: "p1".into(),
            subject: Subject::Operation("cargo build".into()),
            headline: "builds fail".into(),
            detail: "".into(),
            basis: association(20, 0.1, 0.6),
            first_ms: 0,
            last_ms: 1,
            retired: None,
        };
        assert!(pattern.confidence() > 0.0);
        pattern.retire("the failures stopped");
        assert!(!pattern.is_live());
        assert_eq!(pattern.confidence(), 0.0);
    }

    #[test]
    fn subjects_key_distinctly() {
        let keys = [
            Subject::Operation("cargo build".into()).key(),
            Subject::OperationHere {
                operation: "cargo build".into(),
                workspace: "app".into(),
            }
            .key(),
            Subject::Workspace("app".into()).key(),
            Subject::Agent("claude-code".into()).key(),
            Subject::MachineState(21).key(),
        ];
        let mut sorted = keys.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len());
    }

    #[test]
    fn a_pattern_survives_storage() {
        let pattern = Pattern {
            id: "p1".into(),
            subject: Subject::MachineState(21),
            headline: "h".into(),
            detail: "d".into(),
            basis: association(9, 0.1, 0.5),
            first_ms: 1,
            last_ms: 2,
            retired: None,
        };
        let json = serde_json::to_string(&pattern).expect("serialise");
        assert_eq!(
            serde_json::from_str::<Pattern>(&json).expect("deserialise"),
            pattern
        );
    }
}
