import Link from "next/link";

import { Demo } from "../components/Demo";
import { Mirror } from "../components/Mirror";

/**
 * The one page.
 *
 * Short on purpose. Everything below the hero exists to answer one of three
 * questions: what does it do, does it actually work, and what does it cost me.
 * There is no pricing section because there is no price, and no testimonials
 * because there are no users yet, and saying so is better than inventing them.
 */
export default function Page() {
  return (
    <main>
      <Hero />
      <Forgets />
      <How />
      <Loop />
      <Tools />
      <Result />
      <Inheritance />
      <Privacy />
      <Open />
      <Download />
    </main>
  );
}

function Section({
  id,
  children,
  className = "",
}: {
  id?: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <section id={id} className={`mx-auto max-w-5xl px-6 py-20 sm:py-24 ${className}`}>
      {children}
    </section>
  );
}

function Hero() {
  return (
    <div className="relative overflow-hidden">
      <div className="mx-auto grid max-w-5xl items-center gap-10 px-6 pb-8 pt-16 sm:pt-24 lg:grid-cols-[1.05fr_1fr]">
        <div className="rise">
          <h1 className="text-4xl font-semibold leading-[1.08] tracking-[-0.03em] sm:text-5xl">
            Your AI gets smarter.
            <br />
            <span className="text-ink-muted">Now its computer can too.</span>
          </h1>
          <p className="mt-6 max-w-lg text-[15px] leading-relaxed text-ink-soft">
            CoreScout learns how your machine and your AI work together,
            remembers what reality teaches them, and turns useful discoveries
            into better ways of working.
          </p>
          <div className="mt-8 flex flex-wrap items-center gap-3">
            <Link
              href="/download"
              className="rounded-md bg-accent px-5 py-2.5 text-sm font-medium text-paper transition-transform hover:-translate-y-px"
            >
              Download for Windows
            </Link>
            <a
              href="https://github.com/mattbusel/corescout"
              className="rounded-md border border-line-strong px-5 py-2.5 text-sm font-medium transition-colors hover:border-ink-faint"
            >
              View on GitHub
            </a>
          </div>
          <p className="mt-4 text-[13px] text-ink-faint">
            Free. Open source. Local-first.
          </p>
        </div>

        <div className="rise" style={{ animationDelay: "120ms" }}>
          <Mirror />
        </div>
      </div>
    </div>
  );
}

function Forgets() {
  return (
    <Section className="hairline">
      <h2 className="max-w-2xl text-2xl font-semibold tracking-[-0.02em] sm:text-3xl">
        Your AI forgets what the computer learns.
      </h2>
      <div className="mt-8 grid gap-8 text-[15px] leading-relaxed text-ink-soft sm:grid-cols-2">
        <p>
          Every session starts from nothing. The build that fails the same way
          each time, the deploy that reports success forty seconds before the
          service is reachable, the test that only breaks when the machine is
          busy: your AI rediscovers all of it, every time, and then the window
          closes.
        </p>
        <p>
          Nobody writes any of it down, because the thing that would know is the
          computer, and computers do not learn from their own experience. This
          one does.
        </p>
      </div>
      <p className="mt-10 border-l-2 border-accent pl-5 text-lg font-medium tracking-[-0.01em]">
        The model doesn&rsquo;t learn from every task. Its computer does.
      </p>
    </Section>
  );
}

function How() {
  const steps = [
    {
      title: "See",
      body: "CoreScout watches the machine several times a second, and watches what your AI does to it: commands, exit codes, retries, what changed.",
    },
    {
      title: "Learn",
      body: "It notices what recurs. Which operations fail here, what the successful runs had in common, which state the machine was in.",
    },
    {
      title: "Verify",
      body: "Then it does the part almost nobody does: it tests. On a fraction of occasions it decides by coin flip rather than by belief, because only those trials can tell a cause from a coincidence.",
    },
    {
      title: "Improve",
      body: "What survives becomes a capability with its evidence attached. You approve it. Your AI can then use it, and so can the next one.",
    },
  ];
  return (
    <Section id="how" className="hairline">
      <p className="mb-3 text-[11px] font-semibold uppercase tracking-[0.08em] text-ink-faint">
        How it works
      </p>
      <div className="grid gap-x-10 gap-y-8 sm:grid-cols-2">
        {steps.map((step, index) => (
          <div key={step.title}>
            <div className="flex items-baseline gap-3">
              <span className="font-mono text-[12px] text-ink-faint">
                0{index + 1}
              </span>
              <h3 className="text-lg font-semibold tracking-[-0.01em]">{step.title}</h3>
            </div>
            <p className="mt-2 text-[15px] leading-relaxed text-ink-soft">{step.body}</p>
          </div>
        ))}
      </div>
    </Section>
  );
}

function Loop() {
  return (
    <Section className="hairline">
      <div className="mb-8 max-w-2xl">
        <h2 className="text-2xl font-semibold tracking-[-0.02em] sm:text-3xl">
          What that looks like
        </h2>
        <p className="mt-3 text-[15px] leading-relaxed text-ink-soft">
          One failure, noticed. One correlation, tested rather than trusted. One
          procedure that survives it.
        </p>
      </div>
      <Demo />
    </Section>
  );
}

