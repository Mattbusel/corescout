# Ontology

What the system is allowed to assume exists, what it is allowed to discover, and
which of those two categories each name in this codebase belongs to.

This document exists because the project's central risk is not a bug. It is
quietly telling the machine what it is, and then being impressed when it agrees.

---

## 1. The two kinds of name

Every identifier in the mirror is one of two things, and the distinction is
load-bearing.

**Given names** are ours. `cpu/11`, `core/0/5`, `cache/0/L3/unified/0`,
`cpu.frequency.current`, `shares_cache`. We chose them, they encode a human
model of what a computer is, and they are attached to entities and channels as
*annotations*.

**Discovered names** are the machine's. `latent_state_13`, `cluster_4`,
`relation_family_7`, `self_candidate_a`. Nothing in the system knows what these
mean. They are counters attached to structures that recurred often enough to be
worth naming.

The rule:

> A discovered structure keeps its discovered name, permanently. It is never
> renamed to the given name it appears to correspond to.

If `latent_state_3` reliably coincides with what we would call "package thermal
throttling", the interesting fact is *that correspondence*, and it is a finding
to be reported. Renaming the state to `thermal_throttling` destroys the finding
and replaces it with an assumption. It also makes the next such correspondence
undetectable, because the vocabulary now contains our answer.

---

## 2. Structure the mirror imposes

The mirror commits to exactly four things existing. Everything else is an
annotation on top of them.

| commitment | what it means | why it is unavoidable |
|---|---|---|
| **identity** | a thing persists across observations and is the same thing | without it there is no "change", only a new matrix |
| **state** | a thing has numeric variables | this is what a numeric mirror is |
| **relation** | two things can be connected | a graph with no edges is a table |
| **time** | observations are ordered and timestamped | without it there is no before or after |

`G(t) = (V, E, X(t))` is that and nothing more.

Notably absent from the list: core, cache, NUMA node, package, temperature,
frequency, hertz, watt. Those all exist in the mirror, and none of them is
*required* by it. An entity is a row with a stable identity. That it is called
`cpu/11` is a string attached to the row, which `Lens::Unlabelled` withholds and
`Lens::Labelled` shows, and the two-ontology experiment measures whether the
withholding costs anything.

---

## 3. What "unobserved" means

A cell with no value is `NaN` in the state matrix and carries a reason code in
the parallel availability matrix:

| code | meaning | what an operator should do |
|---|---|---|
| `Observed` | the value was read | nothing |
| `NotApplicable` | the measurement does not apply to this entity | nothing; the question was wrong |
| `Unsupported` | this hardware or kernel does not expose it | buy different hardware |
| `Unavailable` | the source exists but returned nothing | investigate |
| `PermissionDenied` | this process lacks the privilege | run it differently |
| `Stale` | the last reading is too old to trust | sample faster |
| `Unknown` | no reason was recorded | fix the sensor |

Collapsing these into "missing" throws away the only part of a gap that is
useful. `Unsupported` and `PermissionDenied` are different facts about the world
and lead to different actions.

**A gap is never filled with a plausible value.** A zero in a power channel would
propagate silently into every model downstream, and there would be no way, later,
to distinguish a machine that drew no power from a machine that would not say.

---

## 4. What the mirror is not

`M(t)` is not any of the following, and the type system is arranged so it cannot
accidentally become them:

| not a... | because | where that lives instead |
|---|---|---|
| recommendation | it describes, it does not advise | `analysis`, `intent` |
| prediction | it is the present, not the future | `selfmodel`, `counterfactual` |
| historical summary | no averages, no rates, no EWMAs | `memory` |
| benchmark result | nothing in it was produced by perturbation | `experiment` |

The sharpest case is `/proc/loadavg`. The load averages on that line are parsed
and **deliberately discarded**: they are exponentially weighted moving averages,
which is history. The `runnable/total` field on the same line is instantaneous,
and is published. The test is not where a number came from. It is whether it is a
fact about the present.

