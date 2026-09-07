//! Human-readable rendering.
//!
//! Plain ASCII, no colour, no cursor control: output is as likely to be read
//! through a pipe, in a CI log or in a bug report as on a terminal.
//!
//! Every renderer returns a `String` rather than printing, so the output is
//! testable and the caller decides where it goes.

use corescout_analysis::{Analysis, Recommendation};
use corescout_core::cpuset::format_list;
use corescout_experiment::benchmarks::{BenchmarkResults, Measurement};
use corescout_human::{bytes, mhz, pad, rpad};
use corescout_substrate::topology::{CacheKind, Topology};

/// How many entries each ranking shows.
const TOP_N: usize = 5;

/// `corescout info`.
pub fn topology(topology: &Topology) -> String {
    let mut out = String::new();
    out.push_str("CoreScout topology\n\n");

    let smt = if topology.smt_enabled() {
        format!(
            "{} logical CPUs, SMT width {}",
            topology.logical_count(),
            topology.smt_width()
        )
    } else {
        format!("{} logical CPUs, no SMT", topology.logical_count())
    };

    push_field(&mut out, "CPU", &topology.model_name);
    push_field(&mut out, "Vendor", &topology.vendor);
    push_field(
        &mut out,
        "Physical cores",
        &topology.physical_count().to_string(),
    );
    push_field(&mut out, "Logical CPUs", &smt);
    if topology.hybrid {
        let counts: Vec<String> = topology
            .cores_by_type()
            .into_iter()
            .map(|(k, v)| format!("{v} x {k}"))
            .collect();
        push_field(&mut out, "Hybrid", &counts.join(", "));
    }
    push_field(
        &mut out,
        "NUMA nodes",
        &if topology.numa_nodes.is_empty() {
            "not exposed by this kernel".to_string()
        } else {
            topology
                .numa_nodes
                .iter()
                .map(|n| {
                    let mem = n
                        .memory_kb
                        .map(|kb| format!(", {}", bytes(kb * 1024)))
                        .unwrap_or_default();
                    format!("node {} (CPUs {}{})", n.id, format_list(&n.cpus), mem)
                })
                .collect::<Vec<_>>()
                .join("\n               ")
        },
    );
    push_field(
        &mut out,
        "Online CPUs",
        &or_none(&format_list(&topology.online_cpus)),
    );
    push_field(
        &mut out,
        "Offline CPUs",
        &or_none(&format_list(&topology.offline_cpus)),
    );
    push_field(
        &mut out,
        "This process",
        &format!(
            "{} ({} of {} CPUs available)",
            or_none(&format_list(&topology.process_affinity)),
            topology.process_affinity.len(),
            topology.logical_count()
        ),
    );

    if let Some(favoured) = topology.firmware_favored_cpu() {
        if let Some(hint) = &favoured.favored {
            push_field(
                &mut out,
                "Preferred core",
                &format!(
                    "CPU {} (highest_perf {}, from {})",
                    favoured.id, hint.highest_perf, hint.source
                ),
            );
        }
    }

    out.push_str("\nCaches\n");
    out.push_str(&caches(topology));

    out.push_str("\nCores\n");
    out.push_str(&cores(topology));
    out
}

fn caches(topology: &Topology) -> String {
    if topology.caches.is_empty() {
        return "  not exposed by this kernel\n".to_string();
    }
    let mut out = String::new();
    out.push_str(&format!(
        "  {} {} {} {}\n",
        pad("level", 7),
        rpad("size", 9),
        rpad("instances", 10),
        "shared by"
    ));

    // Group identical caches so a 32-way L1 does not print 32 times.
    let mut groups: Vec<(String, Option<u64>, usize, usize)> = Vec::new();
    for cache in &topology.caches {
        let label = cache_label(cache.level, cache.kind);
        let width = cache.shared_cpus.len();
        match groups
            .iter_mut()
            .find(|g| g.0 == label && g.1 == cache.size_bytes && g.3 == width)
        {
            Some(g) => g.2 += 1,
            None => groups.push((label, cache.size_bytes, 1, width)),
        }
    }

    for (label, size, count, width) in groups {
        let shared = match width {
            1 => "1 CPU (private)".to_string(),
            n => format!("{n} CPUs"),
        };
        out.push_str(&format!(
            "  {} {} {} {}\n",
            pad(&label, 7),
            rpad(&size.map(bytes).unwrap_or_else(|| "-".into()), 9),
            rpad(&count.to_string(), 10),
            shared
        ));
    }
    out
}

