/// A stopwatch must never read ahead of the time that has passed, and must
/// never go backwards.
///
/// It did both. `fmtStopwatch` composed a ROUNDED seconds field from
/// `fmtDuration` with its own FLOORED hundredths, so anything past the half
/// second showed a second that had not happened yet - and crossing a minute the
/// reading fell from `01:00.99` to `01:00.00`. Rounding is right for
/// `fmtDuration`'s other caller, a countdown, which is why the fix floors here
/// rather than there. These tests pin both behaviours so the next person
/// changing one does not quietly take the other with it.

import { describe, expect, it } from "vitest";
import { fmtDuration, fmtStopwatch, fmtDays } from "./format";

describe("fmtStopwatch", () => {
  it("never shows a second that has not elapsed", () => {
    // The regression, at the boundaries that used to be wrong.
    expect(fmtStopwatch(499)).toBe("00:00.49");
    expect(fmtStopwatch(500)).toBe("00:00.50");
    expect(fmtStopwatch(999)).toBe("00:00.99");
    expect(fmtStopwatch(1000)).toBe("00:01.00");
    expect(fmtStopwatch(1500)).toBe("00:01.50");
  });

  it("crosses a minute without going backwards", () => {
    // 59999 used to read 01:00.99 and 60000 read 01:00.00, so the display fell
    // by almost a second while the clock ran forward.
    expect(fmtStopwatch(59_999)).toBe("00:59.99");
    expect(fmtStopwatch(60_000)).toBe("01:00.00");
  });

  it("runs monotonically over a stretch that spans the old fault", () => {
    let previous = "";
    for (let ms = 0; ms <= 61_000; ms += 10) {
      const now = fmtStopwatch(ms);
      expect(now >= previous, `${ms}ms gave ${now} after ${previous}`).toBe(true);
      previous = now;
    }
  });

  it("prepends hours once they are reached", () => {
    expect(fmtStopwatch(3_600_000)).toBe("1:00:00.00");
  });

  it("clamps a negative reading rather than rendering a minus", () => {
    expect(fmtStopwatch(-1)).toBe("00:00.00");
  });
});

describe("fmtDuration", () => {
  it("still rounds, which is what a countdown wants", () => {
    // Not an accident of the stopwatch fix: with 2:59.7 left, a countdown
    // reading 3:00 is the honest one, and this is its only other caller.
    expect(fmtDuration(179_700)).toBe("03:00");
    expect(fmtDuration(59_500)).toBe("01:00");
  });

  it("switches to hours above an hour", () => {
    expect(fmtDuration(3_599_000)).toBe("59:59");
    expect(fmtDuration(3_600_000)).toBe("1:00:00");
  });

  it("clamps a negative duration to zero", () => {
    expect(fmtDuration(-5000)).toBe("00:00");
  });
});

describe("fmtDays", () => {
  it("names the two ends rather than listing seven days", () => {
    expect(fmtDays([0, 1, 2, 3, 4, 5, 6], "en", "Every day", "Once")).toBe("Every day");
    expect(fmtDays([], "en", "Every day", "Once")).toBe("Once");
  });
});
