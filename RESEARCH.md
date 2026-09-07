# Research questions

These are engineering questions, not a manifesto. Each one is here because it
has a concrete effect on what gets built next, and most of them are already
constraining a decision somewhere in the code. Where the current implementation
has taken a position, it is stated, along with what would change our mind.

---

## 1. Representation

### What is the minimal representation required for a computer to model itself?

**Current position.** `G(t) = (V, E, X(t))`: entities with persistent identity,
typed edges, and a dense `entities x channels` matrix of `f64` with `NaN` for
unobserved. Roughly 70 entities and 40 channels on a desktop, about 25 KB.

**What we do not know.** Whether the matrix is the right shape at all. It is
dense because dense is simple and vectorises, but most cells are permanently
empty: a thermal zone has no idle residency, a cache has no interrupt count. On
a large machine the emptiness dominates. A sparse or per-class layout would be
smaller and would reintroduce exactly the class-based structure the design is
trying to avoid.

**What would settle it.** Build the memory layer, store an hour of snapshots,
and measure how much of the matrix ever takes a value. If it is under 20%,
density is costing more than it buys.

### Should the mirror include processes, or only physical substrate?

**Current position.** Physical substrate only. Processes appear in the mirror
solely as aggregate pressure on hardware entities: runnable task count, CPU time
by class, interrupt counts.

**The argument for keeping them out.** Processes are the machine's *occupants*,
not the machine. They arrive and leave thousands of times a minute, which makes
persistent identity nearly meaningless for them, and including them would grow
the entity set by orders of magnitude and make every snapshot a different shape.

**The argument for letting them in.** An agent that wants to place a thread must
reason about the thread. If the mirror cannot represent it, the agent has to
build a second, unmirrored model of it, and the two will disagree.

**Probably the answer.** A separate plane with different lifetime semantics, not
a bigger `M(t)`. Worth revisiting before the agency layer grows.

### Are processes part of the machine, or occupants of the machine?

The question above, asked honestly. A virtual machine is a process that believes
it is a machine. A container is a process that has been told it is a machine. If
the boundary is "what the kernel schedules on hardware", then a VM's vCPU thread
is an occupant here and substrate one level up, and the answer depends entirely
on which side of the virtualisation boundary you are standing on. The mirror
currently reflects whichever machine the kernel it is running on presents, and
inherits that kernel's lies without being able to detect them.

### Where does the boundary of the computational self end?

Currently: CPUs, caches, NUMA nodes, thermal zones, power domains. That boundary
was chosen by what CoreScout already knew how to read, which is not a principle.

- **Is RAM part of the self?** Its topology is (NUMA nodes are entities); its
  contents are not, and probably should never be. Memory bandwidth and
  controller pressure are observable and are not yet observed.
- **Is an attached accelerator part of the self?** A GPU has its own cores, its
  own thermal domains, its own power limits and its own scheduler. Every
  argument for including a CPU core applies to it. The honest answer is that it
  is excluded because nobody has written the sensor.
- **Is a network interface part of the self?** It has queues, interrupt
  affinity, and a very direct effect on tail latency. The interrupt sensor
  already observes its *shadow* without representing it.
- **Is another machine in the cluster part of the self?** At some distance the
  answer becomes obviously no, but no principled threshold has been identified.
  "What this kernel schedules" is a boundary of convenience.

### What information should be directly represented versus learned?

**Current position.** Anything the hardware or kernel states directly is
represented. Anything requiring two observations, an assumption, or a judgement
is left to be learned. That is why cumulative counters are published and rates
are not.

**The interesting case.** Cache sharing is *stated* by sysfs, so it is an edge.
But "these two cores interfere with each other under load" is a much more useful
fact, is not stated anywhere, and is learnable from correlated state. If a model
can learn interference from behaviour, is publishing the sysfs-stated sharing
edge helping it or biasing it toward the human abstraction?

---

## 2. Observation

### How much does observation perturb the machine?

Not a rhetorical question: the mirror measures it. Every sensor declares a
perturbation class and the collector publishes what each pass actually cost.

Known cases, in increasing order of awkwardness:

- **Reading `scaling_cur_freq` on `acpi-cpufreq`** sends an IPI to the observed
  core to read `APERF`/`MPERF`. Observing a core's frequency makes that core do
  less work.
- **Reading a `coretemp` sensor** is an `rdmsr` on the target core, also by IPI.
  Polling temperature at high rates prevents cores from reaching the deep idle
  states whose thermal effect is being measured.
