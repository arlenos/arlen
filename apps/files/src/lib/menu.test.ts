import { describe, it, expect } from "vitest";
import { get } from "svelte/store";
import { appMenuGroups } from "./menu";
import { t, locale } from "./i18n/messages";

describe("the global menu", () => {
  it("is written in the reader's language, not the source one", () => {
    locale.set("de");
    const groups = appMenuGroups(get(t));
    expect(groups.map((g) => g.label)).toEqual([
      "Datei",
      "Bearbeiten",
      "Ansicht",
      "Gehe zu",
      "Hilfe",
    ]);
    const view = groups[2].items;
    expect(view[1].label).toBe("Versteckte Dateien anzeigen");
    expect(view[3].children?.map((c) => c.label)).toEqual([
      "Name",
      "Größe",
      "Typ",
      "Geändert",
    ]);
    locale.set("en");
  });

  it("names an action on every item that is not a separator", () => {
    const walk = (items: ReturnType<typeof appMenuGroups>[number]["items"]): void => {
      for (const i of items) {
        if (i.type === "separator") expect(i.action).toBeUndefined();
        else if (i.type === "submenu") walk(i.children ?? []);
        else expect(i.action).toMatch(/^[a-z]+\.[a-z_.]+$/);
        // A label that came back as its own message id means a missing entry.
        if (i.type !== "separator") expect(i.label).not.toMatch(/^f\./);
      }
    };
    for (const g of appMenuGroups(get(t))) {
      expect(g.label).not.toMatch(/^f\./);
      walk(g.items);
    }
  });
});
