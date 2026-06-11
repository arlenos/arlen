/// Pointer-based drag & drop for the workspace overlay.
///
/// The HTML5 drag API (dragstart/dragover/dragend + setDragImage) kept
/// freezing WebKitGTK when combined with a custom ghost — see debug
/// sessions 2026-04-19. Pointer events give us full control without
/// the browser's drag abstraction interfering:
///   pointerdown  → capture pointer, stash start position
///   pointermove  → once moved past threshold, create ghost; then
///                  position ghost + update hover column every tick
///   pointerup    → if dragged: fire move_to_workspace + cleanup;
///                  if not dragged: treat as a click on the card
///   pointercancel → cleanup (browser abort)
///   watchdog     → 8s fallback cleanup
///
/// Column hit-testing uses `document.elementFromPoint` plus a
/// `data-ws-id` attribute on each column. The ghost is
/// `pointer-events: none` so it never shadows the real hit-test.

import { invoke } from "@tauri-apps/api/core";
import { get } from "svelte/store";
import { windows } from "$lib/stores/windows.js";
import type { WorkspaceInfo } from "$lib/stores/workspaces.js";
import {
  restoreWindow,
  restoreWindowToWorkspace,
  minimizeWindow,
} from "$lib/stores/minimizedWindows.js";
import {
  isSelected,
  selectionSnapshot,
  selectOnly,
  clearSelection,
  toggleSelection,
} from "$lib/stores/overlaySelection.js";
import { GhostController } from "./dragGhost.js";

/// Discriminates the source card subregion. The drop handler uses
/// (kind, target-section, same/different workspace) to decide the
/// action. All drags go through the overlay; the topbar pills are
/// click-only.
export type DragSourceKind = "active" | "minimized";

/// A drop-target location resolved from a cursor position. The
/// section tells the drop handler whether the user dropped on the
/// "active windows" area (top 75%) or the "minimized" area
/// (bottom 25%) of a workspace card — the action matrix branches
/// on this.
export type DropSection = "active" | "minimized";
export type DropTarget = { wsId: string; section: DropSection };

const DRAG_THRESHOLD_PX = 5;

/// What the host component provides: the workspace list the drop
/// resolution runs against (live, per output), and the overlay
/// close-and-collapse path a plain card click triggers. The latter
/// is the input-region-collapsing close — strictly distinct from
/// the state-only hide used elsewhere.
export interface DragEngineDeps {
  getWorkspaces: () => WorkspaceInfo[];
  closeOverlayAndCollapse: () => void;
}

/// Finds the drop-target column + section under (x, y) via
/// elementFromPoint. Walks the DOM for the closest `data-ws-id`
/// (gives the workspace) AND the closest `data-ws-section`
/// (gives active vs minimized). The data attributes are set on
/// the overlay's card subregions — pills in the topbar don't
/// carry them, so the topbar indicator never acts as a drop
/// zone.
function dropTargetAt(clientX: number, clientY: number): DropTarget | null {
  const el = document.elementFromPoint(
    clientX,
    clientY,
  ) as HTMLElement | null;
  if (!el) return null;
  const column = el.closest("[data-ws-id]") as HTMLElement | null;
  if (!column) return null;
  const sectionEl = el.closest("[data-ws-section]") as HTMLElement | null;
  // Missing section element = cursor is over the column header or
  // padding. We still return a target with a default section of
  // "active" so users who drop slightly off the cards still get a
  // reasonable action (move-to-workspace keeps the window open).
  const section =
    (sectionEl?.dataset.wsSection as DropSection | undefined) ?? "active";
  return { wsId: column.dataset.wsId!, section };
}

