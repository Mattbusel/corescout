# Changelog

## 1.4.0 (2026-11-15)

- `corescout report` and `corescout gate` read OpenAI Codex and Gemini CLI sessions as
  well as Claude Code transcripts, each into the same claims and checks: a turn, the
  shell commands run in it with their exit codes, and the text the assistant wrote.
  - Codex: `~/.codex/sessions/**/*.jsonl` and `~/.codex/archived_sessions`
    (`$CODEX_HOME`). All three rollout formats are read: current rollouts' completed
    items (`UserMessage`, `AgentMessage`, `CommandExecution`), the 2025 to mid-2026
    `user_message`/`agent_message` events with `shell`, `shell_command`,
    `exec_command`, `local_shell_call` and code-mode `exec` tool calls, and the
    original unwrapped format. Commands you ran yourself (`!cmd`) are not counted;
    a command still running when Codex moved on has no exit code.
  - Gemini CLI: `~/.gemini/tmp/<project hash>/chats/` (`$GEMINI_CLI_HOME`), both the
    JSON-lines sessions, replayed with their rewinds, patches and re-written
    messages the way Gemini CLI loads them, and the older single-object `.json`.
    Shell commands are `run_shell_command` calls; the exit code comes from the
    `Exit Code:` trailer.
  - Cursor is not read: its transcript format is not documented.
- `report --source claude|codex|gemini|all` (default `all`) picks the agents. With more
  than one agent installed the report and the card give each agent's share and the
  report names the agent on every claim it lists; with only Claude Code the output is
  what it was. `--json` adds `sources`, `sources_searched`, `transcripts_by_source`,
  `by_source` and a `source` on each item in `top_unbacked`.
- `report --from <DIR>` tells each file's format from its contents, so one folder can
  hold transcripts from every agent.
- `gate` works as a Codex `Stop` hook and a Gemini CLI `AfterAgent` hook: it reads
  the final message from `last_assistant_message` or `prompt_response` and the
  transcript in whichever format it is; `--source` forces one. Codex setup is in
  `docs/CODEX.md`, Gemini CLI's in `docs/CLAIMS.md`.

- The release carries `CoreScout.mcpb`, the MCP bridge and the service as one Claude Desktop
  extension (Windows). `scripts/mcpb.ps1` builds it in the release workflow and refuses to pack
  unless the manifest's version and tool list match the workspace and the staged server answers
  `initialize` and `tools/list`. The bundle is listed in the official MCP Registry as
  `com.tensorust/corescout`, pinned to the release asset by SHA-256.

## 1.3.0 (2026-11-14)

- `corescout report` reads Claude Code transcripts (`~/.claude/projects/**/*.jsonl`)
  and finds the assistant turns that claimed success ("tests pass", "done", "fixed",
  "deployed", "all green", ...). It prints the share of those claims in the last
  `--days` days (30 by default) with no passing test, build or lint run in that turn
  or the one before, and the ten most recent unbacked claims with session, time,
  the claim and the last command with its exit code. `--card` prints a one-screen
  card with no paths, session ids or message text; `--json` prints everything;
  `--top <N>` lists N claims instead of ten. A `--days` larger than the history
  means "everything" rather than overflowing.
- A negation, a failure word or "when" after a claim qualifies it rather than
  taking it back, so "All 52 tests pass, 0 failed.", "All tests pass with no
  warnings." and "Fixed the crash when input is empty." are claims, and so is
  "Tests are green". An admission after the claim ("Done, but I haven't run the
  tests") still is not.
- `report` copies new and grown transcripts into CoreScout's data directory
  (`claude-transcripts/`) before reading, so the history survives Claude Code's
  30-day cleanup. `--no-store` turns that off. The first copy says how much it is
  about to copy before it starts and prints progress; a file that cannot be copied
  is skipped and read in place, and a copy is renamed into place so it never
  replaces a good one with half a file. `--from <DIR>` reads only `DIR`, in place,
  and never touches the store.
- The Privacy page lists the transcript copies with their count and size, and its
  byte total now counts subfolders.
- `corescout gate` is a Claude Code `Stop` hook: it exits 2, with a message for the
  model, when the final assistant message claims success and no check passed in that
  turn. It never blocks twice in a row and lets the stop through on anything it
  cannot read. Setup is in `docs/CLAIMS.md`.

## 1.2.0 (2026-10-04)

- `cargo install corescout-cli`, `corescout-service` and `corescout-mcp` now work: the
  1.1 notes said so, but only 14 of the library crates were on crates.io and none of
  the programs were. Every crate in the workspace is published now.
- The MCP server negotiates the protocol version: it answers a client in 2025-06-18,
  2025-03-26 or 2024-11-05, whichever the client asks for, and offers 2025-06-18
  otherwise. It used to announce 2024-11-05 while already sending `structuredContent`
  and tool annotations from the newer revisions.
- A test runs the official Rust MCP SDK (`rmcp`) as the client against the server:
  handshake, `tools/list` (every tool has a description and an object schema) and
  `tools/call` with structured content.
- `rust-version` is 1.85 (checked); the declared 1.75 no longer built.

## 1.1.0 (2026-09-25)

- First GitHub Release with downloads: the Windows installer (`CoreScoutSetup.exe`,
  `CoreScout.msi`) and command-line archives for Windows, macOS (Apple Silicon
  and Intel) and Linux, each containing `corescout`, `corescout-service` and
  `corescout-mcp`, with `SHA256SUMS.txt`.
- Published to crates.io, so `cargo install corescout-cli` works.
- CI is green again: newer Rust made the `CPUID` intrinsics safe and added two
  clippy lints, which turned warnings into build failures.

## 1.0.0

- The first complete product: service, desktop app, command line and MCP server.
