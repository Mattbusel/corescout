//! Turning a series of reflections into changes.
//!
//! # This is where counters become rates
//!
//! The mirror publishes `cpu.time.idle` as a running total, because that is
//! what the machine actually holds and it is exact. It never publishes an idle
//! *percentage*, because that requires two observations and an interval, which
//! is memory.
//!
//! This module is that interval. A [`Transition`] is a pair of reflections plus
//! everything derivable from having both: deltas, rates, and which cells
//! changed at all.
//!
//! # Wrapping is handled, not ignored
//!
//! RAPL energy counters wrap every minute or so under load. A naive difference
//! then produces a large negative number, which as a "rate" is nonsense and, fed
//! to a learner, is worse than nonsense: it is a rare, extreme, systematically
//! timed outlier. Where a counter goes backwards, the transition marks the cell
//! as wrapped rather than reporting the difference.
//!
//! The mirror publishes `power.energy_wrap_at` precisely so a consumer *can*
//! correct for it, and that correction belongs here, not in the reflection.

use corescout_mirror::state::Semantics;
use corescout_mirror::{ChannelSpec, MirrorSnapshot};
use serde::{Deserialize, Serialize};

/// What one cell did between two reflections.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CellChange {
    /// Observed in both, and this is the difference.
    Delta(f64),
    /// A cumulative counter that went backwards: it wrapped, or was reset.
    Wrapped,
    /// Not observed in one or both reflections.
    Unobservable,
}

impl CellChange {
    pub fn delta(self) -> Option<f64> {
        match self {
            CellChange::Delta(value) => Some(value),
            _ => None,
        }
    }
}

/// The change between two consecutive reflections.
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub from_sequence: u64,
    pub to_sequence: u64,
    /// Nanoseconds between them. The denominator of every rate.
    pub interval_ns: u64,
    rows: usize,
    cols: usize,
    changes: Vec<CellChange>,
}

impl Transition {
    /// Compute the change between two reflections.
    ///
    /// Returns `None` when they are not comparable: different epochs mean the
    /// rows describe different hardware, and differencing them would silently
    /// compare one core against another.
    pub fn between(
        before: &MirrorSnapshot,
        after: &MirrorSnapshot,
        channels: &[ChannelSpec],
    ) -> Option<Transition> {
        if before.epoch != after.epoch {
            return None;
        }
        let rows = after.state.rows();
        let cols = after.state.cols();
        if before.state.rows() != rows || before.state.cols() != cols {
            return None;
        }

        let mut changes = Vec::with_capacity(rows * cols);
        for row in 0..rows {
            for col in 0..cols {
                let a = before.state.get(row, col);
                let b = after.state.get(row, col);
                changes.push(if !a.is_finite() || !b.is_finite() {
                    CellChange::Unobservable
                } else {
                    let cumulative = channels
                        .get(col)
                        .map(|c| c.semantics == Semantics::Cumulative)
                        .unwrap_or(false);
                    if cumulative && b < a {
                        CellChange::Wrapped
                    } else {
                        CellChange::Delta(b - a)
                    }
                });
            }
        }

        Some(Transition {
            from_sequence: before.sequence,
            to_sequence: after.sequence,
            interval_ns: after.monotonic_ns.saturating_sub(before.monotonic_ns),
            rows,
            cols,
            changes,
        })
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn change(&self, row: usize, col: usize) -> CellChange {
        if row >= self.rows || col >= self.cols {
            return CellChange::Unobservable;
        }
        self.changes[row * self.cols + col]
    }

    /// The change per second, for a cell that changed.
    ///
    /// `None` for an unobservable or wrapped cell, and for a zero interval:
    /// dividing by it would produce an infinity that poisons everything
    /// downstream.
    pub fn rate_per_second(&self, row: usize, col: usize) -> Option<f64> {
        if self.interval_ns == 0 {
            return None;
        }
        let delta = self.change(row, col).delta()?;
        Some(delta * 1e9 / self.interval_ns as f64)
    }

    /// Cells that moved at all.
    pub fn changed_cells(&self) -> usize {
        self.changes
            .iter()
            .filter(|c| matches!(c, CellChange::Delta(d) if *d != 0.0))
            .count()
    }

    pub fn wrapped_cells(&self) -> usize {
        self.changes
            .iter()
            .filter(|c| matches!(c, CellChange::Wrapped))
            .count()
    }

    /// Flat deltas, `NaN` where not derivable. The form a learner wants.
    pub fn as_delta_vector(&self) -> Vec<f64> {
        self.changes
            .iter()
            .map(|c| c.delta().unwrap_or(f64::NAN))
            .collect()
    }
}

/// Transitions over a whole series.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TransitionSet {
    transitions: Vec<Transition>,
    /// Pairs skipped because the epoch changed between them.
    discontinuities: usize,
}

impl TransitionSet {
    /// Compute every consecutive transition in a series.
    pub fn from_series(snapshots: &[MirrorSnapshot], channels: &[ChannelSpec]) -> TransitionSet {
        let mut transitions = Vec::new();
        let mut discontinuities = 0;
        for pair in snapshots.windows(2) {
            match Transition::between(&pair[0], &pair[1], channels) {
                Some(transition) => transitions.push(transition),
                None => discontinuities += 1,
            }
        }
        TransitionSet {
            transitions,
            discontinuities,
        }
    }

