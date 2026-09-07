import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Download CoreScout for Windows",
  description: "Free, open source, local-first. Windows 10 and 11.",
};

/**
 * The download page.
 *
 * It says what Windows will do about an unsigned installer before the reader
 * finds out from Windows. A download page that lets someone meet a scary blue
 * dialogue with no warning has spent trust it did not have to.
 */
export default function Page() {
  return (
    <main className="mx-auto max-w-3xl px-6 py-20">
      <h1 className="text-3xl font-semibold tracking-[-0.03em]">Download CoreScout</h1>
      <p className="mt-3 text-[15px] leading-relaxed text-ink-soft">
        Free, open source, and it keeps everything on your machine. Windows 10
        and 11, 64-bit.
      </p>

      <div className="mt-8 grid gap-3 sm:grid-cols-2">
        <a
          className="rounded-xl border border-line bg-raised p-5 transition-colors hover:border-line-strong"
          href="https://github.com/mattbusel/corescout/releases/latest/download/CoreScoutSetup.exe"
        >
          <div className="font-medium">CoreScoutSetup.exe</div>
          <p className="mt-1 text-[13px] text-ink-muted">
            The usual one. Installs for you alone, no administrator needed.
          </p>
        </a>
        <a
          className="rounded-xl border border-line bg-raised p-5 transition-colors hover:border-line-strong"
          href="https://github.com/mattbusel/corescout/releases/latest/download/CoreScout.msi"
        >
          <div className="font-medium">CoreScout.msi</div>
          <p className="mt-1 text-[13px] text-ink-muted">
            For deploying it across several machines.
          </p>
        </a>
      </div>

      <section className="mt-12">
        <h2 className="text-lg font-semibold tracking-[-0.01em]">
          Windows will warn you, and it is right to
        </h2>
        <p className="mt-2 text-[15px] leading-relaxed text-ink-soft">
          These builds are not signed with a code-signing certificate, so
          SmartScreen will show &ldquo;Windows protected your PC&rdquo;. Choose{" "}
          <span className="text-ink">More info</span>, then{" "}
          <span className="text-ink">Run anyway</span> &mdash; or check the
          SHA-256 published beside the file on the releases page first, and
          build it yourself from source if you would rather not take anyone else
          on trust.
        </p>
      </section>

      <section className="mt-10">
        <h2 className="text-lg font-semibold tracking-[-0.01em]">What it installs</h2>
        <ul className="mt-3 space-y-2 text-[14px] text-ink-soft">
          {[
            "CoreScout, the application and its tray icon.",
            "A small background service that does the watching.",
            "corescout-mcp, the bridge your AI connects to.",
            "corescout, a command line for the same thing.",
          ].map((line) => (
            <li key={line} className="flex gap-3">
              <span className="mt-2 h-1 w-1 flex-none rounded-full bg-ink-faint" />
              {line}
            </li>
          ))}
        </ul>
        <p className="mt-4 text-[14px] text-ink-muted">
          No Rust, Node, Python, WSL or Docker. Nothing starts at sign-in unless
          you turn that on.
        </p>
      </section>

      <section className="mt-10">
        <h2 className="text-lg font-semibold tracking-[-0.01em]">Removing it</h2>
        <p className="mt-2 text-[15px] leading-relaxed text-ink-soft">
          Uninstall from Settings, as usual. What CoreScout learned is left
          behind on purpose, so reinstalling does not start you from nothing.
          The uninstaller offers to delete it, and the Privacy page in the
          application names the exact folder if you would rather do it yourself.
        </p>
      </section>

      <section className="mt-10">
        <h2 className="text-lg font-semibold tracking-[-0.01em]">Building it yourself</h2>
        <pre className="mt-3 overflow-x-auto rounded-xl border border-line bg-sunken p-4 font-mono text-[12px] leading-relaxed text-ink-soft">
{`git clone https://github.com/mattbusel/corescout
cd corescout
cargo test --workspace
pwsh scripts/release.ps1`}
        </pre>
        <p className="mt-3 text-[13px] text-ink-muted">
          Needs Rust and Node. The installers land in <code>dist/</code>.
        </p>
      </section>
    </main>
  );
}
