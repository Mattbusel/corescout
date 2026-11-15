# Changelog

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
