# Status

Where the Microsoft Store submission actually stands, on 7 September 2026.

Written so that the things that are not done are as easy to find as the things
that are. A status document that only lists progress is a status document
nobody can act on.

## What you can do right now

`dist\CoreScout.msix` is uploadable. It carries the real Partner Center
identity, it is version 1.0.0.0, it is unsigned because Microsoft signs Store
submissions, and it contains all four programs.

```
powershell -File scripts\msix.ps1
```

## Done

**The package.** MSIX with the real identity `Tensorust.CoreScout`. Three
`<Application>` entries so the CLI and the MCP bridge get execution aliases
without putting three tiles in the Start menu. `runFullTrust`, and a startup
task that ships disabled. `msix.ps1` now verifies its own staging, after an
earlier build shipped the command line tool in place of the window.

**Commerce, in the product.** Everything is a Store add-on. The desktop shell
opens Microsoft's purchase dialogue over its own window, because a subscription
add-on cannot be sold by deep link. The service asks the Store what the account
owns at startup and every six hours, and again immediately after a purchase.
Nothing is unlocked on the strength of a dialogue returning success: the Store
is the record, and CoreScout asks it.

**Entitlements.** Basic, Trial and Pro. One week of Pro on install, with no
card and no Microsoft account. After that CoreScout keeps watching and keeps
everything it learned; what stops is learning more, capabilities, cross-agent
memory, history beyond the last day, and the higher autonomy levels. There are
tests for all of that, including one that fails if the upgrade screen ever
lists something free as a Pro feature, which it once did.

**Listing.** `LISTING.md` has every Partner Center field. The price block is
generated from `crates/licence/src/plans.rs`, and
`build-listing.ps1 -Check` fails if they drift.

**Compliance.** `COMPLIANCE.md` has the age-rating answers, the justification
for full trust, and a written answer for the loopback socket before a reviewer
asks about it. `PRIVACY.md` has the policy text and the data declarations.

**Screenshots.** Seven, from a real running CoreScout.
`screenshots/capture.ps1` reproduces them end to end.

## Not done, and why

### Blocked on Partner Center

**The three add-ons do not exist.** Nothing can be bought until they are
created and published. `ADDONS.md` has the exact Product ID strings, which must
match the tokens in the code, and the three fields Partner Center will not let
you change afterwards. This is the single most consequential remaining step:
the price can never be raised once published.

**The purchase path is unverified.** Every layer beneath the Store API is
tested, and the Store API cannot be reached from a build that Windows did not
install from the Store. `corescout_store_commerce::owned` correctly returns
`NotPackaged` on every development build, and its tests assert that. What
cannot be checked here is whether a real purchase of a real add-on grants Pro.
It should: the code matches on the in-app offer token and the token is chosen
by us rather than assigned. But it has not happened, and this document is not
going to say it has.

### Blocked on an elevated prompt

**Install, upgrade, uninstall and reinstall are unverified on Windows.**
Side-loading a package requires trusting a certificate in the machine store,
which needs administrator rights. `scripts\install-local.ps1` does the whole
sequence in one command and is the only script here that needs elevation.
Until it is run:

- The package has never been installed.
- The Windows App Certification Kit has never been run against it, because
  `appcert.exe` also needs elevation.
- Whether data survives an uninstall-with-reinstall is unconfirmed, though
  `crates/storage/src/packaged.rs` puts packaged data in the package's own
  store, which is where it should go.

### Blocked on a published website

**The privacy policy URL does not resolve.** `PRIVACY.md` is written, but
Microsoft checks that `https://corescout.dev/privacy` loads before it will
certify. The site in `web/website` has to be deployed and the domain pointed at
it, or the URL changed to wherever it actually lives.

### Worth doing before submitting

**Retake the screenshots from an installed package.** The current set was taken
from an unpackaged build, so the Privacy page shows a temporary folder and the
AI page shows a developer's path to `corescout-mcp.exe`. A packaged build shows
the package data folder and the bare execution aliases, which is what a buyer
gets. This is cosmetic but it is on two of the seven screenshots, and one of
them has a username in it.

## One decision that cannot be undone

The Microsoft Store never allows an add-on's price to be raised. Not with a
sale, not with a promotional code, which do not work on subscription add-ons
either. $49.99 a month, $399.99 a year and $999.99 once are permanent ceilings
from the moment they publish.

Lowering is always allowed. If there is any doubt, publish higher.

## What was dropped, and why

Seat-based pricing at $99 per user per month, and negotiated enterprise
pricing. The Store cannot express either, and the alternative was selling them
through a second channel with its own payment processor, licence keys and
delivery. Two ways to pay is two ways to get billing wrong.

The `Team` and `Enterprise` tiers were removed from the code along with the
plans, rather than left in place unreachable. `crates/licence/src/tier.rs` has
a test that every entitlement is reachable by buying something, which is what
stops that decision from quietly rotting into features nobody can have.
