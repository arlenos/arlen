import { describe, it, expect } from "vitest";
import { applyDraft } from "$lib/stores/drafts";

describe("applyDraft", () => {
  it("stores a draft for a session", () => {
    expect(applyDraft({}, "s1", "half a thought")).toEqual({ s1: "half a thought" });
  });

  it("replaces an existing draft", () => {
    expect(applyDraft({ s1: "old" }, "s1", "new")).toEqual({ s1: "new" });
  });

  it("keeps other sessions' drafts untouched", () => {
    expect(applyDraft({ s1: "a", s2: "b" }, "s1", "a2")).toEqual({ s1: "a2", s2: "b" });
  });

  it("preserves leading and trailing whitespace in a kept draft", () => {
    expect(applyDraft({}, "s1", "  spaced  ")).toEqual({ s1: "  spaced  " });
  });

  it("removes the entry when the draft becomes empty or whitespace", () => {
    expect(applyDraft({ s1: "x", s2: "y" }, "s1", "")).toEqual({ s2: "y" });
    expect(applyDraft({ s1: "x" }, "s1", "   ")).toEqual({});
  });

  it("clearing a session with no draft is a no-op (same reference)", () => {
    const d = { s2: "y" };
    expect(applyDraft(d, "s1", "")).toBe(d);
  });

  it("does not mutate the input map", () => {
    const d = { s1: "x" };
    applyDraft(d, "s1", "changed");
    expect(d).toEqual({ s1: "x" });
  });
});