- **Holding PMU counters** does not disturb the hardware at all, and degrades
  every *other* profiler on the machine into multiplexed estimates. The
  perturbation is to the observability of the system, not to its physics, and
  the current type system cannot express that difference.
- **Reading `/proc/interrupts`** costs `O(sources x cpus)` of kernel formatting,
  charged entirely to the reader. On a large server this is the most expensive
  thing the mirror does.

**Open.** No measurement yet of the *aggregate* effect: does a mirror ticking at
100 Hz measurably change the machine it reflects? The apparatus to answer this
exists, which is a pleasing property of a project that contains both a mirror
and a benchmark: run the benchmark with the mirror stopped, then running, and
compare the distributions. Nobody has done it.

### How accurately can a computational system observe itself without materially altering the state it is trying to observe?

The general form of the above, and the question the `Perturbation` type exists to
keep visible. There is presumably a frontier: for each quantity, a maximum
sampling rate beyond which the observation dominates the signal. `max_rate_hz` is
currently a documented estimate per sensor, not a measured bound.

### Can the mirror detect that it is being observed too hard?

It publishes its own cost, so a *consumer* can notice. Nothing closes that loop
today. A mirror that reduced its own sampling rate when it noticed it was
perturbing the machine would be the smallest interesting instance of
self-regulation in this system, and would also be the first time the mirror did
something other than reflect, which may make it no longer a mirror.

---

## 3. Identity and continuity

### How should identity persist across hotplugging, virtualisation, migration, or changing hardware?

**Current position.** An entity's id is a hash of a natural key describing its
position: `cpu/11`, `core/0/5`, `cache/0/L3/unified/0`. Derivable without
coordination, stable across restarts, stable across state change.

**Where it breaks, and these are not hypothetical:**

- Offlining the CPU that anchors a cache changes that cache's key, so the entity
  is replaced rather than updated. The epoch counter makes the discontinuity
  visible; it does not repair it.
- Under a hypervisor, `cpu/11` may be a different physical core minute to
  minute. Every id remains stable while the thing it names is substituted
  underneath. The mirror cannot detect this and does not claim to.
- Live-migrating a VM preserves every key and replaces the entire machine.
- Two sockets of identical parts have distinguishable keys only because Linux
  reports a package id. Where firmware gets that wrong, two cores collide.

**What a better answer might look like.** Identity by *observed behaviour* rather
than by position: an entity is the same entity if it continues to behave like
itself. That is circular for a mirror, which must assign identity before any
behaviour has been observed, but might work as a repair mechanism at the memory
layer, which has the history to make such a judgement.

### When is an entity dead rather than merely unobserved?

An offlined CPU produces no observations. So does a CPU whose sensor lost
permission, and so does one whose sysfs node vanished in a race. The mirror
publishes `NaN` for all three and cannot distinguish them. The epoch mechanism
covers the first case only when discovery notices, which it does at rebuild time
and not before.

---

## 4. Learning

### Does a global common mode hide local structure, and what should be done about it?

**Answered by running it, and it changed the code.** The first observer reported
one cluster containing every CPU at cohesion 0.84, because the synthetic
machine's workload phase moved everything together. The SMT signal was present
and buried: lift +0.16 against a baseline of +0.84.

Subtracting the cross-entity mean per variable per instant, common average
referencing, dropped the baseline to -0.14 and raised the SMT lift to +1.11.

The open part is what the common mode *is*. Right now it is measured and removed
as a nuisance, and it plainly is not one: "how much of this machine is currently
moving as one thing" is a real, useful, workload-level fact. It probably deserves
to be a first-class finding rather than a subtraction, and possibly to be
decomposed further, since a thermal envelope and a synchronous workload produce
different common modes with different time constants.

### Can a machine discover better hardware abstractions than those exposed by the operating system?

This is the project's central bet, and the representation is shaped around
keeping it possible: uniform entities, typed edges, a dense numeric matrix, and
human category names demoted to annotations a consumer may ignore.

**The test.** A consumer reading only the plane should be able to recover
structure nobody told it about. The weak version is recovering the known
structure: cluster the columns and find that SMT siblings correlate more than
unrelated cores. The strong version is finding a grouping that predicts something
and does not correspond to any label. Neither has been attempted.

### Can useful latent machine states emerge that humans did not name?

