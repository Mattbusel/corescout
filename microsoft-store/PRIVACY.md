# CoreScout privacy policy

This is the source text. It is published at `https://corescout.dev/privacy`,
which is the URL given to Partner Center. Microsoft requires that URL to
resolve before it will certify the submission.

The text is short because the product is. Most of a privacy policy describes
what a company does with data it collects, and CoreScout does not collect any.

---

## Privacy policy

Last updated: 7 September 2026

### The short version

CoreScout runs entirely on your computer. It has no account, no server and no
telemetry. It does not send what it observes anywhere, including to us. We do
not receive it, so there is nothing for us to sell, share, lose or be compelled
to hand over.

### What CoreScout stores, and where

Everything CoreScout knows is in one folder on your machine. The app shows you
the exact path on its Privacy page and will open it for you.

That folder holds:

- **Measurements of your computer.** Processor counters, memory and device
  activity, sampled a few times a second, and the patterns CoreScout derives
  from them.
- **What your AI did.** The commands and tools your coding agent ran, how long
  they took, and whether they worked. Values that look like secrets are
  stripped before anything is written down.
- **What CoreScout concluded.** Patterns, failure modes, verified procedures,
  and the evidence behind each one.
- **Your settings and permissions.**

CoreScout does not read or store your source code, your files, your keystrokes,
your screen, or the contents of your conversations with an AI. It records that
a command was run and how it turned out, not what you were working on.

### Deleting it

The Privacy page has two buttons. "Forget my AI history" removes everything
CoreScout observed about your agent's work. "Forget everything" removes the lot.
Uninstalling CoreScout removes the folder along with the app.

There is no copy anywhere else, so deleting it is the end of it.

### What leaves your computer

Two things, and only when you choose them:

- **Buying a licence.** Pro and Lifetime are sold through the Microsoft Store.
  When you buy, Microsoft handles the payment and the receipt under Microsoft's
  privacy policy. We never see your payment details, your name, or your email
  address; we see aggregate sales figures in Partner Center. CoreScout asks the
  Store whether this Microsoft account owns a licence, which tells us nothing
  and tells the Store only that you opened the app.
- **Updates.** Delivered by the Microsoft Store, on Microsoft's terms.

Nothing else. CoreScout has no analytics, no crash reporting, no update ping of
its own, and no first-run beacon.

### What we collect about you

Nothing. There is no account to create and no way to identify an installation.
If you email support, we have that email, and we keep it only as long as the
conversation needs.

### Children

CoreScout is a developer tool and is not directed at children. It collects
nothing from anyone, including them.

### Changes

If this policy ever changes, the date above changes with it and the previous
version stays available. A change that meant CoreScout started sending
something somewhere would be announced in the app, not only here.

### Contact

`support@corescout.dev`

---

## Notes for the submission, not part of the published policy

Partner Center asks a separate set of data-declaration questions. The answers
that follow from the above:

| Question | Answer |
|---|---|
| Does this product collect personal information? | No |
| Does it transmit data off the device? | Only Store commerce and updates, both Microsoft's |
| Does it use advertising identifiers? | No |
| Does it contain third-party analytics? | No |
| Does it access the network? | Loopback only, for its own processes |

The loopback answer matters and is easy to get wrong. CoreScout binds a port on
127.0.0.1 so its window, its command line tool and its MCP bridge can talk to
the service. That is local inter-process communication, not network access, and
the port is bearer-token authenticated so another program on the machine cannot
read it by guessing the port. `COMPLIANCE.md` covers this in full, because a
reviewer who sees a listening socket will ask.
