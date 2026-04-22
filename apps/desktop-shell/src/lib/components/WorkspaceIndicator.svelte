<script lang="ts">
  // Value and type imports are split — inline mixed form trips a
  // Tailwind Vite plugin bug (CSS-parses the script block). See
  // top-level CLAUDE.md.
  import { primaryWorkspaces, activateWorkspace } from "$lib/stores/workspaces.js";
  import type { WorkspaceInfo } from "$lib/stores/workspaces.js";
  import { windows } from "$lib/stores/windows.js";
  import type { WindowInfo } from "$lib/stores/windows.js";
  import { projectPerWorkspace } from "$lib/stores/workspaceProjects.js";
  import { resolveAppIcon } from "$lib/stores/appIcons.js";
  import {
    minimizedByWorkspace,
    loadMinimizedWindows,
    restoreWindow,
    restoreWindowToWorkspace,
    minimizeWindow,
    closeMinimizedWindow,
  } from "$lib/stores/minimizedWindows.js";
  import type { MinimizedWindow } from "$lib/stores/minimizedWindows.js";
  import {
    selectedWindowIds,
    toggleSelection,
    selectOnly,
    clearSelection,
    isSelected,
    selectionSnapshot,
    pruneSelection,
  } from "$lib/stores/overlaySelection.js";
  import * as ContextMenu from "$lib/components/ui/context-menu/index.js";
  import { scale } from "svelte/transition";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import type { UnlistenFn } from "@tauri-apps/api/event";
  import { AppWindow } from "lucide-svelte";

  /// One-shot primer for the icon cache — runs on mount so the first
  /// paint of minimized-window cards in the overlay doesn't incur
  /// N serial invokes for resolve_app_icon.
  $effect(() => {
    loadMinimizedWindows();
  });

  /// Selection pruning: when a window disappears (closed externally,
  /// crashed), drop it out of the selection set so the multi-menu
  /// doesn't try to act on a dead id. Re-runs on every `$windows`
  /// change; cheap because `pruneSelection` is a no-op when nothing
  /// to prune.
  $effect(() => {
    const live = new Set($windows.map((w) => w.id));
    pruneSelection(live);
  });

  /// Close-overlay side effect: whenever the overlay hides we also
  /// clear any outstanding selection so the next open starts fresh.
  $effect(() => {
    if (!overlayVisible) {
      clearSelection();
    }
  });

  const mode = $derived(
    $primaryWorkspaces.length <= 5
      ? ("pills" as const)
      : $primaryWorkspaces.length <= 9
        ? ("dots" as const)
        : ("text" as const),
  );

  const activeIndex = $derived(
    $primaryWorkspaces.findIndex((w) => w.active),
  );

  // Hover overlay state.
  //
  // Open delay is 50ms (~debounce, not a real wait) so the overlay
  // feels instant to intentional hover without flashing open on rapid
  // topbar traversal. Close delay is 300ms grace to tolerate brief
  // excursions outside the overlay bounds (e.g. pointer jitter while
  // dragging near the edge, or briefly exiting during a drop).
  //
  // `hoverTimer` is reused for both open and close — only one is ever
  // pending because entering cancels pending-close and vice versa.
  let overlayVisible = $state(false);
  let hoverTimer: ReturnType<typeof setTimeout> | null = null;

  function openOverlay() {
    overlayVisible = true;
    invoke("set_popover_input_region", { expanded: true }).catch(() => {});
  }

  /// Tracks whether any card's shadcn ContextMenu is currently open.
  /// When one is, `scheduleClose` is a no-op: the menu Portal renders
  /// in `document.body`, outside `.ws-root`, so moving the pointer
  /// from a card into the menu fires `onmouseleave` on ws-root and
  /// would otherwise close the overlay while the user is picking a
  /// menu item. Wired per-card via `<ContextMenu.Root onOpenChange>`.
  let contextMenuOpen = $state(false);

  function onCardMenuOpenChange(open: boolean): void {
    contextMenuOpen = open;
    invoke("log_frontend", {
      message:
        `[overlay] contextMenu open=${open} hoverInside=${hoverInsideRoot} ` +
        `overlayVisible=${overlayVisible}`,
    }).catch(() => {});
    // Deliberately NO scheduleClose on menu close here. The previous
    // version called scheduleClose when the menu closed and the
    // cursor was outside ws-root, but that fired even for the
    // "user clicked a menu item" case — the cursor is obviously
    // outside ws-root (on the menu item itself) at that moment, so
    // every action would also close the overlay. The overlay now
    // stays open until the user deliberately moves the cursor
    // outside, which triggers a fresh mouseleave on ws-root.
  }

  /// Tracks hover state on ws-root via the existing mouseenter/leave.
  /// Needed so onCardMenuOpenChange can decide whether to re-schedule
  /// a close after the menu dismisses (pointer still inside = don't
  /// close; pointer already gone = close as if no menu was active).
  let hoverInsideRoot = $state(false);

  /// True iff any bits-ui context menu content is currently mounted
  /// in the document. Used as a backup for `contextMenuOpen` — the
  /// Svelte-tracked flag can lag bits-ui's internal state due to
  /// microtask ordering, but a DOM query is always ground truth.
  function anyContextMenuMounted(): boolean {
    return (
      document.querySelector('[role="menu"]:not([hidden])') !== null ||
      document.querySelector("[data-bits-context-menu-content]") !== null
    );
  }

  function scheduleClose() {
    if (hoverTimer) clearTimeout(hoverTimer);
    // Three guards against the menu-Portal race:
    //  1. Svelte-tracked `contextMenuOpen` flag
    //  2. Live DOM query for any bits-ui menu
    //  3. The onLeave handler below also short-circuits when the
    //     pointer moved into a menu (`relatedTarget` check) — that
    //     catches the case where the menu is transitioning open.
    // Any one of these returning true keeps the overlay open.
    if (contextMenuOpen || anyContextMenuMounted()) {
      invoke("log_frontend", {
        message: `[overlay] scheduleClose blocked (ctxOpen=${contextMenuOpen} domMenu=${anyContextMenuMounted()})`,
      }).catch(() => {});
      return;
    }
    hoverTimer = setTimeout(() => {
      // Re-check at fire time: the user may have moved to a menu
      // DURING the 300ms wait (bits-ui's transition delays).
      if (contextMenuOpen || anyContextMenuMounted()) {
        hoverTimer = null;
        return;
      }
      overlayVisible = false;
      invoke("set_popover_input_region", { expanded: false }).catch(
        () => {},
      );
      hoverTimer = null;
    }, 300);
  }

  function onEnter() {
    hoverInsideRoot = true;
    if (hoverTimer) clearTimeout(hoverTimer);
    hoverTimer = setTimeout(() => {
      openOverlay();
      hoverTimer = null;
    }, 50);
  }

  /// Check whether a DOM node belongs to an open context menu portal.
  /// bits-ui decorates menu content with `role="menu"` and several
  /// `data-bits-*` attributes. Any ancestor match counts — the menu
  /// item the cursor is entering might be nested deeper.
  function isInsideContextMenu(el: EventTarget | null): boolean {
    if (!(el instanceof Element)) return false;
    return (
      el.closest('[role="menu"]') !== null ||
      el.closest("[data-bits-context-menu-content]") !== null ||
      el.closest("[data-context-menu-content]") !== null
    );
  }

  function onLeave(e: MouseEvent) {
    hoverInsideRoot = false;
    const related = e.relatedTarget;
    const intoMenu = isInsideContextMenu(related);
    invoke("log_frontend", {
      message:
        `[overlay] ws-root mouseleave intoMenu=${intoMenu} ` +
        `ctxOpen=${contextMenuOpen} domMenu=${anyContextMenuMounted()} ` +
        `related=${related instanceof Element ? related.tagName : String(related)}`,
    }).catch(() => {});
    if (intoMenu) {
      // Pointer moved into a menu portal — keep the overlay open.
      // Don't even schedule a close: the menu-closed path will
      // re-check state and either let the user interact further
      // or schedule close naturally via the next mouseleave.
      return;
    }
    scheduleClose();
  }

  function onOverlayEnter() {
    hoverInsideRoot = true;
    if (hoverTimer) {
      clearTimeout(hoverTimer);
      hoverTimer = null;
    }
    // If the pointer reached the overlay before the 50ms open timer
    // fired (fast mouse), open immediately — otherwise we'd cancel
    // the open and sit with an invisible overlay under the cursor.
    if (!overlayVisible) openOverlay();
  }

  // NOTE: no `onOverlayLeave`. Closing is handled exclusively by the
  // `.ws-root` mouseleave (which fires when the pointer leaves the
  // whole indicator — pills + overlay). A dedicated overlay-leave
  // handler would close the overlay immediately the moment the user
  // moved from overlay → pills (both are inside `.ws-root`), and it
  // would also snap the overlay shut the instant the user released
  // a drag on the outside edge — neither is desired UX.

  function pillLabel(_ws: WorkspaceInfo, i: number): string {
    return String(i + 1);
  }

  function fullLabel(ws: WorkspaceInfo, i: number): string {
    return ws.name.trim() || `Workspace ${i + 1}`;
  }

  function handlePillClick(id: string) {
    activateWorkspace(id);
  }

  // ── Keyboard navigation ─────────────────────────────────────────────────
  //
  // Activated by the compositor's `workspace_overlay_open` event
  // (Super+Tab by default; see `compositor/src/config/mod.rs`). When
  // active, a focus ring sits on `focusedWindowId` and arrow / Tab /
  // 1-9 keys move it. The hover open path leaves `focusedWindowId`
  // null and shows no ring — keyboard mode toggles on first nav key.
  //
  // FOCUS GRAB CAVEAT: the topbar layer-shell surface only receives
  // DOM keydown events when GTK has routed keyboard focus to it.
  // After Super+Tab the compositor consumes the keystroke and emits
  // the open event but does not move keyboard focus to the shell, so
  // we explicitly call `.focus()` on the overlay element below to
  // request it from WebKitGTK. Whether the compositor actually grants
  // it depends on the layer's `keyboard_interactivity` mode; for
  // V1 we rely on OnDemand + focus-call. If keys still don't fire
  // for the user, the next iteration moves the keyboard-grab into
  // the compositor side.
  let focusedWindowId = $state<string | null>(null);
  // Svelte 5 wants `bind:this` targets to be `$state` so its
  // reactivity tracker doesn't get confused. We never read this
  // reactively, only call `.focus()` imperatively, but the warning
  // is correct on principle.
  let overlayEl = $state<HTMLDivElement | null>(null);

  /// Flat ordering of all visible windows: workspace by workspace,
  /// in the order their cards render. Used by Tab / Shift+Tab to
  /// cycle across workspace boundaries.
  const flatWindowOrder = $derived.by(() => {
    const order: { winId: string; wsId: string }[] = [];
    for (const ws of $primaryWorkspaces) {
      for (const w of $windows) {
        if (w.workspace_ids.includes(ws.id)) {
          order.push({ winId: w.id, wsId: ws.id });
        }
      }
    }
    return order;
  });

  /// ─── Context menu actions ─────────────────────────────────────
  ///
  /// Thin wrappers around `invoke(...)` so the context-menu items
  /// stay declarative in the template. Each returns void — nothing
  /// in the menu path reads a return value. Failures are logged but
  /// never surface to the user: the menu is fire-and-forget, and
  /// the UI re-renders from live Wayland state regardless.

  function closeWindowAction(windowId: string): void {
    invoke("close_window", { windowId }).catch((e) =>
      console.warn("close_window failed:", e),
    );
  }

  function fullscreenWindowAction(
    windowId: string,
    currentlyFullscreen: boolean,
  ): void {
    invoke("fullscreen_window", {
      windowId,
      enabled: !currentlyFullscreen,
    }).catch((e) => console.warn("fullscreen_window failed:", e));
  }

  function tileWindowAction(
    windowId: string,
    direction: "left" | "right",
  ): void {
    invoke("tile_window", { windowId, direction }).catch((e) =>
      console.warn("tile_window failed:", e),
    );
  }

  function moveWindowToWorkspaceAction(windowId: string, wsId: string): void {
    invoke("window_move_to_workspace", {
      windowId,
      targetWorkspaceId: wsId,
    }).catch((e) => console.warn("window_move_to_workspace failed:", e));
  }

  /// Multi-action helpers. Each snapshots the selection at invoke
  /// time so subsequent re-renders (from the actions themselves
  /// causing state transitions) don't cause iteration to drop
  /// mid-loop.
  function closeAllSelected(): void {
    for (const id of selectionSnapshot()) closeWindowAction(id);
    clearSelection();
  }

  function minimizeAllSelected(): void {
    for (const id of selectionSnapshot()) {
      const w = $windows.find((x) => x.id === id);
      if (w && !w.minimized) minimizeWindow(id);
    }
    clearSelection();
  }

  function restoreAllSelected(): void {
    for (const id of selectionSnapshot()) {
      const w = $windows.find((x) => x.id === id);
      if (w && w.minimized) restoreWindow(id);
    }
    clearSelection();
    overlayVisible = false;
  }

  function moveAllSelectedToWorkspace(wsId: string): void {
    for (const id of selectionSnapshot()) {
      const w = $windows.find((x) => x.id === id);
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

  function tileSideBySide(ids: [string, string]): void {
    tileWindowAction(ids[0], "left");
    tileWindowAction(ids[1], "right");
    clearSelection();
    overlayVisible = false;
  }

  function pickInitialFocus(): string | null {
    // Prefer the currently active window so the first Tab move is
    // semantically "show me the next thing after where I am".
    const active = $windows.find((w) => w.active);
    if (active) return active.id;
    const activeWs = $primaryWorkspaces.find((w) => w.active);
    if (activeWs) {
      const wins = $windows.filter((w) =>
        w.workspace_ids.includes(activeWs.id),
      );
      if (wins.length > 0) return wins[0].id;
    }
    return flatWindowOrder[0]?.winId ?? null;
  }

  function cycleWindow(direction: 1 | -1) {
    const order = flatWindowOrder;
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

  function navigateWorkspace(direction: 1 | -1) {
    if ($primaryWorkspaces.length === 0) return;
    let currentWsIdx = -1;
    if (focusedWindowId) {
      const win = $windows.find((w) => w.id === focusedWindowId);
      if (win) {
        currentWsIdx = $primaryWorkspaces.findIndex((ws) =>
          win.workspace_ids.includes(ws.id),
        );
      }
    }
    if (currentWsIdx < 0) {
      currentWsIdx = $primaryWorkspaces.findIndex((ws) => ws.active);
    }
    const wsIdx =
      (currentWsIdx + direction + $primaryWorkspaces.length) %
      $primaryWorkspaces.length;
    const wsId = $primaryWorkspaces[wsIdx].id;
    const wins = $windows.filter((w) => w.workspace_ids.includes(wsId));
    focusedWindowId = wins[0]?.id ?? null;
  }

  function navigateColumn(direction: 1 | -1) {
    if (!focusedWindowId) {
      focusedWindowId = pickInitialFocus();
      return;
    }
    const win = $windows.find((w) => w.id === focusedWindowId);
    if (!win) return;
    const wsId = win.workspace_ids[0];
    const wins = $windows.filter((w) => w.workspace_ids.includes(wsId));
    const idx = wins.findIndex((w) => w.id === focusedWindowId);
    if (idx < 0 || wins.length === 0) return;
    const next = (idx + direction + wins.length) % wins.length;
    focusedWindowId = wins[next].id;
  }

  function jumpToWorkspaceN(n: number) {
    const ws = $primaryWorkspaces[n - 1];
    if (!ws) return;
    const wins = $windows.filter((w) => w.workspace_ids.includes(ws.id));
    focusedWindowId = wins[0]?.id ?? null;
  }

  function activateFocused() {
    const id = focusedWindowId;
    if (!id) return;
    // Enter on a minimized card restores instead of activating —
    // activate alone wouldn't un-minimize on cosmic, it just toggles
    // focus. restoreWindow calls both unset_minimized and activate,
    // which is what the user expects.
    const win = $windows.find((w) => w.id === id);
    if (win?.minimized) {
      restoreWindow(id);
    } else {
      invoke("activate_window", { id }).catch(() => {});
    }
    closeOverlayKeyboard();
  }

  function closeOverlayKeyboard() {
    overlayVisible = false;
    focusedWindowId = null;
    invoke("set_popover_input_region", { expanded: false }).catch(() => {});
  }

  /// Two-key "go to" gesture: press `g`, then within `GOTO_TIMEOUT_MS`
  /// press a digit 1-9 to jump to that workspace AND close the Map.
  /// Any other key cancels the pending state. Matches vim `g` prefix
  /// behaviour — familiar to keyboard-first users.
  const GOTO_TIMEOUT_MS = 800;
  let gotoPending = $state(false);
  let gotoPendingTimer: ReturnType<typeof setTimeout> | null = null;

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
    if (sel.length > 1) {
      // Multi: if any is active, minimize; else restore.
      const selWindows = sel
        .map((id) => $windows.find((w) => w.id === id))
        .filter((w): w is WindowInfo => Boolean(w));
      const anyActive = selWindows.some((w) => !w.minimized);
      if (anyActive) {
        minimizeAllSelected();
      } else {
        restoreAllSelected();
      }
      return;
    }
    const id = focusedWindowId;
    if (!id) return;
    const w = $windows.find((x) => x.id === id);
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
    const w = $windows.find((x) => x.id === id);
    if (!w) return;
    fullscreenWindowAction(id, w.fullscreen ?? false);
  }

  function actionToggleSelection(): void {
    if (focusedWindowId) {
      toggleSelection(focusedWindowId);
    }
  }

  function onKeydown(e: KeyboardEvent) {
    if (!overlayVisible) return;

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
      const ws = $primaryWorkspaces[parseInt(key, 10) - 1];
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

  /// Forwards the compositor's `workspace_overlay_open` event into
  /// the overlay's open / cycle state. First fire opens + seeds focus
  /// on the active window; subsequent fires while the overlay is
  /// already open advance focus by one (Super+Tab as a true cycler,
  /// macOS Cmd+Tab style).
  function onWorkspaceOverlayOpenEvent() {
    if (overlayVisible) {
      cycleWindow(1);
      return;
    }
    openOverlay();
    focusedWindowId = pickInitialFocus();
    // Try to grab DOM focus on the overlay so subsequent keys land
    // here. Layer-shell focus semantics are compositor-driven, so
    // this is best-effort — if the user's Tab still doesn't land
    // here, the compositor needs to set keyboard focus to the layer.
    setTimeout(() => overlayEl?.focus(), 0);
  }

  /// Timestamp of the last drag-drop. Used to suppress the synthesized
  /// `click` that the browser fires on the element under the pointer
  /// immediately after a pointerup — even when pointer capture was
  /// held by a different element (the card). Without this guard,
  /// dropping a card inside a column triggers a column-click cycle:
  /// activateWorkspace → overlayVisible=false → overlay closes, which
  /// contradicts the spec (overlay stays open so the user can chain
  /// more drags).
  let lastDropTime = 0;

  function handleColumnClick(id: string) {
    // Swallow the click synthesized by the browser after a drag-drop.
    // 300ms is generous: a real user click lands within a few ms of
    // pointerup, a synthetic click after drag is even tighter.
    if (performance.now() - lastDropTime < 300) return;
    activateWorkspace(id);
    overlayVisible = false;
    invoke("set_popover_input_region", { expanded: false }).catch(() => {});
  }

  /// Clicking a window card focuses the window. The compositor's
  /// `toplevel_management::activate` handler also switches to the
  /// window's workspace if needed, so one call covers both.
  function handleWindowClick(win: WindowInfo, e: Event) {
    e.stopPropagation();
    invoke("activate_window", { id: win.id }).catch(() => {});
    overlayVisible = false;
    invoke("set_popover_input_region", { expanded: false }).catch(() => {});
  }

  function visibleSlice(list: WindowInfo[]): {
    shown: WindowInfo[];
    overflow: number;
  } {
    if (list.length <= 6) return { shown: list, overflow: 0 };
    return { shown: list.slice(0, 5), overflow: list.length - 5 };
  }

  /// Pre-compute a Map<wsId, WindowInfo[]> once per render tick rather
  /// than filtering `$windows` inline for each of the 9 workspace
  /// columns. With 30+ windows this drops overlay render cost from
  /// O(workspaces × windows) to O(windows).
  const windowsByWorkspace = $derived.by(() => {
    const map = new Map<string, WindowInfo[]>();
    for (const w of $windows) {
      // Minimized windows move to the dedicated minimized section
      // below each workspace card in the overlay. If they also
      // appeared in the regular cards row it would double-count.
      if (w.minimized) continue;
      for (const wsId of w.workspace_ids) {
        const bucket = map.get(wsId);
        if (bucket) bucket.push(w);
        else map.set(wsId, [w]);
      }
    }
    return map;
  });

  function getWindowsForWorkspace(wsId: string): WindowInfo[] {
    return windowsByWorkspace.get(wsId) ?? [];
  }

  function truncateTitle(title: string, appId: string): string {
    const source = title.trim() || appId || "";
    if (source.length <= 10) return source;
    return source.slice(0, 9) + "\u2026";
  }

  // ── Drag & Drop ──────────────────────────────────────────────────────────
  //
  // Native HTML5 drag. No custom ghost — the browser renders a default
  // drag image from the source element, which is good enough for V1
  // and keeps the component stable (custom ghosts previously caused
  // shell freezes; see debug session 2026-04-19).

  /// Drag state. `kind` distinguishes active vs. minimized source
  /// cards so the drop handler knows which row to target on same-
  /// workspace drops. `sourceWs` is "" for sticky/orphan minimized
  /// windows (no workspace attachment).
  type DragSourceKind = "active" | "minimized";
  let dragState = $state<
    | { windowId: string; sourceWs: string; kind: DragSourceKind }
    | null
  >(null);
  let dragOverWs = $state<string | null>(null);
  /// Which subregion the cursor is hovering during drag. Used for
  /// the drop-zone highlight so the user sees exactly whether the
  /// drop will land in the Active or Minimized area.
  let dragOverSection = $state<DropSection | null>(null);

  // ── Pointer-based drag & drop ───────────────────────────────────────────
  //
  // The HTML5 drag API (dragstart/dragover/dragend + setDragImage) kept
  // freezing WebKitGTK when combined with a custom ghost — see debug
  // sessions 2026-04-19. Pointer events give us full control without
  // the browser's drag abstraction interfering:
  //   pointerdown  → capture pointer, stash start position
  //   pointermove  → once moved past threshold, create ghost; then
  //                  position ghost + update hover column every tick
  //   pointerup    → if dragged: fire move_to_workspace + cleanup;
  //                  if not dragged: treat as a click on the card
  //   pointercancel → cleanup (browser abort)
  //   watchdog     → 8s fallback cleanup
  //
  // Column hit-testing uses `document.elementFromPoint` plus a
  // `data-ws-id` attribute on each column. The ghost is
  // `pointer-events: none` so it never shadows the real hit-test.

  const DRAG_THRESHOLD_PX = 5;

  /// Non-reactive pointer-state for the in-flight gesture. Holds
  /// enough info to distinguish a click (pointer released before
  /// moving past `DRAG_THRESHOLD_PX`) from a drag.
  ///
  /// `kind` discriminates the source card subregion. The drop
  /// handler uses (kind, target-section, same/different workspace)
  /// to decide the action. All drags go through the overlay; the
  /// topbar pills are click-only.
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

  let ghostEl: HTMLElement | null = null;
  let ghostOffsetX = 0;
  let ghostOffsetY = 0;
  let dragWatchdog: ReturnType<typeof setTimeout> | null = null;

  // Dynamic tilt. Maps horizontal pointer velocity (delta-X between
  // consecutive pointermove events) to a rotation that feels like the
  // card is swinging while carried. Smoothing is exponential so the
  // ghost doesn't jitter on tiny jumps and doesn't flip instantly on
  // direction changes. `ghostLastX` is reset to the current pointer
  // X when the ghost is created (see `onCardPointerMove`) so the
  // first frame doesn't compute a huge delta against 0.
  const TILT_GAIN = 0.5; // degrees per pixel of delta-X
  const TILT_CLAMP = 10; // max absolute rotation
  const TILT_LERP = 0.25; // 0 = frozen, 1 = no smoothing
  let ghostLastX = 0;
  let ghostRotation = 0;

  /// Builds a drag-ghost element.
  ///
  /// Single-window drag: clones the source card directly.
  ///
  /// Multi-window drag (targets.length > 1): builds a stacked-cards
  /// presentation. Up to 3 card clones are layered with a small
  /// x/y offset so the user sees a physical "stack" under the
  /// cursor. If more than 3 targets exist, a "+N" badge appears in
  /// the bottom-right corner indicating the overflow.
  ///
  /// Returns the container element (already appended to document
  /// body). Tilt / position updates in `positionGhost` apply to
  /// this container as-is — the inner card clones are positioned
  /// absolutely relative to it.
  function buildGhost(sourceCard: HTMLElement, targets: string[]): HTMLElement {
    const rect = sourceCard.getBoundingClientRect();

    if (targets.length <= 1) {
      const clone = sourceCard.cloneNode(true) as HTMLElement;
      clone.removeAttribute("draggable");
      clone.classList.add("drag-ghost");
      clone.style.width = `${rect.width}px`;
      clone.style.height = `${rect.height}px`;
      document.body.appendChild(clone);
      return clone;
    }

    // Multi: stack container. Gets a fixed size equal to the source
    // card plus the total stack offset so the whole stack is one
    // positional unit for translate3d.
    const STACK_VISIBLE = 3;
    const STACK_OFFSET_PX = 4;
    const visible = Math.min(targets.length, STACK_VISIBLE);
    const container = document.createElement("div");
    container.classList.add("drag-ghost", "drag-ghost-stack");
    container.style.width = `${rect.width + (visible - 1) * STACK_OFFSET_PX}px`;
    container.style.height = `${rect.height + (visible - 1) * STACK_OFFSET_PX}px`;

    // Paint back-to-front so the clicked card (index 0) sits on top.
    for (let i = visible - 1; i >= 0; i--) {
      // Pick the card DOM for each target. If another selected card
      // isn't currently in the DOM (off-screen workspace column),
      // fall back to cloning the source card — the visual still
      // reads as "N cards".
      const targetId = targets[i];
      const targetEl =
        targetId === targets[0]
          ? sourceCard
          : (document.querySelector<HTMLElement>(
              `[data-ws-id] [aria-label][title], [data-ws-id]`,
            ) ?? sourceCard);
      const clone = targetEl.cloneNode(true) as HTMLElement;
      clone.removeAttribute("draggable");
      clone.classList.add("drag-ghost-card");
      clone.style.position = "absolute";
      clone.style.top = `${i * STACK_OFFSET_PX}px`;
      clone.style.left = `${i * STACK_OFFSET_PX}px`;
      clone.style.width = `${rect.width}px`;
      clone.style.height = `${rect.height}px`;
      container.appendChild(clone);
    }

    if (targets.length > STACK_VISIBLE) {
      const badge = document.createElement("span");
      badge.classList.add("drag-ghost-badge");
      badge.textContent = `+${targets.length - STACK_VISIBLE}`;
      container.appendChild(badge);
    }

    document.body.appendChild(container);
    return container;
  }

  function removeGhost() {
    if (dragWatchdog) {
      clearTimeout(dragWatchdog);
      dragWatchdog = null;
    }
    if (ghostEl) {
      const el = ghostEl;
      ghostEl = null;
      // Reset tilt state so the next drag starts neutral instead of
      // inheriting the last drag's final angle.
      ghostLastX = 0;
      ghostRotation = 0;
      requestAnimationFrame(() => {
        try {
          el.remove();
        } catch {
          /* already detached */
        }
      });
    }
  }

  function positionGhost(clientX: number, clientY: number) {
    if (!ghostEl) return;
    const x = clientX - ghostOffsetX;
    const y = clientY - ghostOffsetY;
    const deltaX = clientX - ghostLastX;
    ghostLastX = clientX;
    // Target rotation from velocity. Clamp before smoothing so
    // `ghostRotation` itself never exceeds the clamp, even if a
    // pathological single-frame delta is huge.
    const target = Math.max(
      -TILT_CLAMP,
      Math.min(TILT_CLAMP, deltaX * TILT_GAIN),
    );
    ghostRotation = ghostRotation + (target - ghostRotation) * TILT_LERP;
    ghostEl.style.transform =
      `translate3d(${x}px, ${y}px, 0) rotate(${ghostRotation.toFixed(2)}deg) scale(1.05)`;
  }

  /// A drop-target location resolved from a cursor position. The
  /// section tells the drop handler whether the user dropped on the
  /// "active windows" area (top 75%) or the "minimized" area
  /// (bottom 25%) of a workspace card — the action matrix branches
  /// on this.
  type DropSection = "active" | "minimized";
  type DropTarget = { wsId: string; section: DropSection };

  /// Finds the drop-target column + section under (x, y) via
  /// elementFromPoint. Walks the DOM for the closest `data-ws-id`
  /// (gives the workspace) AND the closest `data-ws-section`
  /// (gives active vs minimized). The data attributes are set on
  /// the overlay's card subregions — pills in the topbar don't
  /// carry them, so the topbar indicator never acts as a drop
  /// zone.
  function dropTargetAt(
    clientX: number,
    clientY: number,
  ): DropTarget | null {
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
    const section = (sectionEl?.dataset.wsSection as DropSection | undefined)
      ?? "active";
    return { wsId: column.dataset.wsId!, section };
  }

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

  function resetDragUI() {
    dragState = null;
    dragOverWs = null;
    dragOverSection = null;
    removeGhost();
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
  ) {
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
    // selection state in `cardContextMenu`.
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
    ghostOffsetX = e.clientX - rect.left;
    ghostOffsetY = e.clientY - rect.top;

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

  function onCardPointerMove(e: PointerEvent) {
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
        ghostEl = buildGhost(pointerDrag.card, pointerDrag.targets);
        ghostLastX = e.clientX;
        ghostRotation = 0;
      } catch (err) {
        console.error("drag-ghost setup failed", err);
        removeGhost();
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

    positionGhost(e.clientX, e.clientY);
    scheduleHitTest(e.clientX, e.clientY);
  }

  function onCardPointerUp(e: PointerEvent) {
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
      for (const targetId of captured.targets) {
        const win = $windows.find((w) => w.id === targetId);
        if (!win) continue;
        const perWinKind: DragSourceKind =
          win.minimized ? "minimized" : "active";
        const perWinSourceWs =
          win.workspace_ids.find((id) =>
            $primaryWorkspaces.some((ws) => ws.id === id),
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
      overlayVisible = false;
      invoke("set_popover_input_region", { expanded: false }).catch(() => {});
    }
  }

  /// Applies one drop action based on the source card's kind +
  /// workspace and the drop target. Extracted so multi-drag can loop
  /// over targets without duplicating the branch logic.
  function applyDropAction(
    windowId: string,
    sourceKind: DragSourceKind,
    sourceWs: string,
    drop: DropTarget,
  ) {
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

  function onCardPointerCancel(e: PointerEvent) {
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
  /// Registered in the same $effect as the overlay-keydown handler
  /// below. Keeps the cancel path consistent across both drag kinds:
  /// overlay-card and minimized-icon.
  function onDragEscape(e: KeyboardEvent) {
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

  // ── Icon resolution cache ────────────────────────────────────────────────

  let iconUrls = $state<Record<string, string | null>>({});

  const allAppIds = $derived(
    [...new Set($windows.map((w) => w.app_id).filter(Boolean))]
  );

  $effect(() => {
    for (const appId of allAppIds) {
      if (!(appId in iconUrls)) {
        iconUrls[appId] = null;
        resolveAppIcon(appId).then((url) => {
          iconUrls[appId] = url;
        });
      }
    }
  });

  $effect(() => {
    // Subscribe to the compositor's keyboard-triggered open event.
    // Listen returns its unsubscribe handle async; we stash it so the
    // unmount path can still call it cleanly.
    let unlistenWsOverlay: UnlistenFn | null = null;
    listen("lunaris://workspace-overlay-open", onWorkspaceOverlayOpenEvent)
      .then((fn) => {
        unlistenWsOverlay = fn;
      })
      .catch((e) =>
        console.warn("workspace-overlay-open subscribe failed", e),
      );

    document.addEventListener("keydown", onKeydown);
    document.addEventListener("keydown", onDragEscape);

    return () => {
      document.removeEventListener("keydown", onKeydown);
      document.removeEventListener("keydown", onDragEscape);
      if (unlistenWsOverlay) unlistenWsOverlay();
      if (hoverTimer) clearTimeout(hoverTimer);
      pointerDrag = null;
      resetDragUI();
    };
  });
</script>

<!--
  Context-menu content snippet shared by the active-window cards and
  the minimized-window cards. The snippet branches three ways based
  on the current selection:
  - Multi-select: shows Close All / Minimize All / Restore All /
    Move All to / (optional) Tile Side by Side.
  - Single active: Close / Minimize / Move to → / Tile Left / Tile
    Right / Fullscreen.
  - Single minimized: Restore / Close / Move to →.
  The snippet reads `$selectedWindowIds` and `$windows` directly —
  Svelte 5 snippets track reactive dependencies transparently.
-->
{#snippet cardContextMenu(windowId: string, isMinimized: boolean)}
  {@const sel = Array.from($selectedWindowIds)}
  {@const multi = sel.length > 1 && sel.includes(windowId)}
  {@const win = $windows.find((w) => w.id === windowId)}
  {@const currentWs = win?.workspace_ids[0] ?? ""}
  {@const moveTargets = $primaryWorkspaces.filter((ws) => ws.id !== currentWs)}

  {#if multi}
    {@const selWindows = sel
      .map((id) => $windows.find((w) => w.id === id))
      .filter((w): w is WindowInfo => Boolean(w))}
    {@const anyActive = selWindows.some((w) => !w.minimized)}
    {@const anyMinimized = selWindows.some((w) => w.minimized)}
    {@const twoActive = selWindows.length === 2 && selWindows.every((w) => !w.minimized)}

    <ContextMenu.Item onclick={closeAllSelected}>
      Close All ({sel.length})
    </ContextMenu.Item>
    {#if anyActive}
      <ContextMenu.Item onclick={minimizeAllSelected}>Minimize All</ContextMenu.Item>
    {/if}
    {#if anyMinimized}
      <ContextMenu.Item onclick={restoreAllSelected}>Restore All</ContextMenu.Item>
    {/if}
    {#if moveTargets.length > 0}
      <ContextMenu.Separator />
      <ContextMenu.Sub>
        <ContextMenu.SubTrigger>Move All to</ContextMenu.SubTrigger>
        <ContextMenu.Portal>
          <ContextMenu.SubContent class="shell-popover">
            {#each moveTargets as ws, i (ws.id)}
              <ContextMenu.Item onclick={() => moveAllSelectedToWorkspace(ws.id)}>
                {ws.name || `Workspace ${i + 1}`}
              </ContextMenu.Item>
            {/each}
          </ContextMenu.SubContent>
        </ContextMenu.Portal>
      </ContextMenu.Sub>
    {/if}
    {#if twoActive}
      <ContextMenu.Separator />
      <ContextMenu.Item onclick={() => tileSideBySide([sel[0], sel[1]])}>
        Tile Side by Side
      </ContextMenu.Item>
    {/if}
  {:else if isMinimized}
    <ContextMenu.Item onclick={() => { restoreWindow(windowId); overlayVisible = false; }}>
      Restore
    </ContextMenu.Item>
    <ContextMenu.Item onclick={() => closeMinimizedWindow(windowId)}>
      Close
    </ContextMenu.Item>
    {#if moveTargets.length > 0}
      <ContextMenu.Separator />
      <ContextMenu.Sub>
        <ContextMenu.SubTrigger>Move to</ContextMenu.SubTrigger>
        <ContextMenu.Portal>
          <ContextMenu.SubContent class="shell-popover">
            {#each moveTargets as ws, i (ws.id)}
              <ContextMenu.Item onclick={() => restoreWindowToWorkspace(windowId, ws.id)}>
                {ws.name || `Workspace ${i + 1}`}
              </ContextMenu.Item>
            {/each}
          </ContextMenu.SubContent>
        </ContextMenu.Portal>
      </ContextMenu.Sub>
    {/if}
  {:else}
    <ContextMenu.Item onclick={() => closeWindowAction(windowId)}>Close</ContextMenu.Item>
    <ContextMenu.Item onclick={() => minimizeWindow(windowId)}>Minimize</ContextMenu.Item>
    {#if moveTargets.length > 0}
      <ContextMenu.Separator />
      <ContextMenu.Sub>
        <ContextMenu.SubTrigger>Move to</ContextMenu.SubTrigger>
        <ContextMenu.Portal>
          <ContextMenu.SubContent class="shell-popover">
            {#each moveTargets as ws, i (ws.id)}
              <ContextMenu.Item onclick={() => moveWindowToWorkspaceAction(windowId, ws.id)}>
                {ws.name || `Workspace ${i + 1}`}
              </ContextMenu.Item>
            {/each}
          </ContextMenu.SubContent>
        </ContextMenu.Portal>
      </ContextMenu.Sub>
    {/if}
    <ContextMenu.Separator />
    <ContextMenu.Item onclick={() => tileWindowAction(windowId, "left")}>
      Tile Left
    </ContextMenu.Item>
    <ContextMenu.Item onclick={() => tileWindowAction(windowId, "right")}>
      Tile Right
    </ContextMenu.Item>
    <ContextMenu.Item
      onclick={() => fullscreenWindowAction(windowId, win?.fullscreen ?? false)}
    >
      {win?.fullscreen ? "Exit Fullscreen" : "Fullscreen"}
    </ContextMenu.Item>
  {/if}
{/snippet}

{#if $primaryWorkspaces.length > 0}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="ws-root" onmouseenter={onEnter} onmouseleave={onLeave}>
    {#if mode === "pills"}
      <div class="indicator" role="group" aria-label="Workspaces">
        {#each $primaryWorkspaces as ws, i (ws.id)}
          <button
            class="pill"
            class:pill-active={ws.active}
            onclick={() => handlePillClick(ws.id)}
            aria-label={fullLabel(ws, i)}
            aria-pressed={ws.active}
          >
            {pillLabel(ws, i)}
          </button>
        {/each}
      </div>
    {:else if mode === "dots"}
      <div class="indicator" role="group" aria-label="Workspaces">
        {#each $primaryWorkspaces as ws, i (ws.id)}
          <button
            class="dot-btn"
            onclick={() => handlePillClick(ws.id)}
            aria-label={fullLabel(ws, i)}
            aria-pressed={ws.active}
          >
            <span class="dot" class:dot-active={ws.active}></span>
          </button>
        {/each}
      </div>
    {:else}
      <div class="indicator" role="group" aria-label="Workspaces">
        <span class="ws-text">
          {activeIndex >= 0 ? activeIndex + 1 : 1} / {$primaryWorkspaces.length}
        </span>
      </div>
    {/if}

    <!-- Horizontal workspace overview overlay (spec §2.2–2.4).
         No onmouseleave — see the comment on `onOverlayEnter` in the
         script for why. `tabindex="-1"` lets us programmatically
         focus the div from `onWorkspaceOverlayOpenEvent` so the
         document-level keydown handler actually fires. -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      bind:this={overlayEl}
      class="overlay"
      class:overlay-visible={overlayVisible}
      role="dialog"
      aria-label="Workspace overview"
      aria-modal="false"
      tabindex="-1"
      onmouseenter={onOverlayEnter}
    >
      <div class="ws-columns">
        {#each $primaryWorkspaces as ws, i (ws.id)}
          {@const wsWindows = getWindowsForWorkspace(ws.id)}
          {@const { shown, overflow } = visibleSlice(wsWindows)}
          {@const isDropTarget =
            dragState !== null && dragState.sourceWs !== ws.id}
          {@const wsMinimized = $minimizedByWorkspace.get(ws.id) ?? []}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="ws-column"
            class:ws-column-active={ws.active}
            class:ws-column-drop-target={isDropTarget}
            class:ws-column-drop-hover={isDropTarget && dragOverWs === ws.id}
            role="button"
            tabindex="0"
            aria-label={fullLabel(ws, i)}
            data-ws-id={ws.id}
            onclick={() => handleColumnClick(ws.id)}
          >
            <div class="ws-number">{i + 1}</div>
            <!-- Project label: populated by `projectPerWorkspace` when
                 a majority of this workspace's windows map to the
                 same project in the knowledge graph. Empty placeholder
                 keeps the column's vertical rhythm stable when the
                 label is absent (no project majority / graph daemon
                 offline / empty workspace). Guard with `?.` in case
                 the derived store is transiently undefined during
                 component mount — would only happen in pathological
                 HMR states but costs nothing to be explicit. -->
            <div class="ws-project">
              {$projectPerWorkspace?.get(ws.id)?.name ?? ""}
            </div>

            <!--
              Active-windows section: top ~75% of each workspace
              card. Drop target for minimized windows (restores them
              on drop). `data-ws-section` drives the drop-target
              routing in `dropTargetAt` + `onCardPointerUp`.
            -->
            <div
              class="ws-section ws-section-active"
              class:ws-section-drop-hover={isDropTarget
                && dragOverWs === ws.id
                && dragOverSection === "active"}
              data-ws-section="active"
            >
              {#if shown.length === 0}
                <div class="ws-empty">No open windows</div>
              {:else}
                <div class="ws-cards">
                  {#each shown as win (win.id)}
                    <ContextMenu.Root onOpenChange={onCardMenuOpenChange}>
                      <ContextMenu.Trigger>
                        {#snippet child({ props })}
                          <!-- svelte-ignore a11y_click_events_have_key_events -->
                          <button
                            {...props}
                            class="window-card"
                            class:window-card-dragging={dragState?.windowId ===
                              win.id}
                            class:window-card-keyboard-focus={focusedWindowId ===
                              win.id}
                            class:window-card-selected={$selectedWindowIds.has(
                              win.id,
                            )}
                            onpointerdown={(e) =>
                              onCardPointerDown(e, win.id, ws.id, "active")}
                            onpointermove={onCardPointerMove}
                            onpointerup={onCardPointerUp}
                            onpointercancel={onCardPointerCancel}
                            title={win.title || win.app_id}
                            aria-label={`${win.title || win.app_id} on workspace ${i + 1}`}
                          >
                            {#if iconUrls[win.app_id]}
                              <img
                                class="window-card-icon"
                                src={iconUrls[win.app_id]}
                                alt=""
                                width="24"
                                height="24"
                                draggable="false"
                              />
                            {:else}
                              <AppWindow
                                size={20}
                                strokeWidth={1.5}
                                class="window-card-icon-fallback"
                              />
                            {/if}
                            <span class="window-card-title">
                              {truncateTitle(win.title, win.app_id)}
                            </span>
                          </button>
                        {/snippet}
                      </ContextMenu.Trigger>
                      <ContextMenu.Portal>
                        <ContextMenu.Content class="shell-popover">
                          {@render cardContextMenu(win.id, false)}
                        </ContextMenu.Content>
                      </ContextMenu.Portal>
                    </ContextMenu.Root>
                  {/each}
                  {#if overflow > 0}
                    <div class="window-card overflow-badge" aria-hidden="true">
                      +{overflow}
                    </div>
                  {/if}
                </div>
              {/if}
            </div>

            <!--
              Minimized section: bottom ~25%. Only rendered when the
              workspace has at least one minimized window — avoids
              wasting vertical space when none are present. During
              an active-card drag on the same workspace the section
              is forced-visible with a dashed outline so the user
              sees a valid drop zone even on cards that currently
              have no minimized windows (empty workspaces etc.).
            -->
            {#if wsMinimized.length > 0
              || (dragState?.kind === "active"
                && dragState.sourceWs === ws.id)}
              <div
                class="ws-section ws-section-minimized"
                class:ws-section-drop-hover={isDropTarget
                  && dragOverWs === ws.id
                  && dragOverSection === "minimized"}
                class:ws-section-minimized-empty={wsMinimized.length === 0}
                data-ws-section="minimized"
              >
                <div class="ws-minimized-label">Minimized</div>
                <div class="ws-cards">
                  {#each wsMinimized as m (m.windowId)}
                    <ContextMenu.Root>
                      <ContextMenu.Trigger>
                        {#snippet child({ props })}
                          <!-- svelte-ignore a11y_click_events_have_key_events -->
                          <button
                            {...props}
                            class="window-card window-card-minimized"
                            class:window-card-dragging={dragState?.windowId ===
                              m.windowId}
                            class:window-card-keyboard-focus={focusedWindowId ===
                              m.windowId}
                            class:window-card-selected={$selectedWindowIds.has(
                              m.windowId,
                            )}
                            onpointerdown={(e) =>
                              onCardPointerDown(e, m.windowId, ws.id, "minimized")}
                            onpointermove={onCardPointerMove}
                            onpointerup={onCardPointerUp}
                            onpointercancel={onCardPointerCancel}
                            title={m.title || m.appId}
                            aria-label={`Minimized: ${m.title || m.appId} on workspace ${i + 1}`}
                          >
                            {#if iconUrls[m.appId]}
                              <img
                                class="window-card-icon"
                                src={iconUrls[m.appId]}
                                alt=""
                                width="24"
                                height="24"
                                draggable="false"
                              />
                            {:else}
                              <AppWindow
                                size={20}
                                strokeWidth={1.5}
                                class="window-card-icon-fallback"
                              />
                            {/if}
                            <span class="window-card-title">
                              {truncateTitle(m.title, m.appId)}
                            </span>
                          </button>
                        {/snippet}
                      </ContextMenu.Trigger>
                      <ContextMenu.Portal>
                        <ContextMenu.Content class="shell-popover">
                          {@render cardContextMenu(m.windowId, true)}
                        </ContextMenu.Content>
                      </ContextMenu.Portal>
                    </ContextMenu.Root>
                  {/each}
                </div>
              </div>
            {/if}
          </div>
        {/each}
      </div>
    </div>
  </div>
{/if}

<style>
  .ws-root {
    position: relative;
    display: flex;
    align-items: center;
  }

  .indicator {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  /* ── Workspace card: Active + Minimized sections ───────────────── */

  /* The two subregions share a baseline container. Flex-grow on
     the active section gives it the "~75%" weight per spec; the
     minimized section auto-sizes to its content and caps growth. */
  .ws-section {
    width: 100%;
    padding: 6px 6px 4px;
    border-radius: var(--radius-md);
    transition:
      background var(--duration-fast, 150ms) ease,
      outline-color var(--duration-fast, 150ms) ease;
  }

  .ws-section-active {
    flex: 1 1 auto;
    min-height: 0;
  }

  /* Minimized section has its own subtle background tint instead
     of a hard separator — same horizontal padding as the active
     section, but shifted toward the darker end of the surface
     palette so the eye reads it as secondary without needing a
     visible divider line. `margin-top` gives the sections a small
     gap to soften the transition. */
  .ws-section-minimized {
    flex: 0 0 auto;
    /* No max-height: a percentage against the implicit-height
       `.ws-column` collapses the section to zero and the cards
       "disappear". Let the section size to its content — the
       whole overlay absorbs the growth. If a workspace ever has
       enough minimized windows to overflow the screen the
       `.ws-column` itself can grow a vertical scrollbar. */
    margin-top: 6px;
    padding-top: 8px;
    padding-bottom: 8px;
    background: color-mix(in srgb, var(--color-fg-shell) 4%, transparent);
    border-radius: var(--radius-md);
    transition: background var(--duration-fast, 150ms) ease;
  }

  /* Accent-tinted dashed outline when the section is rendered
     empty solely as a drag drop-hint. `background` comes from
     the regular minimized section tint (stays on while dragging). */
  .ws-section-minimized-empty {
    min-height: 56px;
    outline: 1px dashed
      color-mix(in srgb, var(--color-accent) 45%, transparent);
    outline-offset: -4px;
  }

  .ws-section-drop-hover {
    background: color-mix(in srgb, var(--color-accent) 12%, transparent);
    outline: 1px dashed
      color-mix(in srgb, var(--color-accent) 55%, transparent);
    outline-offset: -2px;
  }

  .ws-minimized-label {
    font-size: 9px;
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    opacity: 0.4;
    text-align: center;
    margin-bottom: 6px;
  }


  /* ── Overlay ────────────────────────────────────────────────────────── */

  .overlay {
    position: absolute;
    top: 100%;
    left: 50%;
    transform: translateX(-50%) translateY(-4px);
    /* Sits alongside the system popovers (z=100) and the quick-
       settings power dropdown (z=110). 120 keeps it above both
       while staying well under context menus (z=300). */
    z-index: 120;
    padding: 16px;
    border-radius: var(--radius-lg);
    background: var(--color-bg-shell);
    border: 1px solid
      color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    box-shadow: var(--shadow-lg);
    pointer-events: none;
    opacity: 0;
    transition:
      opacity 150ms ease-out,
      transform 150ms ease-out;
  }

  .overlay-visible {
    opacity: 1;
    pointer-events: auto;
    transform: translateX(-50%) translateY(4px);
  }

  .ws-columns {
    display: flex;
    gap: 12px;
    overflow-x: auto;
    max-width: 90vw;
  }

  .ws-column {
    display: flex;
    flex-direction: column;
    align-items: center;
    min-width: 140px;
    max-width: 200px;
    /* Cap per-column height so a workspace with many minimized
       windows grows a scroll track inside its own column rather
       than pushing the overlay off the screen. 70vh leaves room
       for the topbar + overlay padding + margins. */
    max-height: 70vh;
    overflow-y: auto;
    padding: 12px;
    border-radius: var(--radius-md);
    border: 1px solid transparent;
    background: transparent;
    cursor: pointer;
    transition:
      background-color 120ms ease,
      border-color 120ms ease;
    color: var(--color-fg-shell);
    /* Firefox / WebKit quiet-scrollbar: keep the track invisible
       until hover so the column doesn't show a persistent scrollbar
       for 2 minimized windows. */
    scrollbar-width: thin;
    scrollbar-color: transparent transparent;
  }
  .ws-column:hover {
    scrollbar-color: color-mix(in srgb, var(--color-fg-shell) 30%, transparent)
      transparent;
  }
  :global(.ws-column::-webkit-scrollbar) {
    width: 6px;
  }
  :global(.ws-column::-webkit-scrollbar-thumb) {
    background: transparent;
    border-radius: 3px;
  }
  :global(.ws-column:hover::-webkit-scrollbar-thumb) {
    background: color-mix(in srgb, var(--color-fg-shell) 30%, transparent);
  }

  .ws-column:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 4%, transparent);
  }

  .ws-column-active {
    border-color: color-mix(in srgb, var(--color-accent) 30%, transparent);
    background: color-mix(in srgb, var(--color-accent) 5%, transparent);
  }

  .ws-column-drop-target {
    border-color: color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
  }

  .ws-column-drop-hover {
    border-color: color-mix(in srgb, var(--color-accent) 60%, transparent);
    background: color-mix(in srgb, var(--color-accent) 12%, transparent);
  }

  .ws-number {
    font-size: 20px;
    font-weight: 600;
    line-height: 1;
    color: var(--color-fg-shell);
  }

  .ws-project {
    /* Placeholder row — keeps column heights aligned when the Phase 3
       knowledge-graph project label lands. */
    height: 12px;
    margin-top: 4px;
    margin-bottom: 8px;
    font-size: 10px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
  }

  .ws-empty {
    font-size: 11px;
    opacity: 0.35;
    padding: 8px 0;
  }

  .ws-cards {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    justify-content: center;
    max-width: 198px;
  }

  /* ── Window card ────────────────────────────────────────────────────── */

  .window-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    width: 60px;
    height: 56px;
    padding: 8px 4px;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--color-fg-shell) 6%, transparent);
    border: 1px solid
      color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    cursor: grab;
    color: var(--color-fg-shell);
    transition:
      transform 100ms ease,
      background-color 100ms ease,
      opacity 100ms ease;
  }

  .window-card:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    transform: scale(1.03);
  }

  .window-card:active {
    cursor: grabbing;
  }

  .window-card-dragging {
    /* Source card stays as a faint placeholder — the ghost clone
       carries the pointer-following visual. Scale override comes
       from the `:hover` rule below which we also suppress. */
    opacity: 0.3;
  }
  .window-card-dragging:hover {
    transform: none;
    background: color-mix(in srgb, var(--color-fg-shell) 6%, transparent);
  }

  /* Keyboard-navigation focus ring. Distinct from `.ws-column-active`
     (subtle accent tint on the whole column) — this is a saturated
     accent outline directly on the focused card so it stands out
     even inside the active column. */
  .window-card-keyboard-focus {
    border-color: var(--color-accent);
    box-shadow:
      0 0 0 2px color-mix(in srgb, var(--color-accent) 50%, transparent);
  }
  .window-card-keyboard-focus:hover {
    border-color: var(--color-accent);
  }

  /* Multi-selection ring. Accent border + accent-tinted background
     so the selection reads as distinct from hover (neutral tint)
     and keyboard focus (thin solid ring). A selected card that is
     also keyboard-focused uses the focus ring on top — the
     selection background still shows through. */
  .window-card-selected {
    border-color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent) 18%, transparent);
  }
  .window-card-selected:hover {
    background: color-mix(in srgb, var(--color-accent) 24%, transparent);
  }

  .window-card-icon {
    width: 24px;
    height: 24px;
    object-fit: contain;
    border-radius: 4px;
    pointer-events: none;
  }

  :global(.window-card-icon-fallback) {
    opacity: 0.5;
  }

  /* ── Drag ghost ──────────────────────────────────────────────────────
     The ghost is appended to `document.body`, outside the component's
     scoped DOM subtree. Svelte's scoped `.window-card` styles still
     apply to the clone because the clone carries the generated hash
     class attribute; `:global(.drag-ghost)` layers in only the
     float-effect (rotation, shadow, z-index, pointer-events none). */
  :global(.drag-ghost) {
    position: fixed !important;
    top: 0 !important;
    left: 0 !important;
    z-index: 10001 !important;
    pointer-events: none !important;
    opacity: 0.95 !important;
    /* Inline transform from JS drives position AND rotation via
       translate3d() + rotate(). This static declaration is only the
       initial value before the first positionGhost call lands, so
       rotation=0 looks clean during the single frame between
       `document.body.appendChild(clone)` and the first pointermove. */
    transform: translate3d(0, 0, 0) rotate(0deg) scale(1.05);
    box-shadow:
      0 12px 32px rgba(0, 0, 0, 0.35),
      0 4px 8px rgba(0, 0, 0, 0.2) !important;
    transition: none !important;
    cursor: grabbing !important;
    outline: none !important;
    will-change: transform;
  }

  /* Multi-drag stack: container owns position/transform; the
     inner card clones sit absolutely at staggered offsets so the
     user sees a 3-card stack trailing the pointer. Each inner
     clone gets its own subtle shadow so the layering reads even
     when the outer .drag-ghost shadow is diffuse. */
  :global(.drag-ghost-stack) {
    background: transparent !important;
    border: none !important;
    box-shadow: none !important;
  }
  :global(.drag-ghost-stack .drag-ghost-card) {
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.3);
  }
  :global(.drag-ghost-badge) {
    position: absolute;
    right: -6px;
    bottom: -6px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 22px;
    height: 22px;
    padding: 0 6px;
    border-radius: var(--radius-full);
    background: var(--color-accent);
    color: white;
    font-size: 11px;
    font-weight: 600;
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.35);
    z-index: 1;
  }

  .window-card-title {
    font-size: 10px;
    line-height: 1.1;
    text-align: center;
    color: color-mix(in srgb, var(--color-fg-shell) 70%, transparent);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
  }

  .overflow-badge {
    font-size: 11px;
    font-weight: 600;
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
    cursor: default;
  }

  .overflow-badge:hover {
    transform: none;
  }

  /* ── Minimized card overrides ──────────────────────────────────────
     These MUST come after the .window-card / .window-card-icon /
     .window-card-title blocks above. Svelte scopes both `.window-card`
     and `.window-card-minimized` to the same component hash, giving
     them equal specificity (0,2,0). Source order then decides the
     tie — so the more-specific-looking dual-class selector only wins
     if it's declared later in the file.
     `:global(...)` is used so the ghost clone in document.body (which
     doesn't carry the component's scope hash) still gets the size
     override during drag. The dual-class selector inside `:global`
     has specificity (0,2,0), same as the scoped `.window-card`, so
     the source-order rule applies there too. */
  :global(.window-card.window-card-minimized) {
    width: 48px;
    height: 44px;
    padding: 6px 3px;
    gap: 3px;
    opacity: 0.72;
    transition:
      transform 100ms ease,
      background-color 100ms ease,
      opacity var(--duration-fast, 150ms) ease;
  }
  :global(.window-card.window-card-minimized:hover) {
    opacity: 1;
  }
  :global(.window-card.window-card-minimized .window-card-icon) {
    width: 18px;
    height: 18px;
  }
  :global(.window-card.window-card-minimized .window-card-title) {
    font-size: 9px;
    line-height: 1.05;
  }

  /* ── Pills ──────────────────────────────────────────────────────────── */

  .pill {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 24px;
    min-width: 32px;
    padding: 0 10px;
    border-radius: var(--radius-lg);
    border: none;
    font-size: 0.6875rem;
    font-weight: 500;
    line-height: 1;
    cursor: pointer;
    white-space: nowrap;
    transition:
      background-color 150ms ease,
      color 150ms ease,
      transform 100ms ease;
    background: transparent;
    color: var(--foreground);
  }

  .pill:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }

  .pill:active {
    transform: scale(0.95);
    transition: transform 50ms ease;
  }

  .pill-active {
    background: color-mix(in srgb, var(--color-accent) 18%, transparent);
    color: var(--color-accent);
    animation: pill-activate 100ms ease forwards;
  }

  .pill-active:hover {
    background: color-mix(in srgb, var(--color-accent) 26%, transparent);
  }

  @keyframes pill-activate {
    from {
      transform: scale(0.9);
    }
    to {
      transform: scale(1);
    }
  }

  /* ── Dots ───────────────────────────────────────────────────────────── */

  .dot-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: none;
    background: transparent;
    cursor: pointer;
    border-radius: var(--radius-full);
    transition: transform 100ms ease;
  }

  .dot-btn:active {
    transform: scale(0.85);
  }

  .dot {
    display: block;
    width: 5px;
    height: 5px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--foreground) 45%, transparent);
    transition:
      width 100ms ease,
      height 100ms ease,
      background-color 150ms ease;
  }

  .dot-btn:hover .dot {
    background: color-mix(in srgb, var(--foreground) 70%, transparent);
  }

  .dot-active {
    width: 7px;
    height: 7px;
    background: var(--color-accent);
    animation: dot-activate 100ms ease forwards;
  }

  .dot-btn:hover .dot-active {
    background: color-mix(
      in srgb,
      var(--color-accent) 85%,
      var(--color-fg-shell) 15%
    );
  }

  @keyframes dot-activate {
    from {
      transform: scale(0.7);
    }
    to {
      transform: scale(1);
    }
  }

  /* ── Text ───────────────────────────────────────────────────────────── */

  .ws-text {
    font-size: 0.6875rem;
    font-weight: 500;
    color: var(--foreground);
    letter-spacing: 0.02em;
  }
</style>
