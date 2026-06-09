import { describe, it, expect } from "vitest";
import { planDelete } from "./delete";
import type { Message } from "$lib/stores/conversation";

function m(partial: Partial<Message> & Pick<Message, "id" | "role" | "text">): Message {
  return partial;
}

describe("planDelete", () => {
  const convo = [
    m({ id: 1, role: "user", text: "first" }),
    m({ id: 2, role: "assistant", text: "ans1" }),
    m({ id: 3, role: "user", text: "second" }),
    m({ id: 4, role: "assistant", text: "ans2" }),
  ];

  it("deleting a user turn removes its answer too", () => {
    const left = planDelete(convo, 1);
    expect(left!.map((x) => x.id)).toEqual([3, 4]);
  });

  it("deleting a user turn in the middle keeps the surrounding turns", () => {
    const left = planDelete(convo, 3);
    expect(left!.map((x) => x.id)).toEqual([1, 2]);
  });

  it("deleting an assistant reply removes only it (the question stays)", () => {
    const left = planDelete(convo, 2);
    expect(left!.map((x) => x.id)).toEqual([1, 3, 4]);
  });

  it("deleting a user turn with no reply removes only it", () => {
    const pendingless = [
      m({ id: 1, role: "user", text: "q1" }),
      m({ id: 2, role: "assistant", text: "a1" }),
      m({ id: 3, role: "user", text: "q2, unanswered" }),
    ];
    expect(planDelete(pendingless, 3)!.map((x) => x.id)).toEqual([1, 2]);
  });

  it("deleting an error reply removes only it", () => {
    const withError = [
      m({ id: 1, role: "user", text: "q" }),
      m({ id: 2, role: "error", text: "daemon down" }),
    ];
    expect(planDelete(withError, 2)!.map((x) => x.id)).toEqual([1]);
  });

  it("deleting the only turn yields an empty conversation (not null)", () => {
    const single = [
      m({ id: 1, role: "user", text: "q" }),
      m({ id: 2, role: "assistant", text: "a" }),
    ];
    expect(planDelete(single, 1)).toEqual([]);
  });

  it("refuses while a turn is in flight", () => {
    const inflight = [
      m({ id: 1, role: "user", text: "q" }),
      m({ id: 2, role: "assistant", text: "", pending: true }),
    ];
    expect(planDelete(inflight, 1)).toBeNull();
  });

  it("refuses an id that is not present", () => {
    expect(planDelete(convo, 99)).toBeNull();
  });
});
