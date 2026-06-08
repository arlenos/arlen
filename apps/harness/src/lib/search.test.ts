import { describe, it, expect } from "vitest";
import { sessionMatches } from "./search";
import type { Session } from "$lib/stores/conversation";

function session(title: string, texts: string[]): Session {
  return {
    id: "s",
    title,
    createdAt: 0,
    messages: texts.map((text, i) => ({ id: i, role: "user", text })),
  };
}

describe("sessionMatches", () => {
  it("matches everything for an empty query", () => {
    expect(sessionMatches(session("Anything", []), "")).toBe(true);
    expect(sessionMatches(session("Anything", []), "   ")).toBe(true);
  });

  it("matches on the title, case-insensitively", () => {
    expect(sessionMatches(session("My Project Notes", []), "project")).toBe(true);
  });

  it("matches on message content the title does not mention", () => {
    const s = session("New conversation", ["What is in my downloads folder?"]);
    expect(sessionMatches(s, "downloads")).toBe(true);
  });

  it("does not match when neither title nor any message contains the query", () => {
    const s = session("Trip planning", ["Book a hotel in Vienna"]);
    expect(sessionMatches(s, "kubernetes")).toBe(false);
  });

  it("does not throw on a corrupt persisted record", () => {
    // Old/partial records from the schema-agnostic sessions file: a message
    // with a non-string text, a non-array messages, a non-string title.
    const corrupt = {
      id: "x",
      title: undefined,
      createdAt: 0,
      messages: [{ id: 1, role: "user", text: undefined }],
    } as unknown as Session;
    expect(() => sessionMatches(corrupt, "hi")).not.toThrow();
    expect(sessionMatches(corrupt, "hi")).toBe(false);

    const noArray = { id: "y", title: "ok", createdAt: 0, messages: null } as unknown as Session;
    expect(() => sessionMatches(noArray, "ok")).not.toThrow();
    expect(sessionMatches(noArray, "ok")).toBe(true); // title still matches
  });
});
