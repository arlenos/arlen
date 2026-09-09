import { describe, it, expect } from "vitest";
import { appShortcuts } from "./shortcuts";
import { appMenuGroups } from "$lib/menu";

const t = ((k: string) => k) as never;

describe("appShortcuts", () => {
  it("is a quick list rather than the menu", () => {
    expect(appShortcuts(t).length).toBeLessThan(10);
  });

  it("goes through the catalogue, so the launcher gets the reader's language", () => {
    for (const s of appShortcuts(t)) expect(s.label).toMatch(/^f\./);
  });

  it("carries an icon and an action for every entry", () => {
    for (const s of appShortcuts(t)) {
      expect(s.icon).not.toBe("");
      expect(s.action).not.toBe("");
    }
  });

  /// THE POINT OF THE WHOLE FILE. `shortcuts-api.md`: "Same `action` string in
  /// both calls keeps the dispatch path unified." A shortcut naming an action
  /// the menu does not is one this app's single dispatcher would drop on the
  /// floor, silently, because that is what an unknown action does there.
  it("names only actions the menu already dispatches", () => {
    const known = new Set<string>();
    const walk = (items: { action?: string; items?: unknown[]; children?: unknown[] }[]) => {
      for (const i of items) {
        if (i.action) known.add(i.action);
        if (Array.isArray(i.children)) walk(i.children as never);
      }
    };
    for (const g of appMenuGroups(t, [])) walk(g.items as never);
    for (const s of appShortcuts(t)) expect(known).toContain(s.action);
  });
});
