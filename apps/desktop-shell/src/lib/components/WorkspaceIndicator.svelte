<script lang="ts">
  /// The topbar workspace indicator: hosts the strip (pills / dots /
  /// text), the hover lifecycle of the overview overlay, the drag
  /// and keyboard engines, and the app-icon cache. The visual
  /// pieces live under `components/workspace/`; the gesture and
  /// keyboard logic under `lib/workspace/`.

  // Value and type imports are split — inline mixed form trips a
  // Tailwind Vite plugin bug (CSS-parses the script block). See
  // top-level CLAUDE.md.
  import { workspacesByOutput, activateWorkspace } from "$lib/stores/workspaces.js";
  import type { WorkspaceInfo } from "$lib/stores/workspaces.js";
  import { getContext } from "svelte";
  import type { Readable } from "svelte/store";
  import { windows } from "$lib/stores/windows.js";
  import { resolveAppIcon } from "$lib/stores/appIcons.js";
  import { loadMinimizedWindows } from "$lib/stores/minimizedWindows.js";
  import {
    clearSelection,
    pruneSelection,
  } from "$lib/stores/overlaySelection.js";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import type { UnlistenFn } from "@tauri-apps/api/event";
  import { createDragEngine } from "$lib/workspace/drag.svelte.js";
  import { createKeyboardNav } from "$lib/workspace/keyboard.svelte.js";
  import IndicatorStrip from "./workspace/IndicatorStrip.svelte";
  import WorkspaceOverlay from "./workspace/WorkspaceOverlay.svelte";

  /// Output context published by the parent TopBar. The
  /// connector is `null` until the registry replies; in that
  /// window the legacy primary-only fallback inside
  /// `workspacesByOutput` keeps the strip populated.
  const outputCtx = getContext<
    Readable<{ connector: string | null; primary: boolean }>
  >("topbar-output");
  const outputConnector = $derived($outputCtx?.connector ?? null);
  const outputWorkspaces = $derived.by(() => {
    // Re-derive the store every time the connector changes so the
    // filter follows the bar's identity. Each store instance is
    // tiny (one closure + a Map) so reallocating per change is
    // cheaper than building one mega-store with subscribe-side
    // filtering.
    return workspacesByOutput(outputConnector);
  });
  // Re-subscribe whenever the underlying store handle changes.
  // `$outputWorkspaces` would normally subscribe once at compile
  // time; using `$derived` over the store handle here forces a
  // fresh subscription chain.
  let workspacesView = $state<WorkspaceInfo[]>([]);
  $effect(() => {
    const unsub = outputWorkspaces.subscribe((v) => {
      workspacesView = v;
    });
    return () => unsub();
  });

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
    workspacesView.length <= 5
      ? ("pills" as const)
      : workspacesView.length <= 9
        ? ("dots" as const)
        : ("text" as const),
  );

  const activeIndex = $derived(
    workspacesView.findIndex((w) => w.active),
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

  function handlePillClick(id: string) {
    activateWorkspace(id);
  }

  // ── Keyboard navigation ─────────────────────────────────────────────────
  //
  // The controller lives in `$lib/workspace/keyboard.svelte.ts`
  // (including the focus-grab caveat documented there). The
  // component provides the live workspace view, the overlay
  // visibility, and the two close paths, and wires `kb.onKeydown`
  // to the document in the mount effect below.

  // Svelte 5 wants `bind:this` targets to be `$state` so its
  // reactivity tracker doesn't get confused. We never read this
  // reactively, only call `.focus()` imperatively, but the warning
  // is correct on principle.
  let overlayEl = $state<HTMLDivElement | null>(null);

  /// The two overlay close paths, strictly distinct: `hideOverlay`
  /// flips state only — the compositor input region stays expanded;
  /// `closeOverlayAndCollapse` additionally collapses it. Which one
  /// a caller uses is part of its contract, never interchangeable.
  function hideOverlay(): void {
    overlayVisible = false;
  }

  function closeOverlayAndCollapse(): void {
    overlayVisible = false;
    invoke("set_popover_input_region", { expanded: false }).catch(() => {});
  }

  const kb = createKeyboardNav({
    getWorkspaces: () => workspacesView,
    isOverlayVisible: () => overlayVisible,
    hideOverlay,
    closeOverlayAndCollapse,
  });

  /// Forwards the compositor's `workspace_overlay_open` event into
  /// the overlay's open / cycle state. First fire opens + seeds focus
  /// on the active window; subsequent fires while the overlay is
  /// already open advance focus by one (Super+Tab as a true cycler,
  /// macOS Cmd+Tab style).
  function onWorkspaceOverlayOpenEvent() {
    if (overlayVisible) {
      kb.cycleWindow(1);
      return;
    }
    openOverlay();
    kb.seedInitialFocus();
    // Try to grab DOM focus on the overlay so subsequent keys land
    // here. Layer-shell focus semantics are compositor-driven, so
    // this is best-effort — if the user's Tab still doesn't land
    // here, the compositor needs to set keyboard focus to the layer.
    setTimeout(() => overlayEl?.focus(), 0);
  }

  // ── Drag & Drop ──────────────────────────────────────────────────────────
  //
  // The pointer-gesture engine lives in `$lib/workspace/drag.svelte.ts`
  // (with the ghost in `dragGhost.ts`). The component hands it the
  // live per-output workspace view for drop resolution and the
  // collapsing close path a plain card click triggers; the overlay
  // components forward the card pointer handlers.
  const drag = createDragEngine({
    getWorkspaces: () => workspacesView,
    closeOverlayAndCollapse,
  });

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
    listen("arlen://workspace-overlay-open", onWorkspaceOverlayOpenEvent)
      .then((fn) => {
        unlistenWsOverlay = fn;
      })
      .catch((e) =>
        console.warn("workspace-overlay-open subscribe failed", e),
      );

    document.addEventListener("keydown", kb.onKeydown);
    document.addEventListener("keydown", drag.onDragEscape);

    return () => {
      document.removeEventListener("keydown", kb.onKeydown);
      document.removeEventListener("keydown", drag.onDragEscape);
      if (unlistenWsOverlay) unlistenWsOverlay();
      if (hoverTimer) clearTimeout(hoverTimer);
      drag.resetDragUI();
    };
  });
</script>

{#if workspacesView.length > 0}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="ws-root" onmouseenter={onEnter} onmouseleave={onLeave}>
    <IndicatorStrip
      workspaces={workspacesView}
      {mode}
      {activeIndex}
      onActivate={handlePillClick}
    />
    <WorkspaceOverlay
      bind:el={overlayEl}
      visible={overlayVisible}
      workspaces={workspacesView}
      {drag}
      {kb}
      {iconUrls}
      {onOverlayEnter}
      {onCardMenuOpenChange}
      {hideOverlay}
      {closeOverlayAndCollapse}
    />
  </div>
{/if}

<style>
  .ws-root {
    position: relative;
    display: flex;
    align-items: center;
  }
</style>