/// Applies one drop action based on the source card's kind +
/// workspace and the drop target. Extracted so multi-drag can loop
/// over targets without duplicating the branch logic.
function applyDropAction(
  windowId: string,
  sourceKind: DragSourceKind,
  sourceWs: string,
  drop: DropTarget,
): void {
  const sameWs = drop.wsId === sourceWs;
  const targetSection = drop.section;
  if (sourceKind === "active") {
    if (sameWs && targetSection === "minimized") {
      minimizeWindow(windowId);
    } else if (!sameWs) {
      invoke("window_move_to_workspace", {
        windowId,
        targetWorkspaceId: drop.wsId,
      }).catch((err) =>
        console.error("window_move_to_workspace failed", err),
      );
      // Drop target is the minimized section on a different
      // workspace → move + minimize (spec §Feature 4).
      if (targetSection === "minimized") {
        minimizeWindow(windowId);
      }
    }
  } else {
    if (sameWs && targetSection === "active") {
      restoreWindow(windowId);
    } else if (!sameWs) {
      if (targetSection === "active") {
        restoreWindowToWorkspace(windowId, drop.wsId);
      } else {
        // Minimized → other workspace's minimized section: move
        // without restoring (keeps the minimize state).
        invoke("window_move_to_workspace", {
          windowId,
          targetWorkspaceId: drop.wsId,
        }).catch(() => {});
      }
    }
  }
}

