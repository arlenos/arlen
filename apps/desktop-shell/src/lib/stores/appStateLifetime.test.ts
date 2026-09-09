import { describe, it, expect } from "vitest";
import { outlived } from "./appStateLifetime";

describe("outlived", () => {
  it("drops what has been seen with a window and now has none", () => {
    expect(outlived(["a", "b"], new Set(["a"]), new Set(["a", "b"]))).toEqual(["b"]);
  });

  /// THE CAUTIOUS HALF. An app publishes over the bus and maps its window
  /// through the compositor, and those are two races: a badge can arrive before
  /// the window does. State for an app this watcher has never seen with a window
  /// is not stale, it is early.
  it("leaves alone an app whose window has not appeared yet", () => {
    expect(outlived(["a"], new Set(), new Set())).toEqual([]);
  });

  it("leaves alone an app that still has a window", () => {
    expect(outlived(["a"], new Set(["a"]), new Set(["a"]))).toEqual([]);
  });

  it("says nothing about an app with no state stored", () => {
    expect(outlived([], new Set(), new Set(["a"]))).toEqual([]);
  });
});
