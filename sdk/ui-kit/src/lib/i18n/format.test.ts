import { describe, it, expect } from "vitest";
import { formatDecimal, relativeTime } from "./index";

describe("formatDecimal", () => {
  it("uses the reader's decimal mark", () => {
    // The failure it exists for: `toFixed` writes a point in every language.
    expect(formatDecimal(2.4, 1, "en")).toBe("2.4");
    expect(formatDecimal(2.4, 1, "de")).toBe("2,4");
  });

  it("groups large numbers the reader's way", () => {
    expect(formatDecimal(1234567, 0, "en")).toBe("1,234,567");
    expect(formatDecimal(1234567, 0, "de")).toBe("1.234.567");
  });

  it("keeps the digits asked for", () => {
    expect(formatDecimal(3, 0, "en")).toBe("3");
    expect(formatDecimal(3, 1, "en")).toBe("3.0");
  });
});

describe("relativeTime", () => {
  const now = Date.UTC(2026, 7, 7, 12, 0, 0);
  const ago = (sec: number) => now - sec * 1000;

  it("says how long ago in the reader's language", () => {
    expect(relativeTime(ago(8 * 60), "en", now)).toBe("8 minutes ago");
    expect(relativeTime(ago(8 * 60), "de", now)).toBe("vor 8 Minuten");
  });

  it("uses the word where a language has one", () => {
    // What `numeric: "auto"` buys, and what six hand-written strings could not:
    // no catalog entry says "yesterday", the locale data does.
    expect(relativeTime(ago(26 * 3600), "en", now)).toBe("yesterday");
    expect(relativeTime(ago(26 * 3600), "de", now)).toBe("gestern");
    expect(relativeTime(ago(5), "en", now)).toBe("now");
    expect(relativeTime(ago(5), "de", now)).toBe("jetzt");
  });

  it("steps up through the units", () => {
    expect(relativeTime(ago(90 * 60), "en", now)).toBe("2 hours ago");
    expect(relativeTime(ago(3 * 86_400), "en", now)).toBe("3 days ago");
  });

  it("hands over to a date past a week", () => {
    // "37 days ago" is a worse way of saying a date than the date is.
    const out = relativeTime(ago(40 * 86_400), "en", now);
    expect(out).not.toMatch(/ago/);
    expect(out).toBe(new Date(ago(40 * 86_400)).toLocaleDateString("en"));
  });

  it("does not count into the future", () => {
    // A clock that jumped, or a timestamp from a machine slightly ahead: saying
    // "in 3 minutes" about something that already happened reads as a bug.
    expect(relativeTime(now + 180_000, "en", now)).toBe("now");
  });
});
