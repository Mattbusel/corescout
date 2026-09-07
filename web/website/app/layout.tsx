import type { Metadata } from "next";
import Link from "next/link";
import "./globals.css";

export const metadata: Metadata = {
  title: "CoreScout — your AI gets smarter. Now its computer can too.",
  description:
    "CoreScout learns how your machine and your AI work together, remembers what reality teaches them, and turns useful discoveries into better ways of working. Free, open source, local-first.",
  metadataBase: new URL("https://corescout.dev"),
  openGraph: {
    title: "CoreScout",
    description:
      "The model doesn't learn from every task. Its computer does. Free, open source, local-first.",
    type: "website",
  },
  icons: { icon: "/mark.svg" },
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body className="font-sans antialiased">
        <Header />
        {children}
        <Footer />
      </body>
    </html>
  );
}

function Header() {
  return (
    <header className="sticky top-0 z-20 border-b border-line bg-paper/85 backdrop-blur-md">
      <div className="mx-auto flex max-w-5xl items-center gap-6 px-6 py-4">
        <Link href="/" className="flex items-center gap-2 font-semibold tracking-tight">
          <Mark />
          CoreScout
        </Link>
        <nav className="ml-auto flex items-center gap-5 text-[13px] text-ink-muted">
          <Link className="transition-colors hover:text-ink" href="/#how">
            How it works
          </Link>
          <Link className="transition-colors hover:text-ink" href="/#result">
            Results
          </Link>
          <Link className="transition-colors hover:text-ink" href="/privacy">
            Privacy
          </Link>
          <a
            className="transition-colors hover:text-ink"
            href="https://github.com/mattbusel/corescout"
          >
            GitHub
          </a>
          <Link
            className="rounded-md bg-accent px-3 py-1.5 font-medium text-paper transition-transform hover:-translate-y-px"
            href="/download"
          >
            Download
          </Link>
        </nav>
      </div>
    </header>
  );
}

function Footer() {
  return (
    <footer className="hairline mt-24">
      <div className="mx-auto flex max-w-5xl flex-col gap-4 px-6 py-10 text-[13px] text-ink-muted sm:flex-row sm:items-center">
        <div className="flex items-center gap-2">
          <Mark />
          <span>CoreScout</span>
        </div>
        <p className="sm:ml-6">Free. Open source. Local-first.</p>
        <div className="flex gap-5 sm:ml-auto">
          <Link className="transition-colors hover:text-ink" href="/privacy">
            Privacy
          </Link>
          <a
            className="transition-colors hover:text-ink"
            href="https://github.com/mattbusel/corescout"
          >
            Source
          </a>
          <a
            className="transition-colors hover:text-ink"
            href="https://github.com/mattbusel/corescout/blob/master/docs/RESEARCH.md"
          >
            Research
          </a>
        </div>
      </div>
    </footer>
  );
}

/** The mark: a dot and its reflection, fading. */
function Mark() {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" aria-hidden="true">
      <circle cx="9" cy="6" r="3.2" fill="var(--color-accent)" />
      <circle cx="9" cy="13" r="3.2" fill="var(--color-accent)" opacity="0.28" />
      <line
        x1="2"
        y1="9.5"
        x2="16"
        y2="9.5"
        stroke="var(--color-line-strong)"
        strokeWidth="1"
      />
    </svg>
  );
}
