# The credulity trap suite

Environments where acting on correlation is dangerous, and the measurement that
makes the difference visible.

---

## 1. The thesis

> Causal self-modelling imposes an epistemic tax. Under what environmental
> conditions does that tax buy enough protection from false self-beliefs to be
> worthwhile?

The previous experiment found a correlational agent beating a causal one on
cost. That was a fair result and an incomplete question: its environment
contained no correlation worth being fooled by, so the tax bought nothing.

These five worlds are built so the answer can come out either way. **The
expected result is not that the causal agent always wins**, and a suite in which
it did would be a demonstration rather than a test. There is an assertion,
`no_agent_wins_every_world`, that fails if either agent sweeps.

---

## 2. Measurement

Mean cost hides the thing that matters. Everything is measured as regret against
an oracle that knows the optimal action each tick:

```text
R_T = sum over t of [ cost(chosen) - cost(optimal) ]
```

Counterfactual costs are exact rather than resampled: the world returns the cost
of *every* action under one shared noise draw, so the comparison is against the
same world rather than a different roll of it.

Regret is then split into two components that are morally different:

| component | what it is |
|---|---|
| **exploration regret** | the price of being able to learn. Paid whether the agent is right or wrong. |
| **belief regret** | the price of being wrong. Paid only when a held belief produces a worse action than the optimum. |

An agent with high exploration regret and zero belief regret is expensive and
correct. One with zero exploration regret and high belief regret is cheap and
deluded. Which the environment punishes more is the entire question.

Also tracked: false-belief actions, true-belief actions, and the fraction of
belief-driven actions resting on randomised evidence. That last is 100% for the
causal agent by construction and 0% for the correlational one, and it is checked
from the outside rather than trusted.

---

## 3. The worlds

**Benign.** A real, stable effect and nothing misleading. The control, and the
case where randomising is pure overhead.

**Confounded.** A hidden slow regime, invisible in the mirror, drives the
outcome. `a1` has no benefit and a real overhead. A correlational agent that
happens to favour it during good spells finds it associated with success.

**Sign reversal.** The world improves over time on its own. An agent exploring
sequentially samples one action early and another late, so whichever it tried
second looks better whatever its real effect. `a1` is genuinely worse and looks
genuinely better. The correlation points the wrong way.

**Regime switch.** A real effect that disappears and reverses halfway through.
This exercises the forgetting, de-coinage and preference-withdrawal machinery
directly.

**Deferred cost.** The cost of an action is paid on the *following* tick and
charged to whatever the agent does next. `a1` is the cheapest thing to do now and
the most expensive thing to have done.

Included precisely because it is expected to defeat the causal agent too.
Randomisation fixes confounding by the state; it does nothing about a bill that
arrives one tick later, because shuffling which action is taken does not move
which tick the bill lands on.

---

## 4. Results, seed `0xC0FFEE`, 12000 ticks

```text
world            agent             regret   explore    belief   false    true  causal
benign           correlational       6278      1218       520     130    2361      0%
benign           causal             10074      1798         0       0    1317    100%
confounded       correlational        956       764       192     223       0      0%
confounded       causal               850       850         0       0       0      0%
sign-reversal    correlational        846       643       202     506       0      0%
sign-reversal    causal               727       727         0       0       0      0%
regime-switch    correlational       7380       872      3084     995     976      0%
regime-switch    causal              6935      1372       603     201     423    100%
deferred-cost    correlational       2203       523      1681    1913       0      0%
deferred-cost    causal               645       461       184     238       0    100%
```

### Against the prediction

| world | predicted | observed |
|---|---|---|
| benign | correlational | **correlational**, 6278 vs 10074 |
| confounded | causal | causal, 956 vs 850 |
| sign reversal | causal, by a lot | causal, and 506 false beliefs vs **0** |
| regime switch | depends on forgetting | causal, narrowly; both hold false beliefs |
| deferred cost | neither | **neither**; both fooled, causal less so |

The prediction held in every case.

### The three results worth stating plainly

**The causal agent loses the benign world, and loses it badly.** 10074 against
6278, most of it exploration regret it never recovers because there is nothing
to protect against. This is the honest cost of the method and it is large.

**Belief regret separates the agents far more cleanly than total regret.** In
sign reversal the totals differ by 14% (846 vs 727) while the false-belief
counts differ by 506 to 0. Total cost nearly hides a difference that is
categorical: one agent held a confidently wrong belief about itself five hundred
times and the other never did.

**Deferred cost defeats both.** The causal agent takes 238 false-belief actions.
It is fooled about eight times less than the correlational one, because
interleaving the arms spreads the deferred bill more evenly between them, but it
is fooled. Randomisation is not a solvent for every kind of confounding, and a
suite that did not contain such a case would be misleading about what the
machinery buys.

---

## 5. Two defects this suite exposed

**Ties made "optimal" meaningless.** Two actions cost the same, so `min_by`
picked whichever came first alphabetically, and "the belief was correct" silently
meant "the belief matched an arbitrary tiebreak". The sign-reversal world was
not testing anything for one whole run. Every world now has exactly one optimal
action.

**Concept utility was measured from the agent's own cost stream.** Costs depend
on which action the agent chose, so the measure conflated the machine with the
policy, and a concept would be coined or refused according to how the agent had
been behaving rather than what the machine was doing. The causal agent formed
almost no beliefs anywhere as a result. Utility is now measured over reflections:
a concept is a claim about the machine, so its evidence has to be the machine.

---

## 6. What this does not establish

**Nothing here is real hardware.** Five synthetic worlds, chosen by me, with
mechanisms I picked to be discriminating. A different five could produce a
different table.

**The worlds are not weighted by how common they are.** The result "causal wins
four of five" would be meaningless even if it were the summary, because nobody
knows the base rate of confounded structure in real machine behaviour. That base
rate is the number that would actually decide whether to run this, and it is not
measured anywhere.

**Exploration regret is measured in arbitrary cost units.** On a real machine it
is milliseconds of tail latency on somebody's real work. Until that number
exists, the safety argument for running a randomising controller on anything
that matters is incomplete.

**Only two agent designs are compared.** A correlational agent with better
exploration, or one that detects change points, might close much of the gap
without paying for randomisation. Neither is built.
