use crate::lens::Lens;
use crate::report::Findings;
use corescout_memory::Experience;
use corescout_represent::discover;
use corescout_selfmodel::predict;

use std::collections::BTreeMap;

use corescout_mirror::state::Semantics;
use corescout_mirror::MirrorSnapshot;

/// How much evidence an observer wants before it will claim anything.
#[derive(Debug, Clone, PartialEq)]
pub struct ObserverConfig {
    /// Reflections to remember.
    pub capacity: usize,
    /// Behavioural similarity above which two entities are called alike.
    pub entity_threshold: f64,
    /// Correlation above which two variables are called a group.
    pub variable_threshold: f64,
    /// Fraction of steps that must not decrease for a variable to be judged an
    /// accumulator.
    pub accumulator_monotone: f64,
    /// Fraction of steps that must strictly increase, which excludes constants.
    pub accumulator_increase: f64,
    /// Whole-machine regimes to look for.
    pub regimes: usize,
    /// Share of the series used to fit the predictor.
    pub train_fraction: f64,
    /// Reflections required before any claim is made at all.
    pub minimum_frames: usize,
}

impl Default for ObserverConfig {
    fn default() -> Self {
        ObserverConfig {
            capacity: 4096,
            // 0.7 is deliberately not near 1.0: entities that are genuinely
            // coupled still differ, and a threshold that only accepts near
            // identity would find only trivial structure.
            entity_threshold: 0.7,
            variable_threshold: 0.85,
            accumulator_monotone: 0.98,
            accumulator_increase: 0.3,
            regimes: 4,
            train_fraction: 0.7,
            // Below this, correlation over a handful of samples produces
            // confident nonsense.
            minimum_frames: 30,
        }
    }
}

/// A learner that knows the machine only through its reflection.
#[derive(Debug)]
pub struct Observer {
    lens: Lens,
    config: ObserverConfig,
    experience: Experience,
}

impl Observer {
    pub fn new(lens: Lens, config: ObserverConfig) -> Observer {
        let experience = Experience::new(config.capacity);
        Observer {
            lens,
            config,
            experience,
        }
    }

    pub fn lens(&self) -> Lens {
        self.lens
    }

    pub fn experience(&self) -> &Experience {
        &self.experience
    }

    /// Take in one reflection.
    pub fn observe(&mut self, snapshot: &MirrorSnapshot) {
        self.experience.record(snapshot);
    }

    /// Everything the observer currently believes.
    ///
    /// Returns `None` until it has seen enough to say anything. An observer
    /// that produces claims from four samples is not being cautious about a
    /// detail; it is producing noise with the shape of knowledge.
    pub fn findings(&self) -> Option<Findings> {
        if self.experience.len() < self.config.minimum_frames {
            return None;
        }

        // The usable working set: cells observed in every remembered
        // reflection. Anything else would need a decision about what a gap
        // means, and that decision belongs to whoever is asking.
        let cells = self.experience.complete_cells();
        if cells.is_empty() {
            return None;
        }

        let levels: BTreeMap<(usize, usize), Vec<f64>> = cells
            .iter()
            .map(|(row, col)| ((*row, *col), self.experience.series(*row, *col)))
            .collect();
        let differenced: BTreeMap<(usize, usize), Vec<f64>> = levels
            .iter()
            .map(|(key, series)| (*key, discover::differences(series)))
            .collect();

        let rows = self.experience.rows();
        let cols = self.experience.cols();

        // What accumulates. Under the labelled lens this is read off the
        // mirror; under the unlabelled lens it has to be worked out. The two
        // answers are both recorded so they can be compared.
        let inferred: Vec<usize> = discover::accumulators(
            &levels,
            cols,
            self.config.accumulator_monotone,
            self.config.accumulator_increase,
        )
        .into_iter()
        .map(|a| a.column)
        .collect();

        let declared: Option<Vec<usize>> = if self.lens == Lens::Labelled {
            Some(
                self.experience
                    .channels()
                    .iter()
                    .enumerate()
                    .filter(|(_, spec)| {
                        self.lens.declared_semantics(spec) == Some(Semantics::Cumulative)
                    })
                    .map(|(index, _)| index)
                    .collect(),
            )
        } else {
            None
        };

        // The observer uses whichever it has. This is the point where the two
        // observers actually diverge in behaviour rather than in vocabulary.
        let accumulators_in_use = declared.clone().unwrap_or_else(|| inferred.clone());

        // How much of the machine moves as one. This is a real property of
        // whatever is running, and it has to be measured before it can be
        // removed.
        let raw_similarity = discover::similarity_matrix(&differenced, rows, cols);
        let common_mode = discover::mean_similarity(&raw_similarity);

        // Structure is then sought in what is left once the machine-wide
        // signal is subtracted. Without this step a busy machine looks like one
        // undifferentiated blob, which is true and useless.
        let residual = discover::remove_common_mode(&differenced, rows, cols);
        let similarity = discover::similarity_matrix(&residual, rows, cols);
        let entity_clusters = discover::cluster_entities(&similarity, self.config.entity_threshold);
        let variable_groups =
            discover::variable_groups(&differenced, cols, self.config.variable_threshold);

        let edges: Vec<(usize, usize, u16)> = self
            .experience
            .relations()
            .iter()
            .map(|r| (r.source as usize, r.target as usize, r.kind.as_u16()))
            .collect();
        let relation_lifts = discover::relation_lift(&similarity, &edges);

        // Regimes are found over standardised whole-machine vectors, so that a
        // counter in the hundreds of billions does not define every regime by
        // itself.
        let frames = self.standardised_frames(&cells);
        let (regimes, assignment) =
            discover::regimes(&frames, self.config.regimes.min(frames.len()), 25);
        let transitions = discover::regime_transitions(&assignment, regimes.len());

        let prediction =
            predict::evaluate(&levels, &accumulators_in_use, self.config.train_fraction);

        Some(Findings {
            lens: self.lens,
            frames: self.experience.len(),
            span_ns: self.experience.span_ns(),
            cadence_ns: self.experience.cadence_ns(),
            entities: rows,
            channels: cols,
            complete_cells: cells.len(),
            common_mode,
            entity_names: (0..rows)
                .map(|row| {
                    self.experience
                        .entities()
                        .get(row)
                        .map(|e| self.lens.entity_name(row, e))
                        .unwrap_or_else(|| format!("entity_{row}"))
                })
                .collect(),
            channel_names: (0..cols)
                .map(|col| {
                    self.experience
                        .channels()
                        .get(col)
                        .map(|c| self.lens.channel_name(col, c))
                        .unwrap_or_else(|| format!("x{col}"))
                })
                .collect(),
            relation_names: relation_lifts
                .iter()
                .map(|lift| {
                    let named = self
                        .experience
                        .relations()
                        .iter()
                        .find(|r| r.kind.as_u16() == lift.kind)
                        .map(|r| self.lens.relation_name(r))
                        .unwrap_or_else(|| format!("edge_type_{}", lift.kind));
                    (lift.kind, named)
                })
                .collect(),
            entity_clusters,
            variable_groups,
            relation_lifts,
            regimes,
            regime_transitions: transitions,
            prediction,
            inferred_accumulators: inferred,
            declared_accumulators: declared,
        })
    }

