//! State: channels and the dense state matrix.
//!
//! # The representation
//!
//! `X(t)` is a dense `entities x channels` matrix of `f64`. Row `i` is the state
//! vector of entity `i`; column `j` is the value of channel `j` across the whole
//! machine. A cell that was not observed is `NaN`.
//!
//! Uniformity is the point. A consumer that has never heard of a "cache" or a
//! "NUMA node" can still load the whole machine as a matrix, correlate columns,
//! cluster rows, and discover structure that nobody encoded. Had the mirror used
//! a struct per entity class, every consumer would have to know the classes
//! before it could read anything, and the categories would stop being
//! annotations and start being the ontology.
//!
//! # Why `f64` and not `f32`
//!
//! Cumulative counters. Energy in microjoules and retired-instruction counts
//! pass `2^24` within seconds, and an `f32` starts skipping integers there, so a
//! consumer differencing two samples would see quantised garbage. `f64` holds
//! every integer up to `2^53`, which is centuries of microjoules. The matrix for
//! a 128-CPU machine with 40 channels is 84 KB; the memory saved by `f32` is not
//! worth an entire class of silent numerical error.
//!
//! # Why `NaN` for absent
//!
//! Absence has to be distinguishable from zero: a core reporting 0 C and a core
//! with no thermal sensor are different facts, and a consumer that confuses them
//! will learn something false. `NaN` is self-propagating, so arithmetic that
//! ignores the distinction produces `NaN` rather than a plausible wrong number.
//!
//! # Instantaneous values and cumulative counters
//!
//! Channels declare which they are. The mirror publishes **both** kinds and
//! derives neither: a counter is a present fact about the current value of a
//! register or kernel variable, readable in a single observation. A *rate* is a
//! statement about an interval, needs two observations, and therefore belongs to
//! `corescout_memory`. Consumers difference counters themselves.

use serde::{Deserialize, Serialize};

use crate::schema::SensorId;

/// Index of a channel, which is also its column in the state matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChannelId(pub u16);

impl ChannelId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Physical unit of a channel's values.
///
/// Units are declared rather than encoded in names so a consumer can reason
/// about dimensional compatibility without parsing English.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u16)]
pub enum Unit {
    /// Pure number, no dimension.
    Dimensionless = 0,
    Kilohertz = 1,
    /// Degrees Celsius.
    Celsius = 2,
    Microjoule = 3,
    Microwatt = 4,
    Nanosecond = 5,
    Microsecond = 6,
    Byte = 7,
    /// A monotonic event tally.
    Count = 8,
    /// A value in `0.0 ..= 1.0`.
    Ratio = 9,
    /// A boolean encoded as 0.0 or 1.0.
    Boolean = 10,
    /// An integer code whose meaning is defined by the channel.
    Ordinal = 11,
}

impl Unit {
    pub fn as_u16(self) -> u16 {
        self as u16
    }

    pub fn from_u16(value: u16) -> Unit {
        match value {
            1 => Unit::Kilohertz,
            2 => Unit::Celsius,
            3 => Unit::Microjoule,
            4 => Unit::Microwatt,
            5 => Unit::Nanosecond,
            6 => Unit::Microsecond,
            7 => Unit::Byte,
            8 => Unit::Count,
            9 => Unit::Ratio,
            10 => Unit::Boolean,
            11 => Unit::Ordinal,
            _ => Unit::Dimensionless,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Unit::Dimensionless => "",
            Unit::Kilohertz => "kHz",
            Unit::Celsius => "C",
            Unit::Microjoule => "uJ",
            Unit::Microwatt => "uW",
            Unit::Nanosecond => "ns",
            Unit::Microsecond => "us",
            Unit::Byte => "B",
            Unit::Count => "",
            Unit::Ratio => "",
            Unit::Boolean => "",
            Unit::Ordinal => "",
        }
    }
}

