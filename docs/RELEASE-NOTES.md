# CoreScout 0.3.0

**Your AI gets smarter. Now its computer can too.**

The first release anyone can install. CoreScout watches how your machine and
your AI work together, remembers what reality teaches them, and turns useful
discoveries into better ways of working.

Windows 10 and 11, 64-bit. Free, open source, local-first.

---

## What it does

- **Sees.** The machine, several times a second, and what your AI does to it:
  commands, exit codes, retries, and whether anything checked the result.
- **Learns.** What recurs. Which operations fail here, what the runs that
  worked had in common.
- **Verifies.** On a small fraction of occasions it decides by coin flip rather
  than by belief, because only those trials can tell a cause from a
  coincidence.
- **Improves.** What survives becomes a capability with its evidence attached,
  usable by any AI you connect afterwards.

## Connecting your AI

Anything that speaks the Model Context Protocol. The AI screen generates the
exact configuration for your client and usually writes it for you.

```bash
claude mcp add corescout --scope user -- "C:\Program Files\CoreScout\corescout-mcp.exe"
```

## Privacy

No account, no server, no telemetry — absent rather than disabled. Everything
is in `%LOCALAPPDATA%\CoreScout`, the Privacy page lists it with live counts,
and one button deletes it. Secrets are stripped from commands before anything
is written down. Source code is never read.

## Safety

Four autonomy modes, defaulting to Suggest. Raising the mode never widens what
CoreScout may touch. A Pause that is checked before anything else in the
system. Capabilities validated as untrusted input: no shell, no shell
interpreters, no absolute paths, a required verification step and an enforced
timeout. Every action produces an audit record naming its reason, what was
expected, and what happened.

## What is measured, on the machine this was built on

13th Gen Intel Core i7-13700KF, 86 observable parts, 32 recurring states it
found rather than being told about, prediction error down 42.4% from knowing
which state it is in. Observation costs about 70 microseconds, four times a
second — the exact figure for your machine is in Settings.

## What is not claimed

That a real model does measurably better work with an experienced CoreScout.
The mechanism is built and tested; the claim about what it is worth needs a
real workload and enough runs to say something, and it has not been made yet.
When it is, the numbers will be published whichever way they come out.

## Known limits

- The installers are unsigned. SmartScreen will warn, and it is right to.
- macOS and Linux applications do not exist. The mirror runs on Linux; the app
  does not.
- No shipped capability changes scheduling yet. Today's capabilities run
  programs; the permission layer and audit trail for more are in place.
- Nothing here reads package power, so nothing can say anything about energy.

## Verifying the download

```powershell
Get-FileHash .\CoreScoutSetup.exe -Algorithm SHA256
```

Compare it with the `.sha256` file beside it.
