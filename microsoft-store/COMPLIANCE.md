# Compliance notes

The parts of the Microsoft Store Policies that CoreScout actually touches, and
what the answer is. Written for whoever fills in Partner Center and for
whoever gets the certification report if something is rejected.

## Age rating

**Everyone.**

The rating comes from the questionnaire, not from a choice, so these are the
answers that produce it. Every one is "no": no violence, no sexual content, no
profanity, no gambling or simulated gambling, no in-game currency, no user
interaction, no user-generated content, no sharing of location, no personal
information collected, no advertising.

The one question that gives people pause is **in-app purchases**, which is
"yes". Digital purchases do not affect an Everyone rating; they add a
"Purchases" disclosure to the listing, which is correct and should be there.

## Data and privacy

Answered in full at the bottom of `PRIVACY.md`. The summary: nothing is
collected, nothing is transmitted except Store commerce and Store updates, no
advertising identifiers, no third-party analytics.

## The listening socket, which a reviewer will ask about

CoreScout binds a TCP port on `127.0.0.1`. This is the thing most likely to
generate a question, so the answer is written down before it is asked.

- **Why.** CoreScout is four programs: the window, the always-running service,
  the command line tool and the MCP bridge that a coding agent launches. The
  service holds the state; the other three ask it. A loopback socket is how
  they talk.
- **It is not reachable from anywhere else.** The bind address is
  `127.0.0.1`, never `0.0.0.0`. Nothing outside the machine can connect, and
  Windows Firewall is never asked for an exception, because none is needed.
- **It is authenticated.** The service mints a random bearer token at startup
  and writes it, with the port, into a file in its own data directory. A caller
  without the token gets 401. Another program on the machine cannot reach it by
  scanning ports; it would have to read a file in the user's profile, at which
  point it already has everything CoreScout has.
- **The port is not fixed.** It is whatever the OS gives, published in that
  same file, so two accounts on one machine do not collide.

`crates/product-api/src/http.rs` and `endpoint.rs`.

## Full trust

The package declares `runFullTrust`, and needs it. CoreScout reads processor
performance counters and starts processes; a sandboxed app can do neither.
Every Win32 desktop application in the Store declares this, and it is what
makes it a packaged desktop app rather than a UWP one.

## Starting with Windows

Declared as a `windows.startupTask`, **disabled by default**. Windows then
lists CoreScout under Settings > Apps > Startup, switched off, and the user
turns it on there if they want it.

CoreScout does not write the registry Run key, does not enable itself, and does
not ask to on first launch. Adding yourself to a person's startup uninvited is
not a decision a product gets to make, and Store policy agrees.

## Execution aliases

Two: `corescout` and `corescout-mcp`. They put the command line tool and the
MCP bridge on the user's PATH, which is what makes "add this one line to your
agent's config" true.

Both are declared on `<Application>` entries with `AppListEntry="none"`, so
they do not appear in the Start menu. Three CoreScout tiles would be worse than
useless. This is why the manifest has three `<Application>` elements rather
than one: an execution alias names a single executable and may be declared once
per Application.

## What CoreScout runs

Capabilities are procedures CoreScout has evidence for, which an agent can run
by name. This is the most sensitive thing in the product and is constrained in
the code rather than by policy:

- A capability is a fixed list of steps recorded when it was created. It is not
  an arbitrary command channel and nothing can add a step at run time.
- Every capability has preconditions, a verification step, and a rollback.
- Running one requires the `Automation` entitlement and an autonomy level above
  the default. CoreScout ships in **Suggest**, which proposes and does not act.
- Every run is written to an append-only audit log with the reason.
- There is an emergency pause that stops everything.

Nothing in CoreScout downloads and executes code, and nothing evaluates a
string as a program. There is no remote code execution path because there is no
remote.

## Accessibility

The interface is keyboard navigable, uses semantic elements with accessible
names, respects the system light and dark setting, and honours
`prefers-reduced-motion`. Contrast meets WCAG AA in both themes. The mirror
visualisation is decorative and marked `aria-hidden`; everything it shows is
also written out in text beside it, which is why that is safe.

## Restricted capabilities

None declared beyond `runFullTrust`. No `broadFileSystemAccess`, no
`allowElevation`, no device capabilities. CoreScout never asks for elevation
and does not function differently as administrator.

## Third-party content and licences

No bundled fonts, no stock imagery, no trademarked assets. The icon and every
graphic is original. Rust dependencies are permissive-licensed; `cargo deny` or
`cargo license` will produce the list if a reviewer ever asks, and none are
copyleft.

Claude Code, Codex and Cursor are named in the listing as products CoreScout
interoperates with. That is nominative use and is accurate: CoreScout speaks the
Model Context Protocol and works with anything that does. No logos are used,
and the listing does not imply endorsement by or affiliation with Anthropic,
OpenAI or Anysphere.

## Pricing and commerce

Everything is sold as a Microsoft Store add-on. There is no alternative payment
method inside the app, no external checkout link, and no way to pay that
bypasses Microsoft's commerce, which is what Store policy requires for digital
goods consumed in the app.

`ADDONS.md` has the three add-ons. The one-week trial is CoreScout's own, needs
no card and no Microsoft account, and is not a Store trial.

## Functionality after the trial

CoreScout keeps working when the trial ends. It keeps watching the machine, and
everything already learned stays readable. What stops is learning anything new,
the capability system, cross-agent memory, history beyond the last day, and the
higher autonomy levels.

This matters for certification: an app that becomes a purchase prompt when a
trial ends is a policy problem as well as a bad product. CoreScout does not,
and `integration/tests/product_licence.rs` has the tests that keep it that way.
