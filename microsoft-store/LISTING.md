# CoreScout: Microsoft Store listing

Everything Partner Center asks for, in the order it asks. Paste from here.

Field limits are Microsoft's, and every entry below is inside them; the count
after each heading is the limit, not a target.

---

## Product name (256)

```
CoreScout
```

## Short description (1,000)

```
Your AI learns how your computer actually works.

CoreScout gives Claude Code, Codex, Cursor and other coding agents persistent,
verified knowledge of your machine and your development environment. It watches
what your AI does, notices which approaches actually work here, and tells the
next agent before it repeats the same mistake. Everything stays on your
computer.
```

## Description (10,000)

```
The model doesn't learn from every task. Its computer does.

Every coding agent starts from nothing. It doesn't know that the build fails
unless the client is regenerated first, that one test is flaky on this machine
and not on yours, or that the thing it tried last Tuesday didn't work. You
know. Your computer knows. The agent doesn't, and it will not remember
tomorrow.

CoreScout is the memory that stays behind. It runs on your machine, watches
what your AI actually does, and keeps what turns out to be true. The next
agent, in the next session, using a different model, starts with it.


WHAT IT ACTUALLY DOES

Watches the work. CoreScout observes the commands your agent runs and how they
turn out. Not by asking the agent to report back, which it forgets to do, but
by watching directly.

Notices what recurs. Which operations fail here, how often, and what preceded
the times they did not. Which of your machine's states it has seen before.

Says how it knows. Every conclusion is labelled with the evidence behind it.
Something CoreScout has only seen happen together is shown as exactly that, and
it will not call it a cause until it has tested it under randomised assignment.
Most tools would call both of those "insights". CoreScout will not.

Tells your AI. Through the Model Context Protocol, the standard every serious
coding agent now speaks. Your agent asks CoreScout what is known about this
machine and this repository, and gets an answer grounded in what happened here.

Turns verified knowledge into something runnable. When CoreScout has evidence
that a procedure works, it can package it as a capability with preconditions,
a verification step and a rollback, which your agent can then use by name.


WHAT MAKES IT DIFFERENT

It is not a monitor. htop tells you what is happening. CoreScout tells you what
it has learned, and tells your AI.

It is not a wrapper around a model. There is no inference in CoreScout. It is
measurement, statistics and an explicit refusal to overclaim.

It distinguishes association from cause, in the interface, on every card. This
is unusual and it is the point. A tool that presents a correlation as a cause
will eventually tell your agent to do the wrong thing with great confidence.

It is local. No account, no server, no telemetry. Commands are stored with
secrets stripped before anything is written down. Source code is never read or
stored. One folder, which the app will open for you, and two buttons that
delete it.


WORKS WITH

Claude Code, OpenAI Codex, Cursor, OpenCode, and anything else that speaks the
Model Context Protocol. Connecting one is a single line, and the app writes it
for you.


WHO IT IS FOR

Developers who have handed real work to a coding agent and watched it
rediscover the same failure a fourth time. Teams whose agents keep hitting the
same environment-specific problems on the same machines.


PRICING

One week of CoreScout Pro when you install it, with everything switched on. No
card, no account, no crippled trial.

After that, CoreScout keeps watching your machine and keeps everything it has
already learned, forever, at no cost. Continuing to learn, the capability
system, cross-agent memory, history beyond the last day and the higher autonomy
modes are CoreScout Pro.

Pro is $49 a month or $399 a year. CoreScout Lifetime is $999 once, including
every update. All three are bought here, on your Microsoft account, and work on
every machine you sign in to.


REQUIREMENTS

Windows 10 version 1809 or later, 64-bit. About 200 MB of disk. No GPU.

CoreScout itself needs no network connection and no account: it never sends
anything anywhere. Buying Pro or Lifetime goes through the Microsoft Store,
which needs both, and that is the only part that does.
```

## What's new in this version (1,500)

```
First public release.

- The AI Operational Mirror: CoreScout watches what your coding agent does and
  learns from the outcomes, without depending on the agent to report anything.
- Model Context Protocol server, so Claude Code, Codex, Cursor and anything
  else that speaks MCP can ask what is known about this machine.
- Evidence labelling throughout: everything CoreScout says is marked with how
  it knows, and correlations are never presented as causes.
- Capabilities: verified procedures your agent can run by name, each with
  preconditions, a verification step and a rollback.
- One week of CoreScout Pro on install, with no card and no account.
```

## Search terms (7 terms, 30 characters each)

