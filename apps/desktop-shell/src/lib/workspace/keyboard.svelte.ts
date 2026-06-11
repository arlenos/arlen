/// Keyboard navigation for the workspace overlay.
///
/// Activated by the compositor's `workspace_overlay_open` event
/// (Super+Tab by default; see `compositor/src/config/mod.rs`). When
/// active, a focus ring sits on `focusedWindowId` and arrow / Tab /
/// 1-9 keys move it. The hover open path leaves `focusedWindowId`
/// null and shows no ring — keyboard mode toggles on first nav key.
///
/// FOCUS GRAB CAVEAT: the topbar layer-shell surface only receives
/// DOM keydown events when GTK has routed keyboard focus to it.
/// After Super+Tab the compositor consumes the keystroke and emits
/// the open event but does not move keyboard focus to the shell, so
/// the host explicitly calls `.focus()` on the overlay element to
/// request it from WebKitGTK. Whether the compositor actually grants
/// it depends on the layer's `keyboard_interactivity` mode; for
/// V1 we rely on OnDemand + focus-call. If keys still don't fire
/// for the user, the next iteration moves the keyboard-grab into
/// the compositor side.

import { invoke } from "@tauri-apps/api/core";
import { get } from "svelte/store";
import { windows } from "$lib/stores/windows.js";
import type { WindowInfo } from "$lib/stores/windows.js";
import type { WorkspaceInfo } from "$lib/stores/workspaces.js";
import { activateWorkspace } from "$lib/stores/workspaces.js";
import {
  minimizeWindow,
  restoreWindow,
} from "$lib/stores/minimizedWindows.js";
import {
  toggleSelection,
  clearSelection,
  selectionSnapshot,
} from "$lib/stores/overlaySelection.js";
import {
  closeWindowAction,
  fullscreenWindowAction,
  closeAllSelected,
  minimizeAllSelected,
  restoreAllSelected,
} from "./actions.js";

/// What the host component provides: the live per-output workspace
/// view, the overlay visibility, and the two strictly distinct close
/// paths — `hideOverlay` flips state only, `closeOverlayAndCollapse`
/// additionally collapses the compositor input region.
export interface KeyboardNavDeps {
  getWorkspaces: () => WorkspaceInfo[];
  isOverlayVisible: () => boolean;
  hideOverlay: () => void;
  closeOverlayAndCollapse: () => void;
}

/// Two-key "go to" gesture: press `g`, then within `GOTO_TIMEOUT_MS`
/// press a digit 1-9 to jump to that workspace AND close the Map.
/// Any other key cancels the pending state. Matches vim `g` prefix
/// behaviour — familiar to keyboard-first users.
const GOTO_TIMEOUT_MS = 800;

/// The controller handle components receive — see `createKeyboardNav`.
export type KeyboardNav = ReturnType<typeof createKeyboardNav>;

