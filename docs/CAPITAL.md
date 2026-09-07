# Compute as a capital-producing process

Whether a lineage of self-modified runtimes makes more effective compute, and
the accounting that decides.

---

## 1. The claim

> Spend compute learning a machine, compile that experience into a more
> productive descendant, and the descendant does the same useful work with fewer
> scarce physical inputs.

```text
        verified useful work
Q  =  ------------------------          Q(G0) < Q(G1) < Q(G2) < ...
       scarce physical inputs
```

If that holds and compounds, the process manufactures effective compute out of
knowledge. The same silicon does more work; the extra capacity is the output,
and it exists whether or not anyone buys anything.

---

## 2. Why the accounting came before the mechanism

`Q` is the number the whole project would be judged by. That makes it the number
most worth faking, and a system that measures its own success will discover that
it is succeeding.

So the ledger was built adversarially first. Five ways this result gets faked:

| fraud | defence |
|---|---|
| skip the work, report the throughput | work must **verify**; unverified output is worth zero and its inputs are still charged |
| ignore what the search cost | exploration is capitalised and **amortised** into the descendant that inherits the gain |
| optimise the measured workload | **held-out** work is accounted separately and never explored against |
| take credit for a quieter machine | a **control lineage** spends identical inputs and carries nothing forward |
| define "useful" yourself | work is verified against an expectation the runtime never sees |

The first is the one everything rests on. A runtime that returns wrong answers
in a tenth of the time must score **zero**, not ten times better, and there is a
test asserting exactly that.

`Verdict` can return `LearnedTheBenchmark`, `SearchCostTooHigh`,
`NotImproving` and `Untrustworthy`. A harness that cannot return those is not
measuring anything.

### Scarce inputs are not one number

Silicon time, energy, and memory held over time are tracked separately, because
collapsing them early hides the trade a runtime is most likely to make: buying
throughput with a great deal more memory and calling it an improvement. There
is a test where a configuration wins on silicon alone and loses once memory is
priced.

The weights that combine them come **from outside**. A runtime choosing its own
weights would find that whatever it consumes most of is free.

---

## 3. The experiment

`integration/tests/lineage_productivity.rs`. A synthetic substrate with a
six-knob, nine-level configuration space. Execution cost has three terms:

- a **general** structure  -  distance from a true optimum, which holds across all
  workloads and is the part worth learning;
- a **workload-specific quirk**  -  rewards a different configuration for each
  workload, looks exactly like learning, and transfers to nothing;
- noise.

Each generation gets a search budget of 14 configurations: deliberately far too
small to solve the space. A generation that could find the optimum alone would
make the lineage pointless, since the entire claim is that a descendant starts
where its ancestor left off.

Every work unit returns a checksum derived only from its input, so no
configuration can change it. The expected value is held by the harness.

### Results, seed `0xC0FFEE`, 4 generations, 20000 units per pool

```text
learned                          net Q trained   net Q held-out
  G0  inherited 0 configs            0.000377        0.000383
  G1  inherited 14                   0.000454        0.000457
  G2  inherited 28                   0.000541        0.000533
  G3  inherited 42                   0.000631        0.000610
  -> held-out net productivity rose 59.4% over 4 generations

control (searches identically, keeps nothing)
  G0                                 0.000276        0.000282
  G3                                 0.000269        0.000275
  -> not getting better

overfit (tunes a trained workload's quirk)
  G0                                 0.000263        0.000257
  G3                                 0.000257        0.000252
  -> not getting better

attributable to learning: 61.6%
```

`Q(G0) < Q(G1) < Q(G2) < Q(G3)`, monotonically, on work the lineage never
practised on, net of everything the search cost, and the control shows none of
it.

### The control is the result

The learned line gaining 59% means little alone. The control spends identical
silicon searching and throws the answer away, so anything it also gained would
have been the substrate rather than the runtime. It gained nothing, which is
what makes the 61.6% attributable.

---

## 4. What the first attempt got wrong

The first run of this experiment produced a **declining** `Q` for the learning
lineage, and the reason is worth recording because it is a real property rather
than a bug.

The search space was small enough that `G0` found the optimum outright  -  its `Q`
was already nearly twice the control's. Every later generation then kept
spending its search budget, found nothing further, and inherited a bill that
grew every generation while the gains had stopped. Net productivity fell
monotonically.

That is correct accounting describing a badly designed process: **a lineage that
keeps searching after it has found the optimum destroys value.** The fix was to
make the space genuinely hard, not to stop charging for the search.

A real deployment needs what this experiment does not have: a rule for when to
stop searching. Nothing in the crate currently provides one.

---

## 5. What this does not establish

**It is a simulation with a planted structure.** It shows the accounting is
sound and that a lineage *can* compound when there is something to find. Whether
real silicon contains exploitable general structure of this kind is the question
that decides whether any of this matters, and nothing here speaks to it.

**The search is hill climbing.** Deliberately unsophisticated, because the
experiment is about whether the accounting holds under compounding rather than
about the search being clever. A better search would find more; a worse one
would find less; neither would change what is being tested.

**One substrate, one seed family, one optimum.** A different cost landscape
could produce a different answer, and a landscape with no general structure at
all would correctly produce `NotImproving`.

**Nothing here modifies a real runtime.** A "descendant" is a configuration
vector. Compiling learned experience into an actual rebuilt runtime is a much
larger piece of work, and the gap between a configuration and a runtime is where
most of the difficulty lives.

**Energy is never measured.** The default weights price silicon time only, which
is honest on a machine that cannot read its own power, and it means the results
above say nothing about whether the descendant is more *efficient* in the sense
an operator cares about.