The `Z_41` question. Suppose some recurring configuration of thermal
distribution, cache occupancy, SMT activity, interrupt placement and frequency
trajectory reliably precedes low jitter. It has no name, appears in no
documentation, and would be immediately useful.

**What has to be true first.** The memory layer must exist, the mirror must be
rich enough to contain the signal, and the sampling rate must be high enough to
catch the transition. None of these is established. The most likely failure is
not that no such states exist but that the mirror is too coarse to see them: at
100 Hz, with tick-quantised CPU time, most microarchitectural events are already
averaged away before they reach the matrix.

### Can a machine learn stable concepts about its own physical behaviour?

Stability is the hard part. A concept learned on an idle machine may not survive
contact with a loaded one; a concept learned in winter may not survive summer
ambient temperatures. Any latent state worth having has to be shown to persist
across conditions, which requires a memory layer measured in days rather than the
seconds a snapshot ring would hold.

### Can self-observation improve execution without explicit human scheduling rules?

The original CoreScout answered a much smaller version of this with "sometimes,
by a few percent, on some machines". The larger version stays open, and the
honest prior is that the gains are concentrated in exactly the specialised
workloads the old README listed and are close to zero everywhere else.

---

## 5. Utility

### When does a hardware self-model become more useful than traditional telemetry?

Telemetry is built for humans reading dashboards after the fact: sampled at 1 to
15 second intervals, aggregated, labelled, and stored as text. A self-model is
for software acting on the present: sampled in milliseconds, unaggregated,
numeric, and read as memory.

**The plausible boundary.** When the consumer is software, when the decision
horizon is shorter than a human reaction time, and when what matters is the
relationship between quantities rather than any single one. Every case where a
person is going to look at a graph is a case where Prometheus is the better
answer, and this project should not pretend otherwise.

### Is the mirror rich enough that another system can learn meaningful things from it?

**Partially answered.** `mirror-observer` consumes only the plane and, on a
synthetic machine, recovers the SMT pairing, distinguishes a behaviourally real
edge type from a vacuous one, identifies which variables are counters without
being told, finds the recurring whole-machine regimes, and beats a persistence
baseline at predicting the next reflection.

What that establishes: the representation can carry this class of structure from
producer to consumer intact, and the discovery pipeline finds present structure
without finding absent structure.

What it does not establish: anything about real hardware. The simulator makes SMT
siblings covary by construction. **The open half of this question is one hour of
`corescout-lab mirror --record` on a real machine.**

Two things that will be different there, and are worth predicting in advance so
the result can be a surprise:

- Real coupling will be weaker and intermittent. SMT siblings interfere strongly
  only when both are busy; at idle they are independent, so cohesion should vary
  with load rather than being a constant.
- The mirror samples at 10-100 Hz while the interesting interactions happen at
  microsecond scale. The most likely negative result is not "no structure" but
  "the structure is averaged away before it reaches the matrix", which would be
  an argument for a faster path, not a richer one.

---

## 6. Questions the implementation has already answered

Recorded because they were open at the start and are not any more.

**Should the canonical representation be JSON?** No. It was, in the original
CoreScout, and that made a text format with human-chosen field names the
authoritative artifact. The canonical form is now the binary matrix; JSON is a
projection.

**Can observation and interpretation share a module?** No. They did, and the
result was that reading a frequency and deciding a core was "best" happened in
the same pass, with no way to use one without the other.

**Should the mirror include the raw distributions the benchmark produced?** No.
They are historical summaries. Realising that `p99` cannot live in `M(t)` was
what forced the mirror/memory split to be real rather than nominal.

**Is `NaN` for unobserved worth the trouble?** Yes, and it caused two real bugs
during implementation: JSON cannot represent it, and `NaN != NaN` silently broke
structural equality everywhere. Both are now handled explicitly. The alternative,
using zero, would have made "this core has no thermal sensor" and "this core is
at 0 C" the same fact.

**Does giving a learner our vocabulary help it?** On the machine tested, no, and
it did not hurt either. Two observers over identical data, one seeing
`cpu/3`/`smt_sibling`/`Celsius` and one seeing `entity_7`/`edge_type_2`/no units,
produced identical groupings, identical edge lifts and identical prediction
skill. The only label with any operational content was `Semantics::Cumulative`,
and the unlabelled observer recovered it exactly by noticing that counters never
decrease.

