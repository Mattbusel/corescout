# CoreScout architecture: from benchmark to computational mirror

This describes the research crates and the rules about what each may depend
on. The layers built on top of them to make a product are in
[ARCHITECTURE_PRODUCT.md](ARCHITECTURE_PRODUCT.md).

This document records the reclassification of the original CoreScout codebase and
the reasoning behind the Mirror v0.1 refactor. It is written for whoever picks the
project up next, including the case where that is a model rather than a person.

---

## 1. What CoreScout was, before

CoreScout v0.1 was a per-core benchmark and pinning advisor. Its data flow was a
single straight pipe with no branches:

```
sysfs/procfs  ->  Topology  ->  Runner (pins, runs workloads, times them)
                                   ->  BenchmarkResults
                                       ->  Analysis (scores, rankings, advice)
                                           ->  human text / JSON  ->  a person
```

Nine modules, ~5,000 lines:

| module | what it did |
|---|---|
| `topology/` | platform-neutral model of CPUs, cores, caches, NUMA nodes |
| `platform/` | the only OS-specific code: sysfs parsing, affinity syscalls, CPUID |
| `affinity/` | `CpuSet`, kernel-style CPU list parsing |
| `benchmark/` | pin a thread, run a fixed workload, time it, summarise robustly |
| `ranking/` | normalise measurements to 0-100, rank, recommend a CPU |
| `output/` | render for a terminal or as JSON |
| `cache.rs` | persist an `Analysis` keyed by a machine fingerprint |
| `run.rs` | launch a child process with an affinity mask |
| `cli.rs`, `main.rs` | argument parsing and dispatch |

The architecture was sound for what it was, and two of its properties survive the
refactor unchanged because they turned out to be the right instincts:

- **The platform trait boundary.** Every OS-specific operation already sat behind
  one trait. The mirror needs exactly the same boundary, for the same reason.
- **Root-injectable sysfs parsing.** The parser takes its filesystem roots as a
  parameter, which is what lets it be tested against synthetic machines on any
  host. Every new sensor inherits this and is tested the same way.

---

## 2. Classification of existing components

Applying the rule *observation belongs to the mirror, intentional perturbation
belongs to experimentation*:

### Mirror (passive observation of what is)

| component | disposition |
|---|---|
| `topology/model.rs` | -> `substrate/topology.rs`. Structural facts about the machine. Becomes the seed for entity identity and static relations. |
| `platform/linux/sysfs.rs` | -> stays in `platform/linux/`. Pure passive reads of kernel-maintained state. Now also the template every sensor follows. |
| `platform/cpuid.rs` | -> stays. `CPUID` is an unprivileged, side-effect-free instruction. |
| `affinity/` `CpuSet` | -> `substrate/cpuset.rs`. A representation, not an action. |
| `platform::current_thread_affinity`, `process_affinity` | Mirror. Reading a mask observes; it does not perturb. |

### Experiment (deliberate perturbation to learn something)

| component | disposition |
|---|---|
| `benchmark/` runner, workloads, clock, stats | -> `experiment/benchmarks/`. This is the clearest case in the codebase: it pins a thread it does not own, saturates a core, heats a package, and evicts other tenants' cache lines, *in order to find something out*. Textbook intervention. |
| `platform::pin_current_thread` | Dual-use. Called by the experiment subsystem here; the same primitive is an actuator in `agency/`. The syscall is neutral; the intent is not, so the classification follows the caller. |

### Interpretation (neither mirror nor experiment)

| component | disposition |
|---|---|
| `ranking/` | -> `analysis/`. Scores, weights and "best core" are value judgements applied to observations. Demoted from the centre of the project to one consumer among others. |
| `run.rs` | -> `agency/launch.rs`. It changes the physical machine. It is the codebase's first actuator, and it already had the properties an actuator needs: explicit, logged, and it reports what it did. |
| `cache.rs` | -> `experiment/profile_cache.rs`. It persists experiment results, not machine state. |
| `output/human.rs` | -> `output/human_debug.rs`. Renamed to say what it now is: a debugging projection, not the canonical form. |

---

## 3. Assumptions that conflated observation with interpretation

Five places where the old code silently mixed the two. Each is a real defect under
the new architecture, not a stylistic complaint.