    pub fn len(&self) -> usize {
        self.transitions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.transitions.is_empty()
    }

    /// Pairs that could not be differenced because the machine changed shape.
    pub fn discontinuities(&self) -> usize {
        self.discontinuities
    }

    pub fn iter(&self) -> impl Iterator<Item = &Transition> {
        self.transitions.iter()
    }

    /// The rate series of one cell across every transition.
    pub fn rate_series(&self, row: usize, col: usize) -> Vec<f64> {
        self.transitions
            .iter()
            .map(|t| t.rate_per_second(row, col).unwrap_or(f64::NAN))
            .collect()
    }

    /// Total wrapped cells across the series, which is worth watching: a large
    /// number means the sampling interval is too long for those counters.
    pub fn wrapped_total(&self) -> usize {
        self.transitions.iter().map(|t| t.wrapped_cells()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::test_support::fixture;

    fn pair(before_value: f64, after_value: f64, col: usize) -> (MirrorSnapshot, MirrorSnapshot) {
        let mut before = fixture();
        before.sequence = 1;
        before.monotonic_ns = 1_000_000_000;
        before.state.set(2, col, before_value);

        let mut after = fixture();
        after.sequence = 2;
        after.monotonic_ns = 2_000_000_000;
        after.state.set(2, col, after_value);
        (before, after)
    }

    #[test]
    fn a_counter_becomes_a_rate() {
        // Column 1 of the fixture is cumulative.
        let (before, after) = pair(1_000.0, 3_000.0, 1);
        let channels = fixture().channels;
        let transition = Transition::between(&before, &after, &channels).unwrap();
        assert_eq!(transition.interval_ns, 1_000_000_000);
        assert_eq!(transition.change(2, 1), CellChange::Delta(2_000.0));
        assert_eq!(transition.rate_per_second(2, 1), Some(2_000.0));
    }

    #[test]
    fn a_wrapped_counter_is_marked_rather_than_reported_as_a_huge_negative_rate() {
        // What RAPL does every minute or so under load.
        let (before, after) = pair(4_000_000.0, 12.0, 1);
        let channels = fixture().channels;
        let transition = Transition::between(&before, &after, &channels).unwrap();
        assert_eq!(transition.change(2, 1), CellChange::Wrapped);
        assert_eq!(transition.rate_per_second(2, 1), None);
        assert_eq!(transition.wrapped_cells(), 1);
    }

    #[test]
    fn an_instantaneous_reading_may_fall_without_being_called_wrapped() {
        // Column 0 is Instant: a frequency dropping is normal.
        let (before, after) = pair(4_000_000.0, 800_000.0, 0);
        let channels = fixture().channels;
        let transition = Transition::between(&before, &after, &channels).unwrap();
        assert_eq!(transition.change(2, 0), CellChange::Delta(-3_200_000.0));
        assert_eq!(transition.wrapped_cells(), 0);
    }

    #[test]
    fn an_unobserved_cell_yields_no_change_rather_than_a_zero() {
        let channels = fixture().channels;
        let before = fixture();
        let after = fixture();
        let transition = Transition::between(&before, &after, &channels).unwrap();
        // Row 0 of the fixture observes nothing.
        assert_eq!(transition.change(0, 0), CellChange::Unobservable);
        assert_eq!(transition.rate_per_second(0, 0), None);
    }

    #[test]
    fn reflections_from_different_epochs_are_not_differenced() {
        let channels = fixture().channels;
        let before = fixture();
        let mut after = fixture();
        after.epoch += 1;
        assert!(Transition::between(&before, &after, &channels).is_none());
    }

    #[test]
    fn a_zero_interval_produces_no_rate_rather_than_an_infinity() {
        let channels = fixture().channels;
        let mut before = fixture();
        let mut after = fixture();
        before.monotonic_ns = 5_000;
        after.monotonic_ns = 5_000;
        before.state.set(2, 1, 10.0);
        after.state.set(2, 1, 20.0);
        let transition = Transition::between(&before, &after, &channels).unwrap();
        assert_eq!(transition.change(2, 1), CellChange::Delta(10.0));
        assert_eq!(transition.rate_per_second(2, 1), None);
    }

    #[test]
    fn a_series_counts_its_discontinuities() {
        let channels = fixture().channels;
        let mut series = Vec::new();
        for i in 0..4u64 {
            let mut snapshot = fixture();
            snapshot.sequence = i;
            snapshot.monotonic_ns = i * 100_000_000;
            if i >= 2 {
                snapshot.epoch = 99;
            }
            series.push(snapshot);
        }
        let set = TransitionSet::from_series(&series, &channels);
        assert_eq!(set.len(), 2, "two pairs are within an epoch");
        assert_eq!(set.discontinuities(), 1, "one pair straddles the change");
    }

    #[test]
    fn rate_series_lines_up_with_the_transitions() {
        let channels = fixture().channels;
        let series: Vec<MirrorSnapshot> = (0..4u64)
            .map(|i| {
                let mut snapshot = fixture();
                snapshot.sequence = i;
                snapshot.monotonic_ns = i * 1_000_000_000;
                snapshot.state.set(2, 1, (i as f64) * 500.0);
                snapshot
            })
            .collect();
        let set = TransitionSet::from_series(&series, &channels);
        assert_eq!(set.rate_series(2, 1), vec![500.0, 500.0, 500.0]);
    }
}
