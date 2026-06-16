/// The controller's select-all command channel: `selectAll()` is a
/// monotonic signal the mounted view applies (selection is view state,
/// so the headless controller only signals). The view-side apply is a
/// FileBrowser concern; here we pin the signal contract a host's topbar
/// "Select all" relies on.
import { get } from "svelte/store";
import { describe, expect, it } from "vitest";
import { createBrowserState } from "./controller";
import type { BrowserAdapter, FileEntry } from "./types";

const entry = (name: string): FileEntry => ({
  name,
  kind: "file",
  size: 1,
  modified_unix: 100,
  is_hidden: false,
  readonly: false,
  symlink_target: null,
});

const adapter = (entries: FileEntry[]): BrowserAdapter => ({
  list: () => Promise.resolve(entries),
});

describe("controller select-all signal", () => {
  it("starts at zero and increments once per selectAll()", () => {
    const c = createBrowserState(adapter([entry("a"), entry("b")]), { initial: "/home" });
    expect(get(c.selectAllSignal)).toBe(0);
    c.selectAll();
    expect(get(c.selectAllSignal)).toBe(1);
    c.selectAll();
    expect(get(c.selectAllSignal)).toBe(2);
  });

  it("each controller has its own independent signal", () => {
    const a = createBrowserState(adapter([entry("a")]), { initial: "/home" });
    const b = createBrowserState(adapter([entry("b")]), { initial: "/home" });
    a.selectAll();
    a.selectAll();
    expect(get(a.selectAllSignal)).toBe(2);
    expect(get(b.selectAllSignal)).toBe(0);
  });
});
