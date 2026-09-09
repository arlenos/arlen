import { describe, it, expect } from "vitest";
import { browsingPresence, opRecord } from "./graphInput";

describe("browsingPresence", () => {
  it("says where the person is", () => {
    expect(browsingPresence("/home/tim/Documents")).toEqual({
      activity: "browsing",
      subject: "/home/tim/Documents",
      auto_clear: "on-blur",
    });
  });

  it("claims nothing with no folder", () => {
    expect(browsingPresence(null)).toBeNull();
  });
});

describe("opRecord", () => {
  it("names where a copy ended up, not where it came from", () => {
    expect(opRecord("copy", ["/a/one.txt"], "/b")).toEqual({
      label: "copy",
      subject: "/b",
      type: "copy",
      metadata: { count: "1" },
    });
  });

  it("names what was trashed, because there is no destination to look in", () => {
    expect(opRecord("trash", ["/a/one.txt", "/a/two.txt"])).toEqual({
      label: "trash",
      subject: "/a/one.txt",
      type: "trash",
      metadata: { count: "2" },
    });
  });

  it("records one thing somebody did rather than one row per file", () => {
    expect(opRecord("delete", ["/a/1", "/a/2", "/a/3"])?.metadata?.count).toBe("3");
  });

  it("falls back to the source when the operation has no destination", () => {
    expect(opRecord("copy", ["/a/one.txt"], undefined)?.subject).toBe("/a/one.txt");
  });

  it("names the folder a new-folder made, not the place it was made in", () => {
    expect(opRecord("new_folder", ["/a"], "/a/notes")?.subject).toBe("/a/notes");
  });

  it("records nothing it cannot name a subject for", () => {
    expect(opRecord("trash", [])).toBeNull();
  });
});