fn cache_label(level: u8, kind: CacheKind) -> String {
    match kind {
        CacheKind::L1Data => "L1d".to_string(),
        CacheKind::L1Instruction => "L1i".to_string(),
        CacheKind::L2Unified => "L2".to_string(),
        CacheKind::L3Unified => "L3".to_string(),
        CacheKind::Other => format!("L{level}"),
    }
}

fn cores(topology: &Topology) -> String {
    let mut out = String::new();
    let hybrid = topology.hybrid;
    let has_firmware = topology.logical_cpus.iter().any(|c| c.favored.is_some());

    out.push_str(&format!(
        "  {} {} {} {} {} {}{}{}\n",
        rpad("core", 4),
        pad("cpus", 9),
        rpad("pkg", 3),
        rpad("numa", 4),
        rpad("MHz cur", 7),
        rpad("MHz max", 7),
        if hybrid { "  type" } else { "" },
        if has_firmware { "  fw rank" } else { "" },
    ));

    for core in &topology.physical_cores {
        let primary = topology.cpu(core.primary_cpu());
        let freq = primary.map(|c| c.frequency).unwrap_or_default();
        let rank = primary
            .and_then(|c| c.favored.as_ref().map(|f| f.rank.to_string()))
            .unwrap_or_else(|| "-".to_string());

        out.push_str(&format!(
            "  {} {} {} {} {} {}{}{}\n",
            rpad(&core.id.to_string(), 4),
            pad(&format_list(&core.logical_cpus), 9),
            rpad(&core.package_id.to_string(), 3),
            rpad(
                &core
                    .numa_node
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "-".into()),
                4
            ),
            rpad(&mhz(freq.current_khz), 7),
            rpad(&mhz(freq.max_khz), 7),
            if hybrid {
                format!("  {}", pad(core.core_type.short(), 4))
            } else {
                String::new()
            },
            if has_firmware {
                format!("  {}", rpad(&rank, 7))
            } else {
                String::new()
            },
        ));
    }
    out
}

/// `corescout benchmark`: the raw per-core measurement table.
pub fn benchmark(results: &BenchmarkResults) -> String {
    let mut out = String::new();
    out.push_str("CoreScout benchmark\n\n");
    out.push_str(&format!(
        "{} rounds x {} samples per visit, {} warmup iterations, {:.1}s total\n",
        results.config.rounds,
        results.config.samples_per_visit,
        results.config.warmup_iterations,
        results.duration_seconds
    ));
    out.push_str(&format!(
        "clock: {:.0} ns per read, {} ns resolution\n",
        results.clock_overhead_ns, results.clock_resolution_ns
    ));

    for id in results.workload_ids() {
        let measurements = results.by_workload(&id);
        let Some(first) = measurements.first() else {
            continue;
        };
        out.push_str(&format!("\n{} ({})\n", id, first.kind.label()));
        out.push_str(&format!(
            "  {} {} {} {} {} {} {}\n",
            rpad("cpu", 4),
            rpad("median", 12),
            rpad("p95", 12),
            rpad("p99", 12),
            rpad("min", 12),
            rpad("jitter", 9),
            "discarded"
        ));
        for m in measurements {
            out.push_str(&format!(
                "  {} {} {} {} {} {} {}\n",
                rpad(&m.cpu.to_string(), 4),
                rpad(&duration(m.clean.median), 12),
                rpad(&duration(m.clean.p95), 12),
                rpad(&duration(m.raw.p99), 12),
                rpad(&duration(m.clean.min), 12),
                rpad(&format!("{:.2}%", m.clean.relative_jitter() * 100.0), 9),
                discarded(m),
            ));
        }
    }

    out.push_str(&warnings(&results.warnings));
    out
}

fn discarded(m: &Measurement) -> String {
    format!(
        "{}/{} ({} preempted)",
        m.samples_collected - m.samples_used,
        m.samples_collected,
        m.preempted
    )
}