/// How a channel's values relate to time.
///
/// This is the distinction that keeps the mirror free of history. A consumer
/// must know whether a number is a reading or a running total before it can do
/// anything correct with two of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u16)]
pub enum Semantics {
    /// A reading of the present: the current temperature, the current frequency.
    /// Meaningful on its own.
    Instant = 0,
    /// A monotonically non-decreasing total since some unspecified origin
    /// (usually boot). Meaningless on its own; meaningful when differenced
    /// against an earlier observation, which is the memory layer's job.
    Cumulative = 1,
    /// A configuration value that changes only when something reconfigures the
    /// machine: a frequency limit, a cache size.
    Configured = 2,
    /// A state code, such as a C-state index.
    Ordinal = 3,
}

impl Semantics {
    pub fn as_u16(self) -> u16 {
        self as u16
    }

    pub fn from_u16(value: u16) -> Semantics {
        match value {
            1 => Semantics::Cumulative,
            2 => Semantics::Configured,
            3 => Semantics::Ordinal,
            _ => Semantics::Instant,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Semantics::Instant => "instant",
            Semantics::Cumulative => "cumulative",
            Semantics::Configured => "configured",
            Semantics::Ordinal => "ordinal",
        }
    }
}

/// Description of one column of the state matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelSpec {
    pub id: ChannelId,
    /// Dotted key, e.g. `cpu.frequency.current`. An annotation for humans and
    /// for consumers that want to look up a known column by name; a consumer
    /// working from the matrix alone never needs it.
    pub key: String,
    pub unit: Unit,
    pub semantics: Semantics,
    /// Which sensor fills this column, and therefore what observing it costs.
    pub sensor: SensorId,
}

/// The dense `entities x channels` state matrix.
///
/// Row-major, so one entity's full state vector is contiguous. That is the
/// access pattern for "what is this thing doing"; column access, "what is this
/// quantity doing across the machine", strides and is the rarer of the two.
/// `PartialEq` is hand written for the same reason as [`crate::Relation`]:
/// `NaN` means unobserved, and two snapshots with the same holes in the same
/// places are equal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateMatrix {
    rows: usize,
    cols: usize,
    /// `NaN` means unobserved. JSON has no `NaN`, so the debug projection maps
    /// it to `null` and back; see [`crate::nanjson`].
    #[serde(with = "crate::nanjson::vec")]
    values: Vec<f64>,
}

impl PartialEq for StateMatrix {
    fn eq(&self, other: &StateMatrix) -> bool {
        self.rows == other.rows
            && self.cols == other.cols
            && self
                .values
                .iter()
                .zip(&other.values)
                .all(|(a, b)| crate::relation::same_value(*a, *b))
    }
}

