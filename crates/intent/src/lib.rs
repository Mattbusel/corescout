//! Intent: what outcome is wanted, never which mechanism to use.
//!
//! # The abstraction boundary
//!
//! ```text
//! application intent  ->  self-modelling substrate  ->  physical execution
//! ```
//!
//! An application says what it needs. The substrate decides how. That
//! inversion is the point of the whole project: today an application that
//! cares about latency has to guess at a CPU number, and its guess is frozen
//! into a config file that outlives the machine it was tuned for.
//!
//! ```text
//! bad     pin me to CPU 7
//! good    p99 latency under 30 us; jitter matters more than throughput;
//!         I would rather not be migrated
//! ```
//!
//! The first is a mechanism, is usually wrong, and is unfalsifiable: nothing
//! can tell whether CPU 7 was a good choice. The second is an outcome, can be
//! measured, and can be *scored*, which is what lets a policy learn.
//!
//! # Constraints and preferences are different things
//!
//! A [`Constraint`] is a line that must not be crossed; a candidate that
//! violates one is rejected outright rather than penalised. A [`Preference`] is
//! a direction to move in when there is a choice. Collapsing them into one
//! weighted sum is the classic way to get a controller that trades away a hard
//! deadline for a large enough throughput gain.
//!
//! # Intent is not a self-description
//!
//! Deliberately separate from everything the machine learns about itself. What
//! the machine *is* and what someone *wants from it* are different kinds of
//! claim, and a system that mixes them ends up unable to tell an observation
//! from a wish.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A dimension an application can care about.
///
/// Open enough to grow, closed enough that a policy can enumerate what it is
/// being asked to optimise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Objective {
    /// Time from request to response, at whatever percentile the constraint
    /// names.
    Latency,
    /// Spread of latency. Distinct from latency: a workload can want a
    /// predictable 40 us far more than a variable 20 us.
    Jitter,
    /// Work completed per unit time.
    Throughput,
    /// Energy used doing it.
    Energy,
    /// How much the workload is moved between CPUs. Migration costs cache
    /// warmth, and some workloads pay that price far more than others.
    Migration,
}

impl Objective {
    pub fn label(self) -> &'static str {
        match self {
            Objective::Latency => "latency",
            Objective::Jitter => "jitter",
            Objective::Throughput => "throughput",
            Objective::Energy => "energy",
            Objective::Migration => "migration",
        }
    }

    /// Whether smaller is better on this axis.
    pub fn lower_is_better(self) -> bool {
        !matches!(self, Objective::Throughput)
    }

    pub fn all() -> [Objective; 5] {
        [
            Objective::Latency,
            Objective::Jitter,
            Objective::Throughput,
            Objective::Energy,
            Objective::Migration,
        ]
    }
}

/// A limit that must hold.
///
/// Violating a constraint disqualifies a candidate; it is not traded off.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Constraint {
    pub objective: Objective,
    /// The percentile this applies to, where the objective is a distribution.
    /// 50.0 is the median; 99.0 the usual tail target.
    pub percentile: f64,
    /// The limit, in the objective's natural unit: microseconds for latency
    /// and jitter, operations per second for throughput, microwatts for energy,
    /// migrations per second for migration.
    pub limit: f64,
}

impl Constraint {
    pub fn latency_p99_us(limit: f64) -> Constraint {
        Constraint {
            objective: Objective::Latency,
            percentile: 99.0,
            limit,
        }
    }

    pub fn jitter_p99_us(limit: f64) -> Constraint {
        Constraint {
            objective: Objective::Jitter,
            percentile: 99.0,
            limit,
        }
    }

    pub fn throughput_at_least(limit: f64) -> Constraint {
        Constraint {
            objective: Objective::Throughput,
            percentile: 50.0,
            limit,
        }
    }

    /// Whether a measured value satisfies this constraint.
    pub fn satisfied_by(&self, value: f64) -> bool {
        if !value.is_finite() {
            // An unmeasurable value cannot be shown to satisfy a hard limit,
            // and assuming it does is how a constraint quietly stops meaning
            // anything.
            return false;
        }
        if self.objective.lower_is_better() {
            value <= self.limit
        } else {
            value >= self.limit
        }
    }

    /// How far a value is from satisfying this, as a fraction of the limit.
    /// Negative when satisfied.
    pub fn violation(&self, value: f64) -> f64 {
        if !value.is_finite() || self.limit == 0.0 {
            return f64::INFINITY;
        }
        if self.objective.lower_is_better() {
            (value - self.limit) / self.limit
        } else {
            (self.limit - value) / self.limit
        }
    }
}

impl fmt::Display for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let comparison = if self.objective.lower_is_better() {
            "<="
        } else {
            ">="
        };
        write!(
            f,
            "{} p{:.0} {comparison} {}",
            self.objective.label(),
            self.percentile,
            self.limit
        )
    }
}