    /// Per-reflection vectors over the complete cells, each cell standardised
    /// across time so every variable contributes comparably.
    fn standardised_frames(&self, cells: &[(usize, usize)]) -> Vec<Vec<f64>> {
        let standardised: Vec<Vec<f64>> = cells
            .iter()
            .map(|(row, col)| discover::standardise(&self.experience.series(*row, *col)))
            .collect();

        (0..self.experience.len())
            .map(|frame| {
                standardised
                    .iter()
                    .map(|series| series.get(frame).copied().unwrap_or(0.0))
                    .map(|v| if v.is_finite() { v } else { 0.0 })
                    .collect()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_mirror::test_support::fixture;

    #[test]
    fn an_observer_says_nothing_until_it_has_seen_enough() {
        let mut observer = Observer::new(Lens::Unlabelled, ObserverConfig::default());
        for step in 0..5u64 {
            let mut snapshot = fixture();
            snapshot.sequence = step;
            snapshot.monotonic_ns = step * 1_000_000;
            observer.observe(&snapshot);
        }
        assert!(
            observer.findings().is_none(),
            "five samples is not evidence of anything"
        );
    }

    #[test]
    fn the_unlabelled_observer_never_reports_a_human_name() {
        let mut observer = Observer::new(Lens::Unlabelled, ObserverConfig::default());
        for step in 0..60u64 {
            let mut snapshot = fixture();
            snapshot.sequence = step;
            snapshot.monotonic_ns = step * 1_000_000;
            snapshot
                .state
                .set(2, 0, 3_600_000.0 + (step as f64).sin() * 1000.0);
            snapshot
                .state
                .set(3, 0, 800_000.0 + (step as f64).cos() * 1000.0);
            snapshot.state.set(2, 1, 12_345.0 + step as f64 * 10.0);
            observer.observe(&snapshot);
        }
        let findings = observer.findings().expect("findings");
        let text = format!("{findings:?}");
        for leak in ["cpu/", "frequency", "smt_sibling", "machine", "core/"] {
            assert!(!text.contains(leak), "unlabelled findings leaked `{leak}`");
        }
        assert!(findings.declared_accumulators.is_none());
    }

    #[test]
    fn the_labelled_observer_is_told_what_accumulates() {
        let mut observer = Observer::new(Lens::Labelled, ObserverConfig::default());
        for step in 0..60u64 {
            let mut snapshot = fixture();
            snapshot.sequence = step;
            snapshot.monotonic_ns = step * 1_000_000;
            snapshot
                .state
                .set(2, 0, 3_600_000.0 + (step as f64).sin() * 1000.0);
            snapshot.state.set(2, 1, 12_345.0 + step as f64 * 10.0);
            observer.observe(&snapshot);
        }
        let findings = observer.findings().expect("findings");
        // The fixture declares column 1 (cpu.time.idle) cumulative.
        assert_eq!(findings.declared_accumulators, Some(vec![1]));
    }

    #[test]
    fn an_epoch_change_resets_what_the_observer_knows() {
        let mut observer = Observer::new(Lens::Unlabelled, ObserverConfig::default());
        for step in 0..60u64 {
            let mut snapshot = fixture();
            snapshot.monotonic_ns = step * 1_000_000;
            observer.observe(&snapshot);
        }
        assert!(observer.findings().is_some());

        let mut changed = fixture();
        changed.epoch += 1;
        observer.observe(&changed);
        assert!(
            observer.findings().is_none(),
            "the machine's shape changed; prior series describe different hardware"
        );
    }
}
