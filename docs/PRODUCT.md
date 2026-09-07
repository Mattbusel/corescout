# The product

What each screen is for, and why it is worded the way it is.

---

## The one rule

**The first thing on every screen is a sentence.** Numbers come after it, and
the technical layer comes after those, behind a control. A screen that opens
with a table has already failed the person who installed this because their AI
kept failing at something.

The words *mirror*, *concept*, *latent state* and *atlas* appear nowhere a user
can see, though that is exactly what is underneath. They are in the code, the
crate names and this repository's documentation, which is where they belong.

## Six places

**Home.** Four questions answered without scrolling: is CoreScout running,
which AI is connected, what did the computer learn, and is it doing anything.
The live mirror is here because it is the one image that carries the idea
before anyone reads a word.

**AI.** Which systems have connected, and how to connect one. Configuration is
generated with the real path filled in, and written for you where CoreScout can
safely edit the file.

**Learned.** The most important screen. Cards, in two groups — *Verified* and
*Noticed, not yet tested* — and a third for what is still uncertain. Clicking
one opens the evidence.

**Activity.** A timeline of what happened, and why. Simple by default;
Technical shows the low-level entries and the structured detail underneath each
one.

**Computer.** What this machine is, what CoreScout can see of it, and the
"what my computer knows" summary grouped four ways: about your AI, about the
work, about this machine, and what is still uncertain.

**Settings.** Autonomy, what may be touched, Privacy, and Diagnostics.

## Things the interface will not print

**`0%` for something never measured.** A capability that has never run reads
*never used*. Zero out of zero is not zero per cent, and a fresh install
reporting 0% failures and 100% reliability lies about both. Every rate in the
view types is an `Option`, so the honest answer is expressible and the
dishonest one takes effort.

**A progress bar for confidence.** A bar invites comparison between an
association and a randomised measurement, which are not on the same scale, and
a reader comparing two bars will not notice that they are not. So confidence is
words, and the vocabularies do not overlap: an association is *seen once or
twice* through *seen often*, and only a measurement gets *early*, *good* or
*strong evidence*.

**A confidence of 100% for an association.** However many times two things have
been seen together, they have still only been seen together. The scale is
capped at 0.75 in the code.

**An empty dashboard.** Every screen has an empty state that explains why it is
empty and what will fill it. The fact that CoreScout has learned nothing yet is
itself the honest answer, and saying so reads as a careful product rather than
a broken one.

## The first run

Five screens, each of which says one thing or asks one question.

1. *Meet CoreScout.* Your AI can learn a lot. Its computer usually learns
   nothing.
2. *It watches how the two of them work together.* And everything stays here.
3. *Connect your AI.*
4. *Choose how much CoreScout can do.* Three options with three sentences.
   Autopilot is not offered here; it is worth having only once CoreScout has
   verified something on your machine.
5. *Start learning.* Use your AI as normal. Come back tomorrow.

The mode question is asked at the start rather than later, because asking it
later means shipping a default nobody chose.

## Notifications

Sparse, and only for things a person would want to be interrupted for.

Good: *Your AI has failed this operation four times. CoreScout found a more
reliable procedure.*

Not: *CPU state changed. Latent state changed. Observation complete.* Those are
in the log, at Debug, where the Technical view can find them.

## The tray

```text
CoreScout is learning
Claude Code connected
─────────────────────
Open CoreScout
Pause observation
Autonomy: Assist
─────────────────────
Quit CoreScout
```

Closing the window hides it rather than stopping the service, because the whole
product is that the computer keeps learning while you work. Quit stops both,
and it stops the service only if this application started it: one that was
already running has something depending on it, and killing it would disconnect
an agent mid-session.

## What it costs

Measured on the machine it is running on, in Settings → Diagnostics, rather
than quoted. On the development machine, at four samples a second, an
observation pass costs about 70 microseconds and the service holds around nine
megabytes.

The same pass in a tight loop costs about 7 microseconds; the difference is
cold cache between samples, and the honest number to show a user is the one the
product actually pays.

## Where the words come from

Every sentence a user reads is produced by the engine, not by the interface.
`Autonomy::summary`, `Basis::explain`, `FailureMode::headline`,
`MachineMood::plain` and the rest are Rust functions with tests. That means the
wording is the same in the window, the terminal and the answer an AI gets, and
there is one place to change it.

## What is not built yet

Named honestly, because a feature list that includes things that do not exist
is the fastest way to lose someone on their second day.

- **macOS and Linux applications.** The mirror runs on Linux; the app does not.
- **A capability that changes scheduling.** The permission layer, the runtime
  and the audit trail are all in place, and the actuators from the research
  layer exist, but no shipped capability yet changes affinity or priority.
  Today's capabilities run programs.
- **Energy.** Nothing here reads package power, so nothing can say anything
  about efficiency in the sense an operator pays for.
- **A hosted anything.** The architecture leaves room for optional community
  features later. The core will not need them.