impl StateMatrix {
    /// Allocate a matrix with every cell unobserved.
    pub fn new(rows: usize, cols: usize) -> StateMatrix {
        StateMatrix {
            rows,
            cols,
            values: vec![f64::NAN; rows * cols],
        }
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Reset every cell to unobserved.
    ///
    /// Called at the start of each observation pass so a sensor that fails this
    /// tick leaves a hole rather than a stale value silently presented as
    /// current. A mirror that shows last minute's temperature is worse than one
    /// that shows nothing, because nothing is honest.
    pub fn clear(&mut self) {
        self.values.fill(f64::NAN);
    }

    #[inline]
    pub fn get(&self, row: usize, col: usize) -> f64 {
        self.values[row * self.cols + col]
    }

    #[inline]
    pub fn set(&mut self, row: usize, col: usize, value: f64) {
        self.values[row * self.cols + col] = value;
    }

    /// One entity's complete state vector.
    pub fn row(&self, row: usize) -> &[f64] {
        let start = row * self.cols;
        &self.values[start..start + self.cols]
    }

    /// The whole matrix as a flat row-major slice, which is the form the plane
    /// stores and the form a numeric consumer wants.
    pub fn as_slice(&self) -> &[f64] {
        &self.values
    }

    /// Overwrite from a flat row-major slice.
    pub fn copy_from_slice(&mut self, values: &[f64]) {
        assert_eq!(
            values.len(),
            self.values.len(),
            "state matrix shape mismatch"
        );
        self.values.copy_from_slice(values);
    }

    /// Count of cells that were actually observed this tick.
    ///
    /// Published as part of the mirror's self-description: a snapshot that
    /// filled 40% of its cells is a different kind of evidence from one that
    /// filled 95%, and a consumer should be able to tell without inspecting
    /// every cell.
    pub fn observed_cells(&self) -> usize {
        self.values.iter().filter(|v| !v.is_nan()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrices_with_the_same_holes_are_equal() {
        let a = StateMatrix::new(2, 2);
        let b = StateMatrix::new(2, 2);
        assert_eq!(a, b, "unobserved must compare equal to unobserved");
        let mut c = StateMatrix::new(2, 2);
        c.set(0, 0, 1.0);
        assert_ne!(a, c);
    }

    #[test]
    fn a_new_matrix_is_entirely_unobserved() {
        let m = StateMatrix::new(3, 4);
        assert_eq!(m.rows(), 3);
        assert_eq!(m.cols(), 4);
        assert_eq!(m.observed_cells(), 0);
        assert!(m.get(2, 3).is_nan());
    }

    #[test]
    fn absent_is_distinguishable_from_zero() {
        // The distinction the whole NaN convention exists to preserve.
        let mut m = StateMatrix::new(2, 2);
        m.set(0, 0, 0.0);
        assert_eq!(m.get(0, 0), 0.0);
        assert!(!m.get(0, 0).is_nan(), "an observed zero is an observation");
        assert!(m.get(1, 1).is_nan(), "an unobserved cell is not a zero");
        assert_eq!(m.observed_cells(), 1);
    }

    #[test]
    fn clearing_removes_stale_values() {
        let mut m = StateMatrix::new(2, 2);
        m.set(0, 0, 42.0);
        m.clear();
        assert_eq!(m.observed_cells(), 0, "a failed sensor must leave a hole");
    }

    #[test]
    fn rows_are_contiguous_state_vectors() {
        let mut m = StateMatrix::new(2, 3);
        for col in 0..3 {
            m.set(1, col, col as f64);
        }
        assert_eq!(m.row(1), &[0.0, 1.0, 2.0]);
    }

    #[test]
    fn flat_layout_is_row_major() {
        let mut m = StateMatrix::new(2, 3);
        m.set(0, 0, 1.0);
        m.set(1, 0, 2.0);
        assert_eq!(m.as_slice()[0], 1.0);
        assert_eq!(m.as_slice()[3], 2.0, "row 1 starts at index cols");
    }

    #[test]
    fn round_trips_through_a_flat_slice() {
        let mut a = StateMatrix::new(2, 2);
        a.set(0, 1, 7.5);
        let mut b = StateMatrix::new(2, 2);
        b.copy_from_slice(a.as_slice());
        assert_eq!(a.get(0, 1), b.get(0, 1));
        assert!(b.get(1, 0).is_nan(), "NaN must survive the copy");
    }

    #[test]
    fn wire_codes_round_trip_and_degrade() {
        for unit in [
            Unit::Kilohertz,
            Unit::Celsius,
            Unit::Microjoule,
            Unit::Count,
        ] {
            assert_eq!(Unit::from_u16(unit.as_u16()), unit);
        }
        assert_eq!(Unit::from_u16(9999), Unit::Dimensionless);
        for s in [
            Semantics::Instant,
            Semantics::Cumulative,
            Semantics::Configured,
        ] {
            assert_eq!(Semantics::from_u16(s.as_u16()), s);
        }
        assert_eq!(Semantics::from_u16(9999), Semantics::Instant);
    }
}
