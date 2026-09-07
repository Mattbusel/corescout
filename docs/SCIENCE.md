# The machine as a scientist of itself

What is implemented, what it measured, and what it does not do.

---

## 1. The threshold

> The system constructs a concept that was not explicitly supplied by its
> designers, uses that concept to predict its own behavior, and acts
> successfully because of it.

Three clauses. Each can fail on its own, so each is enforced somewhere
different:

| clause | enforced by | test |
|---|---|---|
| not supplied by designers | `concept::Origin` traces every concept to a discovery | `clause_one_the_concept_was_not_supplied_by_its_designers` |
| used to predict | `CoinageRules::min_utility` gates promotion on measured predictive utility | `clause_two_the_concept_predicts_the_machines_own_behaviour` |
| acts successfully because of it | `VirtualResource::reliability` over recorded attempts | not yet met on real hardware |

The middle clause is the load-bearing one. Without it, coining a concept is
renaming a cluster, and renaming is what the whole project is trying to stop
doing.

---

## 2. What "science" means here, mechanically

The discovery layer finds structure. The self-model predicts. **Neither can be
wrong** in the way that matters, because neither commits to anything in advance:
a predictor that misses simply updates its coefficients.

The science crate commits first.

```text
observation -> hypothesis -> prediction -> experiment -> falsification
                    ^                                          |
                    +------------- revised theory -------------+
```

A `Hypothesis` **cannot be constructed** unless some possible observation would
refute it. `Hypothesis::new` returns `None` otherwise. Three ways to fail that
check:

- a non-finite value or tolerance, which cannot be compared to anything;
- a tolerance wider than the ordinary variation of the quantity, so no plausible
  observation lands outside it;
- a tolerance of zero, which is refuted by arithmetic rather than by evidence.

That is the Popper criterion made mechanical. It is the entire difference
between this and the curiosity module it grew out of:

```text
curiosity:   "try this and see"       -> cannot be refuted
hypothesis:  "this will do X, +/- t"  -> refuted if it does not
```

### The number to watch is the refutation rate

`Theory` keeps a graveyard. A theory full of supported claims is *not* evidence
of understanding: it is equally consistent with a generator that only makes safe
statements, which is the easier thing to build and the harder thing to notice.

`Theory::render` says so out loud when the rate falls below 5%:

```text
0% of settled claims were wrong  (suspiciously low: is the generator only
making safe claims?)
```

---

## 3. Coining a concept

A latent state is cheap. `latent_state_13` is created for any region of state
space visited more than once, and most of them are worthless.

A `Concept` is a latent state that earned promotion:

| requirement | default | why |
|---|---|---|
| recurs | 30 occurrences | a thing seen once is an event, not a category |
| predicts | utility >= 0.05 | **the load-bearing rule** |
| confident | 0.6 | not a handful of coincidental reflections |

Predictive utility is **measured by the self-model and passed in**. The registry
cannot compute it. A registry that could invent its own justification for
coining would coin everything.

Concepts are also **de-coined**. `Registry::audit` retires a concept whose
utility has decayed below `retire_below`, or whose hypotheses were mostly
refuted. Without that, the ontology grows until every reflection is in a
category of its own, and it will have explained nothing while appearing to
explain everything.

### The naming rule

`concept_7` stays `concept_7`. If it turns out to coincide with what we call
thermal throttling, that coincidence is a **finding**, recorded as
`Concept::resembles`, which is a note for humans and is read by no decision.

Renaming it would destroy the finding and replace it with our assumption. It
would also make the *next* such correspondence undetectable, because the
vocabulary would then contain our answer.

---

## 4. The experiment, and what it measured

`integration/tests/concept_threshold.rs`.

A synthetic machine is given a **hidden regime**: entities 2 and 5 elevated on
channel 0 *and* depressed on channel 1, simultaneously. When it holds, another
channel goes quiet. The conjunction is not any single variable, is not a
channel, is not an entity, and appears nowhere in the mirror's vocabulary.
Entity keys are `thing/0`..`thing/5`; channels are `x0`..`x3`; class hints are
`Unclassified`. Nothing in the pipeline is told the regime exists.

Then the identical pipeline is run on a machine with **no hidden regime**, where
the same channels vary independently.

Measured, at seed `0xC0FFEE`, 600 reflections:

| | hidden regime | control |
|---|---|---|
| latent states discovered | 16 | 16 |
| prediction error, no state knowledge | 2.1599 | 0.9455 |
| prediction error, conditioned on state | 0.4665 | 0.8627 |
| improvement | **78.4%** | **8.8%** |
| concepts coined | **7** | **0** |
| candidates refused | 9 | 16 |

Both machines discover 16 latent states. Clustering always finds something. The
discrimination is not in the clustering, it is in the coinage bar: on the
machine with a real regime the states pay their way, and on the control they do
not.

The control's residual 8.8% is 16 clusters overfitting 600 points, and it is
below the 20% bar used in that experiment. That margin is the experiment's
safety factor and it is not large. On a machine with more channels or fewer
frames, the control would score higher and the bar would need to move.

### What this establishes and what it does not

