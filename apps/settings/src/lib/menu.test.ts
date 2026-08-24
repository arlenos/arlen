import { describe, it, expect } from "vitest";
import { get } from "svelte/store";
import { appMenuGroups } from "./menu";
import { t, locale } from "./i18n/messages";

describe("the global menu", () => {
  it("is written in the reader's language, not the source one", () => {
    locale.set("de");
    const groups = appMenuGroups(get(t));
    expect(groups.map((g) => g.label)).toEqual(["Ansicht", "Gehe zu"]);
    locale.set("en");
  });

  it("names an action on every item that is not a separator, from the catalogue", () => {
    for (const g of appMenuGroups(get(t))) {
      expect(g.label).not.toMatch(/^s\./);
      const walk = (items: typeof g.items) => {
        for (const i of items) {
          if (i.type === "separator") continue;
          if (i.type === "submenu") { walk((i as { children?: typeof g.items }).children ?? []); continue; }
          expect(i.action).toMatch(/^[a-z]+\.[a-z_.-]+$/);
          expect(i.label).not.toMatch(/^s\./);
        }
      };
      walk(g.items);
    }
  });
});
