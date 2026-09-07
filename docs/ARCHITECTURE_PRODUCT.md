# The product layers

[ARCHITECTURE.md](ARCHITECTURE.md) describes the research crates and the rules
about what each may depend on. This describes the layers built on top of them
to make a product, and the rules those follow.

```text
                        ┌──────────────┐
your machine ─────────▶ │  substrate   │ the only crate that reads hardware
                        └──────┬───────┘
                        ┌──────▼───────┐
                        │    mirror    │ entities, channels, availability
                        └──────┬───────┘
        ┌──────────────────────┼──────────────────────┐
 ┌──────▼──────┐        ┌──────▼──────┐        ┌──────▼───────┐
 │  represent  │        │   science   │        │   concept    │  research
 └──────┬──────┘        └──────┬──────┘        └──────────────┘
        │                      │
 ┌──────▼──────────────────────▼──────┐  ┌─────────────────────┐
 │       agent-experience             │◀─┤ agent-observation   │  product
 └──────┬─────────────────────────────┘  └─────────────────────┘
 ┌──────▼──────────────┐  ┌──────────────┐  ┌─────────────────┐
 │ capability-runtime  │◀─┤ permissions  │  │     storage     │
 └──────┬──────────────┘  └──────────────┘  └────────┬────────┘
 ┌──────▼──────────────────────────────────────────────▼──────┐
 │                       product-api                          │
 └──────┬──────────────────────┬────────────────────┬─────────┘
 ┌──────▼──────┐        ┌──────▼──────┐      ┌──────▼──────────┐
 │   desktop   │        │     CLI     │      │   mcp-server    │
 └─────────────┘        └─────────────┘      └─────────────────┘
```

---

## The rules

**One method table.** The desktop app, the command line and a connected AI all
call `Api::call`. There is no second path with different rules, so a permission
check cannot be present on one route and missing from another, and the set of
capabilities an agent can run is exactly the set the window can run. A test
asserts that every MCP tool names a method the API actually has.

**Permissions touch nothing.** `crates/permissions` has no hardware access,
spawns no process and opens no file. It decides. Whatever carries a decision
out lives elsewhere, so a bug there can only be too strict or too permissive on
paper, and there is one enforcement point rather than several.

**Storage opens no socket.** Local-first is a property of the dependency graph,
not a promise in a document. If a future version transmits anything it will
have to be written somewhere that does not exist.

**The engine does not own the hardware.** The reflector lives in the service's
observation thread and hands finished reflections in. So the engine can be
constructed and driven entirely from recorded frames, which is what every test
and the demo do.

**One object, one lock.** The service holds a single `Engine` behind one mutex.
Unfashionable, and right for a small amount of state updated a few times a
second: it makes "what did CoreScout know when it decided that" a question with
one answer rather than a race.

---

## What each product crate is for

| crate | what it is |
|---|---|
| `storage` | A document store, an append-only event log, and a fixed-size ring for observation-rate telemetry. Three shapes because the data is three shapes. |
| `permissions` | Four autonomy modes, an authority boundary no mode can widen, rate limits, and a pause that is checked before anything else. |
| `agent-observation` | Sessions, tasks and actions as persistent entities. What a tool *reported* kept apart from what was *verified*. Secrets scrubbed where data enters. |
| `agent-experience` | Failure modes, precursor associations, and procedures under test. Uses `science` for the causal machinery rather than reimplementing it. |
| `capability-runtime` | Learned procedures as validated, bounded, reversible capabilities, and the runtime that executes them. |
| `product-api` | The engine, the method table, a loopback server, a client, and the integration setup helpers. |
| `mcp-server` | The tool table and the stdio protocol. Knows nothing about the engine; it takes a `Backend`. |

## What each program is

| | |
|---|---|
| `corescout-service` | Observes, learns, stores, and answers the local API. Two threads. |
| `corescout` | The command line. Formatting only; every subcommand is one API call. |
| `corescout-mcp` | The bridge an AI talks to. Holds no state, owns no files, starts the service if it is not there. |
| `CoreScout.exe` | The window and the tray. Keeps the service running and holds the loopback token so the web view never has to. |
| `corescout-lab` | The research command line. Every experiment the papers in `docs/` refer to. |

---

## The loop, as code

```text
corescout_before  ──▶ Engine::advise
                        picks the procedure for this operation here
                        Exploration::decide_scaled  ← the coin
                        records the assignment as pending
                      ◀── steps, or a deliberate silence

  ... the agent does the work ...

corescout_observe ──▶ Engine::agent_observe
                        Ingest::observe        scrub, fingerprint, retries
                        Experience::observe    counts, precursors
                        settles the pending trial with its outcome
                        Experience::propose    a new association becomes a test
                        Engine::promote        a settled test becomes a capability
```

`Exploration::decide_scaled` is the only place in the workspace that produces
an honest `randomised` flag, and `CausalEstimate::effect` returns an error
rather than a number when there are no randomised trials on both arms. Those
two facts are what the word *Verified* means in the interface.

## Where the words come from

Every sentence a user reads is produced by a Rust function with a test:
`Autonomy::summary`, `Basis::explain`, `FailureMode::headline`,
`Ruling::because`, `Invalid::to_string`, `Exhausted::to_string`. The interface
renders them; it does not write them. So the wording is identical in the
window, the terminal and the answer an AI receives, and there is one place to
change it.

## What is deliberately outside the workspace

`apps/corescout-desktop/src-tauri`. Tauri's dependency tree is an order of
magnitude larger than everything else here, and excluding it keeps
`cargo test --workspace` finishing in seconds — which is the difference between
gates that get run and gates that get skipped.
