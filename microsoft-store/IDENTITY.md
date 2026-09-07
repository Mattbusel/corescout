# Store identity

Assigned by Partner Center when the name `CoreScout` was reserved, on
2026-09-07. These are the values the package must carry, exactly.

None of this is secret. Every one of these values ships inside the published
package or appears in the Store URL. The signing key, which is secret, is
Microsoft's: Store submissions are signed by Microsoft, and this repository
never holds a key that can sign a package the public would trust.

## In the manifest

| Element | Value |
|---|---|
| `Package/Identity/Name` | `Tensorust.CoreScout` |
| `Package/Identity/Publisher` | `CN=98B77C5C-8582-4364-B50E-0922AE25FBF6` |
| `Package/Properties/PublisherDisplayName` | `Tensorust` |

`scripts/msix.ps1` defaults to these, so an ordinary build is uploadable and
nobody has to remember to pass them. They are substituted into
`packaging/msix/AppxManifest.xml`, which holds placeholders so the manifest can
be read without a hex string in the middle of it.

## Derived, and not written in the manifest

| | |
|---|---|
| Package Family Name | `Tensorust.CoreScout_3amv67zrsdrs2` |
| Package SID | `S-1-15-2-2216470811-4283592702-2581633224-892295566-1957665442-2738017114-3241628879` |

The family name is what a packaged CoreScout reports at runtime, and it is what
decides where its data lives: `%LOCALAPPDATA%\Packages\Tensorust.CoreScout_3amv67zrsdrs2\LocalCache\Local\CoreScout`.
`crates/storage/src/packaged.rs` asks Windows for it rather than hard-coding
it, so this table is documentation and not a dependency.

## The listing

| | |
|---|---|
| Store ID | `9PJPNV5VDBV0` |
| Store page | `https://apps.microsoft.com/detail/9PJPNV5VDBV0` |
| Deep link | `ms-windows-store://pdp/?productid=9PJPNV5VDBV0` |

The Store ID is a constant in `crates/licence/src/plans.rs`, which is what the
upgrade screen's Lifetime button opens.

## The add-on, which does not exist yet

CoreScout Lifetime is sold as a durable add-on and has to be created in
Partner Center under Add-ons before it can be bought. Two values matter when
it is:

| Field | Value it must have |
|---|---|
| Product type | Durable, with no expiry |
| Product ID (in-app offer token) | `corescout.lifetime` |
| Price | $999.99 USD tier |

The token is the important one. `crates/store-commerce/src/lib.rs` decides what
an add-on grants by matching that token, deliberately not by matching the Store
product ID, because the Store product ID does not exist until the add-on is
created and would have to be pasted back into the code. Set the token to
exactly `corescout.lifetime` and the code already knows what to do with it.

Once the add-on exists, its Store product ID can optionally be put in
`STORE_LIFETIME_ADDON_ID` in `plans.rs` so the buy button deep-links to the
add-on rather than to the app page. That is a refinement, not a requirement:
with it `None`, the button opens the CoreScout listing, where the add-on is
purchasable anyway.

## MSA app id

`8eea9990-3735-4069-b6c8-225aebfa66bb`

Not used. It exists for apps that authenticate users against a Microsoft
account, and CoreScout has no accounts. Recorded here so that its being unused
is a decision on the record rather than an oversight.
