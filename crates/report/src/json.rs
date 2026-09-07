//! JSON projections of benchmark, topology and ranking results.
//!
//! Separate from [`corescout_human::json`] because these need the topology, the
//! benchmark and the analysis, and those crates reach hardware. Keeping them
//! apart is what lets the mirror consumers link the human crate without
//! inheriting a path to `/sys`.

use corescout_analysis::Analysis;
use corescout_core::error::{Error, Result};
use corescout_experiment::benchmarks::BenchmarkResults;
use corescout_substrate::topology::Topology;
use serde::Serialize;

/// The machine's structure as JSON.
pub fn topology(topology: &Topology) -> Result<String> {
    encode(topology, "topology")
}

/// Raw benchmark distributions as JSON.
pub fn benchmark(results: &BenchmarkResults) -> Result<String> {
    encode(results, "benchmark results")
}

/// Rankings and recommendations as JSON.
///
/// This is also the cached machine profile format, which is why it carries a
/// schema version and the full topology rather than just the scores.
pub fn analysis(analysis: &Analysis) -> Result<String> {
    encode(analysis, "analysis")
}

fn encode<T: Serialize>(value: &T, what: &str) -> Result<String> {
    serde_json::to_string_pretty(value)
        .map_err(|error| Error::invalid(format!("could not serialise the {what}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_substrate::test_support::fake_topology;

    #[test]
    fn topology_json_is_parseable_and_keeps_the_key_fields() {
        let json = topology(&fake_topology()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["model_name"], "Test CPU");
        assert_eq!(value["physical_cores"].as_array().unwrap().len(), 2);
        assert_eq!(value["logical_cpus"].as_array().unwrap().len(), 4);
        // SMT relationships must survive into JSON: downstream tooling needs
        // them to reason about what to leave idle.
        assert_eq!(value["logical_cpus"][0]["smt_siblings"][0], 2);
    }

    #[test]
    fn analysis_json_exposes_scores_and_recommendations() {
        let a = corescout_analysis::test_support::analysis_fixture();
        let json = analysis(&a).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["schema_version"], corescout_analysis::SCHEMA_VERSION);
        assert!(value["rankings"].as_array().unwrap().len() >= 4);
        assert!(value["recommendations"]["latency_critical"]["cpu"].is_number());
        // The raw distributions are part of the contract, not just the scores.
        assert!(value["benchmark"]["measurements"][0]["raw"]["p99"].is_number());
    }
}
