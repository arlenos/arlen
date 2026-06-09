import { describe, it, expect } from "vitest";
import { planFork } from "./fork";
import type { Message } from "$lib/stores/conversation";

function m(partial: Partial<Message> & Pick<Message, "id" | "role" | "text">): Message {
  return partial;
}

describe("planFork", () => {
  const convo = [
    m({ id: 1, role: "user", text: "first" }),
    m({ id: 2, role: "assistant", text: "ans1" }),
    m({ id: 3, role: "user", text: "second" }),
    m({ id: 4, role: "assistant", text: "ans2" }),
  ];

  it("copies the prefix up to and including the branch point", () => {
    const prefix = planFork(convo, 2);
    expect(prefix!.map((x) => x.id)).toEqual([1, 2]);
  });

  it("forking at the last message copies the whole conversation", () => {
    expect(planFork(convo, 4)!.map((x) => x.id)).toEqual([1, 2, 3, 4]);
  });

  it("forking at the first message copies just it", () => {
    expect(planFork(convo, 1)!.map((x) => x.id)).toEqual([1]);
  });

  it("refuses an id that is not present", () => {
    expect(planFork(convo, 99)).toBeNull();
  });

  it("refuses while a turn is in flight", () => {
    const inflight = [
      m({ id: 1, role: "user", text: "q" }),
      m({ id: 2, role: "assistant", text: "", pending: true }),
    ];
    expect(planFork(inflight, 1)).toBeNull();
  });

  it("does not alias the original array", () => {
    const prefix = planFork(convo, 2)!;
    prefix.push(m({ id: 9, role: "user", text: "added to the fork" }));
    expect(convo.map((x) => x.id)).toEqual([1, 2, 3, 4]);
  });
});
