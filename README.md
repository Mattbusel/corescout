# CoreScout

**A computational mirror: a machine-native, read-only representation of what a
computer physically is, right now.**

CoreScout continuously observes a machine's own hardware and publishes the result
as a numeric matrix in shared memory. Other software maps that region read-only
and reads the machine's current state at memory speed, without parsing `/proc`,
without knowing what a NUMA node is, and without touching the hardware at all.

```text
physical machine state H(t)
        |
   observation              passive only; every sensor declares what looking costs
        |
        v
    mirror M(t)             entities, relations, state. No history, no advice.
        |
        v
 self-state plane           fixed-layout shared memory, mmap PROT_READ
        |
        v
    consumer                memcpy
```

CoreScout began as a tool that benchmarked CPU cores and recommended which one to
pin a thread to. That still works, and it turned out to be one application of a
deeper primitive: a machine having a usable representation of its own physical
state. The benchmark now lives in `experiment/` and is no longer the point.

---

## What a mirror is

> A computational mirror is a persistent, read-only, causally accessible
> representation of a computational substrate's current observable state and
> relationships, expressed in a form optimised for machine consumption rather
> than human interpretation.

It is defined as much by what it excludes:

```text
M(t) != recommendation      that is analysis
M(t) != prediction          that is a model
M(t) != historical summary  that is memory
M(t) != benchmark result    that is an experiment
```

A mirror reflects the present. The moment an average, a trend or a "best core"
gets stored inside it, it stops being a function of the machine and starts being
a function of what the observer happened to see before, and two consumers
sampling at different rates will disagree about the present.

This is why cumulative counters are published and rates never are. A counter is a
present fact: the current value of a register or kernel variable, readable in one
observation. A rate is a statement about an interval, needs two observations, and
is therefore memory.

## Observation is not free

A computational mirror is an unusual mirror: looking consumes the thing being
looked at.

- Reading `scaling_cur_freq` on `acpi-cpufreq` sends an **IPI to the observed
  core** to read `APERF`/`MPERF`. Observing a core's frequency makes that core do
  less work.
- Reading a `coretemp` sensor is an `rdmsr` on the target core, also by IPI.
  Polling temperature fast enough prevents cores from reaching the idle states
  whose thermal effect you were trying to measure.
- Holding PMU counters disturbs no hardware at all, and degrades every *other*
  profiler on the machine into multiplexed estimates.
- `/proc/interrupts` costs the kernel `O(sources x cpus)` of text formatting per
  read, charged entirely to the reader.

So every sensor must declare what physical fact it approximates, where it comes
from, how fast it can usefully be sampled, how wrong it may be, and **how much
observing it perturbs what is observed**. The mirror then publishes what each
observation pass actually cost, inside the snapshot. A mirror that does not know
the price of looking is lying about its own completeness.

```rust
pub enum Perturbation {
    None,        // touches only memory the observing CPU already owns
    Negligible,  // a kernel read on the observing CPU
    Low,         // reaches the observed entity: an IPI, a bus transaction, a shared counter
    Material,    // changes state in order to read it. No sensor may declare this.
}
```

`Material` exists so the type can name the boundary it enforces. Anything that
perturbs the machine on purpose in order to learn something is an **experiment**,
not a reflection, and lives in a different subsystem. A test asserts that no
sensor declares it.

## The representation

```text
G(t) = (V, E, X(t))
```

- **`V`, entities.** Anything with persistent identity: the machine, packages,
  physical cores, logical CPUs, caches, NUMA nodes, thermal zones, power domains.
  An id is a hash of a natural key describing *position*, never state, so
  `cpu/11` is the same entity across snapshots, across restarts, and to two
  independent observers that never coordinated.
- **`E`, relations.** Typed edges: containment, SMT siblinghood, cache
  membership, NUMA locality, frequency domain, thermal domain, power domain. In
  the old CoreScout, "CPUs 0 and 4 are SMT siblings" was a field on a struct and
  was only reachable if you knew the field existed. As an edge it is discoverable
  by iteration.
- **`X(t)`, state.** A dense `entities x channels` matrix of `f64`. `NaN` means
  unobserved, which is deliberately not zero: a core reporting 0 C and a core
  with no thermal sensor are different facts.

