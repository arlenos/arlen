import { describe, it, expect } from "vitest";
import { sortSessions } from "./pin-session";
import type { Session } from "$lib/stores/conversation";

function s(id: string, pinned?: boolean): Session {
  return { id, title: id, createdAt: 0, messages: [], ...(pinned ? { pinned } : {}) };
}

describe("sortSessions", () => {
  it("floats pinned conversations to the top, keeping each group's order", () => {
    const out = sortSessions([s("a"), s("b", true), s("c"), s("d", true)]);
    expect(out.map((x) => x.id)).toEqual(["b", "d", "a", "c"]);
  });

  it("leaves an all-unpinned list unchanged", () => {
    const out = sortSessions([s("a"), s("b"), s("c")]);
    expect(out.map((x) => x.id)).toEqual(["a", "b", "c"]);
  });

  it("leaves an all-pinned list unchanged", () => {
    const out = sortSessions([s("a", true), s("b", true)]);
    expect(out.map((x) => x.id)).toEqual(["a", "b"]);
  });

  it("does not mutate the input array", () => {
    const input = [s("a"), s("b", true)];
    sortSessions(input);
    expect(input.map((x) => x.id)).toEqual(["a", "b"]);
  });
});