That is a weaker answer than either hoped-for result. It says our ontology is
currently *inert* rather than either helpful or constraining, which is at least
the right property for something meant to be an annotation. It would become
interesting again with a learner sophisticated enough to exploit units and
declared semantics, or on a machine where the labels are wrong.

**Should structure discovery use levels or differences?** Differences. On levels,
two unrelated counters both rising correlate at nearly 1.0, which is true and
carries no information about coupling. This was not a close call once the first
results came back.

---

## 7. Questions opened by the self-modelling runtime

The mirror, memory, self-model, counterfactual, intent, agency and autonomy
layers are now implemented. These are the questions that raises, none of which
were answerable before there was something to run.

### 7.1 Does modelling itself help the machine act better than not modelling itself?

The whole project reduces to this. The comparison is built
(`corescout experiment compare`), the baselines are chosen to be hard, and the
control is included.

**What would count as an answer.** On at least one workload scenario, the
self-model policy beats `os-scheduler` by more than the run-to-run spread, and
also beats `corescout-ranking`. Beating the scheduler while losing to a static
ranking would mean the *modelling* contributed nothing: the win came from
pinning, which `taskset` does in one line.

**What would refute it.** Losing to `random`, or being inconclusive against
`corescout-ranking` everywhere. Either result is worth publishing and neither is
currently ruled out.

**Status.** Unrun at length. The harness is honest; the table is empty.

### 7.2 Is prediction skill the right internal objective?

The controller's own health signal is whether it predicts its next state better
than assuming nothing changes. On an idle machine that baseline is nearly
unbeatable, which means a well-behaved controller on a quiet machine looks
identical to a broken one.

The watchdog's skill floor is set just above zero for exactly this reason, but
"just above zero" is a guess. Whether prediction skill correlates with *acting
well* is untested, and it is entirely possible to build a system that predicts
itself beautifully and places work badly.

### 7.3 What does the machine learn when its only objective is to reduce its own uncertainty?

The original suggestion, still the sharpest experiment available: give the
controller one actuator (move its own thread between permitted CPUs) and the
objective *minimise uncertainty in predicting your own next state*. Not "be
fast".

`Curiosity` implements the mechanism: it proposes experiments on the action whose
effect is least known, and concludes them against what actually happened. What it
has never been given is a run long enough to see whether the resulting behaviour
looks like investigation.

**The prediction worth recording in advance:** the controller will migrate more
than a performance-seeking one would, will concentrate its migrations early, and
will settle once the effect model's consistency stops improving. If it instead
migrates at a constant rate forever, the curiosity signal is not saturating and
the design is wrong.

### 7.4 Where does the machine decide it ends?

`identity::boundary` produces `self_candidate_a`, `self_candidate_b` and so on
along four axes: coupling, controllability, persistence, observability. It
declines to pick.

The interesting axis is controllability, because it is the one that separates
"correlated with me" from "part of me". A busy neighbour process correlates with
everything because it is *causing* everything. An entity that responds to this
system's own actuators is a candidate for "me" in a way a merely correlated
entity is not.

**Open:** does the boundary the evidence supports match any human intuition about
where the machine ends? Does it include the caches? The thermal zone? A GPU it
never acts on? Nobody has run this long enough to find out.

### 7.5 Can the mirror abstraction survive leaving the CPU?

Nothing in `MirrorSnapshot` mentions a CPU. Entities have identity, state and
relations; `EntityClass` has CPU-shaped variants but the structure does not
require them.

That is the design claim. It is untested, because every sensor written so far
reads a CPU. Adding a RAM, GPU or NIC sensor is the test, and the failure mode to
watch for is the one that will not announce itself: needing to add a field to
`MirrorSnapshot` to accommodate the new device. If the snapshot has to change
shape, the abstraction was CPU-specific and we had not noticed.

### 7.6 Does an ontology the machine discovered stay stable?

`LatentCatalogue` founds states, consolidates ones that converge, and prunes ones
that stop earning their place. Over a long run, does the catalogue settle to a
stable set, or does it churn?

Churn would mean the states are an artifact of the novelty threshold rather than
structure in the machine. Stability would be the first evidence that the machine
has found something real about itself. Both are measurable from a recording, and
neither has been measured.

### 7.7 Is the observer effect large enough to model?

Every sensor declares its cost, and the mirror publishes the cost alongside the
readings. A 100 Hz observation pass over eighty entities is not free, and it
shows up in the very scheduler statistics it is reading.

