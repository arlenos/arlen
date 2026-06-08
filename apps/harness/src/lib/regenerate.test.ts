import { describe, it, expect } from "vitest";
import { planRegenerate } from "./regenerate";
import type { Message } from "$lib/stores/conversation";

function m(partial: Partial<Message> & Pick<Message, "id" | "role" | "text">): Message {
  return partial;
}

describe("planRegenerate", () => {
  it("keeps up to the last user turn and re-sends its text", () => {
    const plan = planRegenerate([
      m({ id: 1, role: "user", text: "first" }),
      m({ id: 2, role: "assistant", text: "ans1" }),
      m({ id: 3, role: "user", text: "second" }),
      m({ id: 4, role: "assistant", text: "ans2" }),
    ]);
    expect(plan).not.toBeNull();
    expect(plan!.prompt).toBe("second");
    expect(plan!.keep.map((x) => x.id)).toEqual([1, 2, 3]);
  });

  it("regenerates after an error turn too", () => {
    const plan = planRegenerate([
      m({ id: 1, role: "user", text: "q" }),
      m({ id: 2, role: "error", text: "daemon down" }),
    ]);
    expect(plan).not.toBeNull();
    expect(plan!.prompt).toBe("q");
    expect(plan!.keep.map((x) => x.id)).toEqual([1]);
  });

  it("refuses while a turn is in flight", () => {
    expect(
      planRegenerate([
        m({ id: 1, role: "user", text: "q" }),
        m({ id: 2, role: "assistant", text: "", pending: true }),
      ]),
    ).toBeNull();
  });

  it("refuses when the last turn is the user's (no response yet)", () => {
    expect(planRegenerate([m({ id: 1, role: "user", text: "q" })])).toBeNull();
  });

  it("refuses an attachment-bearing question (its files are not persisted)", () => {
    expect(
      planRegenerate([
        m({ id: 1, role: "user", text: "summarise", mentions: ["a.md"] }),
        m({ id: 2, role: "assistant", text: "ok" }),
      ]),
    ).toBeNull();
  });

  it("is null for an empty conversation", () => {
    expect(planRegenerate([])).toBeNull();
  });
});
