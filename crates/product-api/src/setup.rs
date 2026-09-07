//! Connecting an AI, without the user editing JSON by hand if it can be helped.
//!
//! # The generic path is the real path
//!
//! CoreScout speaks the Model Context Protocol over stdio. Any client that
//! does too can use it, and the per-client entries below are conveniences that
//! know where each one keeps its configuration file. A client nobody here has
//! heard of is not a second-class case: it gets the same snippet and the same
//! server.
//!
//! # Automatic configuration is offered only where it is safe
//!
//! Writing into someone's editor configuration is not a thing to do casually.
//! It happens only when asked, only into the one key CoreScout owns, and only
//! after the existing file has been parsed successfully. A file that cannot be
//! parsed is left exactly as it was and the user is given the snippet.

use std::path::PathBuf;

use corescout_agent_observation::AgentKind;
use corescout_core::error::{Error, Result};

use crate::view::Setup;

/// The name CoreScout registers itself under.
pub const SERVER_NAME: &str = "corescout";

/// The MCP bridge executable, as it should appear in a configuration file.
pub fn bridge_command() -> String {
    beside("corescout-mcp")
}

/// The command line executable, which is what a hook invokes.
pub fn cli_command() -> String {
    beside("corescout")
}

/// A program installed next to this one.
///
/// Next to, rather than found on the path: a machine with two CoreScout builds
/// on it would otherwise have an agent talking to whichever one the path
/// happened to name, which is the most confusing failure available here.
fn beside(name: &str) -> String {
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&file)))
        .filter(|path| path.exists())
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| name.to_string())
}

/// Where an agent keeps its hook configuration, where CoreScout knows.
pub fn hooks_path(kind: &AgentKind) -> Option<PathBuf> {
    match kind {
        AgentKind::ClaudeCode => Some(home().join(".claude").join("settings.json")),
        _ => None,
    }
}

/// The hooks block for a client, as JSON to paste or write.
///
/// This is what turns CoreScout from something an AI reports to when it thinks
/// of it into something that sees the work. It matches the tools that do
/// something and deliberately not the ones that read.
pub fn hooks_snippet(kind: &AgentKind) -> Option<String> {
    hooks_path(kind)?;
    let command = format!("\"{}\" hook", cli_command());
    let block = serde_json::json!({
        "hooks": {
            "PostToolUse": [{
                "matcher": "Bash|Edit|Write|MultiEdit|NotebookEdit|Task",
                "hooks": [{ "type": "command", "command": command, "timeout": 5 }]
            }]
        }
    });
    serde_json::to_string_pretty(&block).ok().map(|mut text| {
        text.push('\n');
        text
    })
}

/// Write the hooks block into a client's settings, keeping everything else.
///
/// Hooks are somebody's agent configuration. A file that cannot be parsed is
/// left exactly as it was, and CoreScout's own entry is replaced rather than
/// appended, so running this twice does not produce two of them.
pub fn apply_hooks(kind: &AgentKind) -> Result<PathBuf> {
    let path = hooks_path(kind).ok_or_else(|| {
        Error::invalid(format!(
            "CoreScout does not know where {} keeps its hooks. The setup screen has the block to \
             paste in.",
            kind.title()
        ))
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;
    }
    let text = merge_hooks(&path, &cli_command())?;
    std::fs::write(&path, text).map_err(|source| Error::io(&path, source))?;
    Ok(path)
}

/// Put CoreScout's hook into a settings file, keeping everything else.
///
/// Separated from the writing so it can be tested without a home directory,
/// which is exactly the sort of thing that otherwise gets checked by hand once
/// and never again.
fn merge_hooks(path: &std::path::Path, executable: &str) -> Result<String> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut root: serde_json::Value = if existing.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&existing).map_err(|error| {
            Error::invalid(format!(
                "{} could not be read as JSON ({error}), so CoreScout left it alone. Paste the \
                 block in yourself and nothing will be lost.",
                path.display()
            ))
        })?
    };

    let command = format!("\"{executable}\" hook");
    let entry = serde_json::json!({
        "matcher": "Bash|Edit|Write|MultiEdit|NotebookEdit|Task",
        "hooks": [{ "type": "command", "command": command, "timeout": 5 }]
    });

    let hooks = root
        .as_object_mut()
        .ok_or_else(|| Error::invalid(format!("{} is not a JSON object", path.display())))?
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    let post = hooks
        .as_object_mut()
        .ok_or_else(|| {
            Error::invalid(format!(
                "{} has a hooks that is not an object",
                path.display()
            ))
        })?
        .entry("PostToolUse")
        .or_insert_with(|| serde_json::json!([]));
    let list = post.as_array_mut().ok_or_else(|| {
        Error::invalid(format!(
            "{} has a PostToolUse that is not a list",
            path.display()
        ))
    })?;
    // Replace CoreScout's own entry rather than adding a second one.
    list.retain(|item| !mentions_corescout(item));
    list.push(entry);

    serde_json::to_string_pretty(&root)
        .map(|mut text| {
            text.push('\n');
            text
        })
        .map_err(|error| Error::invalid(format!("could not write the hooks: {error}")))
}

