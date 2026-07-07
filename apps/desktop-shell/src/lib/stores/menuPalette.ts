/// The app-menu context popup (tier-c-gaps-plan.md §2): a keyboard shortcut summons
/// the ACTIVE window's menu as a floating, searchable command-palette, so any menu
/// item is reachable by keyboard - the palette pattern applied to the app's own
/// menu. Reuses the already-built dbusmenu data (menus.ts / gtk_menu_bridge); this
/// only flattens it into a searchable leaf list.
///
/// Mock-vs-live: the menu DATA (`activeMenu`) + `dispatchMenuAction` are real. Under
/// vite there is no active window, so a fixture menu stands in. The shortcut -> open
/// wiring + the layer-shell overlay (the Waypointer pattern) is the coder seam.

import { writable, derived, get } from "svelte/store";
import {
  activeMenu,
  activeAppId,
  dispatchMenuAction,
  type MenuGroup,
  type MenuItem,
} from "$lib/stores/menus";

/// One actionable menu leaf, with its breadcrumb path for context.
export interface FlatMenuItem {
  action: string;
  label: string;
  path: string[];
  shortcut?: string;
  checked?: boolean;
  disabled?: boolean;
}

const MOCK_MENU: MenuGroup[] = [
  {
    label: "File",
    items: [
      { label: "New Window", action: "file.new", shortcut: "Ctrl+N", type: "item" },
      { label: "Open…", action: "file.open", shortcut: "Ctrl+O", type: "item" },
      {
        label: "Export",
        action: "",
        type: "submenu",
        children: [
          { label: "PDF…", action: "file.export.pdf", type: "item" },
          { label: "PNG…", action: "file.export.png", type: "item" },
        ],
      },
      { label: "", action: "", type: "separator" },
      { label: "Save", action: "file.save", shortcut: "Ctrl+S", type: "item" },
      { label: "Close Window", action: "file.close", shortcut: "Ctrl+W", type: "item" },
    ],
  },
  {
    label: "Edit",
    items: [
      { label: "Undo", action: "edit.undo", shortcut: "Ctrl+Z", type: "item" },
      { label: "Redo", action: "edit.redo", shortcut: "Ctrl+Shift+Z", type: "item", disabled: true },
      { label: "Find…", action: "edit.find", shortcut: "Ctrl+F", type: "item" },
    ],
  },
  {
    label: "View",
    items: [
      { label: "Show Sidebar", action: "view.sidebar", shortcut: "Ctrl+B", type: "item", checked: true },
      { label: "Enter Full Screen", action: "view.fullscreen", shortcut: "F11", type: "item" },
    ],
  },
];

/// The palette open state (live: set by the shortcut action).
export const menuPaletteOpen = writable(false);
export function openMenuPalette(): void {
  menuPaletteOpen.set(true);
}
export function closeMenuPalette(): void {
  menuPaletteOpen.set(false);
}

/// Flatten a menu tree to its actionable leaves, carrying the breadcrumb path.
function flatten(groups: MenuGroup[]): FlatMenuItem[] {
  const out: FlatMenuItem[] = [];
  const walk = (items: MenuItem[], path: string[]) => {
    for (const it of items) {
      if (it.type === "separator") continue;
      if (it.type === "submenu" && it.children?.length) {
        walk(it.children, [...path, it.label]);
      } else if (it.type === "item") {
        out.push({
          action: it.action,
          label: it.label,
          path,
          shortcut: it.shortcut,
          checked: it.checked,
          disabled: it.disabled,
        });
      }
    }
  };
  for (const g of groups) walk(g.items, [g.label]);
  return out;
}

/// The active window's menu, flattened + searchable; the fixture under vite.
export const paletteItems = derived(activeMenu, ($menu) =>
  flatten($menu && $menu.length ? $menu : MOCK_MENU),
);

/// Invoke a menu item on the active window, then close.
export function activate(item: FlatMenuItem): void {
  if (item.disabled) return;
  const appId = get(activeAppId);
  if (appId) dispatchMenuAction(appId, item.action);
  closeMenuPalette();
}