/// A direction to move in, weighted against the others.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Preference {
    pub objective: Objective,
    /// Relative importance. Only ratios matter; the set is normalised.
    pub weight: f64,
}

/// How willing a workload is to be moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationTolerance {
    /// Do not move it once placed. For workloads whose working set is large
    /// and whose warm cache is most of their performance.
    None,
    /// Move it only for a large predicted gain.
    Low,
    Medium,
    /// Move freely; the workload is stateless enough not to care.
    High,
}

impl MigrationTolerance {
    /// The minimum predicted improvement, as a fraction, that justifies moving.
    ///
    /// The numbers are judgement calls, stated here so they can be argued with
    /// rather than buried in a policy.
    pub fn improvement_threshold(self) -> f64 {
        match self {
            MigrationTolerance::None => f64::INFINITY,
            MigrationTolerance::Low => 0.20,
            MigrationTolerance::Medium => 0.08,
            MigrationTolerance::High => 0.02,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MigrationTolerance::None => "none",
            MigrationTolerance::Low => "low",
            MigrationTolerance::Medium => "medium",
            MigrationTolerance::High => "high",
        }
    }
}

/// What an application needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Intent {
    /// A name for this workload, for audit records and reports.
    pub name: String,
    /// Limits that must hold. A candidate violating any is rejected.
    pub constraints: Vec<Constraint>,
    /// Directions to move in, among candidates that satisfy the constraints.
    pub preferences: Vec<Preference>,
    pub migration_tolerance: MigrationTolerance,
    /// A wall-clock deadline for the work, where there is one.
    pub deadline_ns: Option<u64>,
}

impl Intent {
    pub fn new(name: impl Into<String>) -> Intent {
        Intent {
            name: name.into(),
            constraints: Vec::new(),
            preferences: Vec::new(),
            migration_tolerance: MigrationTolerance::Medium,
            deadline_ns: None,
        }
    }

    pub fn with_constraint(mut self, constraint: Constraint) -> Intent {
        self.constraints.push(constraint);
        self
    }

    pub fn preferring(mut self, objective: Objective, weight: f64) -> Intent {
        self.preferences.push(Preference { objective, weight });
        self
    }

    pub fn migration(mut self, tolerance: MigrationTolerance) -> Intent {
        self.migration_tolerance = tolerance;
        self
    }

    /// Normalised weight for one objective. Zero when unmentioned.
    pub fn weight(&self, objective: Objective) -> f64 {
        let total: f64 = self
            .preferences
            .iter()
            .map(|p| p.weight.max(0.0))
            .sum::<f64>();
        if total <= 0.0 {
            return 0.0;
        }
        self.preferences
            .iter()
            .filter(|p| p.objective == objective)
            .map(|p| p.weight.max(0.0) / total)
            .sum()
    }

    /// Objectives this intent actually cares about, either as a constraint or
    /// a preference.
    pub fn objectives(&self) -> Vec<Objective> {
        let mut out: Vec<Objective> = self
            .constraints
            .iter()
            .map(|c| c.objective)
            .chain(self.preferences.iter().map(|p| p.objective))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The well-known profiles, so a caller does not have to build one to get
    /// started. Each is a starting point, not a recommendation.
    pub fn preset(name: &str) -> Option<Intent> {
        match name {
            "latency-critical" => Some(
                Intent::new("latency-critical")
                    .with_constraint(Constraint::latency_p99_us(30.0))
                    .preferring(Objective::Jitter, 1.0)
                    .preferring(Objective::Latency, 0.8)
                    .preferring(Objective::Throughput, 0.2)
                    .preferring(Objective::Energy, 0.05)
                    .migration(MigrationTolerance::Low),
            ),
            "throughput" => Some(
                Intent::new("throughput")
                    .preferring(Objective::Throughput, 1.0)
                    .preferring(Objective::Energy, 0.2)
                    .migration(MigrationTolerance::High),
            ),
            "efficient" => Some(
                Intent::new("efficient")
                    .preferring(Objective::Energy, 1.0)
                    .preferring(Objective::Throughput, 0.3)
                    .migration(MigrationTolerance::High),
            ),
            "balanced" => Some(
                Intent::new("balanced")
                    .preferring(Objective::Latency, 0.5)
                    .preferring(Objective::Throughput, 0.5)
                    .preferring(Objective::Jitter, 0.3)
                    .preferring(Objective::Energy, 0.2)
                    .migration(MigrationTolerance::Medium),
            ),
            _ => None,
        }
    }

    /// Names of the available presets.
    pub fn preset_names() -> &'static [&'static str] {
        &["latency-critical", "throughput", "efficient", "balanced"]
    }
}

/// What was actually achieved, on the axes the intent cares about.
///
/// The other half of the contract. An intent that cannot be scored is a wish,
/// and a policy optimising an unscored objective is unfalsifiable.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Achievement {
    /// Measured value per objective, in the objective's natural unit.
    pub measured: Vec<(Objective, f64)>,
}