/// Whether a hook entry is one CoreScout put there.
fn mentions_corescout(entry: &serde_json::Value) -> bool {
    entry
        .get("hooks")
        .and_then(serde_json::Value::as_array)
        .map(|hooks| {
            hooks.iter().any(|hook| {
                hook.get("command")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|command| command.to_lowercase().contains("corescout"))
            })
        })
        .unwrap_or(false)
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// The configuration file a client reads, where CoreScout knows it.
pub fn config_path(kind: &AgentKind) -> Option<PathBuf> {
    match kind {
        AgentKind::Cursor => Some(home().join(".cursor").join("mcp.json")),
        AgentKind::Codex => Some(home().join(".codex").join("config.toml")),
        AgentKind::OpenCode => Some(
            home()
                .join(".config")
                .join("opencode")
                .join("opencode.json"),
        ),
        // Claude Code stores servers in its own configuration, and ships a
        // command that edits it correctly. Writing that file directly would be
        // reaching into a format CoreScout does not own.
        AgentKind::ClaudeCode => None,
        _ => None,
    }
}

/// The snippet for one client.
pub fn snippet(kind: &AgentKind) -> String {
    let command = bridge_command();
    let escaped = command.replace('\\', "\\\\");
    match kind {
        AgentKind::Codex => {
            format!("[mcp_servers.{SERVER_NAME}]\ncommand = \"{escaped}\"\nargs = []\n")
        }
        AgentKind::OpenCode => format!(
            "{{\n  \"mcp\": {{\n    \"{SERVER_NAME}\": {{\n      \"type\": \"local\",\n      \
             \"command\": [\"{escaped}\"],\n      \"enabled\": true\n    }}\n  }}\n}}\n"
        ),
        _ => format!(
            "{{\n  \"mcpServers\": {{\n    \"{SERVER_NAME}\": {{\n      \"command\": \
             \"{escaped}\",\n      \"args\": []\n    }}\n  }}\n}}\n"
        ),
    }
}

/// Everything the AI screen needs to show for one client.
pub fn setup(kind: &AgentKind) -> Setup {
    let command = bridge_command();
    let path = config_path(kind);
    let (automatic, instructions, shell) = match kind {
        AgentKind::ClaudeCode => (
            false,
            vec![
                "Run this once, in any project folder:".to_string(),
                format!("claude mcp add {SERVER_NAME} --scope user -- \"{command}\""),
                "Then start Claude Code as usual. It will see CoreScout's tools.".to_string(),
            ],
            Some(format!(
                "claude mcp add {SERVER_NAME} --scope user -- \"{command}\""
            )),
        ),
        AgentKind::Codex => (
            true,
            vec![
                "Add this block to your Codex configuration.".to_string(),
                "CoreScout can append it for you.".to_string(),
            ],
            None,
        ),
        AgentKind::Cursor | AgentKind::OpenCode => (
            true,
            vec![
                "Add CoreScout as an MCP server.".to_string(),
                "CoreScout can write this for you.".to_string(),
            ],
            None,
        ),
        _ => (
            false,
            vec![
                "Point your AI at CoreScout's MCP server.".to_string(),
                format!("It runs over stdio: {command}"),
                "Any MCP-compatible client will work.".to_string(),
            ],
            None,
        ),
    };
    let hooks = hooks_snippet(kind);
    let mut instructions = instructions;
    if hooks.is_some() {
        instructions.push(
            "Then let CoreScout watch the work itself, so it does not depend on your AI \
             remembering to mention it."
                .to_string(),
        );
    }
    Setup {
        agent: kind.slug(),
        title: kind.title(),
        automatic: automatic && path.is_some(),
        config_path: path.map(|p| p.display().to_string()),
        snippet: snippet(kind),
        command: shell,
        instructions,
        hook_snippet: hooks,
        hook_path: hooks_path(kind).map(|p| p.display().to_string()),
    }
}