function Tools() {
  const tools = ["Claude Code", "Codex", "OpenCode", "Cursor", "Any MCP client"];
  return (
    <Section className="hairline">
      <div className="grid gap-8 lg:grid-cols-[1fr_1.1fr]">
        <div>
          <h2 className="text-2xl font-semibold tracking-[-0.02em] sm:text-3xl">
            It works with the AI you already use
          </h2>
          <p className="mt-3 text-[15px] leading-relaxed text-ink-soft">
            CoreScout speaks the Model Context Protocol, so connecting it is one
            line and nothing is built around a single vendor. It generates the
            exact configuration for your client, and usually writes it for you.
          </p>
          <ul className="mt-6 flex flex-wrap gap-2">
            {tools.map((tool) => (
              <li
                key={tool}
                className="rounded-full border border-line px-3 py-1 text-[13px] text-ink-soft"
              >
                {tool}
              </li>
            ))}
          </ul>
        </div>
        <div className="rounded-xl border border-line bg-sunken p-5">
          <p className="mb-3 text-[11px] font-semibold uppercase tracking-[0.08em] text-ink-faint">
            What your AI is told when it connects
          </p>
          <pre className="overflow-x-auto font-mono text-[12px] leading-relaxed text-ink-soft">
{`You are operating inside a computer running CoreScout.

CoreScout keeps an empirical model of this machine and of
how AI tools have worked with it. You can query it for
operational knowledge that outlives your session.

CoreScout separates association from causal evidence.
Anything labelled "Seen together" is a correlation it has
not tested; anything labelled "Verified" was measured under
randomised assignment. Check which before you rely on it.`}
          </pre>
          <p className="mt-3 text-[12px] text-ink-faint">
            Generated from what CoreScout currently knows and what it is
            currently permitted to do, so it cannot promise something the
            settings do not allow.
          </p>
        </div>
      </div>
    </Section>
  );
}

function Result() {
  return (
    <Section id="result" className="hairline">
      <p className="mb-3 text-[11px] font-semibold uppercase tracking-[0.08em] text-ink-faint">
        A real machine
      </p>
      <h2 className="max-w-2xl text-2xl font-semibold tracking-[-0.02em] sm:text-3xl">
        The first time this ran on hardware, it described itself
      </h2>
      <p className="mt-3 max-w-2xl text-[15px] leading-relaxed text-ink-soft">
        13th Gen Intel Core i7-13700KF, 16 physical cores, 24 logical. 900
        reflections over 36 seconds. Everything below was measured on that
        machine, and is a fact about it rather than a claim about yours.
      </p>

      <dl className="mt-10 grid grid-cols-2 gap-x-8 gap-y-8 sm:grid-cols-4">
        {[
          ["86", "observable parts"],
          ["32", "recurring states, self-discovered"],
          ["3", "concepts promoted, 29 refused"],
          ["42.4%", "less prediction error, knowing the state"],
        ].map(([value, label]) => (
          <div key={label}>
            <dt className="text-3xl font-semibold tracking-[-0.03em] tabular-nums">
              {value}
            </dt>
            <dd className="mt-1 text-[13px] leading-snug text-ink-muted">{label}</dd>
          </div>
        ))}
      </dl>

      <div className="mt-10 grid gap-4 sm:grid-cols-2">
        <blockquote className="rounded-xl border border-line bg-raised p-5">
          <p className="text-[15px] leading-relaxed">
            &ldquo;I distinguish 32 recurring states of myself, which I found
            rather than being told about.&rdquo;
          </p>
          <footer className="mt-3 text-[12px] text-ink-faint">
            The machine&rsquo;s own words, generated from what it had measured.
          </footer>
        </blockquote>
        <blockquote className="rounded-xl border border-line bg-raised p-5">
          <p className="text-[15px] leading-relaxed">
            &ldquo;I do not predict my own next state better than assuming
            nothing changes.&rdquo;
          </p>
          <footer className="mt-3 text-[12px] text-ink-faint">
            Also its own words. Results that go the wrong way are kept.
          </footer>
        </blockquote>
      </div>

      <p className="mt-8 max-w-2xl text-[13px] leading-relaxed text-ink-muted">
        There is more of this, including the experiments that produced nothing
        and the bugs only real hardware could expose, in{" "}
        <a
          className="underline decoration-line-strong underline-offset-2 hover:text-ink"
          href="https://github.com/mattbusel/corescout/blob/master/docs/REAL.md"
        >
          the results from real hardware
        </a>
        .
      </p>
    </Section>
  );
}

