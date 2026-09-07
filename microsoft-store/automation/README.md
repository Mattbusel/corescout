# Filling the submission automatically

Partner Center has no API for a product's first submission: Microsoft requires
one completed submission, age-rating questionnaire included, before the
submission API will touch an app. So this drives the browser instead.

```
node fill.mjs           every section
node fill.mjs listings  one section
node fill.mjs --dry     report what would change, change nothing
node status.mjs         what Partner Center currently says
```

A real Chromium window opens against a profile in `.browser`. The first run
waits while you sign in; after that the session is remembered.

**It never presses "Submit for certification."** The last look before this
becomes public is a person's.

## What it fills

| Section | What it sets |
|---|---|
| `availability` | All worldwide markets, public, discoverable, release on certification |
| `properties` | Category, website, support contact, product declarations, privacy policy text |
| `ageratings` | The whole IARC questionnaire |
| `packages` | Uploads `dist/CoreScout.msix`, deletes a rejected one first |
| `listings` | Description and release notes from `../LISTING.md`, plus the screenshots |
| `options` | Release timing and the `runFullTrust` justification |

Every step reads the current state first and skips what is already right, so
running it twice is safe and a re-run after a failure does not undo anything.

## What it deliberately does not do

**Tick the IARC terms checkbox.** The last control on the age-ratings page is
"I agree to the IARC Terms of Use and I am the age of majority in my
jurisdiction". That is a personal legal attestation about a specific human
being. Everything else on that page is a checkable fact about the software;
this one is not, and nothing automated should make it on someone's behalf.

**Set the price.** A Store price can never be raised after publishing, only
lowered. That is the one decision in the submission that cannot be undone, and
it should be typed by the person whose product it is.

**Submit.** See above.

## Why headed, and why the odd selectors

Partner Center sits behind bot protection that returns "Access Denied" to a
headless browser. It also builds its forms from web components: the real input
sits inside a shadow root, its label is slotted in from outside, and the
checked state is a class on a wrapping element rather than the input's own
`checked`. So `getByRole("checkbox", { name })` matches nothing and
`isChecked()` lies.

What does work is walking the shadow trees looking for the input's `name`
attribute, which is stable and meaningful (`accessibility-checkbox`,
`usesGenAI-checkbox`, `question#1109`). `check`, `checkByLabel`, `pick` and
`press` in `fill.mjs` all do that, each for a slightly different case, and each
says why in its own comment.

`discover.mjs` re-derives the page structure when Microsoft changes it, and
writes `discovered.json`. Use it rather than guessing twice.

## Things this found that a person would have missed

The package was rejected the first time with *"specifies a headless app... you
don't have permission to create a headless app"*. `AppListEntry="none"` on the
command line and MCP applications, which existed so only the window got a Start
menu tile, needs a HeadlessAppBypass waiver this account does not have. The
manifest now lists all three.

The `runFullTrust` justification field silently truncates at 500 characters.
The first version was 1074 characters and the second was 583; both saved
looking complete, with half the reasoning gone.
