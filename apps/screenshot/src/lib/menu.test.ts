/// The menu the app publishes, and the actions behind it.
///
/// `appMenuGroups` carries a comment saying it is pure "so a test can read the
/// labels without a running app". There was no test. This is it, and it checks
/// the half a catalogue gate cannot: that every action a menu item dispatches is
/// one the page actually handles. A menu entry naming an action nobody answers is
/// a control that does nothing, which looks exactly like a control that works.
import { describe, expect, it } from "vitest";
import { appMenuGroups } from "./menu";

/// What `+page.svelte` branches on when `menuAction` fires.
const HANDLED = ["edit.undo", "edit.redo", "edit.copy"];

describe("appMenuGroups", () => {
  const groups = appMenuGroups((id: string) => id);

  it("dispatches only actions the page answers", () => {
    const actions = groups
      .flatMap((g) => g.items)
      .filter((i) => i.type === "item")
      .map((i) => i.action);
    expect(actions.length).toBeGreaterThan(0);
    for (const a of actions) expect(HANDLED).toContain(a);
  });

  /// Every label goes through the translator it is handed - none is written
  /// here. The identity translator above makes that visible: a hardcoded label
  /// would come back as prose rather than as its key.
  it("takes every label from the catalogue", () => {
    for (const g of groups) {
      expect(g.label).toMatch(/^s\./);
      for (const i of g.items) {
        if (i.type === "separator") continue;
        expect(i.label).toMatch(/^s\./);
      }
    }
  });

  /// A separator carries no action, so a click on one cannot dispatch.
  it("gives a separator nothing to dispatch", () => {
    for (const i of groups.flatMap((g) => g.items)) {
      if (i.type === "separator") expect(i.action).toBeUndefined();
    }
  });
});