function Inheritance() {
  return (
    <Section className="hairline">
      <div className="max-w-2xl">
        <h2 className="text-2xl font-semibold tracking-[-0.02em] sm:text-3xl">
          Same AI. More experienced computer.
        </h2>
        <p className="mt-3 text-[15px] leading-relaxed text-ink-soft">
          What CoreScout learns belongs to the machine, not to the model that
          was there when it learned it. Close the session, switch to a different
          AI entirely, and the procedures, the failure modes and the machine
          states are all still there.
        </p>
        <p className="mt-3 text-[15px] leading-relaxed text-ink-soft">
          The repository ships a reproducible experiment for exactly this: one
          model works, its session ends, another connects, and what the second
          one can reach is measured.{" "}
          <span className="text-ink">
            The stronger claim &mdash; that this makes the second one measurably
            better &mdash; is not made here, because the experiment has to be run
            on a real workload first.
          </span>{" "}
          When it has been, the numbers will be on this page whichever way they
          come out.
        </p>
      </div>
    </Section>
  );
}

function Privacy() {
  return (
    <Section className="hairline">
      <div className="grid gap-8 lg:grid-cols-[1fr_1fr]">
        <div>
          <h2 className="text-2xl font-semibold tracking-[-0.02em] sm:text-3xl">
            It never leaves your machine
          </h2>
          <p className="mt-3 text-[15px] leading-relaxed text-ink-soft">
            No account, no server, no telemetry, not even an opt-out. Everything
            CoreScout has is in one folder that the application names, and one
            button deletes.
          </p>
          <p className="mt-3 text-[15px] leading-relaxed text-ink-soft">
            Secrets are stripped from commands before anything is written down,
            not before it is displayed. Source code is never read.
          </p>
          <Link
            className="mt-5 inline-block text-[14px] underline decoration-line-strong underline-offset-4 hover:text-ink"
            href="/privacy"
          >
            Exactly what it observes
          </Link>
        </div>
        <ul className="space-y-3 text-[14px] text-ink-soft">
          {[
            "Machine observations stay local.",
            "AI operational history stays local.",
            "Shell commands are stored with credentials removed.",
            "Nothing is transmitted, with or without consent.",
            "Storage is bounded: the history file is a fixed size, forever.",
          ].map((line) => (
            <li key={line} className="flex gap-3">
              <span className="mt-2 h-1 w-1 flex-none rounded-full bg-ink-faint" />
              {line}
            </li>
          ))}
        </ul>
      </div>
    </Section>
  );
}

function Open() {
  return (
    <Section className="hairline">
      <div className="grid gap-8 lg:grid-cols-[1.1fr_1fr]">
        <div>
          <h2 className="text-2xl font-semibold tracking-[-0.02em] sm:text-3xl">
            Open, and awkward about evidence
          </h2>
          <p className="mt-3 text-[15px] leading-relaxed text-ink-soft">
            CoreScout grew out of a research project about whether a machine can
            discover useful structure in itself. That project&rsquo;s habit was
            to publish results that went the wrong way, and the product kept it:
            the interface tells you when something is a correlation it has not
            tested, and it counts the ideas it rejected next to the ones it
            kept.
          </p>
          <p className="mt-3 text-[15px] leading-relaxed text-ink-soft">
            MIT or Apache-2.0, your choice. Around 1,300 tests, and the ones
            that record a negative result are kept deliberately.
          </p>
        </div>
        <div className="rounded-xl border border-line bg-sunken p-5 font-mono text-[12px] leading-relaxed text-ink-soft">
          <div className="text-ink-faint"># what the interface will tell you</div>
          <div className="mt-2">
            <span style={{ color: "var(--color-verified)" }}>Verified</span>
            <span className="text-ink-faint">
              {"  "}measured under randomised assignment
            </span>
          </div>
          <div className="mt-1">
            <span style={{ color: "var(--color-observed)" }}>Seen together</span>
            <span className="text-ink-faint">
              {"  "}a correlation, not yet tested
            </span>
          </div>
          <div className="mt-3 text-ink-faint">
            # and what it will not
          </div>
          <div className="mt-1">never used</div>
          <div>not measured</div>
          <div className="text-ink-faint">
            {"  "}rather than 0%, which is a claim
          </div>
        </div>
      </div>
    </Section>
  );
}

function Download() {
  return (
    <Section className="hairline">
      <div className="rounded-2xl border border-line bg-raised px-6 py-12 text-center sm:px-12">
        <h2 className="text-2xl font-semibold tracking-[-0.02em] sm:text-3xl">
          Let your computer start learning
        </h2>
        <p className="mx-auto mt-3 max-w-md text-[15px] leading-relaxed text-ink-soft">
          Install it, connect your AI, and carry on as normal. Come back
          tomorrow and see what it found.
        </p>
        <div className="mt-7 flex flex-wrap justify-center gap-3">
          <Link
            href="/download"
            className="rounded-md bg-accent px-5 py-2.5 text-sm font-medium text-paper transition-transform hover:-translate-y-px"
          >
            Download for Windows
          </Link>
          <a
            href="https://github.com/mattbusel/corescout"
            className="rounded-md border border-line-strong px-5 py-2.5 text-sm font-medium transition-colors hover:border-ink-faint"
          >
            Read the source
          </a>
        </div>
        <p className="mt-4 text-[13px] text-ink-faint">
          Windows 10 and 11. macOS and Linux are not supported yet.
        </p>
      </div>
    </Section>
  );
}
