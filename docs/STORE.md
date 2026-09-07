# The Microsoft Store

CoreScout builds a valid MSIX today:

```powershell
pwsh scripts/msix.ps1
# dist/CoreScout.msix  ~1.8 MB
```

That package installs and runs. It will **not** be accepted by the Store until
it carries an identity from Partner Center, because a package's identity is
issued rather than chosen. This document is the sequence.

---

## What is already done

| | |
|---|---|
| MSIX manifest | `packaging/msix/AppxManifest.xml`, validated by MakeAppx |
| Packaging script | `scripts/msix.ps1` |
| Store tiles | Every size the manifest names, generated with the rest of the icons |
| `runFullTrust` | Declared. CoreScout reads processor counters and starts processes. |
| Startup task | Declared **disabled**, so Windows lists it and the user turns it on |
| Execution aliases | `corescout` and `corescout-mcp` on the path |
| Packaged data path | Detected at runtime, so the Privacy page names the real folder |
| No network capability | Declared nowhere, because nothing opens a connection |

## What has to happen next, in order

### 1. A Partner Center account

<https://partner.microsoft.com/dashboard>. An individual account is a one-off
fee; a company account needs verification and takes longer. Either works.

### 2. Reserve the name

Dashboard → Apps and games → New product → MSIX or PWA app. Reserve
**CoreScout** if it is free.

Then Product identity gives three values that must be copied exactly:

```text
Package/Identity/Name          e.g. 12345Publisher.CoreScout
Package/Identity/Publisher     e.g. CN=ABCD1234-5678-...
Publisher display name         e.g. Matthew Busel
```

### 3. Build with them

```powershell
pwsh scripts/msix.ps1 `
  -IdentityName "12345Publisher.CoreScout" `
  -Publisher "CN=ABCD1234-5678-90AB-CDEF-1234567890AB" `
  -PublisherDisplayName "Your name here"
```

Do **not** sign it. The Store signs uploads with its own certificate, and a
package signed with anything else is rejected.

### 4. Submit

Upload `dist/CoreScout.msix` under Packages. Then fill in:

**Age rating** — the questionnaire. CoreScout has no user-generated content, no
advertising, no purchases, and no data collection, so this is short.

**Privacy policy URL** — required, because the manifest declares
`runFullTrust`. Use <https://corescout.dev/privacy>, which is a page in
`web/website` and describes actual behaviour rather than reserving rights.

**Category** — Developer tools.

**Pricing** — Free.

**Properties → Product declarations** — tick *This app has been tested to meet
accessibility guidelines* only if that is true. It has not been tested with a
screen reader yet, so leave it off.

### 5. Certification

`runFullTrust` sends this to manual review, which takes longer than an
automated pass. The two things a reviewer will look at:

- **Why full trust.** CoreScout reads processor performance counters through
  `NtQuerySystemInformation` and `CallNtPowerInformation`, and sets thread and
  process affinity. Neither is possible in the sandbox. Say that in the
  submission notes.
- **What it does with the access.** Point at
  [SECURITY.md](SECURITY.md) and [PRIVACY.md](PRIVACY.md). The short version:
  it defaults to changing nothing, no autonomy mode widens what it may touch,
  and it opens no network connection.

---

## What differs in a packaged build, and is handled

**Where the data goes.** A packaged app's writes under `%LOCALAPPDATA%` are
redirected by Windows into the package's own store, so the environment variable
would name a folder the files are not in. `packaged::package_data_dir` detects
the package and uses `LocalCache\Local\CoreScout`, which is where the files
actually land and therefore what the Privacy page shows. Without this, "delete
everything" would have deleted nothing.

**Starting at sign-in.** The registry `Run` key is not allowed and would fail
certification. The manifest declares a startup task, disabled, and Settings →
Apps → Startup is where it is turned on. The Settings page inside CoreScout
detects the packaged case and says so rather than offering a switch it cannot
operate.

**Sidecars.** All four programs are in the package and start each other by
absolute path from `current_exe().parent()`, which works unchanged.

**Updates.** The Store handles them. The in-app update check is for the
installer builds and should stay quiet in a packaged one.

## What is not ready and should be finished before submitting

- **Accessibility.** The interface has focus-visible styles, keyboard-reachable
  controls and honest contrast, and it has not been tested with a screen
  reader. Do that before ticking the accessibility declaration.
- **Store listing copy and screenshots.** At least one 1366×768 screenshot is
  required. The website has the wording; the screenshots have to be taken from
  a build with real data in it, and a screenshot of an empty install would be
  an accurate but very poor listing.
- **A support URL.** The GitHub issues page will do.

## Side-loading, meanwhile

The MSIX can be installed today without the Store, with any code-signing
certificate the machine trusts:

```powershell
pwsh scripts/msix.ps1 -Sign <thumbprint>
Add-AppxPackage dist\CoreScout.msix
```

For most people the NSIS installer from
[WINDOWS.md](WINDOWS.md) is simpler, and it is what the download page offers.