```
AI agent memory
Claude Code
MCP server
coding agent tools
persistent agent memory
AI automation Windows
developer environment
```

Additional keywords worth carrying in the description body, which Store search
also indexes: Codex, Cursor, OpenCode, Model Context Protocol, agent
reliability, local AI, Windows AI, developer tools.

## Category

```
Developer tools  >  Development kits
```

`Developer tools` is right and `Utilities & tools` is not. CoreScout is bought
by developers to change how their development tooling behaves; a utilities
listing puts it beside disk cleaners, and buyers at this price do not shop
there.

## Product features (up to 20, 200 characters each)

```
Gives Claude Code, Codex, Cursor and other MCP agents persistent knowledge of your machine
Watches what your AI actually does, instead of relying on it to report back
Labels every conclusion with the evidence behind it, and never calls a correlation a cause
Learns which operations fail on this machine, how often, and what preceded the times they did not
Turns verified procedures into capabilities your agent can run by name, with rollback
Recognises the states your computer recurs into, found rather than configured
Four autonomy levels, from observing only to acting unattended, and it starts at the cautious one
Entirely local: no account, no server, no telemetry, and one folder you can delete
Commands are stored with secrets stripped. Source code is never read or stored.
One week of CoreScout Pro on install, with everything switched on and no card
```

## System requirements

| | |
|---|---|
| OS | Windows 10 version 1809 (build 17763) or later |
| Architecture | x64 |
| Memory | 4 GB (CoreScout itself uses well under 100 MB) |
| Disk | 200 MB, plus what it learns |
| Network | Not required to run. CoreScout has no server to talk to. Buying a licence goes through the Store, which does. |
| Hardware | None. No GPU, no NPU, no specific processor. |

CoreScout reads more from Intel and AMD processors that expose performance
counters, and says so in the app when a machine exposes less. It works either
way.

## Support and contact

| Field | Value |
|---|---|
| Website | `https://corescout.dev` |
| Support contact | `support@corescout.dev` |
| Privacy policy URL | `https://corescout.dev/privacy` |

The privacy policy text is in `PRIVACY.md` in this directory and is published
to that URL from `website/`. Partner Center requires the URL to resolve before
submission, so publish the site first.

## Age rating

Everyone. See `COMPLIANCE.md` for the questionnaire answers, which are what the
rating is actually derived from.

## Copyright and trademark

```
Copyright CoreScout. All rights reserved.
```

## Additional licence terms

```
https://corescout.dev/terms
```

---

## Pricing

`ADDONS.md` has the three add-ons to create in Partner Center, the exact
Product ID strings CoreScout matches on, and the three fields that cannot be
changed after publishing.

Everything is sold through the Store. Seat-based and negotiated pricing were
dropped rather than sold through a second channel, because the Store can
express neither and a product with two ways to pay is a product with two ways
to get billing wrong.

<!-- generated:plans -->

Generated from `crates/licence/src/plans.rs` by `build-listing.ps1`. Do not edit by hand.

```
pro-monthly  CoreScout Pro               $49/month  corescout.pro.monthly
             Everything CoreScout can learn about one machine, kept learning.
             One person, one machine.
             $588 a year

pro-yearly   CoreScout Pro               $399/year  corescout.pro.yearly
             The same, billed once a year.
             One person who has decided.
             $399 a year

lifetime     CoreScout Lifetime          $999 once  corescout.lifetime
             Pay once. It keeps working, including every update.
             Anyone who would rather not think about it again.

Trial: One week of CoreScout Pro, with everything switched on.
Every one of these is a Microsoft Store add-on; there is no other way to pay.
```

<!-- /generated:plans -->

---

## Screenshots

In `screenshots/out`, 2732x1536 (1366x768 at 2x), which is above the Store
minimum and downsamples cleanly. Produced by `screenshots/capture.ps1` against
a running CoreScout; see `screenshots/README.md` for what in them is real.

| File | Screen | Caption to use |
|---|---|---|
| `01-home.png` | Home | Your computer is learning how to work with your AI. |
| `02-learned.png` | Learned | Everything it knows, labelled with how it knows it. |
| `03-computer.png` | Computer | The machine, as CoreScout sees it. |
| `04-ai.png` | AI | Connect Claude Code, Codex or Cursor in one line. |
| `05-activity.png` | Activity | Everything CoreScout has done, and why. |
| `06-plan.png` | Plan | What this machine worked out, and what Pro keeps doing. |
| `07-privacy.png` | Privacy | No account, no server, no telemetry. One folder. |

Store logo, 300x300, is `assets/StoreLogo300.png`.
