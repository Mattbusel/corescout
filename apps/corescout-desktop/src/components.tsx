/**
 * The pieces every screen is built from.
 *
 * Nothing here declares a colour or a size directly; everything comes from the
 * tokens in theme.css. That is what makes the dark mode correct by
 * construction rather than by inspection.
 */

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { call, type Json } from "./api";
import { basisLabel, confidenceWords, share } from "./format";

/* ------------------------------------------------------------------ data */

interface Loaded<T> {
  data: T | null;
  error: string | null;
  loading: boolean;
  reload: () => void;
}

/**
 * Call a method, and call it again on an interval if asked.
 *
 * The interval is the only polling in this application. A socket would be
 * fewer round trips and considerably more machinery, and at one request every
 * couple of seconds to a process on the same machine the difference is not
 * measurable against the thing it would complicate.
 */
export function useCall<T>(
  method: string,
  params: Record<string, unknown> = {},
  everyMs = 0,
): Loaded<T> {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const key = JSON.stringify(params);
  const [nonce, setNonce] = useState(0);

  const reload = useCallback(() => setNonce((n) => n + 1), []);

  useEffect(() => {
    let alive = true;
    const run = async () => {
      try {
        const result = (await call(method, JSON.parse(key))) as T;
        if (!alive) return;
        setData(result);
        setError(null);
      } catch (problem) {
        if (!alive) return;
        setError(String(problem instanceof Error ? problem.message : problem));
      } finally {
        if (alive) setLoading(false);
      }
    };
    void run();
    if (everyMs <= 0) return () => { alive = false; };
    const timer = window.setInterval(run, everyMs);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [method, key, everyMs, nonce]);

  return { data, error, loading, reload };
}

/* --------------------------------------------------------------- pieces */

export function Badge({
  kind,
  children,
}: {
  kind: "verified" | "observed" | "quiet" | "warn" | "stop";
  children: ReactNode;
}) {
  return <span className={`badge ${kind}`}>{children}</span>;
}

/** The badge that says how something is known. Used everywhere, defined once. */
export function EvidenceBadge({ causal }: { causal: boolean }) {
  return <Badge kind={causal ? "verified" : "observed"}>{basisLabel(causal)}</Badge>;
}

export function Empty({ title, body }: { title: string; body: string }) {
  return (
    <div className="empty">
      <h3>{title}</h3>
      <p>{body}</p>
    </div>
  );
}

export function Stat({ value, label }: { value: ReactNode; label: string }) {
  return (
    <div className="stat">
      <div className="value">{value}</div>
      <div className="label">{label}</div>
    </div>
  );
}

export function Loading({ lines = 3 }: { lines?: number }) {
  return (
    <div style={{ display: "grid", gap: "var(--s3)" }}>
      {Array.from({ length: lines }, (_, index) => (
        <div
          key={index}
          className="skeleton"
          style={{ width: `${100 - index * 12}%` }}
        />
      ))}
    </div>
  );
}

export function Problem({ message, retry }: { message: string; retry?: () => void }) {
  return (
    <div className="banner warn">
      <span style={{ flex: 1 }}>{message}</span>
      {retry ? (
        <button className="action" onClick={retry}>
          Try again
        </button>
      ) : null}
    </div>
  );
}

/** Simple / Technical, the one control that governs progressive disclosure. */
export function Depth({
  technical,
  onChange,
}: {
  technical: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <div className="toolbar" role="group" aria-label="How much detail to show">
      <button aria-pressed={!technical} onClick={() => onChange(false)}>
        Simple
      </button>
      <button aria-pressed={technical} onClick={() => onChange(true)}>
        Technical
      </button>
    </div>
  );
}

/** A side panel. Escape closes it, and so does the backdrop. */
export function Sheet({ onClose, children }: { onClose: () => void; children: ReactNode }) {
  const panel = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    panel.current?.focus();
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div
      className="sheet"
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="sheet-panel" ref={panel} tabIndex={-1} role="dialog" aria-modal="true">
        {children}
      </div>
    </div>
  );
}

/**
 * The confidence line under a card.
 *
 * Deliberately words rather than a bar. A bar invites comparison between an
 * association and a measurement, which are not on the same scale, and a
 * reader comparing two bars will not notice they are not.
 */
export function Confidence({ value, causal }: { value: number; causal: boolean }) {
  return (
    <span title={`${share(value)} on CoreScout's own scale`}>
      {confidenceWords(value, causal)}
    </span>
  );
}

/* -------------------------------------------------------------- actions */

/**
 * A button that runs a method and reports what happened.
 *
 * Every mutating control in this application goes through this, so none of
 * them can silently swallow a refusal: the reason comes back from the
 * permission layer and is shown here verbatim.
 */
export function Do({
  method,
  params,
  label,
  primary,
  danger,
  confirm,
  onDone,
}: {
  method: string;
  params?: Record<string, unknown>;
  label: string;
  primary?: boolean;
  danger?: boolean;
  confirm?: string;
  onDone?: (result: Json) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [said, setSaid] = useState<string | null>(null);
  const [asking, setAsking] = useState(false);

  const go = async () => {
    if (confirm && !asking) {
      setAsking(true);
      return;
    }
    setAsking(false);
    setBusy(true);
    try {
      const result = await call(method, params ?? {});
      setSaid(null);
      onDone?.(result);
    } catch (problem) {
      setSaid(problem instanceof Error ? problem.message : String(problem));
    } finally {
      setBusy(false);
    }
  };

  return (
    <span style={{ display: "inline-flex", flexDirection: "column", gap: "var(--s1)" }}>
      <span className="row">
        <button
          className={`action${primary ? " primary" : ""}${danger ? " danger" : ""}`}
          onClick={go}
          disabled={busy}
        >
          {asking ? "Are you sure?" : busy ? "Working…" : label}
        </button>
        {asking ? (
          <button className="action" onClick={() => setAsking(false)}>
            Cancel
          </button>
        ) : null}
      </span>
      {asking && confirm ? <span className="faint">{confirm}</span> : null}
      {said ? <span style={{ color: "var(--stop)" }}>{said}</span> : null}
    </span>
  );
}
