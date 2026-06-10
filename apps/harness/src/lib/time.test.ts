import { describe, expect, it } from "vitest";
import { relativeTime } from "./time";

const NOW = 1_750_000_000_000; // fixed wall clock (ms)
const us = (msAgo: number) => (NOW - msAgo) * 1000;

describe("relativeTime", () => {
  it("reads as 'just now' under 45 seconds", () => {
    expect(relativeTime(us(0), NOW)).toBe("just now");
    expect(relativeTime(us(44_000), NOW)).toBe("just now");
  });

  it("reads as 'a minute ago' between 45 and 90 seconds", () => {
    expect(relativeTime(us(60_000), NOW)).toBe("a minute ago");
  });

  it("rounds to minutes under an hour", () => {
    expect(relativeTime(us(8 * 60_000), NOW)).toBe("8 min ago");
    expect(relativeTime(us(59 * 60_000), NOW)).toBe("59 min ago");
  });

  it("rounds to hours under a day", () => {
    expect(relativeTime(us(3 * 3_600_000), NOW)).toBe("3 h ago");
  });

  it("rounds to days under a week", () => {
    expect(relativeTime(us(2 * 86_400_000), NOW)).toBe("2 d ago");
  });

  it("falls back to a date beyond a week", () => {
    const out = relativeTime(us(10 * 86_400_000), NOW);
    expect(out).not.toMatch(/ago|just now/);
  });

  it("clamps future timestamps to 'just now'", () => {
    expect(relativeTime(us(-30_000), NOW)).toBe("just now");
  });
});
