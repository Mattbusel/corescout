//! What an observer believes, and how to print it.
//!
//! The rendering here obeys the lens: an unlabelled observer's report contains
//! no human names, because it never had any. That is not cosmetic. If the
//! report quietly reached past the lens to look up a friendlier name, the A/B
//! comparison would be measuring nothing.

use serde::{Deserialize, Serialize};

use crate::lens::Lens;
use corescout_represent::discover::{EntityCluster, Regime, RelationLift, VariableGroup};
use corescout_selfmodel::predict::PredictionReport;

/// The complete set of claims an observer is prepared to make.
#[derive(Debug, Clone, PartialEq)]
pub struct Findings {
    pub lens: Lens,
    pub frames: usize,
    pub span_ns: u64,
    pub cadence_ns: f64,
    pub entities: usize,
    pub channels: usize,
    pub complete_cells: usize,
    /// Mean similarity across every pair of entities *before* the machine-wide
    /// common mode was removed. High means most of the machine is moving
    /// together, which is a fact about the workload rather than the structure.
    pub common_mode: f64,
    /// Names as this observer is permitted to see them.
    pub entity_names: Vec<String>,
    pub channel_names: Vec<String>,
    pub relation_names: Vec<(u16, String)>,
    pub entity_clusters: Vec<EntityCluster>,
    pub variable_groups: Vec<VariableGroup>,
    pub relation_lifts: Vec<RelationLift>,
    pub regimes: Vec<Regime>,
    pub regime_transitions: Vec<Vec<usize>>,
    pub prediction: PredictionReport,
    /// Columns the observer worked out were accumulators by watching them.
    pub inferred_accumulators: Vec<usize>,
    /// Columns the mirror said were accumulators. `None` under the unlabelled
    /// lens, which is the point.
    pub declared_accumulators: Option<Vec<usize>>,
}

impl Findings {
    fn entity(&self, row: usize) -> &str {
        self.entity_names
            .get(row)
            .map(|s| s.as_str())
            .unwrap_or("?")
    }

    fn channel(&self, col: usize) -> &str {
        self.channel_names
            .get(col)
            .map(|s| s.as_str())
            .unwrap_or("?")
    }

