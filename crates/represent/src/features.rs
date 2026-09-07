//! Turning reflections into something a learner can cluster.
//!
//! # The bug this module exists to fix
//!
//! The mirror publishes cumulative counters as cumulative counters. That is
//! correct: a counter's current value is a fact about the present, and
//! differencing is memory's job rather than the sensor's.
//!
//! A consumer that then clusters on those raw levels learns nothing. On a real
//! machine, `cpu.time.idle` sits around 9x10^14 nanoseconds and moves by about
//! 4x10^7 per frame, a relative change of one part in twenty million. Normalise
//! a window of those and every frame is identical, so the clustering finds
//! exactly one state and the self-model has nothing to predict.
//!
//! That is what happened the first time this pipeline was pointed at real
//! hardware: 900 reflections, one latent state, zero predictions scored. The
//! synthetic substrates it had been developed against all published
//! instantaneous values, so the defect could not appear.
//!
//! # What this does
//!
//! Reads the `Semantics` the mirror already publishes and treats each channel
//! accordingly:
//!
//! | semantics | feature |
//! |---|---|
//! | `Instant` | the value |
//! | `Cumulative` | the per-second rate since the previous reflection |
//! | `Delta` | the value, already a difference |
//!
//! The mirror labels its channels for exactly this reason. Not using the label
//! and then clustering on levels is the consumer's mistake, not the mirror's.
//!
//! # Why rates and not raw differences
//!
//! Reflections do not arrive on an even cadence. A difference between two
//! frames 40 ms apart and one between frames 400 ms apart are not comparable,
//! and a learner fed both is learning about the sampler's jitter. Dividing by
//! the elapsed time makes the feature a property of the machine instead.

use corescout_mirror::state::Semantics;
use corescout_mirror::MirrorSnapshot;

/// Feature vectors, and what was done to produce them.
#[derive(Debug, Clone, PartialEq)]
pub struct Features {
    /// One row per usable reflection, each `entities * channels` wide.
    pub rows: Vec<Vec<f64>>,
    /// The monotonic timestamp of each row.
    pub times_ns: Vec<u64>,
    /// Columns turned into rates because they were cumulative.
    pub differenced: usize,
    /// Columns passed through because they were already instantaneous.
    pub passed_through: usize,
    /// Reflections dropped because they had no usable predecessor.
    pub dropped: usize,
}

impl Features {
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn width(&self) -> usize {
        self.rows.first().map(|row| row.len()).unwrap_or(0)
    }

    pub fn describe(&self) -> String {
        format!(
            "{} feature rows of width {}; {} channels differenced to rates, {} passed \
             through, {} reflections dropped",
            self.rows.len(),
            self.width(),
            self.differenced,
            self.passed_through,
            self.dropped
        )
    }
}

/// Build feature vectors from a series of reflections.
///
/// The first reflection produces no row: a rate needs two observations. An
/// epoch change also breaks the series, because rows mean different things
/// either side of one and differencing across it would produce a number about
/// two different machines.
pub fn extract(frames: &[MirrorSnapshot]) -> Features {
    let mut features = Features {
        rows: Vec::new(),
        times_ns: Vec::new(),
        differenced: 0,
        passed_through: 0,
        dropped: 0,
    };
    if frames.len() < 2 {
        features.dropped = frames.len();
        return features;
    }

    let channels = &frames[0].channels;
    for channel in channels {
        match channel.semantics {
            Semantics::Cumulative => features.differenced += 1,
            _ => features.passed_through += 1,
        }
    }

    for index in 1..frames.len() {
        let current = &frames[index];
        match row_from(&frames[index - 1], current) {
            Some(row) => {
                features.rows.push(row);
                features.times_ns.push(current.monotonic_ns);
            }
            None => features.dropped += 1,
        }
    }
    // The first reflection never becomes a row.
    features.dropped += 1;
    features
}