/// `corescout analyze`.
pub fn analysis(analysis: &Analysis) -> String {
    let mut out = String::new();
    out.push_str("CoreScout Analysis\n\n");
    push_field(&mut out, "CPU", &analysis.topology.model_name);
    push_field(
        &mut out,
        "Physical cores",
        &analysis.topology.physical_count().to_string(),
    );
    push_field(
        &mut out,
        "Logical CPUs",
        &analysis.topology.logical_count().to_string(),
    );
    push_field(
        &mut out,
        "Measured",
        &format!(
            "{} cores in {:.1}s",
            analysis.cores.len(),
            analysis.benchmark.duration_seconds
        ),
    );

    for ranking in &analysis.rankings {
        out.push_str(&format!("\n{}\n", ranking.title));
        for (i, entry) in ranking.entries.iter().take(TOP_N).enumerate() {
            out.push_str(&format!(
                "{} CPU {} {} {} {}\n",
                rpad(&format!("{}.", i + 1), 3),
                rpad(&entry.cpu.to_string(), 4),
                rpad(&format!("{:.1}", entry.score), 7),
                rpad(&format!("{:.1}", entry.value), 10),
                ranking.unit,
            ));
        }
    }

    out.push_str("\nRecommended\n");
    push_recommendation(
        &mut out,
        "Latency-critical thread",
        analysis.recommendations.latency_critical.as_ref(),
    );
    push_recommendation(
        &mut out,
        "Compute-heavy thread",
        analysis.recommendations.compute_heavy.as_ref(),
    );
    push_recommendation(
        &mut out,
        "Memory-heavy thread",
        analysis.recommendations.memory_heavy.as_ref(),
    );
    if let Some(pair) = &analysis.recommendations.worker_pair {
        out.push_str(&format!(
            "  {} CPU {} + CPU {}\n",
            pad("Worker pair", 24),
            pair.cpus[0],
            pair.cpus[1]
        ));
        for reason in &pair.reasons {
            out.push_str(&format!("  {}  {}\n", pad("", 24), reason));
        }
    }

    if !analysis.observations.is_empty() {
        out.push_str("\nObservations\n");
        for note in &analysis.observations {
            out.push_str(&format!("  - {note}\n"));
        }
    }

    out.push_str(&warnings(&analysis.benchmark.warnings));
    out
}

fn push_recommendation(out: &mut String, label: &str, rec: Option<&Recommendation>) {
    let Some(rec) = rec else {
        return;
    };
    out.push_str(&format!(
        "  {} CPU {}  (score {:.1})\n",
        pad(label, 24),
        rec.cpu,
        rec.score
    ));
    for reason in &rec.reasons {
        out.push_str(&format!("  {}  {}\n", pad("", 24), reason));
    }
}

fn warnings(warnings: &[String]) -> String {
    if warnings.is_empty() {
        return String::new();
    }
    let mut out = String::from("\nCaveats\n");
    for w in warnings {
        out.push_str(&format!("  ! {w}\n"));
    }
    out
}

/// Render a nanosecond duration at a sensible scale.
pub fn duration(ns: f64) -> String {
    if ns >= 1_000_000.0 {
        format!("{:.3} ms", ns / 1e6)
    } else if ns >= 1_000.0 {
        format!("{:.2} us", ns / 1e3)
    } else {
        format!("{ns:.0} ns")
    }
}

fn push_field(out: &mut String, label: &str, value: &str) {
    out.push_str(&format!("{} {value}\n", pad(label, 14)));
}

fn or_none(s: &str) -> String {
    if s.is_empty() {
        "none".to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corescout_substrate::test_support::fake_topology;

    #[test]
    fn duration_scales() {
        assert_eq!(duration(900.0), "900 ns");
        assert_eq!(duration(1_500.0), "1.50 us");
        assert_eq!(duration(2_500_000.0), "2.500 ms");
    }

    #[test]
    fn info_output_names_the_essentials() {
        let out = topology(&fake_topology());
        assert!(out.contains("Test CPU"));
        assert!(out.contains("Physical cores"));
        assert!(out.contains("SMT width 2"));
        // Cache sharing must be visible, it is the point of the section.
        assert!(out.contains("L3"));
        assert!(out.contains("4 CPUs"));
    }

    #[test]
    fn info_reports_an_empty_offline_list_as_none() {
        let out = topology(&fake_topology());
        let line = out
            .lines()
            .find(|l| l.starts_with("Offline CPUs"))
            .expect("offline line");
        assert!(line.ends_with("none"), "got {line:?}");
    }

    #[test]
    fn info_handles_a_machine_without_numa_or_caches() {
        let mut t = fake_topology();
        t.numa_nodes.clear();
        t.caches.clear();
        let out = topology(&t);
        assert!(out.contains("not exposed by this kernel"));
    }

    #[test]
    fn analysis_output_contains_rankings_and_recommendations() {
        let a = corescout_analysis::test_support::analysis_fixture();
        let out = analysis(&a);
        assert!(out.contains("CoreScout Analysis"));
        assert!(out.contains("Single-thread compute"));
        assert!(out.contains("Lowest jitter"));
        assert!(out.contains("Latency-critical thread"));
        assert!(out.contains("Worker pair"));
    }

    #[test]
    fn benchmark_output_shows_discarded_samples() {
        let a = corescout_analysis::test_support::analysis_fixture();
        let out = benchmark(&a.benchmark);
        assert!(out.contains("discarded"));
        assert!(out.contains("preempted"));
    }
}