The same rule makes the mirror publish cumulative counters and never rates. A
rate is a difference between two observations, and differencing is memory's job.
A consumer that wants a rate has a `Transition` for it.

---

## 5. Given vocabulary, in full

Everything below is ours, and could be wrong. It is listed so that it can be
argued with.

### Entity classes

`Cpu`, `Core`, `Cache`, `NumaNode`, `Package`, `ThermalZone`, `PowerDomain`.

These follow the shape Linux exposes under `/sys/devices/system/cpu`, which
follows the shape x86 firmware reports, which follows a particular history of
how processors were built. On a machine organised differently, this vocabulary
would be a poor fit, and the mirror would still work: the entities would still
have identity, state and relations.

### Relation kinds

`SiblingOf` (SMT), `ContainedIn`, `SharesCache`, `SameNumaNode`, `SamePackage`,
`ThermallyCoupled`.

Every one of these is a claim we are making about the hardware based on what
firmware told us. `ThermallyCoupled` is the weakest: two cores on one die are
thermally coupled in a way no table reports precisely, and the edge we draw is
an approximation of a continuous physical reality.

The observer experiment asks whether these edges *earn their place*: does an
observer that is shown them predict better than one that is not? If not, we have
been decorating.

### Channel keys

`cpu.frequency.current`, `cpu.time.idle`, `cpu.irq.count`, `package.power.uw`,
and the rest. Each is documented with the physical fact it approximates, its
sampling cost, its uncertainty, and whether observing it perturbs the system.

The word "approximates" is doing real work. `cpu.frequency.current` is not the
core's clock rate. It is what `scaling_cur_freq` reported, which on many drivers
is the last frequency *requested*, sampled at a moment, for a core whose actual
clock changed several times during the read.

---

## 6. Discovered vocabulary, in full

| name | produced by | what it is |
|---|---|---|
| `latent_state_N` | `represent::latent` | a region of state space the machine returns to |
| `cluster_N` | `represent::discover` | entities that behave alike |
| `relation_family_N` | `represent::discover` | edges that predict behaviour similarly |
| `self_candidate_a` | `identity::boundary` | a hypothesis about where the machine ends |

None of these is a conclusion. `self_candidate_a` is the most obviously
provisional: "is RAM part of the self" is an experimental question, and the crate
is the apparatus, not the answer. Candidates are constructed along four axes of
evidence (coupling, controllability, persistence, observability) and the crate
declines to pick between them.

---

## 7. The experiment this ontology exists to enable

> Does our ontology help the machine understand itself, or does it constrain the
> machine's understanding of itself?

`corescout observe --both` runs two observers over identical reflections.
Observer A sees the vocabulary in section 5. Observer B sees `entity_7`, `x2`,
`edge_type_2`, and time. They are compared on prediction accuracy, compression,
what structure each discovered, and how fast each adapted.

Three outcomes, all interesting:

1. **A beats B.** Our ontology carries real information the numbers do not, and
   the labels are earning their place.
2. **They tie.** The structure is in the data; our names are a convenience for
   people and nothing more. This is the result we half expect.
3. **B beats A.** Our categories are actively misleading, grouping things the
   machine's behaviour says do not belong together. This would be the most
   valuable result and the most uncomfortable one.

The experiment is set up so that (3) is reachable. That is the point of running
it.

---

## 8. Rules, restated as rules

1. A discovered structure keeps its discovered name.
2. An unobserved cell states why, and is never given a plausible value.
3. The mirror publishes the present. Rates, averages and histories belong to
   memory.
4. Entity classes, relation kinds and channel keys are annotations. Nothing in
   the mirror's structure requires them.
5. A proposition about the machine carries the evidence it rests on, or is not
   made.
6. The machine makes no claim about its own experience, awareness, or
   intelligence, because nothing in the mirror could ground one.
