//! `corescout`: the command line.
//!
//! # Not a second implementation
//!
//! Every subcommand here is one call to the same method the desktop app calls.
//! There is no logic in this binary beyond formatting, which is why a
//! permission rule cannot be enforced in the window and missing from the
//! terminal.
//!
//! # Human first, `--json` when asked
//!
//! The default output is written to be read. `--json` gives the raw answer for
//! anything that wants to parse it, and it is the same object the app receives,
//! not a summary of it.

use corescout_core::error::{Error, Result};
use corescout_product_api::Client;
use serde_json::{json, Value};

const USAGE: &str = "\
corescout - your computer, learning how to work with your AI

USAGE:
    corescout <COMMAND> [OPTIONS]

SEEING WHAT IT KNOWS
    status                  Is CoreScout running, and what is connected
    home                    The one-screen summary
    learned                 Everything it has learned
    explain <ID>            The evidence behind one of those
    failures                Operations that recur and go wrong here
    hypotheses              What it is testing and cannot yet answer
    knows                   What this computer knows, grouped
    activity [N]            What has happened recently
    computer                What this machine is
    states                  The recurring states it found in itself

CONNECTING AN AI
    ai                      Which AI systems have connected
    ai setup [NAME]         How to connect one
    ai connect <NAME>       Write the configuration for one

CAPABILITIES
    capabilities            Verified procedures
    approve <ID>            Approve one
    automate <ID>           Approve one and let your AI use it unprompted
    disable <ID>            Switch one off
    run <ID> [--live]       Run one. Without --live it decides and starts nothing

CONTROL
    mode <MODE>             observe, suggest, assist or autopilot
    pause                   Stop everything now
    resume                  Start again
    settings                The current settings and permissions
    grant --folder <PATH>   Give CoreScout access to a folder
    grant --command <NAME>  Let it run a program
    revoke --folder <PATH>  Take a folder back

PRIVACY AND HEALTH
    privacy                 Exactly what is stored, and where
    forget [--everything]   Delete what it has recorded
    diagnostics             What CoreScout costs to run

OPTIONS
    --json                  The raw answer, as the app receives it
    -h, --help              This

Everything stays on this computer. Nothing is sent anywhere.
";

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("corescout: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<()> {
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }
    let json_output = args.iter().any(|a| a == "--json");
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(String::as_str)
        .collect();
    let command = positional.first().copied().unwrap_or("status");
    let argument = positional.get(1).copied();

    let (method, params) = route(command, argument, args)?;
    let client = Client::connect()?;
    let answer = client.call(&method, params)?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&answer).unwrap_or_else(|_| answer.to_string())
        );
        return Ok(());
    }
    print!("{}", render(command, &answer));
    Ok(())
}