    fn relation(&self, kind: u16) -> String {
        self.relation_names
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| format!("edge_type_{kind}"))
    }

    /// Whether the observer's inferred accumulators match what the mirror
    /// declared. `None` when it was never told.
    ///
    /// This is the measurement of what one human label was worth: if an
    /// observer that was never told can work it out, the label added nothing
    /// but a name.
    pub fn accumulator_agreement(&self) -> Option<AccumulatorAgreement> {
        let declared = self.declared_accumulators.as_ref()?;
        let inferred = &self.inferred_accumulators;
        let hits = declared.iter().filter(|c| inferred.contains(c)).count();
        let false_positives = inferred.iter().filter(|c| !declared.contains(c)).count();
        Some(AccumulatorAgreement {
            declared: declared.len(),
            inferred: inferred.len(),
            agreed: hits,
            missed: declared.len().saturating_sub(hits),
            false_positives,
        })
    }

    /// Side-by-side comparison of two observers' findings.
    ///
    /// The question being answered: did the vocabulary change what was
    /// discovered, or only what it was called?
    pub fn compare(&self, other: &Findings) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Observer comparison: {} vs {}\n\n",
            self.lens.label(),
            other.lens.label()
        ));
        out.push_str(&format!(
            "  {:<34} {:>14} {:>14}\n",
            "",
            self.lens.label(),
            other.lens.label()
        ));

        let row = |out: &mut String, label: &str, a: String, b: String| {
            out.push_str(&format!("  {label:<34} {a:>14} {b:>14}\n"));
        };
        row(
            &mut out,
            "reflections seen",
            self.frames.to_string(),
            other.frames.to_string(),
        );
        row(
            &mut out,
            "entity groups found",
            self.entity_clusters.len().to_string(),
            other.entity_clusters.len().to_string(),
        );
        row(
            &mut out,
            "grouped entities",
            self.entity_clusters
                .iter()
                .map(|c| c.rows.len())
                .sum::<usize>()
                .to_string(),
            other
                .entity_clusters
                .iter()
                .map(|c| c.rows.len())
                .sum::<usize>()
                .to_string(),
        );
        row(
            &mut out,
            "variable groups found",
            self.variable_groups.len().to_string(),
            other.variable_groups.len().to_string(),
        );
        row(
            &mut out,
            "accumulators identified",
            self.accumulators_used().len().to_string(),
            other.accumulators_used().len().to_string(),
        );
        row(
            &mut out,
            "best edge type lift",
            self.relation_lifts
                .first()
                .map(|l| format!("{:+.3}", l.lift))
                .unwrap_or_else(|| "-".into()),
            other
                .relation_lifts
                .first()
                .map(|l| format!("{:+.3}", l.lift))
                .unwrap_or_else(|| "-".into()),
        );
        row(
            &mut out,
            "prediction skill (median)",
            format!("{:+.3}", self.prediction.median_skill),
            format!("{:+.3}", other.prediction.median_skill),
        );
        row(
            &mut out,
            "cells beating the baseline",
            format!(
                "{}/{}",
                self.prediction.cells_with_skill, self.prediction.cells
            ),
            format!(
                "{}/{}",
                other.prediction.cells_with_skill, other.prediction.cells
            ),
        );

        out.push('\n');
        if self.discovered_the_same_structure(other) {
            out.push_str(
                "  The two observers grouped the machine identically. The vocabulary changed\n\
                 \x20 what the findings are called, not what was found.\n",
            );
        } else {
            out.push_str(
                "  The two observers grouped the machine differently. Which one is closer to\n\
                 \x20 the truth is a separate question from which one had the vocabulary.\n",
            );
        }
        out
    }

    /// Whether two observers partitioned the entities the same way.
    pub fn discovered_the_same_structure(&self, other: &Findings) -> bool {
        let mine: Vec<Vec<usize>> = self
            .entity_clusters
            .iter()
            .map(|c| c.rows.clone())
            .collect();
        let theirs: Vec<Vec<usize>> = other
            .entity_clusters
            .iter()
            .map(|c| c.rows.clone())
            .collect();
        mine == theirs
    }

    /// The accumulator set this observer actually used: declared where it was
    /// told, inferred where it was not.
    pub fn accumulators_used(&self) -> Vec<usize> {
        self.declared_accumulators
            .clone()
            .unwrap_or_else(|| self.inferred_accumulators.clone())
    }

    /// Human-readable rendering, in this observer's own vocabulary.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Observer findings ({} lens)\n\n",
            self.lens.label()
        ));
        out.push_str(&format!(
            "  watched          {} reflections over {:.1}s (every {:.0} ms)\n",
            self.frames,
            self.span_ns as f64 / 1e9,
            self.cadence_ns / 1e6
        ));
        out.push_str(&format!(
            "  saw              {} entities, {} variables, {} consistently observed cells\n",
            self.entities, self.channels, self.complete_cells
        ));

        out.push_str("\nWhich variables accumulate\n");
        if self.inferred_accumulators.is_empty() {
            out.push_str("  none inferred from behaviour\n");
        } else {
            let names: Vec<&str> = self
                .inferred_accumulators
                .iter()
                .map(|c| self.channel(*c))
                .collect();
            out.push_str(&format!("  inferred by watching: {}\n", names.join(", ")));
        }
        match self.accumulator_agreement() {
            Some(agreement) => out.push_str(&format!(
                "  the mirror declared {} of these; inference agreed on {}, missed {}, \
                 over-called {}\n",
                agreement.declared, agreement.agreed, agreement.missed, agreement.false_positives
            )),
            None => out.push_str(
                "  this observer was not told which variables accumulate; the list above \
                 was worked out\n",
            ),
        }

        out.push_str("\nHow much of the machine moves as one\n");
        out.push_str(&format!(
            "  {:.3} mean pairwise similarity before the common mode was removed\n",
            self.common_mode
        ));

        out.push_str("\nWhich entities behave alike, once the common mode is removed\n");
        if self.entity_clusters.is_empty() {
            out.push_str("  no groups above the similarity threshold\n");
        }
        for cluster in &self.entity_clusters {
            let names: Vec<&str> = cluster.rows.iter().map(|r| self.entity(*r)).collect();
            out.push_str(&format!(
                "  {{{}}}  cohesion {:.3}\n",
                names.join(", "),
                cluster.cohesion
            ));
        }

        out.push_str("\nWhich variables move together\n");
        if self.variable_groups.is_empty() {
            out.push_str("  none above the correlation threshold\n");
        }
        for group in &self.variable_groups {
            let names: Vec<&str> = group.columns.iter().map(|c| self.channel(*c)).collect();
            out.push_str(&format!(
                "  {{{}}}  cohesion {:.3}\n",
                names.join(", "),
                group.cohesion
            ));
        }

        out.push_str("\nWhich kinds of connection predict shared behaviour\n");
        if self.relation_lifts.is_empty() {
            out.push_str("  no edge type had enough comparable pairs\n");
        }
        for lift in &self.relation_lifts {
            out.push_str(&format!(
                "  {:<20} {:>4} pairs   connected {:+.3} vs baseline {:+.3}   lift {:+.3}  d={:.2}\n",
                self.relation(lift.kind),
                lift.pairs,
                lift.connected_similarity,
                lift.baseline_similarity,
                lift.lift,
                lift.separation
            ));
        }

        out.push_str("\nRecurring whole-machine states\n");
        for regime in &self.regimes {
            out.push_str(&format!(
                "  regime {}  {:>4} reflections ({:>4.1}%)  spread {:.2}\n",
                regime.id,
                regime.occupancy,
                100.0 * regime.occupancy as f64 / self.frames.max(1) as f64,
                regime.spread
            ));
        }
        if self.regimes.len() > 1 {
            out.push_str("  transitions (rows = from, columns = to)\n");
            for (from, row) in self.regime_transitions.iter().enumerate() {
                let cells: Vec<String> = row.iter().map(|c| format!("{c:>5}")).collect();
                out.push_str(&format!("    {from} |{}\n", cells.join("")));
            }
        }

        out.push_str("\nPredicting the next reflection\n");
        out.push_str(&format!(
            "  scored {} cells; {} beat the naive baseline\n",
            self.prediction.cells, self.prediction.cells_with_skill
        ));
        out.push_str(&format!(
            "  skill vs baseline: median {:+.3}, mean {:+.3}   (0 = no better than \
             copying the last value)\n",
            self.prediction.median_skill, self.prediction.mean_skill
        ));
        for score in self.prediction.best.iter().take(3) {
            out.push_str(&format!(
                "    best  {} / {}  skill {:+.3}\n",
                self.entity(score.row),
                self.channel(score.col),
                score.skill
            ));
        }
        for score in self.prediction.worst.iter().take(2) {
            out.push_str(&format!(
                "    worst {} / {}  skill {:+.3}\n",
                self.entity(score.row),
                self.channel(score.col),
                score.skill
            ));
        }

        out
    }
}

