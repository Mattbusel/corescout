# Concept-conditioned agency

Closing the arrow `Concept -> Action`, and what building it exposed.

---

## 1. The milestone

> The machine performs an action that it would not have performed without a
> concept it discovered about itself.

Not "a concept exists". Not "a concept predicts". Not "a concept corresponds to
a hidden regime". Those were the previous thresholds and they are all satisfied
by a system that never does anything.

The claim is measurable because [`Choice`](../crates/autonomy/src/conditioned.rs)
carries `counterfactual`: what the same policy would have chosen, at the same
moment, knowing nothing about concepts. `Choice::attributable()` is true exactly
when the two differ. It is a count, not an assertion.

---

## 2. The rule that makes it hard

The tempting implementation notices that outcomes are better under `C` when `A`
was taken, and prefers `A`. That is the reasoning the whole thing has to refuse:

```text
I acted while concept C was present.
Things improved.
Therefore C caused the improvement.
```

`A` was taken *because* of something, and the something may be what produced the
outcome. So there are two claim classes and they are not interchangeable:

| claim | evidence that settles it |
|---|---|
| `C predicts X` | passive observation |
| `under C, action A causes X` | randomised assignment between A and A' |

The separation is structural. `CausalEstimate::effect()` returns `Err` unless
there are randomised trials on **both** arms; observational data can only
produce `association()`, a different method with a different name returning a
different type. A caller cannot reach for one and get the other by accident.

`Insufficient::NoRandomisation` says why, in words:

```text
500 trials, none randomised: the action was always chosen for a reason, so its
outcome cannot be separated from that reason
```

### The price

Randomisation means sometimes deliberately doing what you believe is worse. That
cost is real, is paid on real work, and shows up in every number below.

---

## 3. The experiment

`integration/tests/concept_driven_action.rs`. A synthetic substrate where the
best action depends on a **relational hidden regime that is not an input
variable**:

```text
regime holds when:  thing/1 . x0  is elevated
              AND   thing/2 . x1  is depressed

under the regime:   a3 is much better than anything else
otherwise:          every action is alike
```

Each half of the conjunction also appears alone in about 70% of non-regime
frames, so neither variable separates anything by itself. There is no channel
called `regime`.

Halfway through, the regime stops occurring **and `a3` becomes the worst action
available**. An agent that keeps its belief is now punished for it. That is what
makes de-coinage worth something rather than merely tidy.

### Four arms, identical environment

| arm | given |
|---|---|
| baseline | the raw mirror, no state inference |
| latent | clustering, no coinage bar, no causal test |
| concept | coined concepts, causal attribution, de-coinage |
| oracle | the hidden regime, directly |

### Measured, seed `0xC0FFEE`, 18000 ticks, turn at 9000

```text
arm           cost bef    cost aft   right   r-blf   stale   s-blf  attrib
baseline         6.499       5.498       0       0       0       0       0
latent           4.667       5.604    2356       0     240       0       0
concept          6.308       5.620     246     193     274      10     315
oracle           4.167       5.498    2999       0       0       0       0
```

- `right` / `stale`: took `a3` while the regime held / after it turned harmful
- `r-blf` / `s-blf`: the same, **restricted to belief rather than probing**
- `attrib`: decisions a self-coined concept changed

### What it shows

**The milestone is crossed.** `attrib = 315`. A concept the machine coined about
itself changed what it did, 315 times, and the counterfactual is recorded at
each one.

**The belief is correct, not lucky.** 193 belief-driven right actions against 10
belief-driven wrong ones after the turn. A 19:1 ratio; the test requires 10:1.

**The lifecycle completes.** Concepts are retired when their regime stops
occurring, preferences are withdrawn, and behaviour reverts. `s-blf = 10` out of
`stale = 274`: almost every dead-action choice after the turn was a probe, not a
belief.

### What it also shows, and this is the uncomfortable part

**The `latent` arm is cheaper.** 4.667 against the concept arm's 6.308, and much
closer to the oracle's 4.167.

That arm is the same pipeline with the two rules this project added taken out:
no coinage bar, no causal test. It acts on correlation, immediately, and it wins
on cost while the world is stable.

It also carries 240 stale actions after the turn and has no mechanism to
de-coin. It is faster because it is credulous, and the environment does not
punish credulity hard enough to show it.

**The honest summary is that the causal bar bought correctness and cost speed,
and on this environment the trade was not obviously worth it.** A fair
comparison needs an environment with spurious correlations that the latent arm
would act on and be wrong about. That environment is not built, and until it is,
`latent` beating `concept` on cost is a real result rather than a detail.

---

## 4. Two defects the experiment exposed

Both were found by the experiment failing, not by review. Both are about the
same thing: **evidence that only accumulates cannot describe a world that
changes.**

### Utility measured over all history never falls

Concept utility is measured over accumulated evidence. A concept whose
conditions have stopped occurring keeps every observation that ever supported it,
so its measured utility never falls, `audit()` never retires it, and it drives
behaviour forever.

Fixed by `Registry::decay_unseen`: standing halves for every half-life of not
being seen, and the ordinary audit does the retiring. A concept is a claim about
the machine *as it now is*, so absence is evidence against it.

This will eventually retire a genuinely rare concept that is real. That is the
intended trade: a rare concept can be re-coined next time it occurs, whereas a
stale one that is never questioned drives behaviour forever.

### Preferences were installed but never withdrawn

`consolidate()` added a preference when an effect was established and had no path
to remove one whose effect had reversed. A one-way door: a preference established
under conditions that have since changed would drive behaviour forever, and the
machine would look like it had learned something when it had merely been trained.

Fixed by withdrawing preferences the evidence no longer supports, and by making
the evidence itself forgetful. `Arm::record_with` weights recent trials more, so
the effective sample size falls when evidence stops arriving, a comparison
becomes *unsettled* again, and exploration resumes on its own.

Without forgetting, history outvotes the present. There is a test that asserts
exactly that, by turning forgetting off and watching the stale belief survive.

### One design choice worth flagging

A settled comparison is revisited at 15% of the usual rate, not 0%. Stopping
entirely would be cheaper and is wrong: the world can change while the condition
persists, and a comparison never revisited would keep driving behaviour from
evidence about a machine that no longer exists. The residual rate is the cost of
staying able to notice.

---

## 5. A third defect, found earlier

`ConditionedPolicy::choose` originally randomised only *after* a preference
existed, but a preference could only come from randomised evidence. A deadlock:
the concept arm was byte-identical to the baseline because it could never
bootstrap.

Fixed by probing when a condition is recognised but nothing is established. The
un-randomised choice in that state is still the concept-blind one, so an unproven
condition changes nothing.

Separately, `observe()` recomputed the comparison pair independently of
`choose()`, so the two could disagree and a trial could be filed against a
comparison that never happened. The pair now travels on the `Choice`.

---

## 6. What this is not

**The science still does not run inside the loop.** `corescout theory` and
`corescout concepts` run the conjecture-and-refutation cycle on demand.
`ConditionedPolicy` consumes concepts but nothing yet drives the whole chain
continuously on a live machine.

**Nothing here has run on real hardware.** The environment is synthetic, the
regime is planted, and the result is about the code rather than about any
machine.

**The comparison is not yet fair to the causal bar.** As above: an environment
where credulity is punished would show what the bar is for. Until that exists,
the strongest honest claim is that the bar produces beliefs that are *correct
and revisable*, at a cost in speed that has not yet been shown to be worth
paying.

**`right = 246` against the oracle's `2999`.** The concept agent does the right
thing under the regime about a twelfth as often as something that was simply
told the answer. Establishing a cause is slow.
