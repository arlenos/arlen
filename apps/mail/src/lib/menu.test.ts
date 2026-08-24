import { describe, it, expect } from "vitest";
import { get } from "svelte/store";
import { appMenuGroups } from "./menu";
import { t, locale } from "./i18n/messages";

describe("the global menu", () => {
  it("is written in the reader's language, not the source one", () => {
    locale.set("de");
    const groups = appMenuGroups(get(t));
    expect(groups.map((g) => g.label)).toEqual(["Nachricht", "Gehe zu"]);
    expect(groups[0].items[0].label).toBe("Neue Nachricht");
    expect(groups[1].items.map((i) => i.label)).toEqual([
      "Posteingang",
      "Gesendet",
      "Entwürfe",
      "Archiv",
      "Papierkorb",
    ]);
    locale.set("en");
  });

  it("names an action on every item that is not a separator", () => {
    for (const g of appMenuGroups(get(t))) {
      expect(g.label).not.toMatch(/^ml\./);
      for (const i of g.items) {
        if (i.type === "separator") expect(i.action).toBeUndefined();
        else expect(i.action).toMatch(/^[a-z]+\.[a-z_.]+$/);
        if (i.type !== "separator") expect(i.label).not.toMatch(/^ml\./);
      }
    }
  });
});
