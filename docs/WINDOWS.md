# Windows

Windows is the first platform CoreScout ships as a product. The research layers
run on Linux too and the mirror has a Linux backend; the application, the
installer and the tray do not, yet.

---

## What gets installed

```text
%LOCALAPPDATA%\Programs\CoreScout\      (per-user, no administrator)
  CoreScout.exe             the window and the tray icon
  corescout-service.exe     the part that is always running
  corescout-mcp.exe         the bridge your AI connects to
  corescout.exe             the command line
```

Data goes to `%LOCALAPPDATA%\CoreScout\`. See [PRIVACY.md](PRIVACY.md).

Nothing starts at sign-in unless you turn it on in Settings. That writes a
per-user `Run` value, which shows up in Task Manager's Startup tab where people
expect to find it and can be removed there without CoreScout's cooperation.
A Windows service would have needed an administrator at install time and would
have been invisible in the place people look, which is the wrong trade for
something meant to be easy to stop.

The web view is Microsoft's own WebView2, already present on Windows 11 and on
current Windows 10. That is why the installer is about two megabytes rather
than a hundred.

---

## What is Windows-specific in the mirror

| | |
|---|---|
| Topology | `GetLogicalProcessorInformationEx(RelationAll)`, walked as packed records |
| Placement | `SetThreadGroupAffinity`, so it works past 64 CPUs |
| CPU time | `NtQuerySystemInformation(SystemProcessorPerformanceInformation)` |
| Frequency and idle | `CallNtPowerInformation(ProcessorInformation)` |
| Cycles | `QueryThreadCycleTime` — cycles executed, not time elapsed |
| Identity | `CPUID` for the brand and vendor strings |
| Shared memory | `CreateFileMappingW` / `MapViewOfFile`, `FILE_MAP_READ` for consumers |

Two of those are worth knowing about.

**Reading affinity is a read implemented as two writes.** Windows has no "get
the current affinity" call; you set it to something and it returns the previous
value, so you set it back. That is documented at the call site because it is
surprising and because it means a read can fail.

**Per-CPU counters update independently**, so kernel-minus-idle can transiently
underflow and saturate. That makes a derived total non-monotonic while every
raw counter is not. `total_ns()` derives from the raw counters for exactly that
reason; it was found as a test that passed alone and failed under parallel
load.

## What is not available

**Per-thread context switches.** Not exposed, so the benchmark falls back to
statistical outlier detection alone. That is a real loss of measurement quality
on this platform and the mirror records it as unavailable rather than guessing.

**Package power.** Neither backend reads it, so nothing in CoreScout can say
anything about energy. Every efficiency figure here is work per cycle.

The mirror represents these as `Unsupported` or `Unavailable`, never as zero.
Diagnostics lists them.

---

## Building

Needs Rust (1.75 or later for the workspace, 1.77 for the desktop shell), Node
20 or later, and the MSVC build tools.

```bash
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all -- --check
```

The desktop shell is outside the Cargo workspace on purpose: Tauri's dependency
tree is an order of magnitude larger than everything else here, and keeping it
out means the commands above still finish in seconds, which is the difference
between gates that get run and gates that get skipped.

```bash
cd apps/corescout-desktop
npm ci
npm run typecheck && npm test && npm run build
```

## Packaging

```powershell
pwsh scripts/release.ps1
```

That builds the three console binaries, copies them where Tauri looks for
sidecars, builds the frontend, runs the bundler, and writes into `dist/`:

```text
CoreScoutSetup.exe                 the NSIS installer, named for a download link
CoreScout_0.3.0_x64-setup.exe      the same file, named for a release page
CoreScout.msi                      for deploying across machines
CoreScout_0.3.0_x64_en-US.msi      the same
*.sha256                           one beside each
```

Tauri downloads NSIS and the WiX toolset on first run. Both are verified
against a published hash by Tauri itself.

## Signing

The published builds are unsigned, so SmartScreen objects. That is correct
behaviour on Windows' part and the download page says so before anyone meets
the dialogue.

With a certificate, add to `tauri.conf.json`:

```jsonc
"bundle": {
  "windows": {
    "certificateThumbprint": "…",
    "digestAlgorithm": "sha256",
    "timestampUrl": "http://timestamp.digicert.com"
  }
}
```

Nothing in this repository contains or expects a certificate, and none should
be committed.

## Releasing

```powershell
pwsh scripts/release.ps1
gh release create v0.3.0 dist/* --title "CoreScout 0.3.0" --notes-file docs/RELEASE-NOTES.md
```

The download page links to
`releases/latest/download/CoreScoutSetup.exe`, so those exact filenames matter.

## Uninstalling

Settings → Apps, as usual. What CoreScout learned is left behind on purpose, so
reinstalling does not start you from nothing. The uninstaller offers to remove
it, and `%LOCALAPPDATA%\CoreScout` is the whole of it if you would rather do it
by hand.

## Troubleshooting

**The app says the service is not reachable.** Check `corescout status` from a
terminal. If the endpoint file is missing, the service is not running; opening
the app starts it, and Activity records why it stopped if it did.

**An agent cannot see CoreScout's tools.** Restart the AI client after
configuring it; MCP servers are read at startup. `corescout ai` shows what has
actually connected.

**Diagnostics says a sensor could not read.** Some counters need privilege
CoreScout does not have and does not ask for. It records the gap rather than
guessing at a value, and the rest continues.