/// Turn a command line into a method call.
///
/// Separated from everything else so it can be tested without a running
/// service, which is most of what can go wrong in a command line tool.
fn route(command: &str, argument: Option<&str>, args: &[String]) -> Result<(String, Value)> {
    let flag = |name: &str| -> Option<String> {
        args.iter()
            .position(|a| a == name)
            .and_then(|at| args.get(at + 1))
            .cloned()
    };
    let needs = |what: &str| -> Result<String> {
        argument
            .map(str::to_string)
            .ok_or_else(|| Error::invalid(format!("{command} needs {what}")))
    };

    Ok(match command {
        "status" => ("status".into(), json!({})),
        "home" => ("home".into(), json!({})),
        "learned" => ("learned".into(), json!({})),
        "explain" => ("explain".into(), json!({ "id": needs("an id")? })),
        "failures" => ("failures".into(), json!({})),
        "hypotheses" => ("hypotheses".into(), json!({})),
        "knows" | "knowledge" => ("knowledge".into(), json!({})),
        "activity" => {
            let limit: u64 = argument.and_then(|a| a.parse().ok()).unwrap_or(20);
            (
                "activity".into(),
                json!({ "limit": limit, "technical": args.iter().any(|a| a == "--technical") }),
            )
        }
        "computer" => ("computer".into(), json!({})),
        "states" => ("states".into(), json!({})),
        "ai" => match argument {
            None => ("agents".into(), json!({})),
            Some("setup") => (
                "setup".into(),
                match args.iter().filter(|a| !a.starts_with("--")).nth(2) {
                    Some(name) => json!({ "agent": name }),
                    None => json!({}),
                },
            ),
            Some("connect") => {
                let name = args
                    .iter()
                    .filter(|a| !a.starts_with("--"))
                    .nth(2)
                    .ok_or_else(|| Error::invalid("ai connect needs a name".to_string()))?;
                ("configure".into(), json!({ "agent": name }))
            }
            Some(other) => {
                return Err(Error::invalid(format!(
                    "{other:?} is not something `ai` does; try `ai`, `ai setup`, or `ai connect`"
                )))
            }
        },
        "capabilities" => ("capabilities".into(), json!({})),
        "approve" => (
            "decide".into(),
            json!({ "id": needs("an id")?, "approved": true }),
        ),
        "automate" => (
            "decide".into(),
            json!({ "id": needs("an id")?, "approved": true, "auto_use": true }),
        ),
        "disable" => (
            "decide".into(),
            json!({ "id": needs("an id")?, "approved": true, "disabled": true }),
        ),
        "run" => (
            "run".into(),
            // A dry run unless told otherwise. Running something on someone's
            // machine because they typed four characters is the wrong default.
            json!({ "id": needs("an id")?, "dry_run": !args.iter().any(|a| a == "--live") }),
        ),
        "mode" => ("autonomy".into(), json!({ "mode": needs("a mode")? })),
        "pause" => ("pause".into(), json!({})),
        "resume" => ("resume".into(), json!({})),
        "settings" => ("settings".into(), json!({})),
        "grant" => {
            let mut params = json!({});
            if let Some(folder) = flag("--folder") {
                params["folder"] = json!(folder);
            }
            if let Some(name) = flag("--command") {
                params["command"] = json!(name);
            }
            if params.as_object().is_some_and(|map| map.is_empty()) {
                return Err(Error::invalid(
                    "grant needs --folder <PATH> or --command <NAME>".to_string(),
                ));
            }
            ("grant".into(), params)
        }
        "revoke" => {
            let folder = flag("--folder")
                .ok_or_else(|| Error::invalid("revoke needs --folder <PATH>".to_string()))?;
            ("revoke".into(), json!({ "folder": folder }))
        }
        "privacy" => ("privacy".into(), json!({})),
        "forget" => (
            "forget".into(),
            json!({ "everything": args.iter().any(|a| a == "--everything") }),
        ),
        "diagnostics" => ("diagnostics".into(), json!({})),
        other => {
            return Err(Error::invalid(format!(
                "{other:?} is not a command; run `corescout --help`"
            )))
        }
    })
}

