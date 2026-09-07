//! Command line parsing.
//!
//! Hand written rather than derived from a framework. An argument framework is a
//! dependency tree that would end up linked into the same binary as the
//! benchmark loop and the mirror's observation pass; fewer moving parts inside
//! the measurement apparatus is worth a little typing.
//!
//! # The shape of the surface
//!
//! The commands are grouped by which layer of the architecture they touch, and
//! the grouping is deliberate. `mirror` and `record` observe. `inspect`,
//! `replay`, `observe`, `learn`, `model` and `latent` consume a reflection and
//! never touch hardware. `benchmark`, `analyze`, `run` and `experiment`
//! deliberately perturb. `agent` acts. Anything that changes the machine says so
//! in its help text.

use std::path::PathBuf;

use corescout_core::error::{Error, Result};
use corescout_experiment::benchmarks::RunConfig;
use corescout_human::Format;

pub const USAGE: &str = "\
corescout - a machine that observes itself, models itself, and acts on itself

USAGE:
    corescout <COMMAND> [OPTIONS]

OBSERVING (passive; changes nothing)
    mirror                  Observe continuously and publish the self-state plane
    record <PATH>           Observe and append every reflection to a trace
    info                    Show CPU topology, caches, SMT and NUMA layout

READING THE MIRROR (no hardware access at all)
    inspect                 Read the live plane and describe what is in it
    replay <PATH>           Read a recorded trace as though it were live
    observe                 Find structure in the reflection, unaided by labels
    learn                   Learn to predict the next reflection, and score it
    model                   Report what the self-model currently believes
    latent                  List the states the machine discovered in itself
    describe                Say what the machine can truthfully say about itself

DOING SCIENCE ON ITSELF (no hardware access at all)
    theory                  Conjecture, test, and report what survived refutation
    concepts                List the concepts the machine coined about itself
    resources               Virtual resources it discovered and can reproduce

PERTURBING (deliberately changes the machine)
    benchmark               Measure every core and print the raw distributions
    analyze                 Benchmark, rank the cores, and recommend pins
    run -- <PROGRAM>        Launch a program placed for an intent
    experiment run          Run the workload scenarios
    experiment compare      Compare placement policies against the baselines

ACTING (bounded, reversible, audited)
    agent start             Run the autonomous decision loop
    agent status            Show what the running agent has done
    agent stop              Stop it and revert what it changed

    demo self               The whole cycle end to end, then an explanation

MIRROR AND RECORD OPTIONS:
    --once                  Observe once, print, and exit
    --interval <MS>         Milliseconds between observations (default 100)
    --ticks <N>             Stop after N observations
    --plane <PATH>          Self-state plane path (default: $XDG_RUNTIME_DIR)
    --record <PATH>         Also append each reflection to a replayable trace
    --no-publish            Observe without writing the plane
    --filter <KEY>          With --once: show only entities matching KEY

CONSUMER OPTIONS:
    --source <PATH>         A plane or a recorded trace; the consumer cannot tell
    --frames <N>            Reflections to consume (default: until the source ends)
    --labels                Observer A: show it the human names for things
    --no-labels             Observer B: identity, state, relations and time only
    --both                  Run both observers and compare what each discovered

INTENT AND AGENT OPTIONS:
    --intent <NAME>         latency-critical | throughput | efficient | balanced
    --dry-run               Decide and report, but perform no action
    --scope <PID>           Also permit acting on this process (repeatable)
    --max-actions <N>       Stop the agent after N actions

EXPERIMENT OPTIONS:
    --scenario <NAME>       A named scenario; repeatable, default all
    --repetitions <N>       Times to measure each policy (default 5)
    --against <POLICY>      Reference policy for comparison (default os-scheduler)

GENERAL OPTIONS:
    --json                  Emit JSON instead of a human-readable report
    --quick                 Fewer rounds and samples: faster, less precise
    --rounds <N>            Interleaved passes over the core set (default 5)
    --samples <N>           Timed samples per core per round (default 6)
    --warmup <N>            Unmeasured iterations before each visit (default 3)
    --all-cpus              Measure every logical CPU, not one per core
    --profile <NAME>        run: latency | compute | memory | pair
    --no-cache              analyze/run: ignore and do not write cached profiles
    --save                  analyze: write the result to the profile cache
    -h, --help              Show this help
    -V, --version           Show the version

