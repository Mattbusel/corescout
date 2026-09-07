# Results from real hardware

Everything here was measured on the machine that ran the tests. No planted
structure, no simulated cost function, no synthetic substrate.

Machine: **13th Gen Intel Core i7-13700KF**, 16 physical cores, 24 logical,
hybrid (8 P-cores with SMT, 8 E-cores), Windows.

---

## 1. What is real

| | source |
|---|---|
| machine structure | `GetLogicalProcessorInformationEx` |
| placements | real `SetThreadGroupAffinity` |
| work | real pointer chasing and arithmetic, result consumed |
| verification | a checksum the placement chooser never sees |
| cost | `QueryThreadCycleTime`: cycles actually executed, not time elapsed |
| structure to find | whatever this silicon is |

Cycles rather than wall time matters: a thread that waited did not thereby use
more of the machine. Sleeping for 80 ms burns essentially no cycles, and there
is a test asserting it.

---

## 2. The machine described itself

`corescout-lab info` on real hardware got the asymmetric cache hierarchy right
without being told: P-cores 48 KiB L1d and 2 MiB L2 per pair, E-cores 32 KiB L1d
in clusters of four sharing 4 MiB L2, 30 MiB L3 across all 24.

`corescout-lab mirror --once`: 86 entities x 9 channels, **6.8 microseconds** per
observation pass.

Its own account of itself, from 900 reflections over 36 seconds:

```text
[structure]
  I distinguish 32 recurring states of myself, which I found rather than
  being told about.
  I am currently in latent_state_21.

[prediction]
  From here I most often move to latent_state_16, 22% of the time.
  I do not predict my own next state better than assuming nothing changes.

[observability]
  I observe 216 of 774 possible variables about myself, across 86 parts.
```

3 concepts coined, 29 refused. Knowing which state the machine is in reduced
prediction error by **42.4%**.

---

## 3. The hybrid split, discovered from cycles alone

Each placement measured on three workload families, median of five interleaved
rounds, 60 iterations each:

```text
cpu             compute          cache        latency
cpu0           98689281       42362012         778963   P
cpu1           99107302       42296724         784089   P
...
cpu15          99588559       43167973         777253   P
cpu16         206957509       82413110        1396937   E
cpu17         206930506       82102757        1396253   E
...
cpu23         207437055       82227510        1396595   E
```

A clean **2.1x** separation with no overlap, and the boundary falls exactly at
cpu16 where the E-cores begin. The core-type column comes from the topology and
was **not** shown to anything doing the measuring; it is printed so a reader can
check that the split found in cycles is the real one.

Cheapest placement to dearest: **111% apart**.

---

## 4. A causal effect, from randomised assignment

48 trials, every one assigned by coin flip rather than chosen:

```text
under cpu0: cpu0 vs cpu23 -> association -107190336.65,
            causal effect 107190336.65 better (+/- 635683.98,
            28 vs 20 randomised trials)
```

About 169 standard errors. The same comparison built from 40 **chosen** trials
establishes nothing, and a test asserts that: `effect()` returns an error, and
`worth_acting_on` is false, however large the observed difference.

That is the rule holding on real data rather than on a fixture.

---

## 5. The lineage found nothing, and that is the result

Four generations, real search over real placements, real verified work, scored
on a held-out workload the search never touched:

```text
learned                       net Q trained   net Q held-out
  G0                              2.152e-8        1.960e-6
  G1  inherited 3 placements      1.832e-8        1.097e-6
  G2  inherited 6                 1.595e-8        7.643e-7
  G3  inherited 9                 1.429e-8        5.903e-7
  -> not getting better

control (searches identically, keeps nothing)
  G0                              1.896e-8        1.577e-6
  G3                              1.160e-8        5.389e-7
  -> gross productivity rose 6.6% but net rose -65.8%:
     finding the improvement cost more than it has returned

attributable to learning: -4.1%
```

**Both lines decline, and the accounting is correct.** The search costs real
cycles every generation, the bill accumulates, and it buys nothing.

### Why it buys nothing

The OS scheduler already places this single-threaded work on a P-core. The
search's best find is also a P-core. There is no gain available, so paying for
the search is a pure loss, and net productivity falls exactly as it should.

