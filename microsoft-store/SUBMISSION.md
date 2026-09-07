# Filling in the submission

What to click, section by section, in the order Partner Center lists them.

## Why any of this is manual

Microsoft's submission API can do all of it, including creating the add-ons.
It cannot do the *first* submission: the app has to have one completed
submission, with the age-rating questionnaire answered, before the API will
touch it. That is Microsoft's rule and there is no way around it.

So: this once, by hand. `automation/` has the tooling for everything after,
and it is worth setting up because it also handles the add-ons and every future
release.

One warning that matters later. Once a submission has been created through the
API, editing it in Partner Center breaks it: the API can no longer commit it,
and it can end up stuck in a state where the only fix is to delete it and start
again. Pick one or the other per submission, never both.

---

## Pricing and availability

| Field | Answer |
|---|---|
| Markets | All markets |
| Audience | Public audience |
| Discoverability | Make this product available and discoverable in the Store |
| Schedule | Release as soon as it passes certification |
| Base price | **Free** |
| Free trial | **No free trial** |
| Sale pricing | None |
| Organisational licensing | Leave both unchecked |

The base price is free and that is deliberate. CoreScout is a free download
with a one-week Pro trial built in; the money is in the three add-ons. Setting
a base price here would put a paywall in front of the download instead.

"No free trial" is about the *Store's* trial mechanism, which is a
time-limited version of a paid app. CoreScout's week of Pro is its own, needs
no card, and has nothing to do with this setting.

## Properties

| Field | Answer |
|---|---|
| Category | **Developer tools** |
| Subcategory | **Development kits** |
| Secondary category | None |
| Product declarations | See below |
| System requirements | See below |
| Support contact info | `support@corescout.dev` |
| Privacy policy URL | `https://corescout.dev/privacy` |
| Website | `https://corescout.dev` |

Not "Utilities & tools". CoreScout is bought by developers to change how their
development tooling behaves, and a utilities listing puts it beside disk
cleaners.

**Product declarations**, all unchecked except where noted:

- This app allows users to make purchases, but does not use the Microsoft Store
  commerce system: **unchecked**. Everything goes through Store commerce.
- This app has been tested to meet accessibility guidelines: **checked**.
- This app depends on non-Microsoft drivers or NT services: **unchecked**.
- This app can function with limited or no network connectivity: **checked**.
  CoreScout needs no network at all except to buy something.
- Customers can install this app to alternate drives: **checked**.
- This app contains or supports advertising: **unchecked**.

**System requirements**, minimum hardware: leave everything unchecked. CoreScout
needs no touch, no camera, no microphone, no GPU, no NPU, no specific memory or
processor. It reads more from processors that expose performance counters and
says so in its own interface when a machine exposes less.

## Age ratings

Answer the questionnaire; do not try to pick a rating directly. Every answer is
**no** except the two noted.

| Question | Answer |
|---|---|
| Is this a game? | **No** |
| Violence of any kind | No |
| Sexual content or nudity | No |
| Profanity or crude humour | No |
| Alcohol, tobacco or drugs | No |
| Gambling, real or simulated | No |
| Horror or fear themes | No |
| Discrimination or hate speech | No |
| Does it allow users to interact or communicate? | No |
| Does it allow sharing of user-generated content? | No |
| Does it share the user's location? | No |
| Does it collect or transmit personal information? | **No** |
| Does it allow purchases of digital goods? | **Yes** |
| Does it contain advertising? | No |
| Does it display a link to a website? | Yes |

That produces **Everyone**. The digital-purchases answer adds a "Purchases"
disclosure to the listing, which is correct and should be there.

"Collect or transmit personal information" is genuinely no. CoreScout has no
account, no telemetry and no server; everything it observes stays in one folder
on the machine. `PRIVACY.md` and `COMPLIANCE.md` have the full reasoning, and
`COMPLIANCE.md` also has the prepared answer for the loopback socket, which is
the thing a reviewer is most likely to ask about.

## Packages

Upload `dist\CoreScout.msix`. Build it with:

```
powershell -File scripts\msix.ps1
```

It is unsigned on purpose: Microsoft signs Store submissions. The script prints
the identity it used and refuses anything that does not look like a Partner
Center identity.

Device families: leave the defaults. The manifest already targets Windows
Desktop from build 17763, and this is an x64 desktop app.

## Store listings

Use **Import listings** rather than typing. `listing-import/` has the filled
file; `LISTING.md` is the same copy in a form a person can read.

Screenshots are uploaded separately, in `screenshots/out`. Captions are in the
table at the bottom of `LISTING.md`. Do not upload anything from
`screenshots/preview` -- those are downscaled copies that exist only so the
originals can be opened in tools that will not load them.

The 300x300 Store logo is `assets/StoreLogo300.png`.

## Submission options

Publishing hold: **none**, unless you want to hold the release and press the
button yourself, in which case choose to publish manually.

Notes for certification: worth filling in. Suggested text:

```
CoreScout is a local developer tool. It has no account, no server and no
telemetry.

It binds a TCP port on 127.0.0.1 so that its four programs (the window, the
background service, the command line tool and the MCP bridge) can talk to each
other. The bind address is never 0.0.0.0, no firewall exception is requested,
and the socket requires a bearer token that the service generates at startup
and writes into its own data directory. This is local inter-process
communication, not network access.

To see the product working you will need an MCP-capable AI client. Without one,
the app still runs and observes the machine: open it and the Home and Computer
screens fill in within a minute or two. The AI screen shows the one-line
configuration for connecting a client.

The app is fully functional with no purchase. The three add-ons unlock
continued learning after a one-week trial; the app keeps observing and keeps
everything already learned, on every tier, indefinitely.
```

---

## After this submission goes through

Set up the API and stop doing this by hand. `automation/README.md` has the
steps. It needs an Azure AD application associated with the Partner Center
account, which is a one-time setup, and then it can create the add-ons, upload
packages, replace listings and submit.

The add-ons in `ADDONS.md` can be created through the API without any manual
submission first, so they do not have to be done by hand at all.