### Human categories are annotations, not structure

Every entity carries a `class_hint` saying "core", "cache", "NUMA node". These
are useful and usually true. They are **not** the structure.

A consumer can load the whole machine as a matrix, correlate columns, cluster
rows and walk edges without ever knowing what a cache is. That is the point: the
mirror should not force a future model to reason exclusively in the categories
its designers happened to name. If some recurring combination of thermal
distribution, cache state, SMT occupancy and interrupt placement turns out to
predict low jitter, the system should be able to treat that as a thing, whether
or not a human ever named it.

## Reflection is read-only, and the kernel enforces it

```text
Mirror(t) -> decision -> action -> physical machine changes -> Mirror(t+1)
```

never

```text
decision -> edit Mirror(t)
```

An agent may affect **reality**. It may not directly alter **its representation
of reality**, except by changing reality and observing again.

This is enforced by the MMU, not by an API. The writer maps the plane
`PROT_READ|PROT_WRITE`; every consumer opens it `O_RDONLY` and maps it
`PROT_READ`. A consumer that tries to edit its own reflection takes a `SIGSEGV`.
A `&` rather than a `&mut` would only protect consumers written in Rust that go
through our API, and the mirror is meant to be read by software not yet written,
in languages we did not choose.

## The Self-State Plane

> Reading the machine's current self-state should feel closer to accessing memory
> than querying a monitoring service.

```text
+------------------------------------------+
| header            256 B  seqlock, counts |  volatile
+------------------------------------------+
| entity records    32 B each               |  static within an epoch
| channel records   32 B each               |
| relation records  48 B each               |
| string table                              |
+------------------------------------------+
| sensor records    48 B each               |  volatile: what looking cost
+------------------------------------------+
| state matrix A    rows x cols x 8 B       |  volatile, double buffered
| state matrix B    rows x cols x 8 B       |
+------------------------------------------+
```

- **Static and volatile are separated.** Entities, relations and channel
  definitions change only when the machine's shape changes, so a consumer decodes
  them once and caches them against the **epoch** number. Only the matrix and a
  few header words move per tick.
- **A seqlock guards consistency.** The writer bumps a counter to odd, writes,
  bumps it to even. A reader samples the counter, copies, samples again, and
  retries if it moved. Readers never block the writer, which matters because a
  mirror that can be slowed down by being looked at is a mirror whose observation
  cost depends on how many observers there are.
- **The matrix is double buffered**, so the large copy essentially never races.
- **The region lives at a path** in `$XDG_RUNTIME_DIR/corescout/mirror.plane` or
  `/dev/shm`. No daemon registry, no socket, no discovery protocol. Filesystem
  permissions decide who may look.

A steady-state read is a sequence check, a `memcpy`, and a second sequence check.
No parsing, no allocation, no syscall.

## Usage

```bash
cargo build --release

# Publish the mirror. Passive: it reads what the kernel already maintains.
./target/release/corescout mirror

# Learn from it, with no hardware access at all.
./target/release/mirror-observer --both

# In another terminal: read it. This binary has no hardware access at all.
./target/release/mirror-inspect
./target/release/mirror-inspect --detail --filter cpu/
./target/release/mirror-inspect --entity core/0/0     # the graph around one core
./target/release/mirror-inspect --json
./target/release/mirror-inspect --watch 10            # liveness and read cost

# One reflection, no plane, for a quick look:
./target/release/corescout mirror --once
```

```text
$ mirror-inspect

Computational Mirror

epoch                  1  sequence 348  t+34.812s
state                  74 entities x 31 channels, 41% observed
observation cost       612.4 us this pass, worst perturbation low

Sensors
  sensor         perturbs           cost  samples  errors
  frequency      low            121.3 us       64       0
  idle           negligible      88.7 us      112       0
  thermal        low             41.2 us       18       0
  power          inactive on this machine
  scheduler      negligible      96.4 us      152       0
  interrupts     negligible     264.8 us       32       0
  counters       inactive on this machine

Entities
  machine        1
  package        1
  numa           1
  core           16
  cpu            32
  cache          21
  thermal        2

Relations
  contains             70
  smt_sibling          32
  cache_member         84
  numa_local           32
  frequency_domain     32
  thermal_domain       2

plane: /run/user/1000/corescout/mirror.plane (23744 bytes, written by pid 48213)
```