/// Every client with a setup flow, plus the generic one.
pub fn all() -> Vec<Setup> {
    AgentKind::with_setup_flow()
        .iter()
        .chain(std::iter::once(&AgentKind::Mcp))
        .map(setup)
        .collect()
}

/// Write CoreScout into a client's configuration.
///
/// Returns the path written. Refuses rather than guessing when the existing
/// file cannot be parsed, because a configuration file rewritten from a failed
/// parse is somebody's editor setup destroyed.
pub fn apply(kind: &AgentKind) -> Result<PathBuf> {
    let Some(path) = config_path(kind) else {
        return Err(Error::invalid(format!(
            "CoreScout cannot configure {} automatically; the setup screen has the exact command \
             to run",
            kind.title()
        )));
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    let updated = match kind {
        AgentKind::Codex => append_toml(&existing, &snippet(kind)),
        AgentKind::OpenCode => merge_json(&existing, "mcp", &path, opencode_entry())?,
        _ => merge_json(&existing, "mcpServers", &path, standard_entry())?,
    };
    std::fs::write(&path, updated).map_err(|source| Error::io(&path, source))?;
    Ok(path)
}

fn standard_entry() -> serde_json::Value {
    serde_json::json!({ "command": bridge_command(), "args": [] })
}

fn opencode_entry() -> serde_json::Value {
    serde_json::json!({
        "type": "local",
        "command": [bridge_command()],
        "enabled": true
    })
}

/// Put one entry under one key of a JSON configuration file.
fn merge_json(
    existing: &str,
    key: &str,
    path: &std::path::Path,
    entry: serde_json::Value,
) -> Result<String> {
    let mut root: serde_json::Value = if existing.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(existing).map_err(|error| {
            Error::invalid(format!(
                "{} could not be read as JSON ({error}), so CoreScout left it alone. Paste the \
                 snippet in yourself and nothing will be lost.",
                path.display()
            ))
        })?
    };
    if !root.is_object() {
        return Err(Error::invalid(format!(
            "{} is not a JSON object, so CoreScout left it alone",
            path.display()
        )));
    }
    let servers = root
        .as_object_mut()
        .expect("checked above")
        .entry(key)
        .or_insert_with(|| serde_json::json!({}));
    if !servers.is_object() {
        return Err(Error::invalid(format!(
            "{} has a {key} that is not an object, so CoreScout left it alone",
            path.display()
        )));
    }
    servers
        .as_object_mut()
        .expect("checked above")
        .insert(SERVER_NAME.into(), entry);
    serde_json::to_string_pretty(&root)
        .map(|mut text| {
            text.push('\n');
            text
        })
        .map_err(|error| Error::invalid(format!("could not write the configuration: {error}")))
}

