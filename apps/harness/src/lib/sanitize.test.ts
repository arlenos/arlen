import { describe, it, expect } from "vitest";
import { sanitizeSession } from "$lib/stores/conversation";

describe("sanitizeSession", () => {
  it("keeps a well-formed session intact", () => {
    const s = sanitizeSession({
      id: "a",
      title: "Notes",
      createdAt: 5,
      messages: [{ id: 1, role: "user", text: "hi" }],
    });
    expect(s).not.toBeNull();
    expect(s!.id).toBe("a");
    expect(s!.title).toBe("Notes");
    expect(s!.messages).toHaveLength(1);
    expect(s!.messages[0].text).toBe("hi");
  });

  it("restores a true bookmark and ignores a non-true pinned flag", () => {
    const s = sanitizeSession({
      id: "a",
      title: "x",
      createdAt: 0,
      messages: [
        { id: 1, role: "assistant", text: "kept", pinned: true },
        { id: 2, role: "assistant", text: "noise", pinned: "yes" },
        { id: 3, role: "assistant", text: "plain" },
      ],
    });
    expect(s!.messages[0].pinned).toBe(true);
    expect(s!.messages[1].pinned).toBeUndefined();
    expect(s!.messages[2].pinned).toBeUndefined();
  });

  it("restores a true session pin and ignores a non-true flag", () => {
    expect(
      sanitizeSession({ id: "a", title: "x", createdAt: 0, messages: [], pinned: true })!.pinned,
    ).toBe(true);
    expect(
      sanitizeSession({ id: "a", title: "x", createdAt: 0, messages: [], pinned: "yes" })!.pinned,
    ).toBeUndefined();
    expect(sanitizeSession({ id: "a", title: "x", createdAt: 0, messages: [] })!.pinned).toBeUndefined();
  });

  it("coerces a non-array messages to empty rather than throwing", () => {
    const s = sanitizeSession({ id: "a", title: "x", createdAt: 0, messages: null });
    expect(s).not.toBeNull();
    expect(s!.messages).toEqual([]);
  });

  it("drops malformed messages (bad role, non-number id, non-string text)", () => {
    const s = sanitizeSession({
      id: "a",
      messages: [
        { id: 1, role: "user", text: "ok" },
        { id: "x", role: "user", text: "bad id" },
        { id: 2, role: "robot", text: "bad role" },
        { id: 3, role: "assistant", text: 42 },
      ],
    });
    expect(s!.messages).toHaveLength(2);
    expect(s!.messages[0].text).toBe("ok");
    // The non-string text is coerced to "", not dropped.
    expect(s!.messages[1]).toMatchObject({ id: 3, role: "assistant", text: "" });
  });

  it("drops a record with no usable id, and defaults a missing title", () => {
    expect(sanitizeSession({ messages: [] })).toBeNull();
    expect(sanitizeSession(null)).toBeNull();
    expect(sanitizeSession("nope")).toBeNull();
    const s = sanitizeSession({ id: "a", messages: [] });
    expect(s!.title).toBe("New conversation");
  });

  it("filters mention names to strings", () => {
    const s = sanitizeSession({
      id: "a",
      messages: [{ id: 1, role: "user", text: "q", mentions: ["a.md", 5, null, "b.md"] }],
    });
    expect(s!.messages[0].mentions).toEqual(["a.md", "b.md"]);
  });
});
