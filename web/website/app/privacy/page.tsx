import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "What CoreScout observes",
  description:
    "Everything CoreScout records, where it is kept, and what it never touches. Nothing is transmitted.",
};

/**
 * The privacy page.
 *
 * Written as a list of what is observed rather than as a policy. A policy is a
 * document about permissions; this is a description of behaviour, and the
 * application shows the same list with live counts beside it.
 */
export default function Page() {
  return (
    <main className="mx-auto max-w-3xl px-6 py-20">
      <h1 className="text-3xl font-semibold tracking-[-0.03em]">
        What CoreScout observes
      </h1>
      <p className="mt-3 text-[15px] leading-relaxed text-ink-soft">
        All of it stays on your machine. There is no account, no server and no
        telemetry, opt-in or otherwise. The application shows this same list
        with live counts beside it, and a button that deletes any of it.
      </p>

      <Group
        title="Your machine"
        items={[
          ["Processor layout", "Cores, threads, caches, and how they are arranged."],
          ["Live counters", "How busy each part is, several times a second."],
          [
            "Recurring states",
            "Patterns CoreScout found in those counters and named itself.",
          ],
        ]}
      />

      <Group
        title="What your AI does"
        items={[
          [
            "Commands and tool calls",
            "The command line, with credentials removed before it is written down.",
          ],
          [
            "Outcomes",
            "Exit codes, how long it took, and whether anything verified the result.",
          ],
          ["Retries", "When the same thing was attempted again after failing."],
          ["Folders", "Which repository or folder the work happened in, by path."],
          ["Sessions", "When each AI was working, and for how long."],
        ]}
      />

      <Group
        title="Never"
        tone="never"
        items={[
          ["Your source code", "CoreScout does not read files inside your repositories."],
          ["Your prompts", "Or the responses, or anything the model was reasoning about."],
          [
            "Credentials",
            "Tokens, keys and passwords are stripped where data enters, not where it is shown.",
          ],
          [
            "Anything, anywhere",
            "Nothing is transmitted off this machine. There is no endpoint to transmit to.",
          ],
        ]}
      />

      <section className="mt-12">
        <h2 className="text-lg font-semibold tracking-[-0.01em]">Where it lives</h2>
        <pre className="mt-3 overflow-x-auto rounded-xl border border-line bg-sunken p-4 font-mono text-[12px] text-ink-soft">
{`%LOCALAPPDATA%\\CoreScout\\
  corescout.redb    what it has learned
  mirror.ring       recent readings, a fixed 32 MB, forever
  endpoint.json     how the application finds the service`}
        </pre>
        <p className="mt-3 text-[14px] leading-relaxed text-ink-muted">
          Local rather than roaming: a file of machine telemetry has no business
          following you onto another computer. Deleting that folder deletes
          everything, and CoreScout starts again from nothing.
        </p>
      </section>

      <section className="mt-10">
        <h2 className="text-lg font-semibold tracking-[-0.01em]">
          How to check any of this
        </h2>
        <p className="mt-2 text-[15px] leading-relaxed text-ink-soft">
          The source is public, and the part that would have to do the
          transmitting does not exist: no crate in this project opens an
          outbound connection. The local API is bound to loopback and guarded by
          a token in a file only your account can read, and the whole dependency
          list is short enough to go through in an afternoon.
        </p>
      </section>
    </main>
  );
}

function Group({
  title,
  items,
  tone,
}: {
  title: string;
  items: [string, string][];
  tone?: "never";
}) {
  return (
    <section className="mt-12">
      <h2 className="text-lg font-semibold tracking-[-0.01em]">{title}</h2>
      <dl className="mt-4 divide-y divide-line border-y border-line">
        {items.map(([name, body]) => (
          <div key={name} className="grid gap-1 py-4 sm:grid-cols-[200px_1fr] sm:gap-6">
            <dt
              className="text-[14px] font-medium"
              style={tone === "never" ? { color: "var(--color-ink-muted)" } : undefined}
            >
              {name}
            </dt>
            <dd className="text-[14px] leading-relaxed text-ink-soft">{body}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}
