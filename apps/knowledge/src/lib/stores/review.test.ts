// The two rules the review list applies before it draws anything: what it
// counts as open, and what it puts first.
import { describe, expect, it } from "vitest";
import { get } from "svelte/store";
import { byStrength, decide, decidedCount, loadReview, openCount, suggestions, type Suggestion } from "./review";

describe("the review list", () => {
  it("counts a candidate as open until it carries a decision", () => {
    loadReview("few");
    expect(get(openCount)).toBe(4);
    const first = get(suggestions)[0].candidates[0].id;
    decide(first, "different");
    expect(get(openCount)).toBe(3);
    expect(get(decidedCount)).toBe(1);
  });

  it("takes a decision back to open, losing nothing", () => {
    loadReview("few");
    const first = get(suggestions)[0].candidates[0].id;
    decide(first, "same");
    decide(first, null);
    expect(get(openCount)).toBe(4);
    expect(get(decidedCount)).toBe(0);
  });

  it("puts the strongest evidence first", () => {
    const g = (id: string, reason: "email" | "together"): Suggestion => ({
      entry: { id, name: id, kind: "person", detail: "", source: "" },
      candidates: [{ id: `${id}-c`, name: id, kind: "person", detail: "", source: "", reason, decision: null }],
    });
    expect(byStrength([g("weak", "together"), g("strong", "email")]).map((s) => s.entry.id)).toEqual(["strong", "weak"]);
  });
});