*(Illustrative. Your machine's numbers will differ.)*

`mirror-inspect` **does not collect any of this**. It maps the plane read-only
and interprets what it finds. That is what demonstrates the abstraction exists
independently of its observer, and an architectural test enforces it by rejecting
any mention of `/proc`, `/sys`, `perf_event`, or the observation and platform
modules in that binary's source.

## The observer: does the mirror carry enough to learn from?

`mirror-inspect` is deliberately stupid: it turns one reflection into text. The
interesting consumer is `mirror-observer`, which learns.

It has **no hardware access**. No `/proc`, no `/sys`, no CPUID, no `perf`, no
syscall that reaches hardware. It maps the plane read-only, watches the numbers
change, and reports what structure it can find. An architectural test enforces
this by reading its source.

```bash
# Record an hour of reflections.
corescout mirror --record trace.jsonl --interval 100 --ticks 36000

# Two observers over identical data. One sees the mirror's names; one sees
# entity_7, x2 and edge_type_2.
mirror-observer --replay trace.jsonl --both

# Or watch a live plane.
mirror-observer --frames 600
```

### What the unlabelled observer is given

```text
entity_7:  x0 = 4181374   x1 = 62.4   x2 = 24176880000   ...
entity_11: x0 = 4180992   x1 = 62.4   x2 = 24170140000   ...
edge_type_2(7, 11)
```

No `cpu`, no `frequency`, no `temperature`, no `smt_sibling`. Just persistent
identity, numeric state, relationships and time.

### What it found

Running against a synthetic machine with known structure (400 reflections,
40 seconds of simulated time):

```text
Which variables accumulate
  inferred by watching: x2, x3, x4, x5
  this observer was not told which variables accumulate; the list above was worked out

How much of the machine moves as one
  0.843 mean pairwise similarity before the common mode was removed

Which entities behave alike, once the common mode is removed
  {entity_7, entity_11}   cohesion 0.970
  {entity_8, entity_12}   cohesion 0.969
  {entity_9, entity_13}   cohesion 0.958
  {entity_10, entity_14}  cohesion 0.976

Which kinds of connection predict shared behaviour
  edge_type_2    8 pairs   connected +0.968 vs baseline -0.140   lift +1.108  d=3.38
  edge_type_5   56 pairs   connected -0.140 vs baseline -0.140   lift +0.000  d=0.00

Predicting the next reflection
  scored 48 cells; 42 beat the naive baseline
  skill vs baseline: median +0.843   (0 = no better than copying the last value)
```

Those four pairs are `cpu/0 + cpu/4`, `cpu/1 + cpu/5`, `cpu/2 + cpu/6` and
`cpu/3 + cpu/7`. They are the SMT siblings. **The observer was never told SMT
exists.** It found them by noticing that those entities' numbers move together
once the machine-wide signal is taken out, and that one kind of edge in the
graph predicts exactly that coupling while another kind predicts nothing.

`edge_type_2` is `smt_sibling`. `edge_type_5` is `frequency_domain`, which in
this machine joins every CPU to every other and is the control: an observer that
reported lift for both would have found structure that is not there.

### The common mode, and a result that only appeared by running it

The first version of the observer had no common-mode removal and reported **one
cluster containing every CPU, cohesion 0.84**, with an SMT edge lift of only
+0.16 against a baseline of +0.84. Every number was correct and the finding was
useless: under a machine-wide workload phase, everything covaries with
everything.

Subtracting the cross-entity mean per variable per instant, which is common
average referencing borrowed from electrophysiology, drops the baseline to -0.14
and raises the SMT lift to +1.11. The global phase is still reported, as its own
number, because "most of this machine is moving as one" is a true and useful
thing to know. It is just not structure.

### Does our ontology help or constrain?

Both observers, identical data:

```text
                                        unlabelled       labelled
  entity groups found                            4              4
  accumulators identified                        4              4
  best edge type lift                       +1.108         +1.108
  prediction skill (median)                 +0.843         +0.843
```

On this machine the answer is **neither**: the labels changed what the findings
were called and nothing else. The unlabelled observer worked out which variables
were counters by noticing they never decrease, which is exactly what the
`Semantics::Cumulative` label declares. That one label was worth a name.

This is a weaker result than "labels constrain understanding", and it is the
result, so it is what is reported. It also has an obvious limit: on a richer
machine, or with a learner that could exploit units and semantics more
aggressively, the two might diverge.

### What this does and does not prove

The machine above is **synthetic**, and it makes SMT siblings covary *by
construction*. Rediscovering that is therefore not evidence about real CPUs. It
is evidence that:

- the discovery pipeline finds structure that is present,
- it does not find structure that is absent (the frequency-domain control),
- the mirror's representation carries this class of signal intact from producer
  to consumer,
- and the two lenses can be compared on identical data.

**The real experiment is an hour of `corescout mirror --record` on real hardware,
followed by exactly this analysis.** It has not been run. The simulator exists so
the harness could be checked against a machine whose answers are known first.

### The original tool is still here

```bash
corescout info                                   # topology
corescout benchmark                              # per-core measurement
corescout analyze                                # ranking and pin recommendations
corescout run --profile latency -- ./my_program  # launch pinned
```

These deliberately perturb the machine, which is why they are experiments rather
than observations, and why the mirror does not depend on them.

## The loop

The mirror is one layer of a cycle that is deliberately not collapsed:

```text
        +-----------------------------------------------------------+
        |                                                           |
        v                                                           |
   mirror M(t)  ->  memory H(t)  ->  representation  ->  self-model  |
   what am I        what have         what structure     how do I    |
                    I been            is there           behave      |
                                                            |        |
                                                            v        |
   reality  <-  action  <-  choice  <-  intent  <-  counterfactual   |
   the machine   bounded,    what is    what          what would     |
   changes       audited     worth      outcomes      happen if      |
        |                    doing      do I want                    |
        +-----------------------------------------------------------+
```

Every arrow is a crate, and the direction of the arrows is the direction of the
dependencies. A layer may read from the layers before it and may never write to
them. The self-model cannot edit the mirror. The mirror cannot see memory. Only
`agency` touches the machine.

**Why not collapse this into one "AI scheduler"?** Because a policy built on
inputs that were never validated separately is unfalsifiable. When it makes a bad
choice, there is no way to tell whether the mirror was incomplete, the memory too
short, the model wrong, the intent misstated, or the policy itself at fault. Kept
apart, each is a research problem with its own success criterion, and each can be
measured and shown to be wrong on its own.

## What the machine may say about itself

`corescout describe` produces statements like:

```text
[observability]
  I observe 412 of 640 possible variables about myself, across 80 parts.
  There are 96 things about myself I cannot observe on this machine: this
  process lacks the privilege to read it.

[structure]
  I distinguish 7 recurring states of myself, which I found rather than being
  told about.
  I am currently in latent_state_3.

[prediction]
  I predict my own next state 34% better than assuming nothing changes.
```

Every one of those carries the evidence it was derived from and could be
confirmed or refuted by anyone holding the recording. That is the entire standard
being applied.

The system does **not** say "I am conscious", "I am aware", "I am alive" or "I am
AGI". Not because those are filtered out of a list of things it wanted to say,
but because there is no generator of unsupported sentences to filter: a
`Proposition` cannot be constructed without a `Ground` naming what supports it. A
test asserts the vocabulary stays out.

## Doing science on itself

The layers above are self-observation and control. This part is different in
kind: the machine forms statements about itself that an observation could
**kill**, and coins concepts that were not in anyone's vocabulary.

```text
observation -> hypothesis -> prediction -> experiment -> falsification
                    ^                                          |
                    +------------- revised theory -------------+
```

A hypothesis cannot be constructed unless some possible reflection would refute
it. That check is the whole difference between this and the curiosity module it
grew out of:

```text
curiosity:   "try this and see"       -> cannot be refuted
hypothesis:  "this will do X, +/- t"  -> refuted if it does not
```

The number to watch is not how many claims survive. It is **how many were
killed**. A theory full of supported claims is equally consistent with a
generator that only makes safe statements, so `corescout theory` prints the
refutation rate and complains when it is suspiciously low.

### Concepts have to earn their name

A recurring state is cheap: `latent_state_13` exists for anything visited twice.
A **concept** is one that earned promotion by improving prediction, and the
utility is measured elsewhere and passed in, because a registry that could
compute its own justification would coin everything.

Concepts are retired when they stop paying. An ontology that can only grow ends
up with a category per reflection, explaining nothing while appearing to explain
everything.

`concept_7` stays `concept_7`. If it turns out to coincide with what we call
thermal throttling, that is recorded as a **finding** and read by no decision.
Renaming it would destroy the finding and replace it with our assumption.

### The threshold experiment

> The system constructs a concept that was not explicitly supplied by its
> designers, uses that concept to predict its own behavior, and acts
> successfully because of it.

A synthetic machine is given a hidden regime: two entities elevated on one
channel *and* depressed on another, simultaneously. It is not any single
variable, not a channel, not an entity, and appears nowhere in the vocabulary.
The identical pipeline is then run on a machine with no regime at all.

| | hidden regime | control |
|---|---|---|
| latent states discovered | 16 | 16 |
| improvement from knowing the state | **78.4%** | **8.8%** |
| concepts coined | **7** | **0** |
| candidates refused | 9 | 16 |

Both machines discover 16 states, because clustering always finds something. The
discrimination is in the coinage bar, not the clustering.

This establishes that the mechanism works. It does **not** establish that real
hardware contains such regimes, or that any survive into the mirror at 10 Hz.

### Concepts change what it does

The arrow the rest of this was building toward. A policy conditioned on
self-coined concepts, where a preference is installed **only** from randomised
evidence, and where the machine records what it would have chosen without the
concept:

```text
arm           cost bef    cost aft   right   r-blf   stale   s-blf  attrib
baseline         6.499       5.498       0       0       0       0       0
latent           4.667       5.604    2356       0     240       0       0
concept          6.308       5.620     246     193     274      10     315
oracle           4.167       5.498    2999       0       0       0       0
```

A synthetic substrate where the best action depends on a conjunction across two
entities and two channels that is not any input variable. Halfway through, the
regime stops occurring and the good action becomes the worst one.

`attrib = 315`: a concept the machine coined changed what it did, with the
counterfactual recorded each time. `r-blf` 193 against `s-blf` 10: the belief
was right nineteen times more often than wrong, and it died when its regime did.

**The uncomfortable line is `latent`.** That arm is the same pipeline with the
coinage bar and the causal test removed. It acts on correlation immediately, and
it is cheaper. It is faster because it is credulous, and this environment does
not punish credulity hard enough to show it. That comparison is reported rather
than tuned away; see `docs/AGENCY.md`.

### Does the same silicon end up doing more work?

The economic question underneath the rest: can compute be spent to manufacture
*more effective compute*? Measured as verified useful work per scarce physical
input, on workloads never practised on, net of what the search cost:

```text
lineage                          net Q trained   net Q held-out
  G0  inherited nothing              0.000377        0.000383
  G1  inherited 14 configs           0.000454        0.000457
  G2  inherited 28                   0.000541        0.000533
  G3  inherited 42                   0.000631        0.000610
                                                     +59.4%

control (searches identically, keeps nothing)
  G0 -> G3                           0.000276        0.000275   flat

attributable to learning: 61.6%
```

The control is the result. It spends identical silicon searching and discards
the answer, so anything it also gained would have been the substrate rather than
the runtime. It gained nothing.

`Q` is the number the project would be judged by, which makes it the number most
worth faking, so the ledger was built adversarially before the mechanism:
unverified work is worth **zero** and still costs, search is charged to whoever
inherits the gain, held-out work is accounted separately, and the weights that
price scarce inputs come from outside. `Verdict` can return
`LearnedTheBenchmark`, `SearchCostTooHigh` and `Untrustworthy`, and a harness
that cannot return those is not measuring anything.

The first run of this experiment produced a *declining* Q, because the space was
small enough that G0 solved it outright and every later generation kept paying
for searches that found nothing. That is correct accounting describing a badly
designed process, and the fix was a harder space rather than a kinder ledger.
`docs/CAPITAL.md` has the full account, including what it does not establish.

### When is the causal bar worth paying for?

The previous result had the correlational agent winning, which was fair and
incomplete: its environment contained no correlation worth being fooled by. Five
worlds where being fooled is possible, scored as regret against an oracle:

```text
world            agent             regret   explore    belief   false    true
benign           correlational       6278      1218       520     130    2361
benign           causal             10074      1798         0       0    1317
confounded       correlational        956       764       192     223       0
confounded       causal               850       850         0       0       0
sign-reversal    correlational        846       643       202     506       0
sign-reversal    causal               727       727         0       0       0
regime-switch    correlational       7380       872      3084     995     976
regime-switch    causal              6935      1372       603     201     423
deferred-cost    correlational       2203       523      1681    1913       0
deferred-cost    causal               645       461       184     238       0
```

`explore` is the price of being able to learn; `belief` is the price of being
wrong. They are different things and reporting them as one number hides the
result.

**The causal agent loses the benign world badly**, 10074 against 6278, and a
test asserts that it does. **Belief regret separates the agents far more cleanly
than total regret**: in sign-reversal the totals differ by 14% while the
false-belief counts differ by 506 to zero.

**Deferred cost defeats both.** When an action's bill arrives one tick later and
is charged to whatever happens next, randomisation does not help: shuffling which
action is taken does not move which tick the bill lands on. The causal agent is
fooled about eight times less, and it is fooled. That world is in the suite
because a trap suite containing only traps the method survives is a
demonstration rather than a test, and there is an assertion that fails if either
agent sweeps.

Full account, including two defects the suite exposed, in `docs/CREDULITY.md`.

### Association is not causation, and the types enforce it

```text
I acted while concept C was present.
Things improved.
Therefore C caused the improvement.
```

Every step is reasonable and the conclusion is unfounded. So there are two claim
classes: `C predicts X` is settled by watching, `under C, A causes X` is not.
`CausalEstimate::effect()` returns an error unless there are randomised trials
on both arms. Observational data can only produce `association()`, a different
method with a different name, so the two cannot be confused by accident.

Building this exposed three defects, all found by the experiment failing rather
than by review, and all the same lesson: **evidence that only accumulates cannot
describe a world that changes.** Utility measured over all history never falls;
preferences were installed but never withdrawn; and the policy could not
bootstrap because it randomised only after a preference existed. All three are
written up in `docs/AGENCY.md`.

### Virtual resources

A concept, plus a recipe that produces it, plus a measured record of how often
the recipe works, is a resource an application can ask to run on:

```text
concept_31 + recipe + "worked 84% of 51 attempts" = v1
```

Nobody manufactured `v1`. Reliability is `None` until there is evidence, and a
resource that falls below its promise is withdrawn, because an unkept guarantee
is worse than no guarantee.

### Inventing a move

When no single action reaches a target, the machine composes primitives into a
new one, tests it, and keeps it only if it works. A compound is built from
actions that are already scoped, bounded and audited, so **it cannot exceed the
authority of its parts**.

This is not the machine writing a scheduler. Generated code would inherit none
of the safety model and the audit trail would describe code no reviewer had
seen. Composition is the form of invention that can be made honest here, and it
is genuinely a restricted form.

### Sharing concepts between machines

A concept travels as dimensionless relational facts: how much of the machine
participates, how long it lasts *relative to that machine's own timescale*, how
strongly its own actions move it. No units, no channel names, no entity indices;
a test asserts none leak in.

Machine B then asks *do I have anything shaped like this?* and searches its own
reflections. Being told a concept exists elsewhere is not evidence it exists
here, so adoption applies the same bar. What is exchanged is a question, not a
setting.

See `docs/SCIENCE.md` for the full account, including a list of what is
deliberately **not** built and why.

## Does it actually work?

The only interesting version of that question is *compared to what*, so the
comparison ships with the code. `corescout experiment compare` runs ten workload
scenarios against six placement policies:

| policy | what it does |
|---|---|
| `os-scheduler` | no affinity at all; the kernel places and rebalances |
| `first-cpu` | the lowest-numbered permitted CPU |
| `preferred-core` | the firmware's own bin sort |
| `corescout-ranking` | whatever the original benchmark ranked first |
| `least-utilised` | the CPU with the most idle residency |
| `random` | a fixed-seed random CPU: **the control** |

The baselines are chosen to be hard to beat. The Linux scheduler is very good.
Static pinning to a well-chosen core is very good. `random` is there because a
comparison without a control cannot distinguish a policy that works from one that
happened to be lucky on the machine it was developed on.

The harness reports **inconclusive** freely, and a verdict requires the
difference to exceed both the run-to-run spread and a minimum effect worth
claiming. Most placement decisions on most machines genuinely do not matter, and
a harness that cannot say so is a machine for generating false claims.

## Safety

The controller may change the machine. It may **not** change its reflection of
the machine: consumers map the plane `PROT_READ` and the MMU enforces it.

| mechanism | what it stops |
|---|---|
| scope | acting on any process nobody opted in; the default is this process alone |
| bounds | values outside the permitted range, and acting too often |
| watchdog | a controller that is thrashing, failing, or making things worse |
| dry run | everything, while still exercising the whole decision path |
| audit log | every action records target, reason, expectation, result, timestamp |
| revert | on every exit path, including ctrl-c |

The watchdog holds no reference to the policy and the policy holds no mutable
reference to the watchdog. A controller that can switch off its own safety layer
has no safety layer. Every trip leads to the same place: freeze actuation, revert
what can be reverted, and hand the machine back to the operating system. That is
a good outcome; the worst case for this project is a machine left worse than it
was found.

Nothing runs system-wide. There is no daemon that tunes your machine.

## Commands

```text
OBSERVING (passive; changes nothing)
  corescout mirror                    publish the self-state plane
  corescout record trace.jsonl        append every reflection to a trace
  corescout info                      topology, caches, SMT, NUMA

READING THE MIRROR (no hardware access at all)
  corescout inspect                   describe the live plane
  corescout replay trace.jsonl        read a recording as though it were live
  corescout observe --both            the two-ontology experiment
  corescout learn                     learn to predict, and score it
  corescout latent                    the states the machine found in itself
  corescout describe                  what it can truthfully say about itself

PERTURBING (deliberately changes the machine)
  corescout benchmark                 measure every core
  corescout analyze                   rank them and recommend placements
  corescout run --intent latency-critical -- ./program
  corescout experiment compare        the baseline comparison

ACTING (bounded, reversible, audited)
  corescout agent start --dry-run     the decision loop, changing nothing
  corescout agent start --intent latency-critical

  corescout demo self                 the whole cycle, then an explanation
```

## Project layout

```text
crates/
  core/           errors, clock, CPU sets: shared primitives
  mirror/         M(t): entities, relations, state, the self-state plane
  substrate/      THE ONLY CRATE THAT TOUCHES HARDWARE
  memory/         H(t): ring buffer, recordings, replay
  represent/      normalisation, correlation, discovered latent states
  science/        hypothesis, experiment, falsification, causal attribution
  concept/        coined concepts, portable signatures, virtual resources
  capability/     the atlas G=(S,A,T): concepts, affordances, capabilities
  lineage/        productivity accounting across generations
  selfmodel/      prediction of the next reflection, with uncertainty
  identity/       self-boundary inference; what the machine may say of itself
  intent/         outcomes, not mechanisms
  counterfactual/ F(M_t, A_t) -> M_{t+dt}
  observer/       learning from the mirror under a chosen ontology
  agency/         actuators: the only path back to the machine
  autonomy/       policy, curiosity, watchdog, the decision loop
  experiment/     deliberate perturbation: benchmarks, scenarios, baselines
  analysis/       value judgements: rankings and placement profiles
  human/          human and JSON projections, for debugging only

apps/
  corescout/      the CLI, plus mirror-daemon and mirror-agent
  mirror-tools/   mirror-inspect, mirror-observer, mirror-learn, mirror-replay
                  THIS PACKAGE DOES NOT DEPEND ON substrate

integration/      cross-crate and architectural tests
```

The dependency graph is the architecture. `mirror-tools` does not list
`corescout-substrate` in its manifest, so those four programs cannot read
`/proc`, `/sys`, CPUID, MSRs or perf **even by mistake**: the code to do it is
not linked into them. Their inability to peek at the machine is a property of the
executable, checkable with `cargo tree`, rather than a promise in a comment. A
test in `integration/tests/mirror_boundary.rs` asserts it.

The same holds one layer down: `memory`, `represent`, `selfmodel`, `observer`,
`counterfactual` and `identity` do not depend on `substrate` either. They know
the machine only through the mirror.

## Status and limitations

Everything described above is implemented and tested. What is **not** true yet:

- **Linux x86-64 only.** Other platforms build, run, and fail with a clear
  message where hardware is needed. The `Platform` trait documents the Windows
  mapping. The consuming half of the system works anywhere.
- **The baseline comparison has not been run at length on real hardware.** The
  harness is built, tested, and honest about noise; the results table is empty
  because filling it needs a quiet Linux box and hours, not more code. Any claim
  that this beats the Linux scheduler would currently be unfounded, and none is
  made.
- **The observer has only been run against a simulator and synthetic traces.**
  The pipeline works and the representation carries the signal. Whether real
  hardware's couplings survive into the mirror at 10 Hz is unknown.
- **No discovery yet of structure the mirror does not already encode.** The
  observer rediscovered structure the mirror carries as edges. Finding a coupling
  nobody wired in requires real hardware where such couplings exist and were not
  put there on purpose.
- **Resolution is coarse relative to the hardware.** At 100 Hz, with
  tick-quantised CPU time, microarchitectural events are averaged away before
  they reach the matrix.
- **`power` and `counters` need privilege** and are commonly inactive. The mirror
  reports that rather than reporting zeros.
- **Identity is stable against state change and restart, not against everything.**
  Under a hypervisor `cpu/11` may be a different physical core minute to minute.
- **The science does not yet drive the acting.** `theory` and `concepts` run the
  conjecture-and-refutation cycle on demand. The autonomous loop does not run it
  continuously and does not yet choose an action *because of* a coined concept.
  That coupling is the thinnest part of the system.
- **The mirror reflects hardware only.** Reflecting running code, memory
  structures and workload alongside it is the next real step, and nothing in
  `MirrorSnapshot` prevents it.
- **Nothing rewrites code.** Machine-specific recompilation needs compiler
  integration and a codegen search. The interface without the search would be a
  mock, and a mock there is indistinguishable from the real thing in a demo.
- **No concept has been exchanged between physically different machines.** The
  mechanism is tested between synthetic machines of different sizes and
  timescales; whether an x86 concept and an ARM concept ever match is open.
- **`agent stop` does not stop a detached agent.** The agent runs in the
  foreground and reverts on interrupt; there is no daemon protocol, and the
  command says so rather than pretending otherwise.

## Why not just let the OS scheduler handle this?

Usually you should. The Linux scheduler sees the whole machine, rebalances
continuously, and is the product of decades of work on exactly this problem. For
almost all software, manual placement is at best a wash and at worst a
significant regression, and this project's own harness will tell you so.

The cases where it can matter are narrow and specific: low-latency trading, game
engines, audio processing, emulators, databases, high-throughput networking, HPC,
robotics, and inference runtimes. What they share is a tail that matters more
than a mean, and a working set whose locality a migration destroys.

**Do not read this project as a claim that manual affinity universally improves
performance.** It does not. The interesting question is not "which core is
fastest" but whether a machine with a faithful representation of its own state
can work out, for itself and for a specific workload, whether placement matters
here at all, and be right about it often enough to be worth the complexity. That
question is open, and the harness is built to let it be answered no.

## Guiding principle

> Do not tell the computer what it is. Give it a sufficiently faithful mirror
> that it can begin learning what it is.

## License

MIT or Apache-2.0, at your option.
