# Screenshots

Seven, in `out/`, at 2732x1536 (1366x768 at 2x). Above the Store minimum and
they downsample cleanly.

## Reproducing them

```
powershell -File microsoft-store\screenshots\capture.ps1
```

Builds the service and the front end, starts the service against a throwaway
data directory, watches this machine for two and a half minutes, feeds it a
week of AI work, and photographs the running application. The throwaway
directory means this never touches the CoreScout you are actually using.

## What is real and what is staged

This matters more than it usually would, because the whole product is an
argument about the difference between something observed and something
asserted. A listing whose screenshots do not match the product is also a
certification failure.

**Real, measured on the machine that ran the capture:**

- The processor, its core and thread counts, and the entities in the mirror.
- The recurring states, and how many times each has been seen. The Home screen
  saying "it has been here 7 times" is seven actual recurrences of a state that
  clustering found rather than something anyone configured.
- How long CoreScout has been learning, and the observation cost.
- Every number on the Plan screen's ledger.

**Staged:** the history of AI work. A week of an agent running `npm run
codegen`, `npm run build`, `npm test` and a deploy, with stated outcomes,
described in full in `seed.mjs`. That work did not happen.

**Real, computed from the staged history by the shipping engine:** everything
CoreScout then says about it. The correlation on the Home screen, the failure
rates in it, the failure mode, and the refusal on the Learned screen to call
any of it a cause. Those were produced by the same code a customer runs, under
the same evidence rules.

So: a screenshot showing "94% reliable" because somebody typed 94 into a design
file would be a lie. A screenshot showing what CoreScout concluded from a
stated history is a demonstration. This is the second kind, and `LISTING.md`
says so.

## Why the history is posted to the API rather than through the hook

In production, timing comes from the clock, because that is when the tool call
happened. A week of history cannot be replayed in real time, and squeezed into
one second every command appears to precede every other one. The API accepts an
explicit timestamp, so the history can be laid out over days. Same engine, same
ingestion, same evidence rules.

## Known problem with the current set

They were taken from an unpackaged build, so two of them show developer paths:

- `07-privacy.png` names a temporary data folder, with a username in it.
- `04-ai.png` shows a full path to `corescout-mcp.exe` in a build directory.

A packaged build shows the package's own data folder and the bare execution
aliases (`corescout-mcp`), which is what a buyer actually gets. These should be
retaken once the package can be installed. `STATUS.md` tracks it.

## The shots

| File | Screen | Theme | Caption for the listing |
|---|---|---|---|
| `01-home.png` | Home | light | Your computer is learning how to work with your AI. |
| `02-learned.png` | Learned | light | Everything it knows, labelled with how it knows it. |
| `03-computer.png` | Computer | dark | The machine, as CoreScout sees it. |
| `04-ai.png` | AI | light | Connect Claude Code, Codex or Cursor in one line. |
| `05-activity.png` | Activity | dark | Everything CoreScout has done, and why. |
| `06-plan.png` | Plan | light | What this machine worked out, and what Pro keeps doing. |
| `07-privacy.png` | Privacy | light | No account, no server, no telemetry. One folder. |

`preview/` holds 1366x768 copies, only so they can be opened in tools that will
not load the full-size ones. Do not upload those.
