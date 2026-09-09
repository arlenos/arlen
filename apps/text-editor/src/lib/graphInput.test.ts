import { describe, it, expect } from "vitest";
import { editingPresence, savedRecord } from "./graphInput";

describe("editingPresence", () => {
  it("says what is being done and to what, in which language", () => {
    expect(editingPresence("/home/tim/a.rs", "rust")).toEqual({
      activity: "editing",
      subject: "/home/tim/a.rs",
      metadata: { language: "rust" },
      auto_clear: "on-blur",
    });
  });

  it("claims nothing when there is no file", () => {
    expect(editingPresence(null, "rust")).toBeNull();
  });

  it("clears when the window is no longer the place it is happening", () => {
    expect(editingPresence("/x.md", "markdown")?.auto_clear).toBe("on-blur");
  });
});

describe("savedRecord", () => {
  it("records the moment the work landed, with its size, and words nothing", () => {
    expect(savedRecord("/home/tim/a.rs", 420, "rust")).toEqual({
      label: "saved",
      subject: "/home/tim/a.rs",
      type: "save",
      metadata: { language: "rust", chars: "420" },
    });
  });

  it("carries a whole, non-negative size whatever it is handed", () => {
    expect(savedRecord("/x", -1, "text").metadata?.chars).toBe("0");
    expect(savedRecord("/x", 12.9, "text").metadata?.chars).toBe("12");
  });
});