**3.1 JSON was the canonical representation.** `output/json.rs` serialised the
in-memory `Analysis` directly, and `cache.rs` stored that JSON as the machine
profile. So the authoritative artifact was a human-readable text format with
human-chosen field names, and any consumer had to parse text to learn anything.
*Fixed by:* the Self-State Plane. The canonical form is a fixed-layout binary
matrix; JSON becomes a projection.

**3.2 Named metrics were the only representation.** A measurement was
`ops_per_second` or `ns_per_op`, hardcoded field by field. There was no way to add
an observation without adding a struct field, and no way for a consumer to iterate
over "all state" generically.
*Fixed by:* a schema-driven channel table and a dense `entities x channels`
matrix. Adding an observation adds a column, not a type.

**3.3 Human ontology was the structure, not an annotation.** `Topology` had
`physical_cores`, `caches`, `numa_nodes` as separate typed collections with
different shapes. Any reasoning had to go through those categories because there
was nothing else to reason with.
*Fixed by:* entities and relations. `EntityClass` still exists and is still
accurate, but it is a `class_hint` annotation on a uniform entity, and a consumer
can ignore it entirely and still see the whole machine.

**3.4 Observation was assumed free.** The old code read `scaling_cur_freq` in the
topology walk without comment. On several cpufreq drivers that read triggers a
cross-CPU IPI or an APERF/MPERF read on the target CPU, which is a perturbation of
the very core being measured, performed in the middle of measuring it.
*Fixed by:* every sensor declares a `Perturbation` class and a documented physical
basis, and the collector measures and publishes what each observation actually
cost. See `observation/mod.rs`.

**3.5 History was folded into the present.** `Summary` carried `median`, `p95`,
`p99` over a window of samples, and `Measurement` carried both a "clean" and a
"raw" distribution. Those are historical summaries wearing the clothes of current
state. They were correct for a benchmark and are wrong for a mirror.
*Fixed by:* the mirror publishes only instantaneous values and cumulative
counters. Rates, trends and percentiles are derived by the memory and model
layers, from a series of snapshots.

A sixth, subtler one is worth naming because it shaped a design decision. Cumulative
counters, `energy_uj` or `instructions retired`, are *present facts* about the
current value of a register or kernel variable, even though their usefulness comes
from differencing them. The mirror therefore publishes the counter, never the rate.
Differencing is the memory layer's job. This keeps `M(t)` a pure function of the
machine at `t`, with no dependence on what the observer saw previously.

---

## 4. The minimal refactor for Mirror v0.1

The goal was the smallest change that makes the boundaries real, not the largest
change that makes the tree look like the target diagram.

**Kept entirely.** The `Platform` trait, the sysfs parser, all benchmark
workloads, the statistics, the ranking maths, the process launcher, every existing
test. Nothing was deleted; 5,000 lines moved and were reclassified.

**Added.**

- `mirror/` — the new core. Entity identity, channel schema, state matrix,
  relations, snapshot, and the Self-State Plane.
- `observation/` — the sensor trait and seven passive Linux sensors, each
  documenting its physical basis, sampling cost and perturbation class.
- `substrate/` — structural discovery: turning a `Topology` into stable entities
  and static relations.

**Declared but not implemented.** `memory/`, `model/`, `agency/policy`,
`autonomy/` exist as modules containing their contracts and the reasoning about
what may and may not cross into them. A boundary that exists only in a design
document is not a boundary.

**The truth boundary is enforced by the kernel, not by convention.** The plane
writer opens its backing file read-write; every consumer opens it `O_RDONLY` and
maps it `PROT_READ`. An agent that tries to edit its own reflection takes a
SIGSEGV. This is deliberate: the read-only property of the mirror should not
depend on the agent's good behaviour or on a `&` versus `&mut` in a language the
agent may not be written in.

---

## 5. Layer contract

```
substrate     what exists, structurally           stable across snapshots
observation   how to sample it, and what it costs passive only
mirror        M(t): what the machine is now       read-only, no history
memory        H(t) = [M(t-n) .. M(t)]             derives change
model         how I behave; what might happen     derives prediction
agency        what I can change                   the only path to the machine
autonomy      what I should choose to change      bounded, auditable
experiment    deliberate perturbation to learn    never part of M(t)
analysis      value judgements over observations  one consumer among many
```