/// Build one feature row from a consecutive pair of reflections.
///
/// The incremental form of [`extract`], for a consumer that sees reflections
/// as they arrive rather than as a batch. Both go through this function, so a
/// service watching a live plane and a tool replaying a recording produce
/// identical rows from identical input, which is the property that makes a
/// recorded trace a usable stand-in for a live machine.
///
/// `None` when the pair cannot produce a rate: a different epoch, a changed
/// shape, or no time between them.
pub fn row_from(previous: &MirrorSnapshot, current: &MirrorSnapshot) -> Option<Vec<f64>> {
    let cols = current.channels.len();
    let entities = current.entities.len();
    // Rows are only comparable within an epoch and within one shape.
    if current.epoch != previous.epoch
        || previous.channels.len() != cols
        || previous.entities.len() != entities
    {
        return None;
    }
    let elapsed_ns = current.monotonic_ns.saturating_sub(previous.monotonic_ns);
    if elapsed_ns == 0 {
        // Two reflections at the same instant: a rate would be a division by
        // zero, and the second carries no new information anyway.
        return None;
    }
    let seconds = elapsed_ns as f64 / 1e9;

    let mut row = Vec::with_capacity(entities * cols);
    for entity in 0..entities {
        for (col, channel) in current.channels.iter().enumerate() {
            let now = current.state.get(entity, col);
            let value = match channel.semantics {
                Semantics::Cumulative => {
                    let before = previous.state.get(entity, col);
                    if now.is_finite() && before.is_finite() {
                        let delta = now - before;
                        // A counter that went backwards wrapped, or the machine
                        // restarted. Either way the difference is not a rate,
                        // and inventing one would put a huge spike into the
                        // feature that a learner would then treat as a state.
                        if delta < 0.0 {
                            f64::NAN
                        } else {
                            delta / seconds
                        }
                    } else {
                        f64::NAN
                    }
                }
                _ => now,
            };
            row.push(value);
        }
    }
    Some(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::entity::{Entity, EntityClass};
    use corescout_mirror::schema::{Availability, AvailabilityMatrix, SensorId};
    use corescout_mirror::state::{ChannelId, ChannelSpec, StateMatrix, Unit};
    use corescout_mirror::FORMAT_VERSION;

    /// One entity, one instantaneous channel and one cumulative one.
    fn frame(sequence: u64, instant: f64, counter: f64) -> MirrorSnapshot {
        let mut state = StateMatrix::new(1, 2);
        state.set(0, 0, instant);
        state.set(0, 1, counter);
        let mut availability = AvailabilityMatrix::new(1, 2);
        availability.set(0, 0, Availability::Observed);
        availability.set(0, 1, Availability::Observed);
        MirrorSnapshot {
            format_version: FORMAT_VERSION,
            epoch: 1,
            sequence,
            // Exactly one second between frames, so a rate is the raw delta.
            monotonic_ns: sequence * 1_000_000_000,
            realtime_ns: 0,
            entities: vec![Entity::new("thing/0", EntityClass::LogicalCpu, Some(0))],
            channels: vec![
                ChannelSpec {
                    id: ChannelId(0),
                    key: "gauge".into(),
                    unit: Unit::Kilohertz,
                    semantics: Semantics::Instant,
                    sensor: SensorId(0),
                },
                ChannelSpec {
                    id: ChannelId(1),
                    key: "counter".into(),
                    unit: Unit::Nanosecond,
                    semantics: Semantics::Cumulative,
                    sensor: SensorId(0),
                },
            ],
            relations: Vec::new(),
            state,
            availability,
            sensors: Vec::new(),
        }
    }

    #[test]
    fn a_cumulative_channel_becomes_a_rate() {
        // The whole point. A counter climbing by 100 per second is a constant
        // rate of 100, not a ramp from 1000 to 1300.
        let frames: Vec<MirrorSnapshot> = (1..=4)
            .map(|i| frame(i, 5.0, 1000.0 + i as f64 * 100.0))
            .collect();
        let features = extract(&frames);
        assert_eq!(features.len(), 3);
        assert_eq!(features.differenced, 1);
        assert_eq!(features.passed_through, 1);
        for row in &features.rows {
            assert_eq!(row[0], 5.0, "an instant channel passes through");
            assert!((row[1] - 100.0).abs() < 1e-9, "got {}", row[1]);
        }
    }

    #[test]
    fn a_huge_counter_with_a_small_change_yields_a_visible_rate() {
        // The real-hardware case that motivated this module: a counter near
        // 9e14 moving by 4e7 a frame. On levels those frames are
        // indistinguishable; as rates they are not.
        let base = 892_466_000_000_000.0;
        let frames: Vec<MirrorSnapshot> = (1..=5)
            .map(|i| frame(i, 3.4e6, base + i as f64 * 40_000_000.0))
            .collect();
        let features = extract(&frames);
        let rates: Vec<f64> = features.rows.iter().map(|row| row[1]).collect();
        assert!(rates.iter().all(|r| (r - 40_000_000.0).abs() < 1.0));

        // And the spread of the raw levels is invisible next to their size,
        // which is exactly why clustering on them found one state.
        let levels: Vec<f64> = frames.iter().map(|f| f.state.get(0, 1)).collect();
        let span = levels.last().unwrap() - levels.first().unwrap();
        assert!(
            span / levels[0] < 1e-6,
            "the levels differ by one part in {:.0}",
            levels[0] / span
        );
    }

    #[test]
    fn a_counter_that_goes_backwards_produces_a_gap_not_a_spike() {
        // A wrap or a restart. Inventing a rate here would put an enormous
        // value into the feature that a learner would happily call a state.
        let frames = vec![frame(1, 1.0, 5000.0), frame(2, 1.0, 10.0)];
        let features = extract(&frames);
        assert_eq!(features.len(), 1);
        assert!(features.rows[0][1].is_nan());
    }

    #[test]
    fn differencing_never_crosses_an_epoch() {
        // Rows mean different things either side of an epoch change, so a
        // difference across one is a number about two different machines.
        let mut frames = vec![frame(1, 1.0, 100.0), frame(2, 1.0, 200.0)];
        frames[1].epoch = 2;
        let features = extract(&frames);
        assert!(features.is_empty());
        assert_eq!(features.dropped, 2);
    }

    #[test]
    fn two_reflections_at_the_same_instant_produce_no_rate() {
        let mut frames = vec![frame(1, 1.0, 100.0), frame(2, 1.0, 200.0)];
        frames[1].monotonic_ns = frames[0].monotonic_ns;
        let features = extract(&frames);
        assert!(features.is_empty());
    }

    #[test]
    fn rates_account_for_an_uneven_cadence() {
        // Otherwise the learner is being taught about the sampler's jitter.
        let mut frames = vec![frame(1, 1.0, 0.0), frame(2, 1.0, 100.0)];
        // Half a second, not a full one: the same delta is twice the rate.
        frames[1].monotonic_ns = frames[0].monotonic_ns + 500_000_000;
        let features = extract(&frames);
        assert!((features.rows[0][1] - 200.0).abs() < 1e-9);
    }

    #[test]
    fn one_reflection_yields_no_features() {
        let features = extract(&[frame(1, 1.0, 100.0)]);
        assert!(features.is_empty());
        assert_eq!(features.dropped, 1);
        assert!(extract(&[]).is_empty());
    }

    #[test]
    fn an_unobserved_cell_stays_unobserved() {
        let mut frames = vec![frame(1, 1.0, 100.0), frame(2, 1.0, 200.0)];
        frames[1].state.set(0, 1, f64::NAN);
        let features = extract(&frames);
        assert!(features.rows[0][1].is_nan());
    }
}
