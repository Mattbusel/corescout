//! The perturbing commands: `benchmark`, `analyze`, `run`, `experiment`.
//!
//! Everything here deliberately changes the machine. That is the line this
//! project draws: observation belongs to the mirror, intentional perturbation
//! belongs to experimentation, and the two do not share a module.
//!
//! # `run` takes an intent, not a core number
//!
//! `--intent latency-critical` states an outcome. What placement satisfies it
//! is a question for the machine, and the answer depends on what it has learned
//! about itself. `--profile latency` remains for the original behaviour, which
//! is to pin to whatever the benchmark ranked first, and the two are separate
//! commands' worth of idea sharing one verb on purpose: the comparison between
//! them is a result.

use corescout_analysis::placement::{self, Profile};
use corescout_analysis::{profile_cache, Analysis};
use corescout_core::error::{Error, Result};
use corescout_experiment::baselines::{self, Baseline};
use corescout_experiment::benchmarks::{workloads, Progress, RunConfig, Runner};
use corescout_experiment::harness::{Harness, HarnessConfig};
use corescout_experiment::workloads::{Scenario, ScenarioKind};
use corescout_human::Format;
use corescout_intent::Intent;
use corescout_report::{human_debug, json};

/// Measure every core and print the raw distributions.
pub fn benchmark(config: RunConfig, format: Format) -> Result<()> {
    let platform = corescout_substrate::platform::detect();
    let topology = platform.discover_topology()?;
    let results = measure(&topology, platform.as_ref(), config)?;
    match format {
        Format::Json => println!("{}", json::benchmark(&results)?),
        Format::Human => print!("{}", human_debug::benchmark(&results)),
    }
    Ok(())
}

/// Benchmark, rank the cores, and recommend placements.
pub fn analyze(config: RunConfig, format: Format, use_cache: bool, save: bool) -> Result<()> {
    let analysis = analysis_for(config, use_cache, save)?;
    match format {
        Format::Json => println!("{}", json::analysis(&analysis)?),
        Format::Human => print!("{}", human_debug::analysis(&analysis)),
    }
    Ok(())
}

/// Launch a program under a placement.
pub fn run(
    program: &[String],
    profile: &str,
    intent: Option<&str>,
    config: RunConfig,
    use_cache: bool,
    format: Format,
) -> Result<()> {
    let analysis = analysis_for(config, use_cache, false)?;

    // An intent names an outcome; a profile names a ranking. When an intent is
    // given it decides, and the profile it implies is reported so the choice is
    // inspectable rather than magic.
    let (cpus, reason) = match intent {
        Some(name) => {
            let intent = Intent::preset(name).ok_or_else(|| {
                Error::invalid(format!(
                    "unknown intent {name:?}; try latency-critical, throughput, \
                     efficient or balanced"
                ))
            })?;
            let profile = profile_for_intent(&intent);
            let selection = placement::select(&analysis, profile)?;
            let reason = format!("intent {name} resolved to the {} ranking", profile.label());
            if format == Format::Human {
                eprint!("{}", placement::explain(&selection, profile, &program[0]));
            }
            (selection.cpus, reason)
        }
        None => {
            let parsed = Profile::parse(profile).ok_or_else(|| {
                Error::invalid(format!(
                    "unknown profile {profile:?}; try latency, compute, memory or pair"
                ))
            })?;
            let selection = placement::select(&analysis, parsed)?;
            if format == Format::Human {
                eprint!("{}", placement::explain(&selection, parsed, &program[0]));
            }
            (selection.cpus, format!("profile {}", parsed.label()))
        }
    };

    if format == Format::Human {
        eprintln!("placing on {} ({reason})", cpus.to_list());
    }
    let platform = corescout_substrate::platform::detect();
    let mut command = std::process::Command::new(&program[0]);
    command.args(&program[1..]);
    let mut child = platform.spawn_with_affinity(&mut command, &cpus)?;
    let status = child.wait().map_err(|source| Error::Io {
        path: program[0].clone().into(),
        source,
    })?;
    if !status.success() {
        // The launched program's exit code is its own business; we report it
        // rather than translating it into ours.
        if format == Format::Human {
            eprintln!("the program exited with {status}");
        }
    }
    Ok(())
}

/// Run the workload scenarios, or compare placement policies on them.
pub fn experiment(
    compare: bool,
    scenarios: &[String],
    repetitions: Option<u32>,
    against: &str,
    format: Format,
) -> Result<()> {
    let mut config = HarnessConfig::default();
    if let Some(repetitions) = repetitions {
        config.repetitions = repetitions.max(1);
    }
    let harness = Harness::detect(config)?;
    let selected = resolve_scenarios(scenarios)?;
    let reference = resolve_baseline(against)?;

    // The ranking is a baseline in its own right, so it is computed here rather
    // than being left absent and silently excluded from the comparison.
    let topology = corescout_substrate::platform::detect().discover_topology()?;
    let ranking = profile_cache::load(&topology)
        .ok()
        .flatten()
        .and_then(|analysis| placement::select(&analysis, Profile::Latency).ok())
        .and_then(|selection| selection.cpus.iter().next());

    if !compare {
        for scenario in &selected {
            if let Some(reason) = harness.cannot_run(scenario) {
                println!("{}: skipped, {reason}", scenario.kind.label());
                continue;
            }
            let placement = baselines::decide(
                Baseline::Scheduler,
                harness.permitted(),
                None,
                ranking,
                harness.config().seed,
            );
            let measurement = harness.measure(scenario, &placement)?;
            match format {
                Format::Json => println!("{}", crate::runtime::to_json(&measurement)?),
                Format::Human => println!(
                    "{:<24} median {:>9.0} ns  p99 {:>9.0} ns  jitter {:.2}  {} contaminated",
                    scenario.kind.label(),
                    measurement.latency.median,
                    measurement.latency.p99,
                    measurement.jitter,
                    measurement.contaminated
                ),
            }
        }
        return Ok(());
    }

    let comparison = harness.run(&selected, ranking)?;
    match format {
        Format::Json => println!("{}", crate::runtime::to_json(&comparison)?),
        Format::Human => print!("{}", comparison.report(reference)),
    }
    Ok(())
}

