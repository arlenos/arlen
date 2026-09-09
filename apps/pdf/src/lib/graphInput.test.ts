import { describe, it, expect } from "vitest";
import { readingPresence } from "./graphInput";

describe("readingPresence", () => {
  it("says the document is being read, and how long it is", () => {
    expect(readingPresence("/home/tim/report.pdf", 42)).toEqual({
      activity: "reading",
      subject: "/home/tim/report.pdf",
      metadata: { pages: "42" },
      auto_clear: "on-blur",
    });
  });

  it("claims nothing with no document open", () => {
    expect(readingPresence(null, 42)).toBeNull();
  });

  it("omits a length it does not have rather than saying it is none", () => {
    for (const n of [0, -1, Number.NaN]) {
      expect(readingPresence("/x.pdf", n)?.metadata).toBeUndefined();
    }
  });
});
