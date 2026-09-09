import { describe, it, expect } from "vitest";
import { badgeFor } from "./badge";

describe("badgeFor", () => {
  it("counts what is waiting", () => {
    expect(badgeFor(1)).toEqual({ kind: "count", count: 1 });
    expect(badgeFor(42)).toEqual({ kind: "count", count: 42 });
  });

  it("clears at zero rather than showing a nought", () => {
    expect(badgeFor(0)).toBeNull();
  });

  it("clears rather than passing on a count that cannot be one", () => {
    expect(badgeFor(-3)).toBeNull();
    expect(badgeFor(Number.NaN)).toBeNull();
    expect(badgeFor(Number.POSITIVE_INFINITY)).toBeNull();
  });

  it("carries a whole number into the badge", () => {
    expect(badgeFor(2.7)).toEqual({ kind: "count", count: 2 });
  });
});
