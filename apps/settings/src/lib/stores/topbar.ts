/// The topbar arrangement: the applets + tray items in the desktop-shell top
/// bar, in order, each shown-in-bar or in-overflow. The Settings panel reorders
/// + toggles them; the shell reads the saved arrangement to render the bar.
///
/// The inventory command (`topbar_items`) and the data-driven shell render are
/// coder seams; until they land the panel drives a mocked inventory. Order +
/// visibility persist to `topbar.toml` via the existing `config_set`.

import { get, writable, derived } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// One arrangeable topbar item.
export interface TopbarItem {
  /// Stable id (the applet's `appletId` or the tray item's SNI `id`).
  id: string;
  /// The display name.
  name: string;
  /// An icon key the panel maps to a glyph (applet id, or "tray" for SNI items).
  icon: string;
  /// First-party applet, or a third-party system-tray item.
  kind: "applet" | "tray";
  /// Shown in the bar, or tucked in the overflow dropdown.
  shown: boolean;
}

interface TopbarState {
  items: TopbarItem[];
  error: string | null;
}

export const topbar = writable<TopbarState>({ items: [], error: null });

/// The shown items, in order - what the live bar (and the preview) renders.
export const shownItems = derived(topbar, ($t) => $t.items.filter((i) => i.shown));

/// Load the arrangeable inventory (applets + tray) with the saved order +
/// visibility. A missing command leaves the panel empty rather than fake.
export async function load(): Promise<void> {
  try {
    const items = await invoke<TopbarItem[]>("topbar_items");
    topbar.set({ items, error: null });
  } catch (e) {
    topbar.update((s) => ({ ...s, error: String(e) }));
  }
}

/// Persist the order + visibility to `topbar.toml`. Best-effort: a write error
/// surfaces but does not roll back the local arrangement.
async function persist(items: TopbarItem[]): Promise<void> {
  try {
    await invoke("config_set", {
      file: "topbar",
      key: "order",
      value: items.map((i) => i.id),
    });
    await invoke("config_set", {
      file: "topbar",
      key: "visible",
      value: Object.fromEntries(items.map((i) => [i.id, i.shown])),
    });
  } catch (e) {
    topbar.update((s) => ({ ...s, error: String(e) }));
  }
}

/// Apply a new id order from a drag, then persist.
export function reorder(ids: string[]): void {
  const items = get(topbar).items;
  const next = ids
    .map((id) => items.find((i) => i.id === id))
    .filter((i): i is TopbarItem => i !== undefined);
  topbar.update((s) => ({ ...s, items: next }));
  void persist(next);
}

/// Toggle one item shown-in-bar vs overflow, then persist.
export function setShown(id: string, shown: boolean): void {
  const next = get(topbar).items.map((i) => (i.id === id ? { ...i, shown } : i));
  topbar.update((s) => ({ ...s, items: next }));
  void persist(next);
}