**It establishes** that the mechanism works: a conjunction that exists is found,
earns promotion, and can be used; and one that does not exist is not invented.
That is a real result about the code.

**It does not establish** that real hardware contains such regimes, or that any
regime survives into the mirror at 10 Hz, or that acting on a concept improves
anything on a real machine. Those are open, and no test in this repository
speaks to them.

---

## 5. Virtual resources

A `VirtualResource` is a concept, plus a recipe for producing it, plus a record
of how often the recipe works.

```text
concept_31   "a configuration I recognise"
   + recipe  "these actions put me into it"
   + record  "which worked 84% of the time, over 51 attempts"
   = v1      "a thing an application can ask to run on"
```

Nobody manufactured `v1`. It is not a component on the die. It is a recurring
configuration the machine found, learned to reproduce, and can now offer.

Two rules keep this honest:

**Reliability is `None` until there is evidence.** Three successes is not 100%
reliability, and reporting it as such is the exact wishful implementation the
type exists to prevent.

**A resource that stops keeping its promise is withdrawn.** A resource offered
but rarely achieved is *worse* than no resource, because a caller has arranged
its work around a guarantee nobody is keeping.

`corescout resources` currently reports an empty catalogue on a recording, and
says why: a recipe requires having acted, and reading a trace involves no
acting. That is the honest state, not a stub.

---

## 6. Inventing a move

`agency::invention`. When the machine wants to reach a target and no single
primitive reaches it, it can compose primitives into a `Compound` and, if that
compound reliably works, add it to what it can do.

```text
"I cannot produce concept_19 with any one action I have."
     -> search combinations of the actions I do have
     -> test the promising one
     -> it works 84% of the time
     -> concept_19 is now something I can bring about on purpose
```

### Why composition and not code generation

Section 8 of the design imagines the machine writing a scheduling policy, a
memory allocator, or an execution shim. **This is not that**, and the difference
is worth stating rather than blurring.

Composition is the form of invention that can be made safe here. Every compound
is built from primitives that are already scoped, bounded, rate-limited,
reversible and audited, so a compound inherits all of that by construction: *it
cannot exceed the authority of its parts*. A generated allocator would inherit
none of it, and the audit trail would be describing code no reviewer had seen.

So this is genuinely a new move, and genuinely a restricted kind of new move.

A compound is also **its own effect family**, not the concatenation of its
parts'. The point of a compound is that it does something its parts do not do
separately; attributing its effects to `affinity` would credit the wrong
mechanism.

---

## 7. Sharing a concept between machines

`concept::Signature`. A concept expressed as **dimensionless relational facts**:

```text
participation       fraction of this machine's entities involved
distinctiveness     fraction of variables distinctively displaced
coherence           how tight the configuration is
relative_dwell      multiple of this machine's own median state dwell
recurrence          fraction of observations
controllability     how strongly this machine's actions move it
predictive_utility  how much knowing it improves prediction
```

No units. No channel names. No entity indices. A test asserts the serialised
form contains none of `cpu`, `core`, `cache`, `channel`, `entity`, `row`, `col`,
`ns`.

Every feature is normalised **against the machine itself**, which is the only
reason a similarity between an x86 concept and an ARM concept could mean
anything. A state lasting 40 ms where states typically last 10 ms is the same
*kind* of thing as one lasting 4 s where they last a second, and the type says
so.

`find_analogue` lets machine B ask: *do I have anything shaped like this?* If it
finds one, the two machines have not exchanged a setting. They have exchanged a
concept, and each holds it in its own body.

`coin_by_analogy` applies **the same bar**. Being told a concept exists
elsewhere is not evidence it exists here, and adopting one on a foreign
machine's word is the mistake this whole design is arranged to avoid.

A strong match requires 0.85 similarity. With seven features and enough
concepts, something always looks a bit similar; the bar is deliberately high and
a match is a **hypothesis**, not an identification.

---

## 8. What is not built

Stated plainly, because the gap between this and the full trajectory is large
and interesting.

**The science does not drive the acting.** `corescout theory` and
`corescout concepts` run the conjecture-and-refutation cycle on demand. The
autonomous loop does not yet run it continuously and does not yet choose actions
because of a coined concept. The coupling between the science and the agency is
the thinnest part of the system.

**No software is in the mirror.** Sections 5 and 6 of the trajectory require
reflecting running code, memory structures, execution graphs and workload
alongside hardware. The mirror reflects hardware only. Nothing in
`MirrorSnapshot` prevents software entities, and nothing has added them.

**Nothing rewrites `foo()`.** Section 6 needs compiler integration and a codegen
search. Building the interface without the search would be a mock, and a mock
here would be indistinguishable from the real thing in a demo, which is the
worst property a mock can have.

**Nothing selects a desired future self.** Section 7's inversion, from choosing
an action to choosing a state and then finding actions that reach it, is
buildable on what exists: `EffectModel` can imagine, and `Recipe` can express
the route. It is not built.

**No self-authored abstraction layer.** Section 4. Not reachable from here.

**Nothing has been run on two physically different machines.** The exchange
mechanism is tested between synthetic machines of different sizes and
timescales. Whether an AMD concept and an ARM concept ever match is the open
question, and the apparatus is ready for it.
