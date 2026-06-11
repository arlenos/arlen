/// Tab state: a tab IS a browser controller (file-manager-ui-plan.md,
/// the stress-tested cut) — N controllers live here, one FileBrowser
/// mounts the active one, background tabs hold zero DOM. IPC results
/// stay inside each controller's stores.

import { derived, get, writable } from "svelte/store";
import {
  createBrowserState,
  type BrowserState,
} from "@arlen/ui-kit/components/browser";
import { fmAdapter } from "$lib/adapter";

export interface Tab {
  id: number;
  controller: BrowserState;
}

let nextId = 1;

export const tabs = writable<Tab[]>([]);
export const activeTabId = writable<number | null>(null);

/// The active tab's controller (null only before the first tab).
export const activeController = derived(
  [tabs, activeTabId],
  ([$tabs, $id]) => $tabs.find((t) => t.id === $id)?.controller ?? null,
);

/// Open a new tab at `path` (defaults to the active tab's location,
/// the desktop convention) and select it.
export function newTab(path?: string): void {
  const current = get(activeController);
  const initial = path ?? (current ? get(current.path) : "/home");
  const tab: Tab = {
    id: nextId++,
    controller: createBrowserState(fmAdapter, { initial }),
  };
  tabs.update((list) => [...list, tab]);
  activeTabId.set(tab.id);
}

/// Close a tab; the neighbor takes over. The last tab stays open —
/// a file manager without a location is nothing.
export function closeTab(id: number): void {
  const list = get(tabs);
  if (list.length <= 1) return;
  const index = list.findIndex((t) => t.id === id);
  const next = list.filter((t) => t.id !== id);
  tabs.set(next);
  if (get(activeTabId) === id) {
    const neighbor = next[Math.min(index, next.length - 1)];
    activeTabId.set(neighbor.id);
  }
}

export function selectTab(id: number): void {
  activeTabId.set(id);
}
