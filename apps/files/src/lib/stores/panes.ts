/// Pane state, shared between the header (view controls) and the
/// page (the panes themselves): the dual-pane second controller and
/// which pane holds the focus. A tab switch always lands in pane A.

import { derived, get, writable } from "svelte/store";
import {
  createBrowserState,
  type BrowserState,
} from "@arlen/ui-kit/components/browser";
import { fmAdapter } from "$lib/adapter";
import { activeController } from "$lib/stores/tabs";

export const splitView = writable(false);
export const paneB = writable<BrowserState | null>(null);
export const focusedPane = writable<"a" | "b">("a");

/// The controller the toolbar, status line and operations follow.
export const focusedController = derived(
  [splitView, focusedPane, paneB, activeController],
  ([$split, $focused, $paneB, $active]) =>
    $split && $focused === "b" && $paneB ? $paneB : $active,
);

export function toggleSplit(): void {
  if (get(splitView)) {
    splitView.set(false);
    focusedPane.set("a");
    paneB.set(null);
  } else {
    const active = get(activeController);
    const initial = active ? get(active.path) : "/home";
    paneB.set(createBrowserState(fmAdapter, { initial }));
    splitView.set(true);
  }
}