EXAMPLES:
    corescout mirror
    corescout record trace.jsonl --ticks 600
    corescout replay trace.jsonl --json
    corescout observe --both --source trace.jsonl
    corescout learn --source trace.jsonl --frames 2000
    corescout theory --source trace.jsonl
    corescout concepts --source trace.jsonl
    corescout run --intent latency-critical -- ./my_program
    corescout experiment compare --against os-scheduler
    corescout agent start --dry-run
    corescout demo self
";

/// Options shared by every command that consumes a reflection.
///
/// One struct because the whole point of the memory layer is that a consumer
/// cannot tell a live plane from a recording; giving them separate option sets
/// would reintroduce the distinction at the command line.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SourceOptions {
    /// A plane path or a trace path. `None` means the default plane.
    pub path: Option<PathBuf>,
    /// Stop after this many reflections.
    pub frames: Option<usize>,
    /// Milliseconds to wait for a live source between polls.
    pub interval_ms: u64,
}

/// Which ontology an observer is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ontology {
    /// Observer A: entity keys, channel names, units, relation kinds.
    Labelled,
    /// Observer B: opaque identity, state, relations and time.
    #[default]
    Unlabelled,
    /// Run both and compare what each discovered.
    Both,
}

/// A parsed command line.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Mirror {
        format: Format,
        once: bool,
        interval_ms: u64,
        ticks: Option<u64>,
        plane: Option<PathBuf>,
        publish: bool,
        record: Option<PathBuf>,
        filter: Option<String>,
    },
    Record {
        path: PathBuf,
        interval_ms: u64,
        ticks: Option<u64>,
        plane: Option<PathBuf>,
        publish: bool,
        format: Format,
    },
    Info {
        format: Format,
    },
    Inspect {
        source: SourceOptions,
        format: Format,
        filter: Option<String>,
    },
    Replay {
        source: SourceOptions,
        format: Format,
    },
    Observe {
        source: SourceOptions,
        ontology: Ontology,
        format: Format,
    },
    Learn {
        source: SourceOptions,
        format: Format,
    },
    Model {
        source: SourceOptions,
        format: Format,
    },
    Latent {
        source: SourceOptions,
        format: Format,
    },
    Describe {
        source: SourceOptions,
        format: Format,
    },
    Theory {
        source: SourceOptions,
        format: Format,
    },
    Concepts {
        source: SourceOptions,
        format: Format,
    },
    Resources {
        source: SourceOptions,
        format: Format,
    },
    Benchmark {
        config: RunConfig,
        format: Format,
    },
    Analyze {
        config: RunConfig,
        format: Format,
        use_cache: bool,
        save: bool,
    },
    Run {
        profile: String,
        intent: Option<String>,
        program: Vec<String>,
        config: RunConfig,
        format: Format,
        use_cache: bool,
    },
    Experiment {
        action: ExperimentAction,
        scenarios: Vec<String>,
        repetitions: Option<u32>,
        against: String,
        format: Format,
    },
    Agent {
        action: AgentAction,
        intent: String,
        dry_run: bool,
        scope: Vec<u32>,
        max_actions: Option<u64>,
        source: SourceOptions,
        format: Format,
    },
    Demo {
        what: String,
        format: Format,
    },
    Help,
    Version,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExperimentAction {
    Run,
    Compare,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentAction {
    Start,
    Status,
    Stop,
}

/// Parse a command line, excluding the program name.
pub fn parse(args: &[String]) -> Result<Command> {
    let Some(first) = args.first() else {
        return Ok(Command::Help);
    };
    match first.as_str() {
        "-h" | "--help" | "help" => return Ok(Command::Help),
        "-V" | "--version" | "version" => return Ok(Command::Version),
        _ => {}
    }

    let mut parser = Parser::new(&args[1..]);
    let command = match first.as_str() {
        "mirror" => {
            let path = parser.positional();
            Command::Mirror {
                format: parser.format()?,
                once: parser.flag("--once"),
                interval_ms: parser.number("--interval")?.unwrap_or(100),
                ticks: parser.number("--ticks")?,
                plane: parser.path("--plane")?,
                publish: !parser.flag("--no-publish"),
                record: parser.path("--record")?.or(path),
                filter: parser.value("--filter")?,
            }
        }
        "record" => {
            let path = parser
                .positional()
                .or(parser.path("--record")?)
                .ok_or_else(|| {
                    Error::invalid("record needs a path: corescout record trace.jsonl")
                })?;
            Command::Record {
                path,
                interval_ms: parser.number("--interval")?.unwrap_or(100),
                ticks: parser.number("--ticks")?,
                plane: parser.path("--plane")?,
                publish: !parser.flag("--no-publish"),
                format: parser.format()?,
            }
        }
        "info" => Command::Info {
            format: parser.format()?,
        },
        "inspect" => Command::Inspect {
            filter: parser.value("--filter")?,
            source: parser.source()?,
            format: parser.format()?,
        },
        "replay" => {
            let path = parser.positional();
            let mut source = parser.source()?;
            source.path = source.path.or(path);
            if source.path.is_none() {
                return Err(Error::invalid(
                    "replay needs a trace: corescout replay trace.jsonl",
                ));
            }
            Command::Replay {
                source,
                format: parser.format()?,
            }
        }
        "observe" => Command::Observe {
            ontology: parser.ontology(),
            source: parser.source()?,
            format: parser.format()?,
        },
        "learn" => Command::Learn {
            source: parser.source()?,
            format: parser.format()?,
        },
        "model" => Command::Model {
            source: parser.source()?,
            format: parser.format()?,
        },
        "latent" => Command::Latent {
            source: parser.source()?,
            format: parser.format()?,
        },
        "describe" => Command::Describe {
            source: parser.source()?,
            format: parser.format()?,
        },
        "theory" => Command::Theory {
            source: parser.source()?,
            format: parser.format()?,
        },
        "concepts" => Command::Concepts {
            source: parser.source()?,
            format: parser.format()?,
        },
        "resources" => Command::Resources {
            source: parser.source()?,
            format: parser.format()?,
        },
        "benchmark" => Command::Benchmark {
            config: parser.run_config()?,
            format: parser.format()?,
        },
        "analyze" => Command::Analyze {
            config: parser.run_config()?,
            format: parser.format()?,
            use_cache: !parser.flag("--no-cache"),
            save: parser.flag("--save"),
        },
        "run" => {
            let program = parser.take_after_separator();
            if program.is_empty() {
                return Err(Error::invalid(
                    "run needs a program: corescout run --intent latency-critical -- ./program",
                ));
            }
            Command::Run {
                intent: parser.value("--intent")?,
                profile: parser
                    .value("--profile")?
                    .unwrap_or_else(|| "latency".into()),
                program,
                config: parser.run_config()?,
                format: parser.format()?,
                use_cache: !parser.flag("--no-cache"),
            }
        }
        "experiment" => {
            let action = match parser.positional_word().as_deref() {
                Some("run") | None => ExperimentAction::Run,
                Some("compare") => ExperimentAction::Compare,
                Some(other) => {
                    return Err(Error::invalid(format!(
                        "unknown experiment action {other:?}; expected run or compare"
                    )))
                }
            };
            Command::Experiment {
                action,
                scenarios: parser.values("--scenario")?,
                repetitions: parser.number("--repetitions")?.map(|n| n as u32),
                against: parser
                    .value("--against")?
                    .unwrap_or_else(|| "os-scheduler".into()),
                format: parser.format()?,
            }
        }
        "agent" => {
            let action = match parser.positional_word().as_deref() {
                Some("start") | None => AgentAction::Start,
                Some("status") => AgentAction::Status,
                Some("stop") => AgentAction::Stop,
                Some(other) => {
                    return Err(Error::invalid(format!(
                        "unknown agent action {other:?}; expected start, status or stop"
                    )))
                }
            };
            let mut scope = Vec::new();
            for value in parser.values("--scope")? {
                scope.push(value.parse::<u32>().map_err(|_| {
                    Error::invalid(format!("--scope expects a process id, got {value:?}"))
                })?);
            }
            Command::Agent {
                action,
                intent: parser
                    .value("--intent")?
                    .unwrap_or_else(|| "balanced".into()),
                dry_run: parser.flag("--dry-run"),
                scope,
                max_actions: parser.number("--max-actions")?,
                source: parser.source()?,
                format: parser.format()?,
            }
        }
        "demo" => Command::Demo {
            what: parser
                .positional_word()
                .unwrap_or_else(|| "self".to_string()),
            format: parser.format()?,
        },
        other => {
            return Err(Error::invalid(format!(
                "unknown command {other:?}; run `corescout --help`"
            )))
        }
    };

    parser.finish()?;
    Ok(command)
}

/// A tiny consuming argument parser.
///
/// Each accessor removes what it consumed, so [`Parser::finish`] can reject
/// anything left over. Silently ignoring an unrecognised flag is how a person
/// ends up believing they ran a benchmark with `--samples 200` when they did
/// not, and then trusting the number.
struct Parser {
    args: Vec<String>,
    after_separator: Vec<String>,
}

impl Parser {
    fn new(args: &[String]) -> Parser {
        let mut before = Vec::new();
        let mut after = Vec::new();
        let mut split = false;
        for arg in args {
            if !split && arg == "--" {
                split = true;
                continue;
            }
            if split {
                after.push(arg.clone());
            } else {
                before.push(arg.clone());
            }
        }
        Parser {
            args: before,
            after_separator: after,
        }
    }

    fn take_after_separator(&mut self) -> Vec<String> {
        std::mem::take(&mut self.after_separator)
    }

    fn flag(&mut self, name: &str) -> bool {
        if let Some(index) = self.args.iter().position(|a| a == name) {
            self.args.remove(index);
            true
        } else {
            false
        }
    }

    fn value(&mut self, name: &str) -> Result<Option<String>> {
        let Some(index) = self.args.iter().position(|a| a == name) else {
            return Ok(None);
        };
        if index + 1 >= self.args.len() {
            return Err(Error::invalid(format!("{name} needs a value")));
        }
        self.args.remove(index);
        Ok(Some(self.args.remove(index)))
    }

    /// Every occurrence of a repeatable option.
    fn values(&mut self, name: &str) -> Result<Vec<String>> {
        let mut found = Vec::new();
        while let Some(value) = self.value(name)? {
            found.push(value);
        }
        Ok(found)
    }

    fn number(&mut self, name: &str) -> Result<Option<u64>> {
        match self.value(name)? {
            Some(text) => text
                .parse::<u64>()
                .map(Some)
                .map_err(|_| Error::invalid(format!("{name} expects a number, got {text:?}"))),
            None => Ok(None),
        }
    }

    fn path(&mut self, name: &str) -> Result<Option<PathBuf>> {
        Ok(self.value(name)?.map(PathBuf::from))
    }

    /// The first argument that is not an option or an option's value.
    fn positional(&mut self) -> Option<PathBuf> {
        self.positional_word().map(PathBuf::from)
    }

    fn positional_word(&mut self) -> Option<String> {
        let index = self.args.iter().position(|a| !a.starts_with('-'))?;
        // Only take it if it is not the value of a preceding option.
        if index > 0
            && self.args[index - 1].starts_with("--")
            && takes_a_value(&self.args[index - 1])
        {
            return None;
        }
        Some(self.args.remove(index))
    }

    fn format(&mut self) -> Result<Format> {
        Ok(if self.flag("--json") {
            Format::Json
        } else {
            Format::Human
        })
    }

    fn ontology(&mut self) -> Ontology {
        if self.flag("--both") {
            Ontology::Both
        } else if self.flag("--labels") {
            let _ = self.flag("--no-labels");
            Ontology::Labelled
        } else {
            let _ = self.flag("--no-labels");
            Ontology::Unlabelled
        }
    }

    fn source(&mut self) -> Result<SourceOptions> {
        Ok(SourceOptions {
            path: self.path("--source")?,
            frames: self.number("--frames")?.map(|n| n as usize),
            interval_ms: self.number("--interval")?.unwrap_or(100),
        })
    }

    fn run_config(&mut self) -> Result<RunConfig> {
        let mut config = if self.flag("--quick") {
            RunConfig::quick()
        } else {
            RunConfig::default()
        };
        if let Some(rounds) = self.number("--rounds")? {
            config.rounds = rounds as u32;
        }
        if let Some(samples) = self.number("--samples")? {
            config.samples_per_visit = samples as u32;
        }
        if let Some(warmup) = self.number("--warmup")? {
            config.warmup_iterations = warmup as u32;
        }
        if self.flag("--all-cpus") {
            config.all_logical_cpus = true;
        }
        Ok(config)
    }

    fn finish(self) -> Result<()> {
        match self.args.first() {
            Some(unexpected) => Err(Error::invalid(format!(
                "unexpected argument {unexpected:?}; run `corescout --help`"
            ))),
            None => Ok(()),
        }
    }
}

/// Whether an option consumes the argument after it.
///
/// Needed only so a positional is not mistaken for an option's value; the
/// option accessors themselves run first and remove their own pairs.
fn takes_a_value(option: &str) -> bool {
    !matches!(
        option,
        "--json"
            | "--once"
            | "--quick"
            | "--all-cpus"
            | "--no-cache"
            | "--no-publish"
            | "--save"
            | "--labels"
            | "--no-labels"
            | "--both"
            | "--dry-run"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn no_arguments_shows_help() {
        assert_eq!(parse(&[]).unwrap(), Command::Help);
    }

    #[test]
    fn help_and_version_are_recognised_in_every_spelling() {
        for spelling in ["-h", "--help", "help"] {
            assert_eq!(parse(&args(spelling)).unwrap(), Command::Help);
        }
        for spelling in ["-V", "--version", "version"] {
            assert_eq!(parse(&args(spelling)).unwrap(), Command::Version);
        }
    }

    #[test]
    fn an_unknown_command_is_an_error_not_a_default() {
        let error = parse(&args("mirrour")).unwrap_err().to_string();
        assert!(error.contains("mirrour"), "{error}");
    }

    #[test]
    fn an_unrecognised_flag_is_rejected_rather_than_ignored() {
        // The important one. Silently ignoring --sampels means someone believes
        // they ran 200 samples and trusts the resulting number.
        let error = parse(&args("benchmark --sampels 200"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("sampels"), "{error}");
    }

    #[test]
    fn mirror_defaults_are_continuous_and_publishing() {
        match parse(&args("mirror")).unwrap() {
            Command::Mirror {
                once,
                publish,
                interval_ms,
                ticks,
                ..
            } => {
                assert!(!once);
                assert!(publish);
                assert_eq!(interval_ms, 100);
                assert_eq!(ticks, None);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn mirror_options_are_read() {
        match parse(&args(
            "mirror --once --interval 50 --ticks 10 --no-publish --json",
        ))
        .unwrap()
        {
            Command::Mirror {
                once,
                interval_ms,
                ticks,
                publish,
                format,
                ..
            } => {
                assert!(once);
                assert_eq!(interval_ms, 50);
                assert_eq!(ticks, Some(10));
                assert!(!publish);
                assert_eq!(format, Format::Json);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn record_requires_a_path() {
        assert!(parse(&args("record")).is_err());
        match parse(&args("record trace.jsonl")).unwrap() {
            Command::Record { path, .. } => assert_eq!(path, PathBuf::from("trace.jsonl")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn replay_requires_a_trace() {
        assert!(parse(&args("replay")).is_err());
        match parse(&args("replay trace.jsonl")).unwrap() {
            Command::Replay { source, .. } => {
                assert_eq!(source.path, Some(PathBuf::from("trace.jsonl")));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_path_given_as_an_option_value_is_not_taken_as_a_positional() {
        match parse(&args("replay --source trace.jsonl")).unwrap() {
            Command::Replay { source, .. } => {
                assert_eq!(source.path, Some(PathBuf::from("trace.jsonl")));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_two_ontologies_are_selectable_and_unlabelled_is_the_default() {
        // Observer B is the default because the project's claim is about what
        // can be learned *without* our names for things.
        let unlabelled = matches!(
            parse(&args("observe")).unwrap(),
            Command::Observe {
                ontology: Ontology::Unlabelled,
                ..
            }
        );
        assert!(unlabelled);
        assert!(matches!(
            parse(&args("observe --labels")).unwrap(),
            Command::Observe {
                ontology: Ontology::Labelled,
                ..
            }
        ));
        assert!(matches!(
            parse(&args("observe --both")).unwrap(),
            Command::Observe {
                ontology: Ontology::Both,
                ..
            }
        ));
    }

    #[test]
    fn run_requires_a_program_after_the_separator() {
        assert!(parse(&args("run --intent latency-critical")).is_err());
        match parse(&args("run --intent latency-critical -- ./prog --flag")).unwrap() {
            Command::Run {
                intent, program, ..
            } => {
                assert_eq!(intent.as_deref(), Some("latency-critical"));
                assert_eq!(program, vec!["./prog".to_string(), "--flag".to_string()]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_programs_own_flags_are_not_parsed_as_ours() {
        // Everything after `--` belongs to the launched program, including
        // things that look exactly like our options.
        match parse(&args("run -- ./prog --json --quick --samples 9")).unwrap() {
            Command::Run {
                program, format, ..
            } => {
                assert_eq!(format, Format::Human, "--json after -- is the program's");
                assert!(program.contains(&"--json".to_string()));
                assert!(program.contains(&"--samples".to_string()));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn experiment_actions_parse_and_default_to_run() {
        assert!(matches!(
            parse(&args("experiment")).unwrap(),
            Command::Experiment {
                action: ExperimentAction::Run,
                ..
            }
        ));
        assert!(matches!(
            parse(&args("experiment compare")).unwrap(),
            Command::Experiment {
                action: ExperimentAction::Compare,
                ..
            }
        ));
        assert!(parse(&args("experiment frobnicate")).is_err());
    }

    #[test]
    fn scenarios_are_repeatable() {
        match parse(&args(
            "experiment run --scenario mixed --scenario thermal-drift",
        ))
        .unwrap()
        {
            Command::Experiment { scenarios, .. } => {
                assert_eq!(scenarios, vec!["mixed", "thermal-drift"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn agent_actions_parse_and_scope_is_repeatable() {
        match parse(&args(
            "agent start --dry-run --scope 42 --scope 43 --max-actions 5",
        ))
        .unwrap()
        {
            Command::Agent {
                action,
                dry_run,
                scope,
                max_actions,
                ..
            } => {
                assert_eq!(action, AgentAction::Start);
                assert!(dry_run);
                assert_eq!(scope, vec![42, 43]);
                assert_eq!(max_actions, Some(5));
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            parse(&args("agent stop")).unwrap(),
            Command::Agent {
                action: AgentAction::Stop,
                ..
            }
        ));
        assert!(parse(&args("agent destroy")).is_err());
    }

    #[test]
    fn a_scope_that_is_not_a_pid_is_rejected() {
        assert!(parse(&args("agent start --scope everything")).is_err());
    }

    #[test]
    fn the_agent_defaults_to_the_balanced_intent() {
        match parse(&args("agent start")).unwrap() {
            Command::Agent { intent, .. } => assert_eq!(intent, "balanced"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn quick_reduces_the_benchmark_and_explicit_flags_still_win() {
        match parse(&args("benchmark --quick --rounds 9")).unwrap() {
            Command::Benchmark { config, .. } => {
                assert_eq!(config.rounds, 9);
                assert!(config.samples_per_visit < RunConfig::default().samples_per_visit);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_non_numeric_number_is_rejected_with_the_option_named() {
        let error = parse(&args("benchmark --rounds many"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("--rounds"), "{error}");
        assert!(error.contains("many"), "{error}");
    }

    #[test]
    fn an_option_missing_its_value_is_rejected() {
        assert!(parse(&args("benchmark --rounds")).is_err());
    }

    #[test]
    fn demo_defaults_to_self() {
        match parse(&args("demo")).unwrap() {
            Command::Demo { what, .. } => assert_eq!(what, "self"),
            other => panic!("{other:?}"),
        }
        match parse(&args("demo self")).unwrap() {
            Command::Demo { what, .. } => assert_eq!(what, "self"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn every_command_in_the_usage_text_actually_parses() {
        // The usage text is the contract. A command documented but unparseable
        // is worse than an undocumented one.
        for command in [
            "mirror",
            "record t.jsonl",
            "info",
            "inspect",
            "replay t.jsonl",
            "observe",
            "learn",
            "model",
            "latent",
            "describe",
            "theory",
            "concepts",
            "resources",
            "benchmark",
            "analyze",
            "run -- ./p",
            "experiment run",
            "experiment compare",
            "agent start",
            "agent status",
            "agent stop",
            "demo self",
        ] {
            parse(&args(command)).unwrap_or_else(|e| panic!("{command:?}: {e}"));
        }
    }
}
