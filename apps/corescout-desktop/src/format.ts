/**
 * Turning numbers into words.
 *
 * Every function here has one job: never show a number that lies. A duration
 * of zero is "just now", not "0 hours". A reliability that has never been
 * measured is "never used", not "0%". A count of one is singular.
 */

/** A duration, said the way a person would say it. */
export function duration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "no time at all";
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 1) return "less than a minute";
  if (minutes === 1) return "a minute";
  if (minutes < 60) return `${minutes} minutes`;
  const hours = Math.floor(minutes / 60);
  if (hours === 1) return "an hour";
  if (hours < 24) return `${hours} hours`;
  const days = Math.floor(hours / 24);
  return days === 1 ? "a day" : `${days} days`;
}

/** How long ago something was. */
export function ago(atMs: number, now = Date.now()): string {
  if (!atMs) return "never";
  const elapsed = now - atMs;
  if (elapsed < 45_000) return "just now";
  return `${duration(elapsed)} ago`;
}

/** A clock time, in the viewer's own locale. */
export function clock(atMs: number): string {
  if (!atMs) return "";
  return new Date(atMs).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  });
}

/**
 * A share, as a percentage, or the honest absence of one.
 *
 * `undefined` means nobody has measured it. Rendering that as 0% is the single
 * most misleading thing this interface could do, so it is impossible here.
 */
export function share(value: number | undefined, absent = "not measured"): string {
  if (value === undefined || value === null || !Number.isFinite(value)) return absent;
  return `${Math.round(value * 100)}%`;
}

/** A count with its noun, pluralised. */
export function count(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** Bytes, at the scale a person reads them. */
export function bytes(value: number): string {
  if (!Number.isFinite(value) || value < 0) return "unknown";
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let scaled = value / 1024;
  let unit = 0;
  while (scaled >= 1024 && unit < units.length - 1) {
    scaled /= 1024;
    unit += 1;
  }
  return `${scaled.toFixed(scaled < 10 ? 1 : 0)} ${units[unit]}`;
}

/** A duration in nanoseconds, at the scale it actually occurs. */
export function nanos(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "not measured";
  if (value < 1_000) return `${Math.round(value)} ns`;
  if (value < 1_000_000) return `${(value / 1_000).toFixed(1)} µs`;
  if (value < 1_000_000_000) return `${(value / 1_000_000).toFixed(1)} ms`;
  return `${(value / 1_000_000_000).toFixed(2)} s`;
}

/**
 * How much of one core something uses.
 *
 * Below a hundredth of a per cent the exact figure is noise, and printing
 * "0.0003%" invites the reader to squint at it rather than to move on.
 */
export function cpuShare(fraction: number): string {
  if (!Number.isFinite(fraction) || fraction <= 0) return "not measured";
  const percent = fraction * 100;
  if (percent < 0.01) return "under 0.01% of one core";
  return `${percent.toFixed(2)}% of one core`;
}

/** The word shown on a card for how something is known. */
export function basisLabel(causal: boolean): string {
  return causal ? "Verified" : "Seen together";
}

/**
 * A sentence about confidence, worded to match how it was arrived at.
 *
 * An association and a randomised measurement are not on one scale, so they do
 * not get one vocabulary.
 */
export function confidenceWords(value: number, causal: boolean): string {
  if (!Number.isFinite(value)) return "unknown";
  if (causal) {
    if (value >= 0.85) return "strong evidence";
    if (value >= 0.5) return "good evidence";
    return "early evidence";
  }
  if (value >= 0.5) return "seen often";
  if (value >= 0.2) return "seen a few times";
  return "seen once or twice";
}
