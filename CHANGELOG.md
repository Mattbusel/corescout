# Changelog

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
