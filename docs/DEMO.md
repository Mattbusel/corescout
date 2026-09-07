# The demonstration

```powershell
cargo run --release -p corescout-demo
```

It writes a real crate with a real trap into a temporary folder, builds it a
few hundred times with `cargo`, and reports what happened. It takes a few
minutes and it uses a core while it does.

---

## The trap

`src/generated.rs` is produced from `schema.txt` by a small generator. Change
the schema and forget to regenerate, and the crate **still compiles** — it just
holds a constant that says what the schema used to say, and the test is what
notices.

That shape is deliberate. A missing file is a failure anybody sees. A stale
constant is a program that builds cleanly and is wrong, which is the class of
failure this whole product is about.

## The two conditions

Identical agent, identical repository, identical trap. The only difference is
whether the CoreScout it is talking to has worked here before.

The agent is a small program rather than a model. It behaves the way a
competent agent that has not been told about the trap behaves: it builds, it
fails, it tries again, and on the second attempt it works out that something
needs regenerating. It asks CoreScout before each operation and reports the
outcome afterwards, exactly as the MCP tool descriptions ask a real agent to.

**Token usage is not measured, because nothing here spends any.** What is
measured is what the harness can actually observe: tasks that ended in a
working build, how many builds that took, how many failed, and elapsed wall
clock.

## What it produces

From a run on a 13th Gen Intel Core i7-13700KF:

```text
                     fresh    experienced
  tasks completed        10             10
  builds run             13             10
  builds failed           3              0
  failure rate          23%             0%
  seconds               8.5           10.3
```

```text
[Verified]  cargo run --bin generate, then cargo build
            cargo build is more reliable here when the first step runs first.
```

## Three things not to skip past

**The experienced run took longer.** Fewer failures, more wall clock, because
the remedy costs something: it regenerates every time, and regenerating is not
free. A product that reported the first number and not the second would be
selling something.

**The fresh CoreScout learned during the ten tasks.** Three failures, then
none, because it noticed the association and began suggesting it. That narrows
the gap between the conditions, and it is the correct outcome. A demonstration
tuned to keep the naive condition naive would demonstrate nothing.

**It is one paired run on one machine.** It shows the mechanism working. It is
not a measurement of how much CoreScout helps in general, and the numbers move
between runs. The program prints that sentence itself, so the output cannot be
quoted without it.

## What the tests establish that this does not

`integration/tests/product_loop.rs` runs the same loop against a known ground
truth, through the real MCP server, and asserts what a single demonstration
cannot:

- a correlation is never promoted, however much of it there is
- randomised assignment on both arms produces a capability, and nothing else does
- CoreScout does sometimes withhold its own suggestion, and less often than it offers it
- an agent that never verifies anything teaches CoreScout nothing false
- an agent whose successes reality contradicts is believed about reality
- what was learned while one model was connected is reachable by the next one

```bash
cargo test -p corescout-integration --test product_loop
```

## What has not been shown

That a real model, with real context and real judgement, does measurably better
work with an experienced CoreScout than with a fresh one. That needs a real
workload, a real model, and enough runs to say something, and it is not
something a simulated agent can stand in for.

The mechanism is here and tested. The claim about what it is worth is not made
yet, and when it is, the numbers will be published whichever way they come out.
