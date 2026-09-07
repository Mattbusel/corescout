//! Turning a named profile into a CPU mask, and saying why.
//!
//! # Why the explanation ships with the mask
//!
//! A tool that pins a program to CPU 3 and says nothing has asked the user to
//! trust it. The reasons are already computed during ranking, so carrying them
//! alongside the mask costs nothing and makes the decision arguable, which is
//! the only way anyone finds out it was wrong.
//!
//! # This is the original CoreScout's answer
//!
//! Everything here is a static ranking derived from one benchmark run. It does
//! not adapt, does not watch what happens next, and does not know the machine
//! has changed since. That is not a criticism: it is a strong, cheap baseline,
//! and [`corescout_experiment::Baseline::CoreScoutRanking`] exists so the
//! self-modelling controller has to beat it rather than merely differ from it.

use corescout_core::error::{Error, Result};
use corescout_core::CpuSet;
use serde::{Deserialize, Serialize};

use crate::{Analysis, PairRecommendation, Recommendation};

/// A named placement profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    /// The best tail latency: for work where a rare slow response is the
    /// failure, not the average.
    Latency,
    /// The most single-thread throughput.
    Compute,
    /// The best behaviour under a working set that does not fit in cache.
    Memory,
    /// Two cores that work well together, chosen by the weaker of the pair.
    Pair,
}

impl Profile {
    pub fn label(self) -> &'static str {
        match self {
            Profile::Latency => "latency",
            Profile::Compute => "compute",
            Profile::Memory => "memory",
            Profile::Pair => "pair",
        }
    }

    /// Parse a profile name, accepting the aliases people actually type.
    pub fn parse(name: &str) -> Option<Profile> {
        match name {
            "latency" | "latency-critical" | "jitter" => Some(Profile::Latency),
            "compute" | "throughput" | "cpu" => Some(Profile::Compute),
            "memory" | "memory-heavy" | "bandwidth" => Some(Profile::Memory),
            "pair" | "workers" | "worker-pair" => Some(Profile::Pair),
            _ => None,
        }
    }

    pub fn all() -> [Profile; 4] {
        [
            Profile::Latency,
            Profile::Compute,
            Profile::Memory,
            Profile::Pair,
        ]
    }
}

/// A chosen placement, with the reasoning that produced it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub cpus: CpuSet,
    /// SMT siblings that should be left idle for the choice to hold. Reported
    /// rather than enforced: taking a sibling away from the rest of the machine
    /// is the user's decision, not ours.
    pub keep_idle: Vec<u32>,
    pub score: f64,
    pub reasons: Vec<String>,
}

/// Choose CPUs for a profile.
pub fn select(analysis: &Analysis, profile: Profile) -> Result<Selection> {
    let recommendations = &analysis.recommendations;
    let single = |recommendation: Option<&Recommendation>| -> Result<Selection> {
        let recommendation = recommendation.ok_or_else(|| {
            Error::invalid(format!(
                "the benchmark produced no {} recommendation for this machine",
                profile.label()
            ))
        })?;
        Ok(Selection {
            cpus: [recommendation.cpu].into_iter().collect(),
            keep_idle: recommendation.keep_idle.clone(),
            score: recommendation.score,
            reasons: recommendation.reasons.clone(),
        })
    };

    match profile {
        Profile::Latency => single(recommendations.latency_critical.as_ref()),
        Profile::Compute => single(recommendations.compute_heavy.as_ref()),
        Profile::Memory => single(recommendations.memory_heavy.as_ref()),
        Profile::Pair => {
            let pair: &PairRecommendation =
                recommendations.worker_pair.as_ref().ok_or_else(|| {
                    Error::invalid(
                        "the benchmark produced no worker pair; this machine may have \
                         only one usable core",
                    )
                })?;
            Ok(Selection {
                cpus: pair.cpus.iter().copied().collect(),
                keep_idle: Vec::new(),
                score: pair.score,
                reasons: pair.reasons.clone(),
            })
        }
    }
}

/// Explain a selection in plain language.
pub fn explain(selection: &Selection, profile: Profile, program: &str) -> String {
    let mut out = format!(
        "running {program} on CPU {} (profile: {})\n",
        selection.cpus.to_list(),
        profile.label()
    );
    for reason in &selection.reasons {
        out.push_str(&format!("  - {reason}\n"));
    }
    if !selection.keep_idle.is_empty() {
        // Stated as a condition, not done silently: the measurement that
        // produced this ranking assumed the sibling was idle, and if it is not,
        // the ranking does not apply.
        out.push_str(&format!(
            "  - this holds only while CPU {} stays idle; it shares a core\n",
            corescout_core::cpuset::format_list(&selection.keep_idle)
        ));
    }
    out.push_str(
        "  - pinning helps some workloads and harms others; measure yours before \
         keeping this\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::analysis_fixture;

    #[test]
    fn profile_names_and_their_aliases_parse() {
        assert_eq!(Profile::parse("latency"), Some(Profile::Latency));
        assert_eq!(Profile::parse("latency-critical"), Some(Profile::Latency));
        assert_eq!(Profile::parse("throughput"), Some(Profile::Compute));
        assert_eq!(Profile::parse("pair"), Some(Profile::Pair));
        assert_eq!(Profile::parse("fastest"), None);
    }

    #[test]
    fn every_profile_has_a_distinct_label_that_parses_back() {
        for profile in Profile::all() {
            assert_eq!(Profile::parse(profile.label()), Some(profile));
        }
    }

    #[test]
    fn the_latency_profile_selects_the_core_with_the_better_tail() {
        // The fixture's core 0 computes faster but has a 50 us tail; core 1 has
        // a 1 us tail. Latency must not pick the faster core.
        let analysis = analysis_fixture();
        let selection = select(&analysis, Profile::Latency).expect("a selection");
        assert_eq!(selection.cpus.len(), 1);
        let compute = select(&analysis, Profile::Compute).expect("a selection");
        assert_ne!(
            selection.cpus, compute.cpus,
            "the fastest core is not automatically the best core"
        );
    }

    #[test]
    fn an_explanation_names_the_program_the_cpu_and_the_profile() {
        let analysis = analysis_fixture();
        let selection = select(&analysis, Profile::Latency).expect("a selection");
        let text = explain(&selection, Profile::Latency, "./trading_engine");
        assert!(text.contains("./trading_engine"), "{text}");
        assert!(text.contains("profile: latency"), "{text}");
        assert!(!selection.reasons.is_empty(), "a selection with no reasons");
    }

    #[test]
    fn an_explanation_says_pinning_is_not_universally_good() {
        // The project's standing claim, printed every time it places anything.
        let analysis = analysis_fixture();
        let selection = select(&analysis, Profile::Compute).expect("a selection");
        let text = explain(&selection, Profile::Compute, "./program");
        assert!(
            text.contains("helps some workloads and harms others"),
            "{text}"
        );
    }

    #[test]
    fn a_pair_selection_contains_two_cpus() {
        let analysis = analysis_fixture();
        let pair = select(&analysis, Profile::Pair).expect("a pair");
        assert_eq!(pair.cpus.len(), 2);
    }

    #[test]
    fn a_missing_recommendation_is_an_error_rather_than_a_guess() {
        let mut analysis = analysis_fixture();
        analysis.recommendations.latency_critical = None;
        let error = select(&analysis, Profile::Latency).unwrap_err().to_string();
        assert!(error.contains("no latency recommendation"), "{error}");
    }
}