This is the project's own standing claim, measured rather than asserted:

> Do not read this project as a claim that manual affinity universally improves
> performance. It does not.

The synthetic lineage experiment showed +59.4% because a general optimum was
planted far from where a fresh machine starts. On this real machine, the
starting point is already good.

### What would change the answer

An idle desktop is the easiest case for the OS. A loaded machine, a
latency-sensitive workload competing with background work, or a workload that
should be on an E-core and is not, would all give the search something to find.
None of those was tested, and until one is, the honest summary is: **on an idle
i7-13700KF, a placement search costs cycles and returns nothing.**

---

## 6. Correlation invents a difference that is not there

The credulity traps, rebuilt from real contention instead of a formula.

**The confounder is physical.** `cpu0` and `cpu1` are the two threads of one
core and share its execution resources. A thread pinned to `cpu1` running a
dependent arithmetic chain makes `cpu0` measurably slower:

```text
cpu0 costs 65927072 cycles with its sibling cpu1 idle,
           87190787 with it busy: 32% slower
```

**The two placements compared are interchangeable.** Two P-cores, measured
undisturbed and interleaved: **0.0% apart**. There is no difference to find.

**The trap is aliasing.** Both siblings follow one schedule, so neither core is
favoured and each is contended for exactly half the run. The sampler alternates
in lockstep, so one core is only ever seen under contention and the other only
ever without:

```text
trial       1      2      3      4      5      6
siblings  BUSY   idle   BUSY   idle   BUSY   idle
measured  cpu0   cpu2   cpu0   cpu2   cpu0   cpu2
```

### Result

```text
                             cpu0            cpu2
undisturbed truth        65725063        65779750      0.1% apart
correlational (aliased)  86916611        66036796       32% apart

correlational concludes: cpu2 is 30% better  <-- INVENTED
causal concludes:        1.4% of the mean, and declines to act on it
```

The correlational agent confidently manufactures a 30% difference between two
cores that differ by 0.1%. The causal agent, under the identical disturbance,
finds 1.4% and refuses to act on it. The only difference between them is that
one decides which arm to measure by the round and the other by a coin flip,
which decouples the arm from the phase.

This is the strongest form of the trap: a confounder that *narrows a real gap*
is a nuisance, while one that *invents a difference out of nothing* is how a
system comes to act confidently on a fact about itself that is false.

### A methodological failure worth recording

The first run of these tests reported SMT contention as 1% and two identical
cores as 30% apart. Both nonsense, and the cause was that five tests each
spawning pinned load were running in parallel: every test's baseline was
polluted by the others' disturbance.

That is not a flaky test. It is the apparatus being part of the machine it is
measuring, which is this project's whole subject. Serialisation is now enforced
by a mutex rather than by a command-line flag, because a flag can be forgotten.

---

## 7. The theory engine's alarm fired on its first real run

```text
7749 hypotheses stated, 3952 trials, 0 refuted and buried
0% of settled claims were wrong  (suspiciously low: is the generator only
making safe claims?)
```

It is. The conjecture generator commits to cell values within half a state
radius, and on real data the discovered states are tight enough that this is
nearly always true. Zero refutations from 3952 trials is not a theory that
understands the machine; it is a theory that has not risked anything.

That alarm exists precisely to catch this, and it caught it immediately on
contact with real data. Fixing the generator to make bolder claims is the open
item.

---

## 8. Four bugs only real hardware could expose

**The consumer pipeline never read `Semantics`.** It clustered on raw cumulative
counters, and a counter near 9e14 moving by 4e7 per frame is constant to one
part in twenty million. First real run: 900 reflections, **one** latent state,
nothing learned. Every synthetic substrate published instantaneous values, so
this could not appear. Adding `represent::features`, which turns cumulative
channels into rates, took it from one state to 32.

**The learn pipeline never called `predict_next()`.** Nothing was ever pending,
so nothing was ever scored: the model learned continuously and was never asked
to be wrong. That is the same failure the science crate exists to prevent, one
layer up, in code written by the same hand.

**`skill()` guarded a near-zero baseline with an absolute threshold** of 1e-12,
on data spanning 1e15, and reported a skill of **-3.7e11**. The guard is now
relative, skill is bounded below, and the aggregate is a median so one
pathological cell cannot decide the figure for all of them.

