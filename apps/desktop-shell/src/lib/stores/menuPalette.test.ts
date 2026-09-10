// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The app-menu palette flattens another app's dbusmenu tree into a searchable
// leaf list. Everything a person can reach through it comes out of that walk, so
// a submenu it fails to descend into is a set of commands that silently stop
// existing - and the palette looks perfectly healthy without them.
import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import { activeAppId, mockRegisterMenu, type MenuGroup } from "./menus.js";
import { paletteItems } from "./menuPalette.js";

const APP = "dev.arlen.demo";

function give(groups: MenuGroup[]): void {
  mockRegisterMenu(APP, groups);
  activeAppId.set(APP);
}

describe("paletteItems", () => {
  beforeEach(() => {
    activeAppId.set(null);
  });

  it("walks a submenu and carries the breadcrumb", () => {
    give([
      {
        label: "File",
        items: [
          { label: "New", action: "file.new", type: "item" },
          {
            // `action` is required even on a submenu - the dbusmenu shape gives
            // every node one and a heading's is empty.
            label: "Export",
            action: "",
            type: "submenu",
            children: [{ label: "As PDF", action: "file.export.pdf", type: "item" }],
          },
        ],
      },
    ]);
    const items = get(paletteItems);
    expect(items.map((i) => i.action)).toEqual(["file.new", "file.export.pdf"]);
    expect(items[0].path).toEqual(["File"]);
    expect(items[1].path).toEqual(["File", "Export"]);
  });

  it("drops separators", () => {
    give([
      {
        label: "Edit",
        items: [
          { label: "Cut", action: "edit.cut", type: "item" },
          { label: "", action: "", type: "separator" },
          { label: "Paste", action: "edit.paste", type: "item" },
        ],
      },
    ]);
    expect(get(paletteItems).map((i) => i.action)).toEqual(["edit.cut", "edit.paste"]);
  });

  it("drops a submenu with nothing in it rather than offering the heading", () => {
    // A heading is not a command: listing it gives a person a row that does
    // nothing when pressed.
    give([
      {
        label: "View",
        items: [
          { label: "Zoom", action: "", type: "submenu", children: [] },
          { label: "Reload", action: "view.reload", type: "item" },
        ],
      },
    ]);
    expect(get(paletteItems).map((i) => i.action)).toEqual(["view.reload"]);
  });

  it("carries the shortcut, the tick and the disabled flag", () => {
    give([
      {
        label: "View",
        items: [
          {
            label: "Sidebar",
            action: "view.sidebar",
            type: "item",
            shortcut: "Ctrl+B",
            checked: true,
          },
          { label: "Print", action: "view.print", type: "item", disabled: true },
        ],
      },
    ]);
    const [sidebar, print] = get(paletteItems);
    expect(sidebar).toMatchObject({ shortcut: "Ctrl+B", checked: true });
    expect(print.disabled).toBe(true);
  });

  it("keeps every group's own leaves apart", () => {
    give([
      { label: "File", items: [{ label: "New", action: "file.new", type: "item" }] },
      { label: "Help", items: [{ label: "About", action: "help.about", type: "item" }] },
    ]);
    const items = get(paletteItems);
    expect(items.map((i) => i.path[0])).toEqual(["File", "Help"]);
  });

  it("shows the fixture menu when no window has one, rather than an empty palette", () => {
    // Under plain vite there is no active window at all. An empty list would
    // read as "this app has no menu", which is a claim about the app.
    activeAppId.set(null);
    expect(get(paletteItems).length).toBeGreaterThan(0);
  });
});
