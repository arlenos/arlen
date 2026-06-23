import { describe, it, expect } from "vitest";
import { formatDuration } from "./duration";

describe("formatDuration", () => {
  it("shows sub-second durations in milliseconds", () => {
    expect(formatDuration(12)).toBe("12ms");
    expect(formatDuration(999)).toBe("999ms");
  });
  it("keeps one decimal under ten seconds", () => {
    expect(formatDuration(1200)).toBe("1.2s");
    expect(formatDuration(9900)).toBe("9.9s");
  });
  it("rounds to whole seconds from ten to under sixty", () => {
    expect(formatDuration(41300)).toBe("41s");
    expect(formatDuration(59400)).toBe("59s");
  });
  it("splits minutes and seconds with a zero-padded remainder", () => {
    expect(formatDuration(125000)).toBe("2m 05s");
    expect(formatDuration(605000)).toBe("10m 05s");
  });
  it("rolls a remainder that rounds up to sixty into the next minute", () => {
    // 119.6s would naively format as "1m 60s"; it must roll to "2m 00s".
    expect(formatDuration(119600)).toBe("2m 00s");
  });
  it("rolls a sub-minute value that rounds up to sixty into one minute", () => {
    // 59.6s would naively format as "60s"; it must become "1m 00s".
    expect(formatDuration(59600)).toBe("1m 00s");
  });
});