**Open:** is the mirror visible in its own reflection? If a learner can detect
the observation pass as a periodic signal, that is a genuinely interesting result
about self-observation, and also a confound in every model trained on the data.
The experiment is cheap: vary the interval and look for the corresponding
frequency in the residuals.

### 7.8 What is the right thing to do when the machine changes shape?

A CPU going offline moves the plane to a new epoch, and every consumer is
required to notice. The model resets the affected cells rather than carrying
learned coefficients across a discontinuity.

That is the conservative choice and it is probably too conservative. A machine
that loses one CPU has not become an entirely different machine, and discarding
everything learned about the other seventy-nine is wasteful. But the alternative
requires deciding which knowledge survives an identity change, and nothing in the
current design says how.

---

## 8. Things that would change our mind

Recorded in advance, so they cannot be reinterpreted afterwards.

| finding | what it would mean |
|---|---|
| self-model loses to `random` on most scenarios | the modelling is noise; the project's premise is wrong |
| self-model ties `corescout-ranking` everywhere | a static benchmark is sufficient; the loop is unnecessary complexity |
| unlabelled observer beats labelled | our ontology is actively misleading and should be reconsidered |
| latent catalogue never stabilises | the discovered states are threshold artifacts, not structure |
| the mirror is detectable in its own reflection | self-observation is confounded and every model needs the pass removed |
| adding a non-CPU sensor requires changing `MirrorSnapshot` | the abstraction is CPU-specific and the generality claim is false |
| prediction skill is uncorrelated with acting well | the internal objective is wrong and the watchdog is watching the wrong thing |

None of these is currently ruled out.

---

## 9. Questions opened by the science layer

### 9.1 Is a high refutation rate achievable, or does the generator only make safe claims?

`Theory` warns when fewer than 5% of settled claims were wrong, because a theory
full of supported claims is equally consistent with a generator that never risks
anything.

**Open:** on real reflections, what rate does the current conjecture generator
actually produce? A rate near zero means it is conjecturing the obvious. A rate
near one means the expectations are badly calibrated. Neither has been measured
outside synthetic data.

### 9.2 Where should the coinage bar sit?

The threshold experiment discriminates cleanly at a 20% utility bar: 78.4%
improvement with a hidden regime, 8.8% on the control. That 8.8% is sixteen
clusters overfitting six hundred points.

**Open:** the margin is not large, and it shrinks as channels increase or frames
decrease. Is there a principled bar, derived from the number of states and
observations, rather than a constant that happened to work? A permutation test
against shuffled reflections would give one, and is not implemented.

### 9.3 Do concepts stay coined?

**Open:** over a long run, does the registry settle, or do concepts churn in and
out as their measured utility drifts? Churn would mean the bar is being applied
to a noisy estimate rather than to a stable property. This is measurable from a
recording and has not been measured.

### 9.4 Can a concept be found that no relation in the mirror already encodes?

The observer experiment's honest result was that it rediscovered structure the
mirror carries as edges. The concept layer raises the same question with sharper
teeth: is there a conjunction across entities that is *not* SMT siblinghood,
cache sharing or NUMA co-residency, and that predicts?

**Open, and the most valuable single result available.** The synthetic
experiment plants such a conjunction and finds it, which shows the mechanism can
do it. Whether real hardware has one is unknown.

### 9.5 Does composition actually reach targets single actions cannot?

`Inventory::propose` builds a compound when the best single primitive gets less
than 60% of the way to a target.

**Open:** on a real machine, are there targets in that band at all? It is
possible that placement targets are either reachable with one action or not
reachable at all, in which case invention has no gap to fill and the module is
dead weight. That would be a clean negative result.

### 9.6 Does a signature match survive contact with a genuinely different machine?

Signatures match across synthetic machines of different sizes and timescales,
which is what the normalisation is for.

**Open:** run it on x86 and ARM. If concepts match at 0.85 across architectures
with nothing physical in common, that is a finding about computation rather than
about either machine. If they never match, the seven features are too coarse, or
the concepts really are machine-specific, and distinguishing those two is itself
work.

### 9.7 Is the 0.85 similarity bar defensible?

With seven features and enough concepts, something always looks similar.

**Open:** what is the null distribution of `similarity` between concepts from
two unrelated machines? Until that is measured, 0.85 is a guess chosen to be
conservative, and the honest statement is that a match is a hypothesis rather
than an identification.

