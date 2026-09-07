# How a correlation becomes something runnable

This is the path from "that keeps happening" to a procedure your AI can invoke,
and the reason it is longer than it needs to be.

```text
actions observed
      ↓                     recurrence, and a rate that beats the base rate
a failure mode
      ↓                     what the runs that worked had in common
an association              ← a lesser system starts recommending here
      ↓                     randomised assignment, both arms
a procedure under test
      ↓                     reliability, and the user
a capability
      ↓                     validation, permission, verification, rollback
something that ran
```

---

## 1. Actions

Everything comes through one door, `Ingest::observe`. It scrubs secrets,
normalises the command into a fingerprint, and infers retries. An adapter
written later cannot skip any of that.

A fingerprint strips what varies and keeps what changes behaviour:

```text
cargo build --release --target-dir C:\tmp\a1b2   ┐
cargo build --release --target-dir C:\tmp\9f3c   ┴→  cargo build --release
cargo build                                       →  cargo build
```

Normalisation has two failure modes and they are not symmetric. Too little and
a recurring operation looks like fifty unique ones, so nothing is learned. Too
much and two genuinely different operations merge, so CoreScout learns a
pattern that does not exist. The second is far worse — the first produces
silence, the second produces confident wrong advice — so the rules keep
anything that plausibly matters: subcommands, flag names, `--release`.

## 2. A failure mode

An operation, in a place, that has been attempted at least five times and
failed at least twice. Fewer than that is not a pattern, and the thresholds are
in `Thresholds`, visible and adjustable.

## 3. An association

For every action, CoreScout notes what preceded it in the same session, within
a window. Over time that gives, for each pair, the failure rate of Y when X came
first and when it did not.

A pair becomes an association when it has been seen at least three times on
each side and the failure rates differ by at least 25 points.

**An operation is never a precursor of itself.** Otherwise a retried command
looks like its own remedy, which is both wrong and the most common shape in the
data.

At this point the honest description is *seen together*, and that is exactly
what the interface says. The agent that regenerated the schema first was
probably already the more careful run; the correlation may be entirely the
agent and not the procedure.

## 4. A procedure under test

This is the step a lesser system skips.

To say more than "seen together", CoreScout has to sometimes *not* suggest the
procedure when it believes it should, and sometimes suggest it when it does
not. `Procedure::decide` delegates that coin flip to
`corescout_science::Exploration`, which is the only place in this workspace
that produces an honest `randomised` flag.

`CausalEstimate::effect` returns an error, not a number with a caveat, unless
there are randomised trials on **both** arms. A number with a caveat gets used
and the caveat gets dropped.

The cost is real: a randomised trial is one where CoreScout may deliberately
let a build fail that it could have saved. So the exploration rate is small
(15%) and the budget finite (400 trials). A settled comparison keeps a residual
rate rather than stopping, because a repository changes, and a procedure that
is never re-tested keeps acting on evidence about a repository that no longer
looks like that.

## 5. A capability

A procedure is promoted when it has randomised evidence on both arms and its
own success rate is at least 85%. Note that those are two different bars: a
procedure can genuinely help and still be a bad capability, because "better
than the alternative" and "reliable enough to hand someone" are different
claims.

It arrives **unapproved**. Creating a capability automatically and enabling it
automatically are different things, and only the first happens without being
asked.

A capability carries:

| | |
|---|---|
| name | what a person would call it |
| purpose | one sentence |
| preconditions | what has to be true for it to make sense |
| steps | programs and arguments, never a shell |
| verification | **required** — the step whose job is to disagree with the others |
| rollback | how to put it back, where that is possible |
| risk | what it could cost if wrong: none, low, moderate, high |
| reversible | whether the rollback is expected to work |
| provenance | which procedure it came from, and what the evidence said |
| reliability | the share of runs verification confirmed — `None` until it has run |

The verification step is why this is a capability rather than a saved script.
Without it, the only thing a run can report is what it was told, and the gap
between what a tool reports and what happened is the entire subject of this
product.

## 6. Running one

```text
validate the definition        → Invalid, with what is wrong with it
rule on permissions            → Refused | NeedsApproval | continue
for each step:
  check the authority again    → a definition can change mid-flight
  run the program, with a timeout
  stop on the first failure
verification                   → only if the steps finished
  failed?                      → run the rollback
record an audited event        → action, target, reason, expected, actual, time
```

**Verification is not attempted when the steps did not finish.** A green
verification after a failed step reads as a success, which is precisely the
failure this product exists to catch. The outcome is `verified: None` — an
unknown — rather than `false`.

Definitions are validated as untrusted input, because the thing generating them
is a program reading agent activity. [SECURITY.md](SECURITY.md) has the full
list of what is refused; the short version is no shell, no shell interpreters,
no metacharacters, no absolute or relative program paths, no `..`, and a
timeout that is enforced by killing the process.

## What you can do with one

Approve it. Rename it. Inspect its evidence. Let your AI use it without asking
each time. Switch it off. Delete it. Test-run it, which walks the whole path
including the permission ruling and starts nothing.

## Worked example

```text
Regenerate schema, then build                         Verified

Learned because:
  8 of 11 builds in this repository failed the same way. The three that worked
  had run the schema generator first.

Tested because:
  That is a correlation. CoreScout suggested the procedure by coin flip on 29
  occasions and measured what happened.

  failure rate with it      7%   (14 randomised trials)
  failure rate without it  43%   (15 randomised trials)

Procedure:
  cargo run --bin gen-schema
  cargo build
  → verify the binary was produced

Reliability:
  94% of 12 uses, verified
```

Every line of that comes from a record CoreScout keeps. The Explain panel shows
the underlying JSON if you want it.
