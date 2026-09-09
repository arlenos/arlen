import { describe, expect, it } from "vitest";
import { trashBadge } from "./trashBadge";

describe("trashBadge", () => {
  it("says how many are in there", () => {
    expect(trashBadge(12)).toBe("12");
  });

  /// The wire clears a badge with the empty string. An empty trash is the
  /// absence of something waiting, and a row reading "Trash 0" is a worse
  /// sentence than a row reading "Trash".
  it("clears rather than showing a nought", () => {
    expect(trashBadge(0)).toBe("");
  });

  it("treats a count that cannot be one as nothing to say", () => {
    expect(trashBadge(-3)).toBe("");
    expect(trashBadge(Number.NaN)).toBe("");
  });
});
