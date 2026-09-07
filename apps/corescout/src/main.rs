//! `corescout`: a machine that observes itself, models itself, and acts on
//! itself.
//!
//! # The layering, as commands
//!
//! ```text
//! mirror -> memory -> representation -> self-model -> counterfactual
//!        -> intent -> choice -> action -> reality -> mirror
//! ```
//!
//! Each command enters that cycle at one point and does not reach past it.
//! `mirror` only observes. `learn` only consumes. `agent` closes the loop.
//! The separation is enforced by which crates each one may link, and the
//! standalone binaries in `src/bin` make the strongest version of that claim:
//! `mirror-observer`, `mirror-learn` and `mirror-replay` do not link the
//! hardware-facing crate at all, so their inability to peek is a fact about the
//! executable rather than a promise in a comment.

mod cli;
mod commands;
mod objectives;
mod runtime;

use std::process::ExitCode;
use std::time::Duration;

use cli::{AgentAction, Command, ExperimentAction};
use corescout_human::Format;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match cli::parse(&args) {
        Ok(command) => command,
        Err(error) => {
            runtime::report_error(&error);
            return ExitCode::FAILURE;
        }
    };

    match dispatch(command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            runtime::report_error(&error);
            ExitCode::FAILURE
        }
    }
}

fn dispatch(command: Command) -> corescout_core::error::Result<()> {
    match command {
        Command::Help => {
            print!("{}", cli::USAGE);
            Ok(())
        }
        Command::Version => {
            println!("corescout {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }

        Command::Mirror {
            format,
            once,
            interval_ms,
            ticks,
            plane,
            publish,
            record,
            filter,
        } => commands::observe::mirror(commands::observe::MirrorRun {
            once,
            interval: Duration::from_millis(interval_ms.max(1)),
            ticks,
            plane,
            publish,
            record,
            filter,
            format,
        }),

        Command::Record {
            path,
            interval_ms,
            ticks,
            plane,
            publish,
            format,
        } => commands::observe::mirror(commands::observe::MirrorRun {
            once: false,
            interval: Duration::from_millis(interval_ms.max(1)),
            ticks,
            plane,
            publish,
            record: Some(path),
            filter: None,
            format,
        }),

        Command::Info { format } => commands::observe::info(format),

        Command::Inspect {
            source,
            format,
            filter,
        } => commands::consume::inspect(&source, format, filter.as_deref()),
        Command::Replay { source, format } => commands::consume::replay(&source, format),
        Command::Observe {
            source,
            ontology,
            format,
        } => commands::consume::observe(&source, ontology, format),
        Command::Learn { source, format } => commands::consume::learn(&source, format),
        Command::Model { source, format } => commands::consume::model(&source, format),
        Command::Latent { source, format } => commands::consume::latent(&source, format),
        Command::Describe { source, format } => commands::consume::describe(&source, format),
        Command::Theory { source, format } => commands::science::theory(&source, format),
        Command::Concepts { source, format } => commands::science::concepts(&source, format),
        Command::Resources { source, format } => commands::science::resources(&source, format),

        Command::Benchmark { config, format } => commands::perturb::benchmark(config, format),
        Command::Analyze {
            config,
            format,
            use_cache,
            save,
        } => commands::perturb::analyze(config, format, use_cache, save),
        Command::Run {
            profile,
            intent,
            program,
            config,
            format,
            use_cache,
        } => commands::perturb::run(
            &program,
            &profile,
            intent.as_deref(),
            config,
            use_cache,
            format,
        ),
        Command::Experiment {
            action,
            scenarios,
            repetitions,
            against,
            format,
        } => commands::perturb::experiment(
            action == ExperimentAction::Compare,
            &scenarios,
            repetitions,
            &against,
            format,
        ),

        Command::Agent {
            action,
            intent,
            dry_run,
            scope,
            max_actions,
            source,
            format,
        } => match action {
            AgentAction::Start => commands::agent::start(commands::agent::AgentRun {
                intent,
                dry_run,
                scope,
                max_actions,
                source,
                format,
            }),
            AgentAction::Status => commands::agent::status(&source, format),
            AgentAction::Stop => commands::agent::stop(format),
        },

        Command::Demo { what, format } => commands::demo::run(&what, format),
    }
}

/// Assert at compile time that the format enum reaches every printer.
///
/// Cheap, but it has caught a command that silently ignored `--json`.
const _: fn() = || {
    let _ = Format::Human;
    let _ = Format::Json;
};
