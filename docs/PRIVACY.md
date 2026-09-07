# What CoreScout observes

All of it stays on your machine. There is no account, no server, and no
telemetry — not disabled by default, absent. If a future version transmits
anything it will have to be written somewhere that does not exist yet, which is
the point: the boundary is visible in the dependency graph rather than in this
document.

The Privacy page inside the application shows this same list with live counts,
and a button that deletes any of it.

---

## Your machine

| | |
|---|---|
| Processor layout | Cores, threads, caches, and how they are arranged. |
| Live counters | How busy each part is, several times a second. |
| Recurring states | Patterns CoreScout found in those counters and named itself. |

## What your AI does

| | |
|---|---|
| Commands and tool calls | The command line, with credentials removed before it is written down. |
| Outcomes | Exit codes, how long it took, and whether anything verified the result. |
| Retries | When the same thing was attempted again after failing. |
| Folders | Which repository or folder the work happened in, by path. |
| Sessions | When each AI was working, and for how long. |

## Never

| | |
|---|---|
| Your source code | CoreScout does not read files inside your repositories. |
| Your prompts | Or the responses, or anything the model was reasoning about. |
| Credentials | Stripped where data enters, not where it is displayed. |
| Anything, anywhere | Nothing is transmitted. There is no endpoint to transmit to. |

---

## Where it lives

```text
%LOCALAPPDATA%\CoreScout\
  corescout.redb    what it has learned
  mirror.ring       recent readings, a fixed 32 MB, forever
  endpoint.json     how the application finds the service
```

Local rather than roaming: a file of machine telemetry has no business
following you onto another computer. Every file is under that one directory, so
"delete my data" is a directory removal you can perform yourself without
trusting the uninstaller — and there is a test asserting that no file escapes
it.

Deleting that folder deletes everything, and CoreScout starts again from
nothing.

## Bounded on purpose

`mirror.ring` is allocated at its full size when it is created and never grows.
A machine that has been running CoreScout for a year uses exactly as much disk
as one that started this morning. At four samples a second it holds about a day
and a half of readings at full resolution; older readings are overwritten,
and what was *learned* from them stays in the database.

The event log has a ceiling too. Nothing here grows without a limit that is
written down.

## Redaction happens at ingestion

Not at display. A command line containing an API token, written into a database
that survives restarts, is a credential at rest that you did not choose to
store, sitting somewhere no credential scanner is looking. A value that was
never written cannot leak from a backup, a support export, or a feature nobody
has written yet.

What gets caught:

- flags whose value is a secret: `--token`, `--password`, `--api-key`,
  `--secret`, `--auth`, `--private-key`, and the `=` forms
- assignments whose name contains `token`, `secret`, `password`, `key`,
  `credential`, `auth`, `session`
- known credential prefixes: `sk-`, `ghp_`, `github_pat_`, `xoxb-`, `glpat-`,
  `AKIA`, `AIza`, `eyJ`, and others
- long high-entropy values: 24 or more characters, unbroken, mixing three
  character classes

What deliberately survives: file paths, commit hashes, semantic versions, and
ordinary command lines, because the whole learning layer is built on those and
over-redaction would quietly stop it working. There are tests for both
directions.

This is a net, not a proof. It catches the common cases and cannot catch all of
them.

## Deleting things

Two buttons, because they are different questions.

**Forget my AI history** removes sessions, tasks, actions, workspaces, and
everything learned from them. What CoreScout worked out about the machine
itself stays.

**Forget everything** removes that too: the states it found in itself, the
capabilities, the settings, the whole log.

Both are immediate and neither is recoverable.

## Checking any of this

The source is public and the part that would have to do the transmitting does
not exist. `crates/storage` opens no socket; the local API binds loopback only;
the whole dependency list for the service and the command line is four crates
plus the standard library.

```bash
corescout privacy          # what is stored, and where, right now
```