The single rule that keeps these apart: **a layer may read from layers above it in
this list, and may never write to them.** The mirror cannot see memory. The model
cannot edit the mirror. Only agency touches the machine, and it does so through
actuators that report whether the physical action actually occurred.

---

## 6. The workspace: dependency graph as architecture

The layer contract above was, for two revisions of this project, a rule enforced
by review and by tests that read source text. It is now enforced by Cargo.

Each layer is a crate, and a crate cannot use what it does not depend on. The
boundary is checked by the compiler on every build, and violating it requires
editing a manifest, which is a visible act rather than an accident.

```text
                        core
                    (errors, clock, cpusets)
                          |
                       mirror                     M(t) and the plane
                     /    |    \
            substrate  memory   human             substrate is the ONLY
          (HARDWARE)     |                        crate that reads hardware
                      represent
                         |
                     selfmodel  ---  observer
                         |               |
                    counterfactual    identity
                         |    \         /
                      intent   \       /
                         |      \     /
                       agency ---+---+
                         |
                      autonomy                    the decision loop
```

`experiment` and `analysis` sit off to the side: they depend on `substrate`
because perturbing the machine requires reaching it, and nothing depends on them
except the CLI.

**What this buys, concretely.** `corescout-selfmodel` does not list
`corescout-substrate` in its manifest. A contributor who needs one more number
and reaches for `/sys` inside the self-model does not get a code review comment.
They get a compile error, immediately, on their own machine.

### The strongest version

`apps/mirror-tools` holds `mirror-inspect`, `mirror-observer`, `mirror-learn` and
`mirror-replay`. Its manifest does not name `corescout-substrate`, so the code to
read `/proc`, `/sys`, CPUID, MSRs or perf is **not linked into those
executables**.

That converts the project's central claim from a promise into a checkable
property:

```console
$ cargo tree -p mirror-tools | grep corescout-substrate
$ echo $?
1
```

`integration/tests/mirror_boundary.rs` asserts it in three ways: by reading the
manifest, by reading the source of every consumer, and by confirming that exactly
one crate in the workspace contains a `/sys` or `/proc` path.

---

## 7. What was added after Mirror v0.1

The layer contract listed `memory`, `model` and `autonomy` as boundaries
containing only their contracts, and a test asserted they stayed empty so that
filling them in would be a deliberate act. They have since been filled in. That
test has been replaced by its opposite: those layers must now contain real code,
because a layer declared and not built is worse than one not declared.

| layer | what it became |
|---|---|
| `memory` | bounded ring, durable recordings, and a `Source` a consumer cannot tell from live |
| `represent` | normalisation, correlation on differences, discovered latent states |
| `selfmodel` | per-cell online prediction with uncertainty, scored against persistence |
| `counterfactual` | `F(M_t, A_t) -> M_{t+dt}`, refusing to predict actions never tried |
| `intent` | outcomes with hard constraints and weighted preferences |
| `identity` | self-boundary candidates, and grounded self-description |
| `agency` | actuators with scope, bounds, rate limits, read-back confirmation, audit, revert |
| `autonomy` | policy, curiosity, an independent watchdog, and the loop that ties them together |

### The one design decision worth recording

The control loop learns from the **previous** tick's action at the start of the
current tick, not at the moment of acting.

The obvious way to write it, attributing the consequence when the action is
taken, means the effect model learns from a reflection sampled *before* the
action had any chance to take effect. The model then reliably concludes that
actions do nothing, which is both wrong and self-confirming: a controller that
believes actions do nothing stops acting, and never gathers the evidence that
would correct it.

---

## 8. Where the human ontology now lives

Section 3 listed five places where the original code conflated observation with
interpretation. All five are resolved, and the resolution is the same in each
case: the interpretation moved to a crate that depends on the mirror rather than
being part of it.

| interpretation | now lives in |
|---|---|
| "best core" | `analysis` |
| "preferred" as a fact rather than a firmware claim | `mirror` entity annotation, consumed by `analysis` |
| p99 and other summaries inside the state | `memory`, derived from history |
| rates published as state | `memory::transitions` |
| profile names as a fixed taxonomy | `analysis::placement`, one consumer among several |

`docs/ONTOLOGY.md` records the rest: which names are ours, which are the
machine's, and the rule that a discovered structure keeps its discovered name.
