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

  it("offers only Go while the mailbox cannot keep a write", () => {
    const groups = appMenuGroups(get(t), false);
    expect(groups.map((g) => g.label)).toEqual(["Go"]);
    expect(groups[0].items.map((i) => i.action)).toEqual([
      "go.inbox",
      "go.sent",
      "go.drafts",
      "go.archive",
      "go.trash",
    ]);
  });

  it("keeps the writes but drops New message on a mailbox with nowhere to send", () => {
    // The live shape since 5 September: a maildir keeps a draft, an archive and a
    // delete, so the Message group is there; sending needs an account and there
    // is none, so the one entry that starts a message from nothing is not.
    const groups = appMenuGroups(get(t), true, false);
    const actions = groups[0].items.map((i) => i.action);
    expect(actions).not.toContain("message.new");
    expect(actions).toContain("message.reply");
    expect(actions).toContain("message.archive");
    expect(actions).toContain("message.delete");
    // And no separator is left leading the group where the entry used to be.
    expect(groups[0].items[0].type).toBe("item");
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
