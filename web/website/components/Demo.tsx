"use client";

/**
 * The loop, in about ten seconds.
 *
 * # Why this is a sequence and not a video
 *
 * A video is a file to load, a player to style and a thing that looks wrong in
 * dark mode. This is seven states of a small state machine, and it costs
 * nothing to ship. It also stays honest: every line in it is wording the
 * product actually uses, so it cannot drift into promising something the
 * application does not say.
 *
 * # What it is not
 *
 * Not live data, and it does not pretend to be. The numbers are from the
 * worked example described underneath it, and the caption says so.
 */

import { useEffect, useState } from "react";

interface Beat {
  actor: "ai" | "world" | "corescout";
  line: string;
  detail?: string;
  tone?: "bad" | "good" | "note";
  hold: number;
}

const BEATS: Beat[] = [
  { actor: "ai", line: "Claude runs the build.", hold: 1400 },
  { actor: "world", line: "The build fails.", tone: "bad", hold: 1400 },
  {
    actor: "corescout",
    line: "CoreScout has seen this before.",
    detail: "8 of the last 11 builds here failed the same way.",
    tone: "note",
    hold: 2000,
  },
  {
    actor: "corescout",
    line: "It notices what the successful ones had in common.",
    detail: "They regenerated the schema first. That is a correlation, not a cause.",
    tone: "note",
    hold: 2400,
  },
  {
    actor: "corescout",
    line: "So it tests it.",
    detail:
      "On some runs it suggests the procedure by coin flip rather than by belief. Only those trials count.",
    hold: 2600,
  },
  {
    actor: "corescout",
    line: "43% failures became 7%.",
    detail: "29 randomised trials. Now it is a capability, and you decide whether to approve it.",
    tone: "good",
    hold: 2600,
  },
  {
    actor: "ai",
    line: "The next build works.",
    detail: "Claude used the procedure. So will the next model you connect.",
    tone: "good",
    hold: 2600,
  },
];

const WHO: Record<Beat["actor"], string> = {
  ai: "Your AI",
  world: "Reality",
  corescout: "CoreScout",
};

export function Demo() {
  const [index, setIndex] = useState(0);
  const [paused, setPaused] = useState(false);

  useEffect(() => {
    if (paused) return;
    const beat = BEATS[index];
    if (!beat) return;
    const timer = window.setTimeout(
      () => setIndex((n) => (n + 1) % BEATS.length),
      beat.hold,
    );
    return () => window.clearTimeout(timer);
  }, [index, paused]);

  return (
    <div
      className="rounded-xl border border-line bg-raised p-6 sm:p-8"
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
    >
      <ol className="space-y-1">
        {BEATS.map((beat, at) => {
          const reached = at <= index;
          const current = at === index;
          return (
            <li
              key={beat.line}
              className="grid grid-cols-[88px_1fr] gap-4 py-2 transition-opacity duration-500"
              style={{ opacity: reached ? 1 : 0.22 }}
            >
              <span
                className="pt-0.5 text-[11px] font-semibold uppercase tracking-wider"
                style={{
                  color:
                    beat.actor === "corescout"
                      ? "var(--color-accent)"
                      : "var(--color-ink-faint)",
                }}
              >
                {WHO[beat.actor]}
              </span>
              <span>
                <span
                  className="block transition-colors duration-500"
                  style={{
                    color: current ? "var(--color-ink)" : "var(--color-ink-soft)",
                    fontWeight: current ? 500 : 400,
                  }}
                >
                  {beat.line}
                </span>
                {beat.detail && reached ? (
                  <span className="mt-0.5 block text-[13px] text-ink-muted">
                    {beat.detail}
                  </span>
                ) : null}
              </span>
            </li>
          );
        })}
      </ol>

      <div className="mt-6 flex items-center gap-3 border-t border-line pt-4">
        <div className="flex gap-1">
          {BEATS.map((beat, at) => (
            <button
              key={beat.line}
              aria-label={`Step ${at + 1}`}
              onClick={() => setIndex(at)}
              className="h-1 w-8 rounded-full transition-colors duration-300"
              style={{
                background:
                  at <= index ? "var(--color-accent)" : "var(--color-line-strong)",
              }}
            />
          ))}
        </div>
        <p className="ml-auto text-[12px] text-ink-faint">
          A worked example, not live data.
        </p>
      </div>
    </div>
  );
}
