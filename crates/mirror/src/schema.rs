//! The schema of a reflection: what was observed, how, and how well.
//!
//! # Why observation metadata is part of the representation
//!
//! A number in the mirror is not self-describing. `47.0` in a temperature cell
//! could be a fresh reading, a value from 400 ms ago, or a cell that has never
//! been filled because this machine has no such sensor and never will. Those
//! are different facts and a consumer that cannot tell them apart will learn
//! something false.
//!
//! So the mirror carries, alongside the numbers:
//!
//! - **why** a cell is empty, if it is ([`Availability`]),
//! - **what it cost** to fill the ones that are ([`SensorReport`]),
//! - **how far** observing perturbed the thing observed ([`Perturbation`]),
//! - **how wrong** the value may be even when everything worked
//!   ([`Uncertainty`]).
//!
//! # Absence is a measurement
//!
//! The rule this module exists to enforce: **never fabricate a value**. A
//! machine with no RAPL support does not report zero watts, and a mirror
//! running unprivileged does not report zero energy. Both report
//! [`Availability::PermissionDenied`] or [`Availability::Unsupported`], and a
//! learner can then treat "I cannot see this" as the information it is, rather
//! than modelling a constant zero as a physical fact.

use serde::{Deserialize, Serialize};

/// Why a cell of the state matrix holds no value.
///
/// Stored in a parallel matrix to the numbers themselves, so every cell has
/// both a value and a reason. `Observed` cells carry a real number; every other
/// code means the value is `NaN` and says why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Availability {
    /// A real observation from this pass.
    Observed = 0,
    /// No sensor claims this cell. The usual case for the large empty regions
    /// of a sparse matrix: a cache has no temperature, a thermal zone has no
    /// instruction count.
    NotApplicable = 1,
    /// A sensor claims it and did not produce a value this pass, for a reason
    /// it could not determine.
    Unknown = 2,
    /// This machine cannot produce the value at all: the hardware lacks the
    /// counter, the kernel lacks the driver, the interface does not exist.
    Unsupported = 3,
    /// The interface exists and did not answer: a file vanished under a
    /// hotplug, a device is asleep, a read failed transiently.
    Unavailable = 4,
    /// The interface exists and refused. Distinguished from `Unsupported`
    /// because it is *actionable*: the same mirror run with more privilege
    /// would fill this cell.
    PermissionDenied = 5,
    /// A value exists but is older than the sensor's declared useful lifetime.
    Stale = 6,
}

impl Availability {
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(value: u8) -> Availability {
        match value {
            0 => Availability::Observed,
            1 => Availability::NotApplicable,
            2 => Availability::Unknown,
            3 => Availability::Unsupported,
            4 => Availability::Unavailable,
            5 => Availability::PermissionDenied,
            6 => Availability::Stale,
            // An unknown code from a newer writer is itself unknown, which is
            // the honest degradation.
            _ => Availability::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Availability::Observed => "observed",
            Availability::NotApplicable => "not_applicable",
            Availability::Unknown => "unknown",
            Availability::Unsupported => "unsupported",
            Availability::Unavailable => "unavailable",
            Availability::PermissionDenied => "permission_denied",
            Availability::Stale => "stale",
        }
    }

    /// Why the cell is empty, in a sentence an operator can act on.
    ///
    /// The distinctions matter: "unsupported" means buy different hardware,
    /// "permission denied" means run it differently, and "not applicable" means
    /// the question was wrong. Collapsing them into "missing" throws away the
    /// only part of a gap that is useful.
    pub fn explain(self) -> &'static str {
        match self {
            Availability::Observed => "the value was read",
            Availability::NotApplicable => "this measurement does not apply to this entity",
            Availability::Unknown => "no reason was recorded",
            Availability::Unsupported => "this hardware or kernel does not expose it",
            Availability::Unavailable => "the source exists but returned nothing",
            Availability::PermissionDenied => "this process lacks the privilege to read it",
            Availability::Stale => "the last reading is too old to be trusted",
        }
    }

    /// Whether the corresponding cell holds a usable number.
    pub fn is_observed(self) -> bool {
        self == Availability::Observed
    }

    /// Whether more privilege would plausibly fix this.
    pub fn is_privilege_problem(self) -> bool {
        self == Availability::PermissionDenied
    }
}

/// Per-cell reasons, parallel to the state matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailabilityMatrix {
    rows: usize,
    cols: usize,
    codes: Vec<u8>,
}

