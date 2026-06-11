/// Context-menu and keyboard actions for the workspace overlay.
///
/// Thin wrappers around `invoke(...)` so menu items and key handlers
/// stay declarative. Each returns void — nothing in the menu path
/// reads a return value. Failures are logged but never surface to
/// the user: the menu is fire-and-forget, and the UI re-renders from
/// live Wayland state regardless.

import { invoke } from "@tauri-apps/api/core";
import { get } from "svelte/store";
import { windows } from "$lib/stores/windows.js";
import {
  minimizeWindow,
  restoreWindow,
} from "$lib/stores/minimizedWindows.js";
import {
  clearSelection,
  selectionSnapshot,
} from "$lib/stores/overlaySelection.js";

/// Closes one window.
export function closeWindowAction(windowId: string): void {
  invoke("close_window", { windowId }).catch((e) =>
    console.warn("close_window failed:", e),
  );
}

/// Toggles fullscreen for one window.
export function fullscreenWindowAction(
  windowId: string,
  currentlyFullscreen: boolean,
): void {
  invoke("fullscreen_window", {
    windowId,
    enabled: !currentlyFullscreen,
  }).catch((e) => console.warn("fullscreen_window failed:", e));
}

/// Tiles one window to the given half of its workspace.
export function tileWindowAction(
  windowId: string,
  direction: "left" | "right",
): void {
  invoke("tile_window", { windowId, direction }).catch((e) =>
    console.warn("tile_window failed:", e),
  );
}

/// Moves one window to another workspace.
export function moveWindowToWorkspaceAction(
  windowId: string,
  wsId: string,
): void {
  invoke("window_move_to_workspace", {
    windowId,
    targetWorkspaceId: wsId,
  }).catch((e) => console.warn("window_move_to_workspace failed:", e));
}

/// Multi-action helpers. Each snapshots the selection at invoke
/// time so subsequent re-renders (from the actions themselves
/// causing state transitions) don't cause iteration to drop
/// mid-loop.

/// Closes every selected window.
export function closeAllSelected(): void {
  for (const id of selectionSnapshot()) closeWindowAction(id);
  clearSelection();
}

/// Minimizes every selected window that isn't minimized already.
export function minimizeAllSelected(): void {
  const wins = get(windows);
  for (const id of selectionSnapshot()) {
    const w = wins.find((x) => x.id === id);
    if (w && !w.minimized) minimizeWindow(id);
  }
  clearSelection();
}

/// Restores every minimized window in the selection. The caller
/// closes the overlay afterwards — state-only, this path never
/// collapses the input region itself.
export function restoreAllSelected(): void {
  const wins = get(windows);
  for (const id of selectionSnapshot()) {
    const w = wins.find((x) => x.id === id);
    if (w && w.minimized) restoreWindow(id);
  }
  clearSelection();
}

/// Moves every selected window to the given workspace.
export function moveAllSelectedToWorkspace(wsId: string): void {
  const wins = get(windows);
  for (const id of selectionSnapshot()) {
    const w = wins.find((x) => x.id === id);
    if (!w) continue;
    if (w.minimized) {
      // Multi-move keeps minimize state — use plain move, NOT
      // restoreWindowToWorkspace (which un-minimizes on arrival).
      invoke("window_move_to_workspace", {
        windowId: id,
        targetWorkspaceId: wsId,
      }).catch(() => {});
    } else {
      moveWindowToWorkspaceAction(id, wsId);
    }
  }
  clearSelection();
}

/// Tiles a selected pair side by side. The caller closes the
/// overlay afterwards — state-only, like `restoreAllSelected`.
export function tileSideBySide(ids: [string, string]): void {
  tileWindowAction(ids[0], "left");
  tileWindowAction(ids[1], "right");
  clearSelection();
}