impl Achievement {
    pub fn new() -> Achievement {
        Achievement::default()
    }

    pub fn with(mut self, objective: Objective, value: f64) -> Achievement {
        self.measured.retain(|(o, _)| *o != objective);
        self.measured.push((objective, value));
        self
    }

    pub fn get(&self, objective: Objective) -> Option<f64> {
        self.measured
            .iter()
            .find(|(o, _)| *o == objective)
            .map(|(_, v)| *v)
    }

    /// Constraints this achievement violates.
    pub fn violations(&self, intent: &Intent) -> Vec<Constraint> {
        intent
            .constraints
            .iter()
            .filter(|constraint| match self.get(constraint.objective) {
                Some(value) => !constraint.satisfied_by(value),
                // An unmeasured objective cannot be shown to satisfy its
                // constraint, and is reported as a violation rather than
                // assumed away.
                None => true,
            })
            .copied()
            .collect()
    }

    pub fn satisfies(&self, intent: &Intent) -> bool {
        self.violations(intent).is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_constraint_is_not_traded_off() {
        let constraint = Constraint::latency_p99_us(30.0);
        assert!(constraint.satisfied_by(25.0));
        assert!(!constraint.satisfied_by(31.0));
        assert!(constraint.violation(33.0) > 0.0);
        assert!(constraint.violation(15.0) < 0.0);
    }

    #[test]
    fn a_throughput_constraint_reads_the_other_way() {
        let constraint = Constraint::throughput_at_least(1000.0);
        assert!(constraint.satisfied_by(1200.0));
        assert!(!constraint.satisfied_by(800.0));
    }

    #[test]
    fn an_unmeasurable_value_never_satisfies_a_hard_limit() {
        // Assuming an unknown is fine is how a constraint stops meaning
        // anything on a machine where the sensor is missing.
        let constraint = Constraint::latency_p99_us(30.0);
        assert!(!constraint.satisfied_by(f64::NAN));
    }

    #[test]
    fn preferences_normalise_to_ratios() {
        let intent = Intent::new("test")
            .preferring(Objective::Latency, 3.0)
            .preferring(Objective::Throughput, 1.0);
        assert!((intent.weight(Objective::Latency) - 0.75).abs() < 1e-9);
        assert!((intent.weight(Objective::Throughput) - 0.25).abs() < 1e-9);
        assert_eq!(intent.weight(Objective::Energy), 0.0);
    }

    #[test]
    fn an_intent_with_no_preferences_weights_nothing() {
        let intent = Intent::new("bare");
        assert_eq!(intent.weight(Objective::Latency), 0.0);
        assert!(intent.objectives().is_empty());
    }

    #[test]
    fn migration_tolerance_sets_the_bar_for_moving() {
        assert!(MigrationTolerance::None
            .improvement_threshold()
            .is_infinite());
        assert!(
            MigrationTolerance::Low.improvement_threshold()
                > MigrationTolerance::High.improvement_threshold()
        );
    }

    #[test]
    fn an_unmeasured_objective_counts_as_a_violation() {
        let intent = Intent::new("test").with_constraint(Constraint::latency_p99_us(30.0));
        let achievement = Achievement::new().with(Objective::Throughput, 500.0);
        assert!(!achievement.satisfies(&intent));
        assert_eq!(achievement.violations(&intent).len(), 1);
    }

    #[test]
    fn a_satisfied_intent_reports_no_violations() {
        let intent = Intent::new("test")
            .with_constraint(Constraint::latency_p99_us(30.0))
            .with_constraint(Constraint::throughput_at_least(100.0));
        let achievement = Achievement::new()
            .with(Objective::Latency, 22.0)
            .with(Objective::Throughput, 150.0);
        assert!(achievement.satisfies(&intent));
    }

    #[test]
    fn every_preset_parses_and_states_something() {
        for name in Intent::preset_names() {
            let intent = Intent::preset(name).unwrap_or_else(|| panic!("preset {name}"));
            assert_eq!(&intent.name, name);
            assert!(
                !intent.objectives().is_empty(),
                "preset {name} expresses nothing"
            );
        }
        assert!(Intent::preset("nonsense").is_none());
    }

    #[test]
    fn the_latency_preset_prioritises_jitter_over_throughput() {
        // The judgement the preset encodes, asserted so a later edit has to be
        // deliberate.
        let intent = Intent::preset("latency-critical").unwrap();
        assert!(intent.weight(Objective::Jitter) > intent.weight(Objective::Throughput));
        assert!(!intent.constraints.is_empty());
    }

    #[test]
    fn an_intent_round_trips_through_json() {
        // Applications will state intent from config files.
        let intent = Intent::preset("latency-critical").unwrap();
        let json = serde_json::to_string(&intent).unwrap();
        let back: Intent = serde_json::from_str(&json).unwrap();
        assert_eq!(intent, back);
    }
}