/// Creates the per-instance drag engine. Reactive fields
/// (`dragState`, `dragOverWs`, `dragOverSection`) are exposed as
/// getters over `$state` so templates track them; the in-flight
/// gesture record stays non-reactive on purpose.
export function createDragEngine(deps: DragEngineDeps) {
  /// Drag state. `kind` distinguishes active vs. minimized source
  /// cards so the drop handler knows which row to target on same-
  /// workspace drops. `sourceWs` is "" for sticky/orphan minimized
  /// windows (no workspace attachment).
  let dragState = $state<
    | { windowId: string; sourceWs: string; kind: DragSourceKind }
    | null
  >(null);
  let dragOverWs = $state<string | null>(null);
  /// Which subregion the cursor is hovering during drag. Used for
  /// the drop-zone highlight so the user sees exactly whether the
  /// drop will land in the Active or Minimized area.
  let dragOverSection = $state<DropSection | null>(null);

  /// Non-reactive pointer-state for the in-flight gesture. Holds
  /// enough info to distinguish a click (pointer released before
  /// moving past `DRAG_THRESHOLD_PX`) from a drag.
  let pointerDrag: {
    pointerId: number;
    startX: number;
    startY: number;
    windowId: string;
    sourceWs: string;
    card: HTMLElement;
    dragging: boolean;
    kind: DragSourceKind;
    /// Was Ctrl held on pointerdown? Used on pointerup-without-drag
    /// to branch between "activate" (plain click) and "toggle
    /// selection" (Ctrl+click).
    ctrlOnDown: boolean;
    /// Ids the drop handler should operate on. Single-element array
    /// for normal drags, multiple for a drag started on a selected
    /// card when the selection had > 1 entries.
    targets: string[];
  } | null = null;

  const ghost = new GhostController();
  let dragWatchdog: ReturnType<typeof setTimeout> | null = null;

  /// Timestamp of the last drag-drop. The column click handler reads
  /// it to suppress the `click` the browser synthesizes on the
  /// element under the pointer immediately after a pointerup — even
  /// when pointer capture was held by a different element (the
  /// card). Without this guard, dropping a card inside a column
  /// triggers a column-click cycle: activateWorkspace →
  /// overlay closes, which contradicts the spec (the overlay stays
  /// open so the user can chain more drags).
  let lastDropTime = 0;

  /// rAF-throttled hit-test for the drag hover state.
  ///
  /// Every `elementFromPoint()` forces a synchronous style+layout pass
  /// which at 60+ Hz pointermove (WebKitGTK fires them faster than that)
  /// causes 100-200ms stutters on constrained machines. Coalescing to
  /// one hit-test per animation frame drops the cost to at most ~60 Hz
  /// while still feeling responsive.
  let pendingHitTest: { x: number; y: number } | null = null;
  let pendingHitTestFrame = 0;

  function scheduleHitTest(x: number, y: number): void {
    // Coalesce: overwrite coords so the scheduled frame hits the latest
    // pointer position, not the stale one from the first event.
    if (pendingHitTest) {
      pendingHitTest.x = x;
      pendingHitTest.y = y;
      return;
    }
    pendingHitTest = { x, y };
    pendingHitTestFrame = requestAnimationFrame(() => {
      if (!pendingHitTest) return;
      const t = dropTargetAt(pendingHitTest.x, pendingHitTest.y);
      dragOverWs = t?.wsId ?? null;
      dragOverSection = t?.section ?? null;
      pendingHitTest = null;
    });
  }

  function cancelPendingHitTest(): void {
    if (pendingHitTestFrame !== 0) {
      cancelAnimationFrame(pendingHitTestFrame);
      pendingHitTestFrame = 0;
    }
    pendingHitTest = null;
  }

  function clearWatchdog(): void {
    if (dragWatchdog) {
      clearTimeout(dragWatchdog);
      dragWatchdog = null;
    }
  }

  function resetDragUI(): void {
    dragState = null;
    dragOverWs = null;
    dragOverSection = null;
    clearWatchdog();
    ghost.remove();
    cancelPendingHitTest();
  }

  /// Unified pointer-down handler for both active and minimized
  /// cards. `kind` routes the action at drop time; the rest of the
  /// gesture (threshold, ghost, hit-test) is identical.
  ///
  /// Multi-select gesture rules (spec §Feature 4):
  /// - If the card is in the current selection AND selection has
  ///   >1 entries → multi-drag: targets = full selection snapshot.
  /// - Otherwise → single-drag: targets = [windowId]. If the card
  ///   was NOT in the selection, clear the selection first so the
  ///   visual state matches the intent ("I'm starting a new drag,
  ///   not operating on the previous multi-select").
  function onCardPointerDown(
    e: PointerEvent,
    windowId: string,
    sourceWs: string,
    kind: DragSourceKind,
  ): void {
    // Right-click (button 2): prepare the selection state that the
    // about-to-open shadcn ContextMenu should see, then fall through
    // so bits-ui's own `oncontextmenu` (bound via `{...props}` on the
    // button) can open the menu unobstructed.
    //
    // This is the spec path — we previously had an `oncontextmenu`
    // handler on the button, but spreading `{...props}` *before* our
    // handler means ours overrode bits-ui's, and the menu never
    // opened at all. Using pointerdown-with-button-2 runs ahead of
    // the contextmenu event, so the menu renders with the right
    // selection state in the card context menu.
    if (e.button === 2) {
      const snap = selectionSnapshot();
      if (!(snap.length > 1 && snap.includes(windowId))) {
        selectOnly(windowId);
      }
      // Use log_frontend so the message lands in the shell's
      // tracing log (console.debug never makes it out of WebKitGTK
      // reliably, so the previous debug lines were invisible in
      // diagnostic sessions).
      invoke("log_frontend", {
        message: `[overlay] right-click card=${windowId} selectionSize=${snap.length}`,
      }).catch(() => {});
      return;
    }
    if (e.button !== 0) return; // left mouse / primary touch only

    const card = e.currentTarget as HTMLElement;
    try {
      card.setPointerCapture(e.pointerId);
    } catch {
      /* capture not supported → we'll still get events on the card */
    }
    const rect = card.getBoundingClientRect();
    ghost.setGrabOffset(e.clientX - rect.left, e.clientY - rect.top);

    // `ctrlKey || metaKey` so Cmd+click on macOS / WebKitGTK-style
    // environments behaves the same as Ctrl+click on Linux. The drag
    // and the click handlers both read this flag.
    const multiKey = e.ctrlKey || e.metaKey;
    const wasSelected = isSelected(windowId);
    const snap = selectionSnapshot();
    let targets: string[];
    if (wasSelected && snap.length > 1) {
      targets = snap.slice();
    } else {
      if (!wasSelected && !multiKey) {
        clearSelection();
      }
      targets = [windowId];
    }

    invoke("log_frontend", {
      message:
        `[overlay] pointerdown card=${windowId} button=${e.button} ` +
        `ctrl=${e.ctrlKey} meta=${e.metaKey} multiKey=${multiKey} ` +
        `wasSelected=${wasSelected} selSize=${snap.length} targets=${targets.length}`,
    }).catch(() => {});

    pointerDrag = {
      pointerId: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      windowId,
      sourceWs,
      card,
      dragging: false,
      kind,
      ctrlOnDown: multiKey,
      targets,
    };
  }

  function onCardPointerMove(e: PointerEvent): void {
    if (!pointerDrag || e.pointerId !== pointerDrag.pointerId) return;

    if (!pointerDrag.dragging) {
      const dx = e.clientX - pointerDrag.startX;
      const dy = e.clientY - pointerDrag.startY;
      if (Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) return;

      // Threshold crossed → promote to a real drag. Build the ghost
      // now (not on pointerdown) so tiny pointer jitter during a
      // plain click doesn't leave stray DOM behind.
      pointerDrag.dragging = true;
      try {
        ghost.build(pointerDrag.card, pointerDrag.targets, e.clientX);
      } catch (err) {
        console.error("drag-ghost setup failed", err);
        clearWatchdog();
        ghost.remove();
      }

      dragState = {
        windowId: pointerDrag.windowId,
        sourceWs: pointerDrag.sourceWs,
        kind: pointerDrag.kind,
      };

      // Backstop in case pointerup/cancel never fire (OS-level
      // grab loss, WebKitGTK quirk). Forces cleanup after 8s.
      dragWatchdog = setTimeout(resetDragUI, 8000);
    }

    ghost.position(e.clientX, e.clientY);
    scheduleHitTest(e.clientX, e.clientY);
  }

  function onCardPointerUp(e: PointerEvent): void {
    if (!pointerDrag || e.pointerId !== pointerDrag.pointerId) return;
    const captured = pointerDrag;
    pointerDrag = null;
    try {
      captured.card.releasePointerCapture(e.pointerId);
    } catch {
      /* capture already released */
    }

    if (captured.dragging) {
      const drop = dropTargetAt(e.clientX, e.clientY);
      lastDropTime = performance.now();
      resetDragUI();

      if (!drop) {
        return;
      }

      // Apply the action matrix (spec §Feature 4) to every target in
      // the captured drag. For single drags `targets.length === 1`.
      // For multi-drags we need per-window classification (active vs
      // minimized) because the source kind of the "anchor" card may
      // differ from the kind of other selected cards (a selection
      // can span both sections on the same workspace).
      const wins = get(windows);
      const workspaces = deps.getWorkspaces();
      for (const targetId of captured.targets) {
        const win = wins.find((w) => w.id === targetId);
        if (!win) continue;
        const perWinKind: DragSourceKind = win.minimized
          ? "minimized"
          : "active";
        const perWinSourceWs =
          win.workspace_ids.find((id) =>
            workspaces.some((ws) => ws.id === id),
          ) ?? "";
        applyDropAction(targetId, perWinKind, perWinSourceWs, drop);
      }
      clearSelection();
    } else {
      // Pointer never moved past the threshold — treat as a click.
      //
      // Multi-select click rules (spec §Feature 2):
      // - Ctrl+click: toggle selection, don't activate, don't close
      // - Plain click: clear selection, activate/restore, close overlay
      if (captured.ctrlOnDown) {
        toggleSelection(captured.windowId);
        invoke("log_frontend", {
          message: `[overlay] toggleSelection card=${captured.windowId}`,
        }).catch(() => {});
        return;
      }
      clearSelection();
      if (captured.kind === "active") {
        invoke("activate_window", { id: captured.windowId }).catch(() => {});
      } else {
        restoreWindow(captured.windowId);
      }
      deps.closeOverlayAndCollapse();
    }
  }

  function onCardPointerCancel(e: PointerEvent): void {
    if (!pointerDrag || e.pointerId !== pointerDrag.pointerId) return;
    const captured = pointerDrag;
    pointerDrag = null;
    try {
      captured.card.releasePointerCapture(e.pointerId);
    } catch {
      /* capture already released */
    }
    resetDragUI();
  }

  /// Document-level Escape handler that aborts an in-flight drag.
  /// Registered alongside the overlay-keydown handler in the host's
  /// mount effect. Keeps the cancel path consistent across both drag
  /// kinds: overlay-card and minimized-icon.
  function onDragEscape(e: KeyboardEvent): void {
    if (e.key !== "Escape" || !pointerDrag) return;
    const captured = pointerDrag;
    pointerDrag = null;
    try {
      captured.card.releasePointerCapture(captured.pointerId);
    } catch {
      /* capture already released */
    }
    resetDragUI();
  }

  return {
    get dragState() {
      return dragState;
    },
    get dragOverWs() {
      return dragOverWs;
    },
    get dragOverSection() {
      return dragOverSection;
    },
    get lastDropTime() {
      return lastDropTime;
    },
    onCardPointerDown,
    onCardPointerMove,
    onCardPointerUp,
    onCardPointerCancel,
    onDragEscape,
    /// Aborts any in-flight gesture and clears all drag UI state.
    /// The host calls this on unmount so a ghost never outlives its
    /// indicator.
    resetDragUI,
  };
}
