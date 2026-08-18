import { describe, expect, it } from "vitest";
import { DEFAULT_RATE, RATES, parseRate, rateLabel } from "./refresh";

describe("parseRate", () => {
  it("takes a rate it offers", () => {
    expect(parseRate("500")).toBe(500);
    expect(parseRate("10000")).toBe(10000);
  });

  it("falls back when nothing was ever stored", () => {
    expect(parseRate(null)).toBe(DEFAULT_RATE);
    expect(parseRate(undefined)).toBe(DEFAULT_RATE);
  });

  it("refuses a zero, because that is a busy loop in a process monitor", () => {
    // `setInterval(fn, 0)` inside the tool that walks /proc for every process
    // would make it the top row of its own list. A hand-edited localStorage
    // entry must not be able to ask for it.
    expect(parseRate("0")).toBe(DEFAULT_RATE);
    expect(parseRate("-1000")).toBe(DEFAULT_RATE);
  });

  it("refuses junk instead of turning it into NaN", () => {
    // `Number("banana")` is NaN, and `setInterval(fn, NaN)` behaves as 0.
    expect(parseRate("banana")).toBe(DEFAULT_RATE);
    expect(parseRate("")).toBe(DEFAULT_RATE);
  });

  it("drops a rate that is no longer offered rather than keeping it forever", () => {
    // An older build could have written 100. It should land on the default, not
    // survive as a rate this build never lists.
    expect(parseRate("100")).toBe(DEFAULT_RATE);
    expect(parseRate("3000")).toBe(DEFAULT_RATE);
  });

  it("offers only rates it would accept back", () => {
    // The list and the validator have to agree, or the control would set a value
    // the next window refuses.
    for (const r of RATES) expect(parseRate(String(r))).toBe(r);
    expect(RATES).toContain(DEFAULT_RATE);
  });
});

describe("rateLabel", () => {
  it("says seconds for a second and milliseconds below one", () => {
    expect(rateLabel(500)).toBe("500 ms");
    expect(rateLabel(1000)).toBe("1 s");
    expect(rateLabel(10000)).toBe("10 s");
  });
});
