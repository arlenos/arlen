import { describe, it, expect } from "vitest";
import { promptHistory, navigateHistory } from "./prompt-history";
import type { Message } from "$lib/stores/conversation";

function m(partial: Partial<Message> & Pick<Message, "id" | "role" | "text">): Message {
  return partial;
}

describe("promptHistory", () => {
  it("collects user prompts oldest-first, skipping replies and empty turns", () => {
    const h = promptHistory([
      m({ id: 1, role: "user", text: "first" }),
      m({ id: 2, role: "assistant", text: "ans" }),
      m({ id: 3, role: "user", text: "   " }),
      m({ id: 4, role: "user", text: "second" }),
    ]);
    expect(h).toEqual(["first", "second"]);
  });
});

describe("navigateHistory", () => {
  const h = ["one", "two", "three"];

  it("from the live composer, up recalls the most recent prompt", () => {
    expect(navigateHistory(h, null, "older")).toEqual({ index: 2, text: "three" });
  });

  it("up steps toward older prompts and stops at the oldest", () => {
    expect(navigateHistory(h, 2, "older")).toEqual({ index: 1, text: "two" });
    expect(navigateHistory(h, 1, "older")).toEqual({ index: 0, text: "one" });
    expect(navigateHistory(h, 0, "older")).toEqual({ index: 0, text: "one" });
  });

  it("down steps toward newer prompts", () => {
    expect(navigateHistory(h, 0, "newer")).toEqual({ index: 1, text: "two" });
  });

  it("down past the newest prompt returns to the live composer", () => {
    expect(navigateHistory(h, 2, "newer")).toEqual({ index: null, text: "" });
  });

  it("down at the live composer stays live", () => {
    expect(navigateHistory(h, null, "newer")).toEqual({ index: null, text: "" });
  });

  it("an empty history is always the live composer", () => {
    expect(navigateHistory([], null, "older")).toEqual({ index: null, text: "" });
    expect(navigateHistory([], 0, "newer")).toEqual({ index: null, text: "" });
  });
});