/// How well an observer's own inference matched what the mirror declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccumulatorAgreement {
    pub declared: usize,
    pub inferred: usize,
    pub agreed: usize,
    pub missed: usize,
    pub false_positives: usize,
}

impl AccumulatorAgreement {
    /// True when inference recovered exactly what the label would have said.
    pub fn is_exact(&self) -> bool {
        self.missed == 0 && self.false_positives == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_represent::discover::EntityCluster;
    use corescout_selfmodel::predict::PredictionReport;

    fn findings(lens: Lens) -> Findings {
        Findings {
            lens,
            frames: 100,
            span_ns: 10_000_000_000,
            cadence_ns: 100_000_000.0,
            entities: 3,
            channels: 2,
            complete_cells: 6,
            common_mode: 0.42,
            entity_names: match lens {
                Lens::Labelled => vec!["cpu/0".into(), "cpu/4".into(), "cpu/1".into()],
                Lens::Unlabelled => vec!["entity_0".into(), "entity_1".into(), "entity_2".into()],
            },
            channel_names: match lens {
                Lens::Labelled => vec!["cpu.frequency.current".into(), "cpu.time.idle".into()],
                Lens::Unlabelled => vec!["x0".into(), "x1".into()],
            },
            relation_names: vec![(
                2,
                match lens {
                    Lens::Labelled => "smt_sibling".to_string(),
                    Lens::Unlabelled => "edge_type_2".to_string(),
                },
            )],
            entity_clusters: vec![EntityCluster {
                rows: vec![0, 1],
                cohesion: 0.94,
            }],
            variable_groups: vec![],
            relation_lifts: vec![RelationLift {
                kind: 2,
                pairs: 2,
                connected_similarity: 0.94,
                baseline_similarity: 0.21,
                lift: 0.73,
                separation: 2.4,
            }],
            regimes: vec![Regime {
                id: 0,
                occupancy: 100,
                spread: 0.5,
            }],
            regime_transitions: vec![vec![99]],
            prediction: PredictionReport {
                cells: 6,
                cells_with_skill: 4,
                median_skill: 0.31,
                mean_skill: 0.28,
                best: vec![],
                worst: vec![],
                accumulator_columns: vec![1],
            },
            inferred_accumulators: vec![1],
            declared_accumulators: match lens {
                Lens::Labelled => Some(vec![1]),
                Lens::Unlabelled => None,
            },
        }
    }

    #[test]
    fn an_unlabelled_report_contains_no_human_vocabulary() {
        let text = findings(Lens::Unlabelled).render();
        for leak in ["cpu", "frequency", "smt", "sibling", "idle"] {
            assert!(
                !text.to_lowercase().contains(leak),
                "the unlabelled report leaked `{leak}`:\n{text}"
            );
        }
        assert!(text.contains("edge_type_2"));
        assert!(text.contains("entity_0"));
    }

    #[test]
    fn a_labelled_report_uses_the_names_it_was_given() {
        let text = findings(Lens::Labelled).render();
        assert!(text.contains("smt_sibling"));
        assert!(text.contains("cpu/0"));
    }

    #[test]
    fn both_reports_state_the_same_numbers() {
        // The lens changes vocabulary, never findings.
        let a = findings(Lens::Labelled);
        let b = findings(Lens::Unlabelled);
        assert_eq!(a.entity_clusters, b.entity_clusters);
        assert_eq!(a.relation_lifts, b.relation_lifts);
        assert_eq!(a.prediction.median_skill, b.prediction.median_skill);
    }

    #[test]
    fn agreement_is_only_computable_when_the_observer_was_told() {
        assert!(findings(Lens::Unlabelled).accumulator_agreement().is_none());
        let agreement = findings(Lens::Labelled).accumulator_agreement().unwrap();
        assert!(agreement.is_exact());
        assert_eq!(agreement.agreed, 1);
    }

    #[test]
    fn disagreement_is_reported_rather_than_smoothed() {
        let mut f = findings(Lens::Labelled);
        f.inferred_accumulators = vec![0];
        let agreement = f.accumulator_agreement().unwrap();
        assert!(!agreement.is_exact());
        assert_eq!(agreement.missed, 1);
        assert_eq!(agreement.false_positives, 1);
    }
}