/// Append a block to a configuration file, unless it is already there.
///
/// Appending rather than parsing: rewriting a file in a format CoreScout does
/// not fully model risks reordering or dropping something. Appending a table
/// that is not already present is the smallest correct change.
fn append_toml(existing: &str, block: &str) -> String {
    if existing.contains(&format!("[mcp_servers.{SERVER_NAME}]")) {
        return existing.to_string();
    }
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(block);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_client_gets_a_snippet_and_instructions() {
        for setup in all() {
            assert!(!setup.snippet.is_empty(), "{} has no snippet", setup.agent);
            assert!(
                !setup.instructions.is_empty(),
                "{} has no instructions",
                setup.agent
            );
            assert!(setup.snippet.contains(SERVER_NAME));
        }
    }

    #[test]
    fn a_client_corescout_has_never_heard_of_still_gets_the_generic_path() {
        // The generic path is the real path. A client with no entry here gets
        // a working snippet, not an apology.
        let unknown = AgentKind::Named("something-new".into());
        let setup = setup(&unknown);
        assert!(setup.snippet.contains("mcpServers"));
        assert!(!setup.automatic, "nothing is written for an unknown client");
    }

    #[test]
    fn claude_code_is_configured_by_its_own_command_rather_than_by_editing_its_file() {
        let setup = setup(&AgentKind::ClaudeCode);
        assert!(!setup.automatic);
        assert!(setup
            .command
            .as_deref()
            .unwrap_or("")
            .contains("claude mcp add"));
        assert!(apply(&AgentKind::ClaudeCode).is_err());
    }

    #[test]
    fn backslashes_in_a_windows_path_survive_into_json() {
        // A path pasted into JSON with single backslashes is invalid JSON, and
        // the user finds out when their editor stops starting.
        let text = snippet(&AgentKind::Cursor);
        let parsed: serde_json::Value =
            serde_json::from_str(&text).expect("the snippet must be valid JSON");
        assert!(parsed["mcpServers"][SERVER_NAME]["command"].is_string());
    }

    #[test]
    fn the_opencode_snippet_is_valid_json_too() {
        let text = snippet(&AgentKind::OpenCode);
        let parsed: serde_json::Value =
            serde_json::from_str(&text).expect("the snippet must be valid JSON");
        assert_eq!(parsed["mcp"][SERVER_NAME]["type"], "local");
    }

    #[test]
    fn merging_keeps_every_other_server_a_user_had() {
        // The failure that would make people uninstall this: CoreScout adds
        // itself and their other tools stop working.
        let existing = r#"{"mcpServers":{"theirs":{"command":"x"}},"other":42}"#;
        let merged = merge_json(
            existing,
            "mcpServers",
            std::path::Path::new("x.json"),
            standard_entry(),
        )
        .expect("merge");
        let parsed: serde_json::Value = serde_json::from_str(&merged).expect("valid");
        assert_eq!(parsed["mcpServers"]["theirs"]["command"], "x");
        assert_eq!(parsed["other"], 42);
        assert!(parsed["mcpServers"][SERVER_NAME].is_object());
    }

    #[test]
    fn merging_into_an_empty_file_creates_the_structure() {
        let merged = merge_json(
            "",
            "mcpServers",
            std::path::Path::new("x.json"),
            standard_entry(),
        )
        .expect("merge");
        let parsed: serde_json::Value = serde_json::from_str(&merged).expect("valid");
        assert!(parsed["mcpServers"][SERVER_NAME].is_object());
    }

    #[test]
    fn a_file_that_cannot_be_parsed_is_left_exactly_as_it_was() {
        // Rewriting from a failed parse is somebody's editor configuration
        // destroyed, so this refuses and says what to do instead.
        let error = merge_json(
            "{ this is not json",
            "mcpServers",
            std::path::Path::new("x.json"),
            standard_entry(),
        )
        .expect_err("should refuse")
        .to_string();
        assert!(error.contains("left it alone"), "{error}");
        assert!(error.contains("nothing will be lost"), "{error}");
    }

    #[test]
    fn merging_twice_does_not_produce_two_entries() {
        let once = merge_json(
            "",
            "mcpServers",
            std::path::Path::new("x.json"),
            standard_entry(),
        )
        .expect("merge");
        let twice = merge_json(
            &once,
            "mcpServers",
            std::path::Path::new("x.json"),
            standard_entry(),
        )
        .expect("merge");
        let parsed: serde_json::Value = serde_json::from_str(&twice).expect("valid");
        assert_eq!(
            parsed["mcpServers"].as_object().expect("an object").len(),
            1
        );
    }

    #[test]
    fn appending_a_toml_block_leaves_what_was_there() {
        let existing = "model = \"gpt-5\"\n\n[mcp_servers.other]\ncommand = \"x\"\n";
        let updated = append_toml(existing, &snippet(&AgentKind::Codex));
        assert!(updated.starts_with(existing));
        assert!(updated.contains("[mcp_servers.corescout]"));
    }

    #[test]
    fn claude_code_gets_a_hooks_block_and_the_others_do_not_yet() {
        // Hooks are what make CoreScout see the work rather than wait to be
        // told about it. Where CoreScout does not know a client's format, it
        // offers nothing rather than guessing at one.
        let claude = setup(&AgentKind::ClaudeCode);
        let hooks = claude.hook_snippet.expect("a hooks block");
        let parsed: serde_json::Value =
            serde_json::from_str(&hooks).expect("the block must be valid JSON");
        assert!(parsed["hooks"]["PostToolUse"][0]["matcher"]
            .as_str()
            .expect("a matcher")
            .contains("Bash"));
        assert!(claude.hook_path.is_some());

        assert!(setup(&AgentKind::Mcp).hook_snippet.is_none());
        assert!(setup(&AgentKind::Named("something-new".into()))
            .hook_snippet
            .is_none());
    }

    #[test]
    fn writing_hooks_keeps_every_other_hook_the_user_had() {
        // The failure that would make somebody uninstall this: CoreScout adds
        // its hook and their own stop running.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            "{\"model\":\"opus\",\"hooks\":{\"PostToolUse\":[{\"matcher\":\"Bash\",\"hooks\":[{\"type\":\"command\",\"command\":\"my-own-thing\"}]}]}}",
        )
        .expect("write");

        let updated = merge_hooks(&path, "corescout.exe").expect("merge");
        let parsed: serde_json::Value = serde_json::from_str(&updated).expect("valid");
        assert_eq!(parsed["model"], "opus", "unrelated settings survive");
        let list = parsed["hooks"]["PostToolUse"].as_array().expect("a list");
        assert_eq!(list.len(), 2, "theirs and ours: {updated}");
        assert_eq!(list[0]["hooks"][0]["command"], "my-own-thing");
    }

    #[test]
    fn writing_hooks_twice_does_not_produce_two_of_them() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{}").expect("write");
        let once = merge_hooks(&path, "corescout.exe").expect("merge");
        std::fs::write(&path, &once).expect("write");
        let twice = merge_hooks(&path, "corescout.exe").expect("merge");
        let parsed: serde_json::Value = serde_json::from_str(&twice).expect("valid");
        assert_eq!(
            parsed["hooks"]["PostToolUse"]
                .as_array()
                .expect("a list")
                .len(),
            1
        );
    }

    #[test]
    fn a_settings_file_that_cannot_be_parsed_is_left_exactly_as_it_was() {
        // Somebody's agent configuration rebuilt from a failed parse is
        // somebody's agent configuration destroyed.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not json").expect("write");
        let error = merge_hooks(&path, "corescout.exe")
            .expect_err("should refuse")
            .to_string();
        assert!(error.contains("left it alone"), "{error}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "{ not json",
            "the file must be untouched"
        );
    }

    #[test]
    fn a_windows_path_in_a_hook_survives_into_valid_json() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{}").expect("write");
        let updated =
            merge_hooks(&path, "C:\\Program Files\\CoreScout\\corescout.exe").expect("merge");
        let parsed: serde_json::Value = serde_json::from_str(&updated).expect("valid JSON");
        let command = parsed["hooks"]["PostToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .expect("a command");
        assert!(command.contains("Program Files"), "{command}");
        assert!(command.ends_with(" hook"), "{command}");
    }

    #[test]
    fn appending_a_toml_block_twice_does_nothing_the_second_time() {
        let once = append_toml("", &snippet(&AgentKind::Codex));
        let twice = append_toml(&once, &snippet(&AgentKind::Codex));
        assert_eq!(once, twice);
    }
}
