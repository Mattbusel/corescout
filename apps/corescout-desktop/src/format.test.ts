import { describe, expect, it } from "vitest";
import {
  ago,
  basisLabel,
  bytes,
  confidenceWords,
  count,
  cpuShare,
  duration,
  nanos,
  share,
} from "./format";

describe("share", () => {
  it("never turns an unmeasured value into zero per cent", () => {
    // The single most misleading thing this interface could do. A fresh
    // capability that reads "0% reliable" is worse than one that reads
    // "never used", because the first is a claim.
    expect(share(undefined)).toBe("not measured");
    expect(share(undefined, "never used")).toBe("never used");
    expect(share(Number.NaN)).toBe("not measured");
  });

  it("renders a real share as a percentage", () => {
    expect(share(0)).toBe("0%");
    expect(share(0.436)).toBe("44%");
    expect(share(1)).toBe("100%");
  });
});

describe("duration", () => {
  it("says less than a minute rather than zero minutes", () => {
    expect(duration(0)).toBe("less than a minute");
    expect(duration(30_000)).toBe("less than a minute");
  });

  it("uses singulars where a person would", () => {
    expect(duration(60_000)).toBe("a minute");
    expect(duration(3_600_000)).toBe("an hour");
    expect(duration(26 * 3_600_000)).toBe("a day");
  });

  it("moves up a unit rather than counting to a thousand", () => {
    expect(duration(14 * 3_600_000)).toBe("14 hours");
    expect(duration(9 * 24 * 3_600_000)).toBe("9 days");
    expect(duration(2 * 24 * 3_600_000)).toBe("2 days");
  });

  it("does not render nonsense from a negative or absent value", () => {
    expect(duration(-5)).toBe("no time at all");
    expect(duration(Number.NaN)).toBe("no time at all");
  });
});

describe("ago", () => {
  it("says never rather than a date in 1970", () => {
    expect(ago(0)).toBe("never");
  });

  it("collapses the recent past into just now", () => {
    const now = 1_700_000_000_000;
    expect(ago(now - 4_000, now)).toBe("just now");
    expect(ago(now - 20 * 60_000, now)).toBe("20 minutes ago");
  });
});

describe("count", () => {
  it("pluralises", () => {
    expect(count(1, "capability", "capabilities")).toBe("1 capability");
    expect(count(3, "capability", "capabilities")).toBe("3 capabilities");
    expect(count(0, "state")).toBe("0 states");
  });
});

describe("bytes", () => {
  it("reads at the scale a person expects", () => {
    expect(bytes(512)).toBe("512 B");
    expect(bytes(33_554_496)).toBe("32 MB");
    expect(bytes(1024 * 1024 * 1024 * 3)).toBe("3.0 GB");
  });

  it("admits when it does not know", () => {
    expect(bytes(-1)).toBe("unknown");
  });
});

describe("nanos", () => {
  it("picks the unit the number actually lives in", () => {
    expect(nanos(400)).toBe("400 ns");
    expect(nanos(70_102)).toBe("70.1 µs");
    expect(nanos(4_500_000)).toBe("4.5 ms");
    expect(nanos(2_000_000_000)).toBe("2.00 s");
  });

  it("does not report a measurement that was never taken", () => {
    expect(nanos(0)).toBe("not measured");
  });
});

describe("cpuShare", () => {
  it("does not invite the reader to squint at four decimal places", () => {
    expect(cpuShare(0.000_03)).toBe("under 0.01% of one core");
    expect(cpuShare(0.0028)).toBe("0.28% of one core");
  });

  it("says nothing rather than zero when there is no measurement", () => {
    expect(cpuShare(0)).toBe("not measured");
  });
});

describe("evidence wording", () => {
  it("labels the two grades differently", () => {
    expect(basisLabel(true)).toBe("Verified");
    expect(basisLabel(false)).toBe("Seen together");
  });

  it("never describes an association in the vocabulary of measurement", () => {
    // An association and a randomised measurement are not on one scale, so
    // they do not get one vocabulary. "Strong evidence" is reserved.
    const associations = [0, 0.2, 0.5, 0.75].map((value) => confidenceWords(value, false));
    for (const words of associations) {
      expect(words).not.toContain("evidence");
      expect(words).toContain("seen");
    }
  });

  it("uses the language of evidence only where there is some", () => {
    expect(confidenceWords(0.9, true)).toBe("strong evidence");
    expect(confidenceWords(0.6, true)).toBe("good evidence");
    expect(confidenceWords(0.2, true)).toBe("early evidence");
  });
});
