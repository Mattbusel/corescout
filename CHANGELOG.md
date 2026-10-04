# Changelog

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
