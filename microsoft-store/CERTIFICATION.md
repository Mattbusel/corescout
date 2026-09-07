# Certification checklist

Run through this before pressing submit. Each item says how to check it, not
just what it is, because "verify the package is valid" is not a check.

Items marked **blocked** cannot be done from this repository and are recorded
in `STATUS.md` with what they need.

## The package

- [x] **Builds.** `powershell -File scripts\msix.ps1`. Prints the identity it
      used and the size.
- [x] **Carries the real identity.** `Tensorust.CoreScout`,
      `CN=98B77C5C-8582-4364-B50E-0922AE25FBF6`, publisher display name
      `Tensorust`. The script defaults to these; it warns if given something
      that does not look like a Partner Center identity.
- [x] **Version is four parts with a zero revision.** `1.0.0.0`. A Store upload
      rejects a non-zero fourth part.
- [x] **Contains all four programs.** `CoreScout.exe`, `corescout-cli.exe`,
      `corescout-service.exe`, `corescout-mcp.exe`. `msix.ps1` now fails the
      build if any is missing or if two payload names collide case-insensitively,
      which is how the CLI silently replaced the window in an earlier build.
- [x] **Every tile the manifest names is present.** The staging step copies them
      by name and throws on a missing one.
- [x] **Unsigned.** Correct for a Store upload; Microsoft signs it.
- [ ] **Windows App Certification Kit passes.** *Blocked:* `appcert.exe` needs
      an elevated prompt. `scripts\install-local.ps1` is the elevated path.

## Identity and commerce

- [x] **App name reserved.** `CoreScout`, Store ID `9PJPNV5VDBV0`.
- [ ] **The three add-ons exist and are published.** *Blocked:* Partner Center.
      `ADDONS.md` has the exact Product ID strings, which must match the tokens
      in `crates/licence/src/plans.rs`. Check with
      `cargo run -p corescout-licensor -- plans`.
- [ ] **A real purchase unlocks Pro.** *Blocked:* needs published add-ons and a
      Store-installed build. Everything below the Store API is tested; the
      Store API itself cannot be exercised outside a Store install.

## Listing

- [x] **Every field is written and inside its limit.** `LISTING.md`.
- [x] **Prices in the listing match the code.** Generated:
      `powershell -File microsoft-store\build-listing.ps1 -Check` fails if they
      have drifted.
- [x] **Category is Developer tools.** Not Utilities.
- [x] **Screenshots are of the real product.** `screenshots/capture.ps1` runs a
      real service against a throwaway data directory, feeds it a stated
      history through its own API, and photographs the built application. What
      is staged and what is computed is written down in `screenshots/README.md`.
- [ ] **Screenshots retaken from an installed package.** *Blocked:* the current
      set shows a developer's paths on the Privacy page and in the setup
      instructions, because they were taken from an unpackaged build. A
      packaged build shows the package data folder and the execution aliases.
- [x] **Store logo, 300x300.** `assets/StoreLogo300.png`.
- [ ] **Privacy policy URL resolves.** *Blocked:* `PRIVACY.md` is written;
      `https://corescout.dev/privacy` has to be live before submission.
      Microsoft checks that the URL loads.

## Policy

- [x] **Age rating answers decided.** `COMPLIANCE.md`. Everyone, with an
      in-app-purchases disclosure.
- [x] **The loopback socket has a written answer.** It is the most likely
      reviewer question. `COMPLIANCE.md`.
- [x] **`runFullTrust` is justified.** Processor counters and starting
      processes.
- [x] **Startup task ships disabled.** Declared, off, and the user turns it on
      in Windows Settings. CoreScout never enables itself.
- [x] **No alternative payment path.** Every purchase is a Store add-on. No
      external checkout, no licence-key sales page.
- [x] **The app still works after the trial.** Enforced by tests, not by
      intention: `integration/tests/product_licence.rs`.
- [x] **Third-party names are nominative use only.** No logos, no implied
      endorsement.

## The product itself

- [x] **All tests pass.** 1,415 across the workspace.
- [x] **Clippy is clean.** Zero warnings, all targets.
- [x] **Formatted.** `cargo fmt --all`.
- [x] **Frontend typechecks, tests and builds.**
- [x] **Reduced motion is honoured in the canvas**, not only in CSS.
- [ ] **Install, upgrade, uninstall and reinstall verified on Windows.**
      *Blocked:* needs `scripts\install-local.ps1` run once from an elevated
      prompt, which also requires Developer Mode or a trusted test certificate.

## After certification

- [ ] Set the add-ons from hidden to public.
- [ ] Confirm the Store page shows all three prices.
- [ ] Buy one on a second machine and confirm the licence follows the account.
- [ ] Refund it and confirm CoreScout drops back to Basic within six hours,
      keeping everything it had learned.