/// Creates the per-instance keyboard controller. `focusedWindowId`
/// is exposed as a getter over `$state` so the card templates track
/// the focus ring reactively.
export function createKeyboardNav(deps: KeyboardNavDeps) {
  let focusedWindowId = $state<string | null>(null);

  let gotoPending = false;
  let gotoPendingTimer: ReturnType<typeof setTimeout> | null = null;

  /// Flat ordering of all visible windows: workspace by workspace,
  /// in the order their cards render. Used by Tab / Shift+Tab to
  /// cycle across workspace boundaries.
  function flatWindowOrder(): { winId: string; wsId: string }[] {
    const order: { winId: string; wsId: string }[] = [];
    const wins = get(windows);
    for (const ws of deps.getWorkspaces()) {
      for (const w of wins) {
        if (w.workspace_ids.includes(ws.id)) {
          order.push({ winId: w.id, wsId: ws.id });
        }
      }
    }
    return order;
  }

  function pickInitialFocus(): string | null {
    // Prefer the currently active window so the first Tab move is
    // semantically "show me the next thing after where I am".
    const wins = get(windows);
    const active = wins.find((w) => w.active);
    if (active) return active.id;
    const activeWs = deps.getWorkspaces().find((w) => w.active);
    if (activeWs) {
      const wsWins = wins.filter((w) =>
        w.workspace_ids.includes(activeWs.id),
      );
      if (wsWins.length > 0) return wsWins[0].id;
    }
    return flatWindowOrder()[0]?.winId ?? null;
  }

  /// Seeds the focus ring on overlay open (keyboard path only — the
  /// hover path leaves focus null).
  function seedInitialFocus(): void {
    focusedWindowId = pickInitialFocus();
  }

  function cycleWindow(direction: 1 | -1): void {
    const order = flatWindowOrder();
    if (order.length === 0) return;
    if (focusedWindowId === null) {
      focusedWindowId = pickInitialFocus();
      return;
    }
    const idx = order.findIndex((e) => e.winId === focusedWindowId);
    if (idx < 0) {
      focusedWindowId = order[0].winId;
      return;
    }
    const next = (idx + direction + order.length) % order.length;
    focusedWindowId = order[next].winId;
  }

  function navigateWorkspace(direction: 1 | -1): void {
    const workspaces = deps.getWorkspaces();
    if (workspaces.length === 0) return;
    const wins = get(windows);
    let currentWsIdx = -1;
    if (focusedWindowId) {
      const win = wins.find((w) => w.id === focusedWindowId);
      if (win) {
        currentWsIdx = workspaces.findIndex((ws) =>
          win.workspace_ids.includes(ws.id),
        );
      }
    }
    if (currentWsIdx < 0) {
      currentWsIdx = workspaces.findIndex((ws) => ws.active);
    }
    const wsIdx =
      (currentWsIdx + direction + workspaces.length) % workspaces.length;
    const wsId = workspaces[wsIdx].id;
    const wsWins = wins.filter((w) => w.workspace_ids.includes(wsId));
    focusedWindowId = wsWins[0]?.id ?? null;
  }

  function navigateColumn(direction: 1 | -1): void {
    if (!focusedWindowId) {
      focusedWindowId = pickInitialFocus();
      return;
    }
    const wins = get(windows);
    const win = wins.find((w) => w.id === focusedWindowId);
    if (!win) return;
    const wsId = win.workspace_ids[0];
    const wsWins = wins.filter((w) => w.workspace_ids.includes(wsId));
    const idx = wsWins.findIndex((w) => w.id === focusedWindowId);
    if (idx < 0 || wsWins.length === 0) return;
    const next = (idx + direction + wsWins.length) % wsWins.length;
    focusedWindowId = wsWins[next].id;
  }

  function jumpToWorkspaceN(n: number): void {
    const ws = deps.getWorkspaces()[n - 1];
    if (!ws) return;
    const wsWins = get(windows).filter((w) =>
      w.workspace_ids.includes(ws.id),
    );
    focusedWindowId = wsWins[0]?.id ?? null;
  }

  function activateFocused(): void {
    const id = focusedWindowId;
    if (!id) return;
    // Enter on a minimized card restores instead of activating —
    // activate alone wouldn't un-minimize on cosmic, it just toggles
    // focus. restoreWindow calls both unset_minimized and activate,
    // which is what the user expects.
    const win = get(windows).find((w) => w.id === id);
    if (win?.minimized) {
      restoreWindow(id);
    } else {
      invoke("activate_window", { id }).catch(() => {});
    }
    closeOverlayKeyboard();
  }

  function closeOverlayKeyboard(): void {
    focusedWindowId = null;
    deps.closeOverlayAndCollapse();
  }

  function startGotoPending(): void {
    gotoPending = true;
    if (gotoPendingTimer) clearTimeout(gotoPendingTimer);
    gotoPendingTimer = setTimeout(() => {
      gotoPending = false;
      gotoPendingTimer = null;
    }, GOTO_TIMEOUT_MS);
  }

  function cancelGotoPending(): void {
    gotoPending = false;
    if (gotoPendingTimer) {
      clearTimeout(gotoPendingTimer);
      gotoPendingTimer = null;
    }
  }

  /// Fire `d` / Delete / `m` / `f` / Space against the currently-
  /// focused window, branching on selection size. Centralised so the
  /// handler switch stays compact.

  function actionDelete(): void {
    const sel = selectionSnapshot();
    if (sel.length > 0) {
      closeAllSelected();
      return;
    }
    if (focusedWindowId) {
      closeWindowAction(focusedWindowId);
    }
  }

  function actionMinimizeToggle(): void {
    const sel = selectionSnapshot();
    const wins = get(windows);
    if (sel.length > 1) {
      // Multi: if any is active, minimize; else restore.
      const selWindows = sel
        .map((id) => wins.find((w) => w.id === id))
        .filter((w): w is WindowInfo => Boolean(w));
      const anyActive = selWindows.some((w) => !w.minimized);
      if (anyActive) {
        minimizeAllSelected();
      } else {
        // Restore closes the overlay state-only, exactly like the
        // menu's Restore All path.
        restoreAllSelected();
        deps.hideOverlay();
      }
      return;
    }
    const id = focusedWindowId;
    if (!id) return;
    const w = wins.find((x) => x.id === id);
    if (!w) return;
    if (w.minimized) {
      restoreWindow(id);
      closeOverlayKeyboard();
    } else {
      minimizeWindow(id);
    }
  }

  function actionFullscreen(): void {
    const id = focusedWindowId;
    if (!id) return;
    const w = get(windows).find((x) => x.id === id);
    if (!w) return;
    fullscreenWindowAction(id, w.fullscreen ?? false);
  }

  function actionToggleSelection(): void {
    if (focusedWindowId) {
      toggleSelection(focusedWindowId);
    }
  }

  function onKeydown(e: KeyboardEvent): void {
    if (!deps.isOverlayVisible()) return;

    // Vim-key alias resolution. `e.key` for letters respects Shift
    // and CapsLock, so `e.key` on `h` is always "h" (lowercase) when
    // CapsLock is off and "H" when on — we case-insensitise by
    // lowering. Shift+H -> "H" means `Shift+m` (Move dialog) still
    // works because Shift+M arrives as "M" and we inspect shiftKey
    // independently.
    const rawKey = e.key;
    const key = rawKey.length === 1 ? rawKey.toLowerCase() : rawKey;

    // `g` pending state: if the user pressed `g` within the last
    // `GOTO_TIMEOUT_MS`, a digit now means "go to workspace N and
    // close the Map", not just "focus workspace N". Any other key
    // cancels pending (including `g` pressed twice — harmless).
    if (gotoPending && key >= "1" && key <= "9") {
      cancelGotoPending();
      clearSelection();
      jumpToWorkspaceN(parseInt(key, 10));
      const ws = deps.getWorkspaces()[parseInt(key, 10) - 1];
      if (ws) activateWorkspace(ws.id);
      closeOverlayKeyboard();
      e.preventDefault();
      e.stopPropagation();
      return;
    }
    if (gotoPending && key !== "g") {
      // Any other key while pending cancels — the user changed mind.
      cancelGotoPending();
      // Fall through to normal handling of this key.
    }

    let handled = true;
    switch (key) {
      // Navigation: arrows + vim hjkl
      case "Tab":
        clearSelection();
        cycleWindow(e.shiftKey ? -1 : 1);
        break;
      case "ArrowLeft":
      case "h":
        clearSelection();
        navigateWorkspace(-1);
        break;
      case "ArrowRight":
      case "l":
        clearSelection();
        navigateWorkspace(1);
        break;
      case "ArrowUp":
      case "k":
        navigateColumn(-1);
        break;
      case "ArrowDown":
      case "j":
        navigateColumn(1);
        break;
      case "g":
        // Start pending-goto mode. Any digit within the timeout
        // jumps and closes; any other key cancels.
        startGotoPending();
        break;
      // Actions
      case "Enter":
        clearSelection();
        activateFocused();
        break;
      case "d":
      case "Delete":
        actionDelete();
        break;
      case "m":
        if (e.shiftKey) {
          // Shift+M: Move dialog — placeholder, spec calls this a
          // keyboard alternative to the context menu "Move to"
          // submenu. The existing context menu already covers this,
          // a dedicated dialog is future work.
          // TODO: render a workspace-picker overlay here.
          handled = false;
        } else {
          actionMinimizeToggle();
        }
        break;
      case "f":
        actionFullscreen();
        break;
      case " ":
      case "Space":
        actionToggleSelection();
        break;
      case "Escape":
        if (selectionSnapshot().length > 0) {
          clearSelection();
        } else {
          closeOverlayKeyboard();
        }
        break;
      default:
        if (key >= "1" && key <= "9") {
          clearSelection();
          jumpToWorkspaceN(parseInt(key, 10));
        } else {
          handled = false;
        }
    }
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
    }
  }

  return {
    get focusedWindowId() {
      return focusedWindowId;
    },
    onKeydown,
    cycleWindow,
    seedInitialFocus,
  };
}
