import { describe, it, expect } from "vitest";
import { planEdit } from "./edit";
import type { Message } from "$lib/stores/conversation";

function m(partial: Partial<Message> & Pick<Message, "id" | "role" | "text">): Message {
  return partial;
}

describe("planEdit", () => {
  const convo = [
    m({ id: 1, role: "user", text: "first" }),
    m({ id: 2, role: "assistant", text: "ans1" }),
    m({ id: 3, role: "user", text: "second" }),
    m({ id: 4, role: "assistant", text: "ans2" }),
  ];

  it("keeps the transcript before the edited turn and resends the new text", () => {
    const plan = planEdit(convo, 3, "second, revised");
    expect(plan).not.toBeNull();
    expect(plan!.prompt).toBe("second, revised");
    // Everything from the edited turn onward is dropped (replaced by the resend).
    expect(plan!.keep.map((x) => x.id)).toEqual([1, 2]);
  });

  it("editing the first turn keeps nothing before it", () => {
    const plan = planEdit(convo, 1, "first, revised");
    expect(plan).not.toBeNull();
    expect(plan!.keep).toEqual([]);
    expect(plan!.prompt).toBe("first, revised");
  });

  it("trims the new text and refuses an empty or whitespace edit", () => {
    expect(planEdit(convo, 3, "   ")).toBeNull();
    expect(planEdit(convo, 3, "")).toBeNull();
    const plan = planEdit(convo, 3, "  spaced  ");
    expect(plan!.prompt).toBe("spaced");
  });

  it("refuses an id that is not in the conversation", () => {
    expect(planEdit(convo, 99, "x")).toBeNull();
  });

  it("refuses editing a non-user (assistant) message", () => {
    expect(planEdit(convo, 2, "x")).toBeNull();
  });

  it("refuses while a turn is in flight", () => {
    const inflight = [
      m({ id: 1, role: "user", text: "q" }),
      m({ id: 2, role: "assistant", text: "", pending: true }),
    ];
    expect(planEdit(inflight, 1, "edited")).toBeNull();
  });

  it("refuses an attachment-bearing question (its files are not persisted)", () => {
    const withFiles = [
      m({ id: 1, role: "user", text: "summarise", mentions: ["a.md"] }),
      m({ id: 2, role: "assistant", text: "ok" }),
    ];
    expect(planEdit(withFiles, 1, "summarise differently")).toBeNull();
  });

  it("is null for an empty conversation", () => {
    expect(planEdit([], 1, "x")).toBeNull();
  });
});