---

## 10. Additions to "things that would change our mind"

| finding | what it would mean |
|---|---|
| refutation rate near zero on real data | the generator only makes safe claims; the theory is decorative |
| the control machine coins concepts at a realistic bar | the coinage rule measures noise and the threshold result is void |
| concepts churn instead of settling | the utility estimate is too noisy to gate on |
| every coined concept maps onto an existing mirror relation | nothing was discovered that our ontology did not already contain |
| no placement target sits in the composition band | invention has no gap to fill; the module should be deleted |
| signatures never match across architectures | either the features are too coarse or concepts are irreducibly machine-specific |
| signatures match between *unrelated* machines at 0.85 | the bar is meaningless and every reported analogy is a false positive |

None of these is currently ruled out.

---

## 11. Questions opened by concept-conditioned agency

### 11.1 Is the causal bar worth what it costs?

The four-arm experiment's uncomfortable result: the `latent` arm, which is the
same pipeline with the coinage bar and the causal test removed, is **cheaper**
(4.667 against 6.308) while the world is stable. It acts on correlation
immediately and wins.

It also carries stale actions after the world changes and has no way to de-coin.
It is faster because it is credulous.

**Open, and the most important open question in the project right now.** The
comparison is not yet fair: the environment contains no spurious correlation
that would punish credulity. Build one, and either the causal bar earns its cost
or it does not. Both outcomes are publishable and the second would be more
useful.

### 11.2 What is the right forgetting factor?

Causal arms weight recent trials more, so a comparison becomes unsettled when
evidence stops arriving. The default is 0.995 per trial, giving roughly a
hundred-trial effective window per arm.

**Open:** too slow and a dead belief survives; too fast and nothing is ever
established. The current value was chosen to make an effect establishable within
the experiment's length, which is not a principled derivation. A principled one
would come from the rate at which the machine's causal structure actually
changes, which nobody has measured.

### 11.3 Should exploration ever stop entirely?

A settled comparison is revisited at 15% of the usual rate. Zero would be
cheaper and would make the belief unrevisable; 100% is a permanent tax.

**Open:** is there a principled schedule? The obvious candidate is to scale the
residual rate by how surprising recent outcomes have been, which is implementable
and is not implemented.

### 11.4 How much does exploration cost on real work?

On the synthetic environment the concept agent pays about 0.12 per decision
after the turn, entirely in probing, on a machine where there is nothing left to
find.

**Open:** on a real machine running someone's real workload, what is that in
milliseconds of tail latency? Randomised placement means sometimes deliberately
putting work on a worse core. Until that number exists, the safety argument for
running this on anything that matters is incomplete.

### 11.5 Can a concept be coined whose value is only causal?

Every concept currently coined has to earn promotion by improving *prediction*.
It is possible for a state to be useless for prediction and highly valuable for
action: knowing you are in it tells you nothing about what happens next, but it
tells you which intervention works.

**Open:** the coinage rule would reject such a concept, and the machinery to
notice it does not exist. This may be a real gap in the ontology layer.

---

## 12. Additions to "things that would change our mind"

| finding | what it would mean |
|---|---|
| the latent arm keeps winning once spurious correlations are added | the causal bar is expensive ceremony and should be removed |
| no forgetting factor establishes effects *and* drops dead ones | the estimator is the wrong shape; a change-point detector is needed instead |
| exploration cost on real hardware exceeds the gains it finds | concept-conditioned agency is not deployable, whatever it demonstrates |
| concepts valuable only for action are common | the prediction-based coinage bar is filtering out the useful half |
| attributable decisions never exceed probing noise on real data | the arrow from concept to action does not close outside a synthetic world |

None of these is currently ruled out.

---

## 13. What the credulity traps settled, and what they did not

### Settled

**The causal bar is not free and not always worth it.** In a benign world it
costs 60% more regret than acting on correlation and buys nothing. That is now a
test, not a caveat.

**Belief regret is the discriminating measure.** Total regret nearly hides the
difference the machinery exists to produce. In the sign-reversal world the totals
differ by 14% while false-belief counts differ by 506 to zero.

**Randomisation is not a universal solvent.** The deferred-cost world defeats
both agents. Confounding by an unobserved *state* is what randomised assignment
fixes; confounding by *time* is not, because shuffling which action is taken does
not move which tick its consequence lands on.

### Not settled

