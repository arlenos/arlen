import { describe, it, expect } from "vitest";
import { togglePinned, pinnedMessages } from "./bookmark";
import type { Message } from "$lib/stores/conversation";

function m(partial: Partial<Message> & Pick<Message, "id" | "role" | "text">): Message {
  return partial;
}

describe("togglePinned", () => {
  it("pins an unmarked message", () => {
    const out = togglePinned([m({ id: 1, role: "assistant", text: "a" })], 1);
    expect(out[0].pinned).toBe(true);
  });

  it("unpins a bookmarked message, removing the key entirely", () => {
    const out = togglePinned([m({ id: 1, role: "assistant", text: "a", pinned: true })], 1);
    expect(out[0].pinned).toBeUndefined();
    expect("pinned" in out[0]).toBe(false);
  });

  it("leaves an unknown id unchanged", () => {
    const input = [m({ id: 1, role: "user", text: "q" })];
    expect(togglePinned(input, 99)).toEqual(input);
  });

  it("does not bookmark an in-flight placeholder", () => {
    const out = togglePinned([m({ id: 1, role: "assistant", text: "", pending: true })], 1);
    expect(out[0].pinned).toBeUndefined();
  });

  it("does not mutate the original messages", () => {
    const input = [m({ id: 1, role: "assistant", text: "a" })];
    togglePinned(input, 1);
    expect(input[0].pinned).toBeUndefined();
  });
});

describe("pinnedMessages", () => {
  it("returns the bookmarked messages in order", () => {
    const pinned = pinnedMessages([
      m({ id: 1, role: "user", text: "q1", pinned: true }),
      m({ id: 2, role: "assistant", text: "a1" }),
      m({ id: 3, role: "assistant", text: "a2", pinned: true }),
    ]);
    expect(pinned.map((x) => x.id)).toEqual([1, 3]);
  });

  it("is empty when nothing is bookmarked", () => {
    expect(pinnedMessages([m({ id: 1, role: "user", text: "q" })])).toEqual([]);
  });
});
