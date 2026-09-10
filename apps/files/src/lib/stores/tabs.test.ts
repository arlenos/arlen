// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Tab bookkeeping, whose two rules are both about what must NOT happen: the last
// tab never closes (a file manager with no location is nothing), and closing the
// active one hands over to a neighbour rather than leaving nothing selected. A
// screenshot of a tab strip cannot tell you either.
import { describe, it, expect, beforeEach, vi } from "vitest";
import { get } from "svelte/store";

// A tab IS a browser controller, and a real one starts listing the moment it is
// made - through Tauri, which is not here. Mocked so this file is about tab
// bookkeeping and not about a listing: without it the controllers fire IPC into
// nothing and leave a pile of unhandled rejections that can fail an unrelated
// test later in the run.
vi.mock("$lib/adapter", () => ({
  fmAdapter: {
    list: async () => ({ entries: [], path: "/", virtual: false }),
    stat: async () => null,
    thumbnails: async () => ({}),
  },
}));

import { tabs, activeTabId, activeController, newTab, closeTab, selectTab } from "./tabs.js";

describe("tabs", () => {
  beforeEach(() => {
    tabs.set([]);
    activeTabId.set(null);
  });

  it("opens a tab and selects it", () => {
    newTab("/home/u");
    const list = get(tabs);
    expect(list).toHaveLength(1);
    expect(get(activeTabId)).toBe(list[0].id);
    expect(get(activeController)).toBe(list[0].controller);
  });

  it("gives every tab its own id and its own controller", () => {
    newTab("/a");
    newTab("/b");
    const [one, two] = get(tabs);
    expect(one.id).not.toBe(two.id);
    expect(one.controller).not.toBe(two.controller);
  });

  it("refuses to close the last tab", () => {
    newTab("/a");
    const only = get(tabs)[0];
    closeTab(only.id);
    expect(get(tabs)).toHaveLength(1);
    expect(get(activeTabId)).toBe(only.id);
  });

  it("hands the selection to the neighbour when the active tab closes", () => {
    newTab("/a");
    newTab("/b");
    newTab("/c");
    const [a, b, c] = get(tabs);
    selectTab(b.id);
    closeTab(b.id);
    // The tab that took its place, not the first one and not none.
    expect(get(activeTabId)).toBe(c.id);
    expect(get(tabs).map((t) => t.id)).toEqual([a.id, c.id]);
  });

  it("keeps the selection when some other tab closes", () => {
    newTab("/a");
    newTab("/b");
    const [a, b] = get(tabs);
    selectTab(b.id);
    closeTab(a.id);
    expect(get(activeTabId)).toBe(b.id);
  });

  it("falls back to the last tab when the rightmost one closes", () => {
    newTab("/a");
    newTab("/b");
    const [a, b] = get(tabs);
    selectTab(b.id);
    closeTab(b.id);
    expect(get(activeTabId)).toBe(a.id);
  });

  it("has no controller before the first tab", () => {
    expect(get(activeController)).toBe(null);
  });
});