impl AvailabilityMatrix {
    /// A matrix where nothing has been claimed by any sensor.
    pub fn new(rows: usize, cols: usize) -> AvailabilityMatrix {
        AvailabilityMatrix {
            rows,
            cols,
            codes: vec![Availability::NotApplicable.as_u8(); rows * cols],
        }
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Reset to "no sensor claims anything", which is what a new pass starts
    /// from before sensors declare what they attempted.
    pub fn clear(&mut self) {
        self.codes.fill(Availability::NotApplicable.as_u8());
    }

    #[inline]
    pub fn get(&self, row: usize, col: usize) -> Availability {
        if row >= self.rows || col >= self.cols {
            return Availability::NotApplicable;
        }
        Availability::from_u8(self.codes[row * self.cols + col])
    }

    #[inline]
    pub fn set(&mut self, row: usize, col: usize, availability: Availability) {
        if row < self.rows && col < self.cols {
            self.codes[row * self.cols + col] = availability.as_u8();
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.codes
    }

    pub fn copy_from_slice(&mut self, codes: &[u8]) {
        assert_eq!(codes.len(), self.codes.len(), "availability shape mismatch");
        self.codes.copy_from_slice(codes);
    }

    /// Count of cells with each reason, for a summary.
    pub fn tally(&self) -> Vec<(Availability, usize)> {
        let mut counts = [0usize; 8];
        for code in &self.codes {
            counts[(*code as usize).min(7)] += 1;
        }
        (0..7u8)
            .map(|code| (Availability::from_u8(code), counts[code as usize]))
            .filter(|(_, count)| *count > 0)
            .collect()
    }
}

/// Identifies a sensor within one mirror.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SensorId(pub u16);

/// How much sampling a channel disturbs the thing it is sampling.
///
/// This is the axis that separates a mirror from a probe, so it is a required
/// declaration rather than documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Perturbation {
    /// Reading touches only memory the observing CPU already owns. The observed
    /// entity does not notice.
    None = 0,
    /// A kernel interface read on the observing CPU. Costs cycles and cache
    /// footprint here; does not reach across to the observed entity.
    Negligible = 1,
    /// Observation reaches the observed entity: an IPI, a cross-CPU MSR read, a
    /// bus transaction, or occupying a shared hardware resource. The observed
    /// core does measurably less work because it was observed.
    Low = 2,
    /// Observation changes hardware or kernel state in order to read it. **No
    /// sensor in the mirror may declare this.** It exists so the type can
    /// express the boundary it is enforcing: anything here is an experiment.
    Material = 3,
}

impl Perturbation {
    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(value: u8) -> Perturbation {
        match value {
            1 => Perturbation::Negligible,
            2 => Perturbation::Low,
            3 => Perturbation::Material,
            _ => Perturbation::None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Perturbation::None => "none",
            Perturbation::Negligible => "negligible",
            Perturbation::Low => "low",
            Perturbation::Material => "material",
        }
    }

    /// Whether a sensor with this class is admissible in the mirror.
    pub fn is_passive(self) -> bool {
        self != Perturbation::Material
    }
}

/// How wrong a reading may be when nothing has gone wrong.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Uncertainty {
    /// Absolute error in the channel's own unit, where it is known.
    pub absolute: Option<f64>,
    /// Fractional error, where that is the better description.
    pub relative: Option<f64>,
    /// Where the uncertainty comes from, in one line.
    pub basis: &'static str,
}

impl Uncertainty {
    pub const fn absolute(value: f64, basis: &'static str) -> Uncertainty {
        Uncertainty {
            absolute: Some(value),
            relative: None,
            basis,
        }
    }

    pub const fn relative(value: f64, basis: &'static str) -> Uncertainty {
        Uncertainty {
            absolute: None,
            relative: Some(value),
            basis,
        }
    }

    pub const fn unknown(basis: &'static str) -> Uncertainty {
        Uncertainty {
            absolute: None,
            relative: None,
            basis,
        }
    }
}

