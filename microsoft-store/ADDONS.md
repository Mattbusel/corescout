# The three add-ons

Everything CoreScout sells is a Microsoft Store add-on. There is no payment
processor, no licence key to email, and no `corescout.dev/buy` page. Microsoft
takes every payment, handles tax and refunds, and the licence follows the
buyer's Microsoft account onto their other machines.

Create these in Partner Center under **CoreScout > Add-ons > Create a new
add-on**. Nothing can be bought until they exist and are published.

## Get the Product ID exactly right

The **Product ID** field is what Partner Center calls the in-app offer token,
and CoreScout matches on it to decide what a purchase granted. It is chosen by
us rather than assigned by Microsoft, which is the point: the code is correct
before the add-on exists, and nothing has to be pasted back into the source
after publishing.

A typo here produces the worst failure available in this product: the purchase
succeeds, Microsoft takes the money, and CoreScout does not unlock. Copy these
strings rather than typing them.

| Product ID | Type | Price | Subscription period | Free trial |
|---|---|---|---|---|
| `corescout.pro.monthly` | Subscription | $49.99 USD | 1 month | None |
| `corescout.pro.yearly` | Subscription | $399.99 USD | 1 year | None |
| `corescout.lifetime` | Durable | $999.99 USD | n/a, no expiry | n/a |

`crates/licence/src/plans.rs` holds the same three strings, and
`cargo run -p corescout-licensor -- plans` prints them to check against.

## Why no free trial on the subscriptions

CoreScout already gives everyone one week of Pro on install, with no card and
no Microsoft account involved. The Store's own trial is a worse version of the
same offer: it is 1 week or 1 month, it requires a valid payment method up
front, and each customer can only ever claim it once, permanently, for that
add-on.

Turning both on would mean a person who used CoreScout's week and then
subscribed gets asked to start another trial, or does not, depending on
something they cannot see. One trial, ours, no card.

## Things that cannot be changed afterwards

Partner Center is unforgiving about three fields, and all three are set at
creation:

- **The price can never be raised.** Lowering is always allowed; raising is
  never. $49.99, $399.99 and $999.99 are therefore permanent ceilings. Sales
  and promotional codes do not work on subscription add-ons either, so the
  usual escape hatch is not there.
- **The subscription period cannot be changed** once the add-on is published.
  That is why monthly and yearly are two separate add-ons rather than one with
  two options.
- **The trial period cannot be changed or removed** once published. Which is
  the other reason to leave it off.

Set the price deliberately. This is the one decision in the whole submission
that cannot be undone.

## Visibility

Public for all three, once the app itself is published. While testing, set
them to one of the **Hidden in the Store** options: a hidden add-on can still
be bought from inside the app by an account with access, which is exactly what
a purchase test needs, without the add-on appearing on a listing for an app
that is not live yet.

## What happens in the product

1. Someone opens Settings and the Plan tab, and presses a plan's button.
2. The desktop shell calls `purchase` with that plan's token, which asks the
   Store to find the add-on with that in-app offer token and opens Microsoft's
   own purchase dialogue over the CoreScout window.
3. On success the shell asks the service to re-read what the Store says this
   account owns. Nothing is unlocked locally on the strength of a dialogue
   returning success: the Store is the record, and CoreScout asks it.
4. The service also re-reads every six hours, so a purchase made on another
   machine, or a refund, is picked up without anybody doing anything.

`crates/store-commerce/src/lib.rs` is all of this, and it is about two hundred
lines because Microsoft is doing the hard part.

## What cannot be tested until these are published

The purchase path needs a real Store identity and a real published add-on.
Until then `corescout_store_commerce::owned` returns `NotPackaged` on every
development build, which is the correct answer and is what its tests assert.

`STATUS.md` records this as an outstanding verification rather than claiming
the path is proven.