### 13.1 What is the base rate of confounded structure in real machine behaviour?

The suite shows the causal bar winning in four of five worlds I designed. That
ratio is meaningless: I chose the worlds. The number that would actually decide
whether to run this is how often real machine behaviour contains the kind of
structure that fools a correlational agent, and nothing measures it.

**Open, and it is now the question that gates deployment.**

### 13.2 Would a better correlational agent close the gap?

The comparison is between epsilon-greedy per-state means and a full causal
apparatus. A correlational agent with change-point detection, or optimism under
uncertainty, or simply a longer exploration schedule, might get most of the
protection without paying for randomisation.

**Open.** If it can, the causal machinery is a expensive way to buy something
cheaper methods also provide.

### 13.3 Can temporal misattribution be fixed at all?

The deferred-cost world is a real limit rather than a tuning problem. Credit
assignment across time is a different problem from confounding, and the current
design has no notion of it: every outcome is attributed to the action that
immediately preceded it.

**Open.** Eligibility traces or an explicit delay model would be the obvious
attempts, and neither is built. Until one is, the honest statement is that this
system can be reliably fooled by any effect whose consequence is not immediate,
which on a real machine includes thermal accumulation, cache pollution and queue
build-up: three of the most important things it would want to reason about.

### 13.4 What does exploration regret cost in real units?

Measured here in arbitrary cost. On a machine it is milliseconds of tail latency
on someone's real work, deliberately incurred. The safety argument is incomplete
until that number exists.

---

## 14. Additions to "things that would change our mind"

| finding | what it would mean |
|---|---|
| real machine behaviour is mostly benign-shaped | the causal bar is a tax with no return and should be off by default |
| a cheaper correlational method matches it on the traps | the apparatus is over-engineered for the protection it provides |
| most real effects are deferred rather than immediate | the whole attribution model is wrong, not merely incomplete |
| exploration regret exceeds any gain on real hardware | not deployable, whatever it demonstrates in simulation |

None of these is currently ruled out.

---

## 15. Questions opened by the capital framing

### 15.1 Does real silicon contain exploitable general structure?

The lineage experiment plants a general optimum and shows a line can compound
toward it. Everything depends on whether a real machine has an analogous thing:
a configuration structure that transfers across workloads and is not already
found by the vendor's defaults.

**Open, and it decides whether any of this matters.** If real machines are
mostly at their optimum already, the ceiling is small and no amount of clever
searching reaches it.

### 15.2 When should a lineage stop searching?

The first run of the experiment declined because every generation kept spending
its search budget after the gains had stopped, while the inherited bill grew.
Correct accounting, badly designed process.

**Open.** Nothing in the crate provides a stopping rule. The obvious candidate
is to stop when the marginal gain of the last search falls below its cost, which
requires estimating a marginal gain that has not been measured yet.

### 15.3 What is a descendant, concretely?

Here a descendant is a configuration vector. That is the weakest possible form
of the idea and it is the form that could be honestly measured.

**Open:** compiling learned experience into an actually rebuilt runtime is a far
larger piece of work, and the gap between a configuration and a runtime is where
most of the difficulty lives. Until that is crossed, "self-modified runtime"
overstates what exists.

### 15.4 Does the gain survive being priced in energy?

The default weights price silicon time only, honestly, because most machines
cannot read their own power. A descendant that is faster and hungrier might be
worse by the measure an operator actually pays.

**Open,** and it needs RAPL on a real machine.

### 15.5 Does a lineage's rate of improvement itself improve?

The interesting version of the claim is `dQ/dt` rising, not just `Q`. In the
experiment each generation gains roughly as much as the last, which is what hill
climbing on a fixed landscape produces.

**Open:** a lineage whose accumulated atlas made its *searching* cheaper would
show acceleration. Nothing currently connects the atlas to the search.

---

## 16. Additions to "things that would change our mind"

| finding | what it would mean |
|---|---|
| real machines sit near their configuration optimum already | the ceiling is small; the process cannot pay for itself |
| gains vanish once energy is priced | the descendant is faster and more expensive, which operators will not want |
| no stopping rule beats "search once and stop" | the compounding story collapses to a single one-off tuning pass |
| held-out gains never materialise on real workloads | the process learns benchmarks, which is the failure the harness was built to catch |
| the rate of improvement is flat across many generations | it is tuning, not capital formation, and the recursive framing is wrong |

None of these is currently ruled out.