/// What one observation pass cost and produced, per sensor.
///
/// Published inside the snapshot: the mirror describing its own act of looking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SensorReport {
    pub id: SensorId,
    pub key: String,
    pub perturbation: Perturbation,
    /// Wall time this sensor took on the most recent pass. The observation cost.
    pub last_cost_ns: u64,
    /// How long between asking and the value being available, where that
    /// differs from the cost. Zero when they are the same.
    pub sampling_latency_ns: u64,
    /// Age of the underlying value at publication time. Non-zero for sensors
    /// whose source updates more slowly than the mirror ticks.
    pub sample_age_ns: u64,
    /// Cells it filled on the most recent pass.
    pub samples: u32,
    /// Reads that failed on the most recent pass.
    pub errors: u32,
    /// Confidence in this pass, `0.0 ..= 1.0`. Falls when reads fail.
    pub confidence: f64,
    /// Why this sensor's cells are empty, when they are.
    pub availability: Availability,
    /// True when the sensor could not bind at all, so its columns will be
    /// unobserved for this whole epoch.
    pub inactive: bool,
}

impl SensorReport {
    /// A report for a sensor that bound and has not yet run.
    pub fn pending(
        id: SensorId,
        key: impl Into<String>,
        perturbation: Perturbation,
    ) -> SensorReport {
        SensorReport {
            id,
            key: key.into(),
            perturbation,
            last_cost_ns: 0,
            sampling_latency_ns: 0,
            sample_age_ns: 0,
            samples: 0,
            errors: 0,
            confidence: 1.0,
            availability: Availability::Unknown,
            inactive: false,
        }
    }

    /// A report for a sensor that could not bind, and why.
    pub fn unavailable(
        id: SensorId,
        key: impl Into<String>,
        perturbation: Perturbation,
        availability: Availability,
    ) -> SensorReport {
        SensorReport {
            inactive: true,
            confidence: 0.0,
            availability,
            ..SensorReport::pending(id, key, perturbation)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_codes_round_trip_and_degrade() {
        for availability in [
            Availability::Observed,
            Availability::NotApplicable,
            Availability::Unknown,
            Availability::Unsupported,
            Availability::Unavailable,
            Availability::PermissionDenied,
            Availability::Stale,
        ] {
            assert_eq!(Availability::from_u8(availability.as_u8()), availability);
        }
        // A code from a newer writer is unknown, not silently "observed".
        assert_eq!(Availability::from_u8(200), Availability::Unknown);
    }

    #[test]
    fn permission_denied_is_distinguishable_from_unsupported() {
        // The distinction that makes the difference actionable: one of these
        // is fixed by running with more privilege, the other never is.
        assert!(Availability::PermissionDenied.is_privilege_problem());
        assert!(!Availability::Unsupported.is_privilege_problem());
        assert!(!Availability::Observed.is_privilege_problem());
    }

    #[test]
    fn a_new_availability_matrix_claims_nothing() {
        let matrix = AvailabilityMatrix::new(3, 4);
        assert_eq!(matrix.get(0, 0), Availability::NotApplicable);
        assert_eq!(matrix.tally(), vec![(Availability::NotApplicable, 12)]);
    }

    #[test]
    fn reasons_are_recorded_per_cell() {
        let mut matrix = AvailabilityMatrix::new(2, 2);
        matrix.set(0, 0, Availability::Observed);
        matrix.set(0, 1, Availability::PermissionDenied);
        matrix.set(1, 0, Availability::Unsupported);
        assert_eq!(matrix.get(0, 0), Availability::Observed);
        assert_eq!(matrix.get(0, 1), Availability::PermissionDenied);
        assert_eq!(matrix.get(1, 0), Availability::Unsupported);
        assert_eq!(matrix.get(1, 1), Availability::NotApplicable);

        let tally = matrix.tally();
        assert_eq!(tally.len(), 4, "four distinct reasons: {tally:?}");
    }

    #[test]
    fn out_of_range_access_does_not_panic() {
        let mut matrix = AvailabilityMatrix::new(2, 2);
        matrix.set(99, 99, Availability::Observed);
        assert_eq!(matrix.get(99, 99), Availability::NotApplicable);
    }

    #[test]
    fn a_failed_sensor_reports_why_rather_than_zero() {
        let report = SensorReport::unavailable(
            SensorId(4),
            "power",
            Perturbation::Negligible,
            Availability::PermissionDenied,
        );
        assert!(report.inactive);
        assert_eq!(report.confidence, 0.0);
        assert!(report.availability.is_privilege_problem());
        assert_eq!(report.samples, 0);
    }

    #[test]
    fn material_perturbation_is_not_passive() {
        assert!(!Perturbation::Material.is_passive());
        assert!(Perturbation::Low.is_passive());
        assert!(Perturbation::None < Perturbation::Negligible);
        assert!(Perturbation::Low < Perturbation::Material);
    }
}