**Windows updates per-CPU counters independently**, so kernel-minus-idle can
transiently underflow and saturate, making a derived total non-monotonic while
every raw counter is not. Found as a test that passed alone and failed under
parallel load.

A fifth, in the harness itself: the workloads are stateful, so a checksum
depends on how many warmup iterations preceded it. A reference taken with two
warmups and trials run with six made **every honest trial report itself as
corrupt**.

---

## 9. What is still not real

**The capability atlas.** Its transitions come from synthetic observations. It
has never been fed real actions on this machine.

**The concept-driven action experiment.** The 315 attributable decisions were
made in a simulated substrate.

**Energy.** Neither backend reads package power, so every `Q` here is work per
cycle and says nothing about efficiency in the sense an operator pays for.

**Context switches on Windows.** Not exposed per thread, so the benchmark falls
back to statistical outlier detection alone. That is a real loss of measurement
quality on this platform.

**A model in the loop.** Everything in section 10 was produced with a program
standing in for an agent. Nothing here measures what happens with a real model
attached, and the demo says so where it prints its results.

---

## 10. The product, on this machine

The application, the service and the bridge all run here.

```text
corescout status
  CoreScout 0.3.0 is running.
  Mode suggest.
  55 reflections this run, 0 recurring states, 0 AI actions observed.

corescout computer
  13th Gen Intel(R) Core(TM) i7-13700KF
  16 cores, 24 threads, mixed core types
  86 entities x 9 channels, 216 observed

corescout diagnostics
  Observation costs 70102 ns on average, 521800 at worst.
  That is 0.0280% of one core at the current interval of 250 ms.
```

### An honest correction to a number this project has quoted

Earlier sections report an observation pass at **6.8 microseconds**. That is
`corescout-lab mirror --once` in a tight loop, with warm caches, and it is a true
measurement of that.

The number the product actually pays is **70 microseconds**, measured as the
mean over a service sampling four times a second. The difference is cold cache
between samples. It is ten times larger and it is the honest figure to show
somebody deciding whether to leave this running, so Settings shows that one,
measured on their machine rather than quoted from this one.

### The demonstration

`corescout-demo` builds a real crate with a real trap: a source file generated
from a schema, where a stale generated file compiles cleanly and fails its
test. Same agent, same repository, twice.

```text
                     fresh    experienced
  tasks completed         8              8
  builds run             11              8
  builds failed           3              0
  failure rate          27%             0%
  seconds               8.7            9.5
```

Two things in that are worth not skipping past.

**The experienced run took longer.** Fewer failures, more wall clock, because
the remedy has a cost: it regenerates every time, and regenerating is not free.
Failure rate went to zero and elapsed time went up by 9%. A product that
reported only the first number would be selling something.

**The fresh CoreScout learned during the eight tasks.** Three failures, then
none, because it noticed the association and started suggesting it. That
narrows the gap between the two conditions and it is the correct outcome; a
demonstration tuned to keep the naive condition naive would be a demonstration
of nothing.

What it ended with, after 260 warm-up tasks:

```text
[Verified]  cargo run --bin generate, then cargo build
            cargo build is more reliable here when the first step runs first.
```

Reaching *Verified* took that many occasions because the exploration rate is
small on purpose: every randomised trial is one where CoreScout may withhold
something that would have helped. At thirty tasks it had the correlation and
said so, and nothing more:

```text
[Seen together]  cargo build works better after cargo run --bin generate
                 When the generator ran first, cargo build failed 0% of the
                 time instead of 100%.
[still testing]  2 and 1 randomised trials, 8 needed on each side
```

That is the product refusing to promote something it has not tested, on real
builds, with a real difference sitting right in front of it.

### What the tests establish that the demo does not

`integration/tests/product_loop.rs` runs the same loop with a known ground
truth, through the real MCP server, and asserts the things a single
demonstration cannot:

- a correlation is never promoted, however much of it there is
- randomised assignment on both arms produces a capability, and nothing else does
- CoreScout does sometimes withhold its own suggestion, and less often than it offers it
- an agent that never verifies anything teaches CoreScout nothing false
- what was learned while one model was connected is reachable by the next one
