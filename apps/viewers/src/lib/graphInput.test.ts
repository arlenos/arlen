import { describe, it, expect } from "vitest";
import { activityFor, viewingPresence, printedRecord } from "./graphInput";

describe("activityFor", () => {
  it("separates listening from looking", () => {
    expect(activityFor("audio")).toBe("listening");
    expect(activityFor("image")).toBe("viewing");
  });

  it("does not invent a verb for a kind it does not know", () => {
    expect(activityFor("video")).toBe("viewing");
    expect(activityFor("")).toBe("viewing");
  });
});

describe("viewingPresence", () => {
  it("says what is being done, to what, and of what kind", () => {
    expect(viewingPresence("/home/tim/a.png", "image")).toEqual({
      activity: "viewing",
      subject: "/home/tim/a.png",
      metadata: { kind: "image" },
      auto_clear: "on-blur",
    });
  });

  it("claims nothing with no file open", () => {
    expect(viewingPresence(null, "image")).toBeNull();
  });

  it("omits the kind it does not know yet rather than saying it is empty", () => {
    // Presence goes out the moment the window has a path, which is before the
    // file has been decoded. A drive caught this publishing `kind: ""`.
    const p = viewingPresence("/home/tim/a.png", "");
    expect(p).toEqual({ activity: "viewing", subject: "/home/tim/a.png", auto_clear: "on-blur" });
    expect(p?.metadata).toBeUndefined();
  });
});

describe("printedRecord", () => {
  it("records the print as a moment and words nothing", () => {
    expect(printedRecord("/home/tim/a.png", "image")).toEqual({
      label: "printed",
      subject: "/home/tim/a.png",
      type: "print",
      metadata: { kind: "image" },
    });
  });
});