/// Turn an answer into something worth reading in a terminal.
fn render(command: &str, answer: &Value) -> String {
    let mut out = String::new();
    match command {
        "status" => {
            out.push_str(&format!(
                "CoreScout {} is running.\n",
                text(answer, "version")
            ));
            let connected = list(answer, "connected");
            out.push_str(&match connected.len() {
                0 => "No AI connected.\n".to_string(),
                _ => format!("Connected: {}\n", connected.join(", ")),
            });
            out.push_str(&format!(
                "Mode {}{}.\n",
                text(answer, "autonomy"),
                if answer["paused"] == json!(true) {
                    ", paused"
                } else {
                    ""
                }
            ));
            out.push_str(&format!(
                "{}, {}, {} observed.\n",
                plural(
                    number(answer, "observations"),
                    "reflection this run",
                    "reflections this run"
                ),
                plural(
                    number(answer, "states"),
                    "recurring state",
                    "recurring states"
                ),
                plural(number(answer, "actions"), "AI action", "AI actions"),
            ));
        }
        "home" => {
            out.push_str(&format!("{}\n\n", text(answer, "headline")));
            if let Some(empty) = answer.get("empty").filter(|value| value.is_object()) {
                out.push_str(&format!(
                    "{}\n{}\n",
                    text(empty, "title"),
                    wrap(&text(empty, "body"))
                ));
                return out;
            }
            out.push_str(&format!("{}\n", text(&answer["machine"], "plain")));
            let today = &answer["today"];
            out.push_str(&format!(
                "\nToday: {} learned, {} capabilities, {} withdrawn.\n",
                number(today, "learned"),
                number(today, "capabilities"),
                number(today, "retired"),
            ));
            if let Some(latest) = answer.get("latest").filter(|value| value.is_object()) {
                out.push_str(&format!(
                    "\nLatest: {}\n  {}\n  {}\n",
                    text(latest, "title"),
                    text(latest, "detail"),
                    text(latest, "basis"),
                ));
            }
        }
        "learned" | "capabilities" => {
            let cards = answer.as_array().cloned().unwrap_or_default();
            if cards.is_empty() {
                out.push_str("Nothing yet. That is the honest answer, not an error.\n");
            }
            for card in cards {
                out.push_str(&format!(
                    "{}\n  {}\n  {} - confidence {:.0}%{}\n  id {}\n\n",
                    text(&card, "title"),
                    text(&card, "detail"),
                    text(&card, "basis"),
                    card["confidence"].as_f64().unwrap_or(0.0) * 100.0,
                    match card["reliability"].as_f64() {
                        Some(rate) => format!(", reliability {:.0}%", rate * 100.0),
                        None => ", never used".into(),
                    },
                    text(&card, "id"),
                ));
            }
        }
        "explain" => {
            out.push_str(&format!("{}\n\n", text(answer, "title")));
            out.push_str(&format!("{}\n\n", wrap(&text(answer, "simple"))));
            out.push_str(&format!(
                "Evidence ({}):\n{}\n",
                text(answer, "kind"),
                wrap(&text(answer, "evidence"))
            ));
            if let Some(alternative) = answer.get("alternative").and_then(Value::as_str) {
                out.push_str(&format!("\nAlternative considered: {alternative}\n"));
            }
            if let Some(result) = answer.get("result").and_then(Value::as_str) {
                out.push_str(&format!("Result: {result}\n"));
            }
        }
        "failures" => {
            let modes = answer.as_array().cloned().unwrap_or_default();
            if modes.is_empty() {
                out.push_str("No recurring failures recorded here.\n");
            }
            for mode in modes {
                out.push_str(&format!(
                    "{}  {} of {} attempts failed, {} retries\n",
                    text(&mode, "fingerprint"),
                    number(&mode, "failures"),
                    number(&mode, "attempts"),
                    number(&mode, "retries"),
                ));
            }
        }
        "hypotheses" => {
            let open = answer.as_array().cloned().unwrap_or_default();
            if open.is_empty() {
                out.push_str("CoreScout is not testing anything right now.\n");
            }
            for question in open {
                out.push_str(&format!(
                    "{}\n  {} randomised of {} trials - {}\n\n",
                    text(&question, "name"),
                    number(&question, "randomised_trials"),
                    number(&question, "observed_trials"),
                    text(&question, "missing"),
                ));
            }
        }
        "activity" => {
            for moment in answer.as_array().cloned().unwrap_or_default().iter().rev() {
                out.push_str(&format!(
                    "{}  {}\n",
                    stamp(moment["at_ms"].as_u64().unwrap_or(0)),
                    text(moment, "summary")
                ));
                if let Some(reason) = moment.get("reason").and_then(Value::as_str) {
                    out.push_str(&format!("          why: {reason}\n"));
                }
            }
        }
        "computer" => {
            out.push_str(&format!(
                "{}\n{} cores, {} threads{}\n",
                text(answer, "cpu"),
                number(answer, "cores"),
                number(answer, "threads"),
                if answer["hybrid"] == json!(true) {
                    ", mixed core types"
                } else {
                    ""
                }
            ));
            out.push_str(&format!(
                "{} entities x {} channels, {} observed, {} ns per pass\n",
                number(answer, "entities"),
                number(answer, "channels"),
                number(answer, "observed_cells"),
                number(answer, "observe_ns"),
            ));
            for line in list(answer, "self_description") {
                out.push_str(&format!("\n  {line}"));
            }
            out.push('\n');
        }
        "states" => {
            for state in answer.as_array().cloned().unwrap_or_default() {
                out.push_str(&format!(
                    "state {:<4} {:>6} entries  {}\n",
                    number(&state, "id"),
                    number(&state, "entries"),
                    text(&state, "plain"),
                ));
            }
        }
        "ai" => {
            let agents = answer.as_array().cloned().unwrap_or_default();
            if agents.is_empty() {
                out.push_str(
                    "No AI has connected yet. `corescout ai setup` shows how to connect one.\n",
                );
            }
            for agent in agents {
                out.push_str(&format!("{}\n", text(&agent, "summary")));
            }
        }
        "privacy" => {
            out.push_str(&format!(
                "Everything is in {}\n{} bytes. Telemetry: {}.\n\n",
                text(answer, "data_dir"),
                number(answer, "bytes"),
                if answer["telemetry"] == json!(true) {
                    "on"
                } else {
                    "none, ever"
                }
            ));
            for holding in answer["holdings"].as_array().cloned().unwrap_or_default() {
                if holding["count"].as_u64().unwrap_or(0) == 0 {
                    continue;
                }
                out.push_str(&format!(
                    "{:>6}  {}\n",
                    number(&holding, "count"),
                    text(&holding, "describes")
                ));
            }
        }
        "diagnostics" => {
            out.push_str(&format!(
                "Observation costs {} ns on average, {} at worst.\nThat is {:.4}% of one core at \
                 the current interval of {} ms.\n{} samples in a {} byte ring{}.\n{} documents, {} \
                 events, schema {}.\n",
                number(answer, "mean_observe_ns"),
                number(answer, "worst_observe_ns"),
                answer["cpu_share"].as_f64().unwrap_or(0.0) * 100.0,
                number(answer, "interval_ms"),
                number(answer, "ring_samples"),
                number(answer, "ring_bytes"),
                if answer["ring_wrapped"] == json!(true) {
                    " (wrapped; the oldest history has been overwritten)"
                } else {
                    ""
                },
                number(answer, "documents"),
                number(answer, "events"),
                number(answer, "schema"),
            ));
            for gap in list(answer, "gaps") {
                out.push_str(&format!("  {gap}\n"));
            }
        }
        _ => {
            out.push_str(&format!(
                "{}\n",
                serde_json::to_string_pretty(answer).unwrap_or_else(|_| answer.to_string())
            ));
        }
    }
    out
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// A count with its noun, pluralised.
///
/// "1 AI actions observed" is the kind of thing that makes a product feel like
/// a debug build.
fn plural(count: u64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn list(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Wrap prose at a width a terminal will not fold badly.
fn wrap(text: &str) -> String {
    let mut out = String::new();
    let mut column = 0;
    for word in text.split_whitespace() {
        if column + word.len() > 76 && column > 0 {
            out.push('\n');
            column = 0;
        } else if column > 0 {
            out.push(' ');
            column += 1;
        }
        out.push_str(word);
        column += word.len();
    }
    out
}

/// A wall clock time, without pulling in a date library for one line.
fn stamp(ms: u64) -> String {
    let total = ms / 1000;
    let hours = (total / 3600) % 24;
    let minutes = (total / 60) % 60;
    let seconds = total % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    fn routed(line: &str) -> (String, Value) {
        let args = args(line);
        let positional: Vec<&str> = args
            .iter()
            .filter(|a| !a.starts_with("--"))
            .map(String::as_str)
            .collect();
        route(positional[0], positional.get(1).copied(), &args).expect("a route")
    }

    #[test]
    fn every_command_in_the_help_can_be_routed() {
        // A command documented and not routed is one someone will type.
        for line in USAGE.lines() {
            let trimmed = line.trim_start();
            if !line.starts_with("    ") || trimmed.starts_with('-') || trimmed.is_empty() {
                continue;
            }
            let name = trimmed.split_whitespace().next().unwrap_or_default();
            if name.is_empty() || name.starts_with('<') || name == "corescout" {
                continue;
            }
            let attempted = route(name, Some("x"), &args(&format!("{name} x --folder p")));
            if let Err(error) = attempted {
                assert!(
                    !error.to_string().contains("is not a command"),
                    "{name} appears in the help and is not routed"
                );
            }
        }
    }

    #[test]
    fn running_a_capability_is_a_dry_run_unless_asked_otherwise() {
        // Running something on someone's machine because they typed four
        // characters is the wrong default.
        let (method, params) = routed("run cap-1");
        assert_eq!(method, "run");
        assert_eq!(params["dry_run"], true);
        assert_eq!(routed("run cap-1 --live").1["dry_run"], false);
    }

    #[test]
    fn approve_automate_and_disable_are_three_different_decisions() {
        assert_eq!(routed("approve c").1, json!({"id":"c","approved":true}));
        assert_eq!(
            routed("automate c").1,
            json!({"id":"c","approved":true,"auto_use":true})
        );
        assert_eq!(
            routed("disable c").1,
            json!({"id":"c","approved":true,"disabled":true})
        );
    }

    #[test]
    fn a_command_that_needs_an_id_says_so_rather_than_sending_an_empty_one() {
        let error = route("explain", None, &args("explain"))
            .expect_err("should fail")
            .to_string();
        assert!(error.contains("needs an id"), "{error}");
    }

    #[test]
    fn grant_needs_something_to_grant() {
        let error = route("grant", None, &args("grant"))
            .expect_err("should fail")
            .to_string();
        assert!(error.contains("--folder"), "{error}");
    }

    #[test]
    fn grant_takes_a_folder_a_command_or_both() {
        assert_eq!(
            routed("grant --folder C:\\Projects\\app").1["folder"],
            "C:\\Projects\\app"
        );
        assert_eq!(routed("grant --command cargo").1["command"], "cargo");
    }

    #[test]
    fn an_unknown_command_points_at_the_help() {
        let error = route("frobnicate", None, &args("frobnicate"))
            .expect_err("should fail")
            .to_string();
        assert!(error.contains("--help"), "{error}");
    }

    #[test]
    fn the_ai_subcommands_route_separately() {
        assert_eq!(routed("ai").0, "agents");
        assert_eq!(routed("ai setup").0, "setup");
        let (method, params) = routed("ai connect claude-code");
        assert_eq!(method, "configure");
        assert_eq!(params["agent"], "claude-code");
    }

    #[test]
    fn an_unknown_ai_subcommand_lists_the_real_ones() {
        let error = route("ai", Some("frobnicate"), &args("ai frobnicate"))
            .expect_err("should fail")
            .to_string();
        assert!(error.contains("ai setup"), "{error}");
    }

    #[test]
    fn forget_deletes_only_ai_history_unless_told_everything() {
        assert_eq!(routed("forget").1["everything"], false);
        assert_eq!(routed("forget --everything").1["everything"], true);
    }

    #[test]
    fn a_fresh_install_renders_as_words_rather_than_an_empty_screen() {
        let home = json!({
            "headline": "Your computer is watching how it runs.",
            "connected": [],
            "today": {"learned":0,"capabilities":0,"retired":0,"rejected":0},
            "machine": {"plain":"Still working out what normal looks like.","seen":0,"unfamiliar":false},
            "empty": {"title":"CoreScout hasn't learned anything yet.","body":"That's normal."}
        });
        let rendered = render("home", &home);
        assert!(rendered.contains("hasn't learned anything yet"));
        assert!(rendered.contains("That's normal"));
    }

    #[test]
    fn a_card_that_has_never_been_used_says_so_rather_than_showing_zero_percent() {
        let cards = json!([{
            "id":"c","title":"t","detail":"d","basis":"Seen together",
            "causal":false,"confidence":0.4,"uses":0,"last_ms":0,
            "usable_by_ai":false,"is_capability":false,"approved":false
        }]);
        let rendered = render("learned", &cards);
        assert!(rendered.contains("never used"), "{rendered}");
        assert!(!rendered.contains("reliability 0%"), "{rendered}");
    }

    #[test]
    fn an_empty_learned_list_says_that_is_the_honest_answer() {
        let rendered = render("learned", &json!([]));
        assert!(rendered.contains("honest answer"), "{rendered}");
    }

    #[test]
    fn prose_is_wrapped_and_nothing_is_lost() {
        let long = "word ".repeat(60);
        let wrapped = wrap(&long);
        assert!(wrapped.lines().all(|line| line.len() <= 80));
        assert_eq!(wrapped.split_whitespace().count(), 60);
    }

    #[test]
    fn a_timestamp_is_rendered_without_a_date_library() {
        assert_eq!(stamp(0), "00:00:00");
        assert_eq!(stamp(3_661_000), "01:01:01");
    }

    #[test]
    fn missing_fields_render_as_nothing_rather_than_panicking() {
        // Answers cross a version boundary: an older CLI may be talking to a
        // newer service. Missing keys must not be fatal.
        for command in ["status", "home", "computer", "privacy", "diagnostics"] {
            let _ = render(command, &json!({}));
            let _ = render(command, &json!([]));
            let _ = render(command, &Value::Null);
        }
    }
}