/// Which ranking an intent implies.
fn profile_for_intent(intent: &Intent) -> Profile {
    use corescout_intent::Objective;
    // The first objective by weight decides. An intent with no objectives is
    // not constructible, so there is no empty case to handle.
    let dominant = intent.objectives().into_iter().max_by(|a, b| {
        intent
            .weight(*a)
            .partial_cmp(&intent.weight(*b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    match dominant {
        Some(Objective::Throughput) => Profile::Compute,
        // Energy has no ranking of its own: the benchmark measures no power, so
        // the honest fallback is the core that finishes soonest and idles.
        Some(Objective::Energy) => Profile::Compute,
        // Latency, jitter, migration and the empty case all want the core with
        // the best tail, which is what the latency ranking measures.
        _ => Profile::Latency,
    }
}

/// Run the benchmark, reporting progress to stderr so `--json` on stdout stays
/// machine-readable.
fn measure(
    topology: &corescout_substrate::Topology,
    platform: &dyn corescout_substrate::platform::Platform,
    config: RunConfig,
) -> Result<corescout_experiment::benchmarks::BenchmarkResults> {
    let runner = Runner::new(platform, config);
    let set = workloads::default_workloads();
    // Progress goes to stderr so `--json` on stdout stays machine-readable.
    runner.run(topology, &set, &mut |progress| match progress {
        Progress::RoundStarted {
            round,
            of,
            workload,
        } => {
            eprint!("\rround {round}/{of}: {workload}                    ");
        }
        Progress::CoreFinished { core, cpu } => {
            eprint!("\r  core {core} (cpu {cpu}) done                    ");
        }
    })
}

fn analysis_for(config: RunConfig, use_cache: bool, save: bool) -> Result<Analysis> {
    if use_cache {
        let platform = corescout_substrate::platform::detect();
        if let Ok(topology) = platform.discover_topology() {
            if let Ok(Some(cached)) = profile_cache::load(&topology) {
                return Ok(cached);
            }
        }
    }
    let platform = corescout_substrate::platform::detect();
    let topology = platform.discover_topology()?;
    let results = measure(&topology, platform.as_ref(), config)?;
    let analysis = Analysis::build(topology, results)?;
    if save || use_cache {
        // A failure to cache is not a failure to analyse.
        let _ = profile_cache::save(&analysis);
    }
    Ok(analysis)
}

fn resolve_scenarios(names: &[String]) -> Result<Vec<Scenario>> {
    if names.is_empty() {
        return Ok(ScenarioKind::all().into_iter().map(Scenario::new).collect());
    }
    names
        .iter()
        .map(|name| {
            ScenarioKind::all()
                .into_iter()
                .find(|kind| kind.label() == name)
                .map(Scenario::new)
                .ok_or_else(|| {
                    Error::invalid(format!(
                        "unknown scenario {name:?}; known scenarios are: {}",
                        ScenarioKind::all()
                            .iter()
                            .map(|k| k.label())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                })
        })
        .collect()
}

fn resolve_baseline(name: &str) -> Result<Baseline> {
    Baseline::all_static()
        .into_iter()
        .chain([Baseline::SelfModel])
        .find(|baseline| baseline.label() == name)
        .ok_or_else(|| {
            Error::invalid(format!(
                "unknown policy {name:?}; known policies are: {}",
                Baseline::all_static()
                    .iter()
                    .map(|b| b.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_named_scenarios_means_all_of_them() {
        assert_eq!(
            resolve_scenarios(&[]).unwrap().len(),
            ScenarioKind::all().len()
        );
    }

    #[test]
    fn an_unknown_scenario_lists_the_known_ones() {
        let error = resolve_scenarios(&["nonsense".into()])
            .unwrap_err()
            .to_string();
        assert!(error.contains("nonsense"), "{error}");
        assert!(error.contains("thermal-drift"), "{error}");
    }

    #[test]
    fn named_scenarios_are_resolved_in_order() {
        let resolved = resolve_scenarios(&["mixed".into(), "branch-heavy".into()]).unwrap();
        assert_eq!(resolved[0].kind, ScenarioKind::Mixed);
        assert_eq!(resolved[1].kind, ScenarioKind::BranchHeavy);
    }

    #[test]
    fn baselines_resolve_by_their_printed_label() {
        assert_eq!(
            resolve_baseline("os-scheduler").unwrap(),
            Baseline::Scheduler
        );
        assert_eq!(resolve_baseline("random").unwrap(), Baseline::Random);
        assert_eq!(resolve_baseline("self-model").unwrap(), Baseline::SelfModel);
        assert!(resolve_baseline("best").is_err());
    }

    #[test]
    fn each_preset_intent_resolves_to_a_ranking() {
        for name in ["latency-critical", "throughput", "efficient", "balanced"] {
            let intent = Intent::preset(name).expect("a preset");
            let _ = profile_for_intent(&intent);
        }
        assert_eq!(
            profile_for_intent(&Intent::preset("latency-critical").unwrap()),
            Profile::Latency
        );
        assert_eq!(
            profile_for_intent(&Intent::preset("throughput").unwrap()),
            Profile::Compute
        );
    }
}
