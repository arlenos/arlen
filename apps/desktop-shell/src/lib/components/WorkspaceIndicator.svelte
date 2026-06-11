<script lang="ts">
  // Value and type imports are split — inline mixed form trips a
  // Tailwind Vite plugin bug (CSS-parses the script block). See
  // top-level CLAUDE.md.
  import { workspacesByOutput, activateWorkspace } from "$lib/stores/workspaces.js";
  import type { WorkspaceInfo } from "$lib/stores/workspaces.js";
  import { getContext } from "svelte";
  import type { Readable } from "svelte/store";
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
    clearSelection,
    pruneSelection,
  } from "$lib/stores/overlaySelection.js";
  import * as ContextMenu from "@arlen/ui-kit/components/ui/context-menu/index.js";
  import { scale } from "svelte/transition";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import type { UnlistenFn } from "@tauri-apps/api/event";
  import { AppWindow } from "lucide-svelte";
  import {
    pillLabel,
    fullLabel,
    truncateTitle,
    visibleSlice,
  } from "$lib/workspace/format.js";
  import {
    closeWindowAction,
    fullscreenWindowAction,
    tileWindowAction,
    moveWindowToWorkspaceAction,
    closeAllSelected,
    minimizeAllSelected,
    restoreAllSelected,
    moveAllSelectedToWorkspace,
    tileSideBySide,
  } from "$lib/workspace/actions.js";
  import { createDragEngine } from "$lib/workspace/drag.svelte.js";
  import { createKeyboardNav } from "$lib/workspace/keyboard.svelte.js";

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


  /// The context-menu / keyboard actions live in
  /// `$lib/workspace/actions.ts`. The two below additionally close
  /// the overlay — state-only, exactly as the original inline
  /// versions did: these paths never collapse the input region
  /// themselves (the compositor refocuses the restored / tiled
  /// windows on its own).
  function restoreAllSelectedAndClose(): void {
    restoreAllSelected();
    hideOverlay();
  }

  function tileSideBySideAndClose(ids: [string, string]): void {
    tileSideBySide(ids);
    hideOverlay();
  }

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

  function handleColumnClick(id: string, e: MouseEvent) {
    // Window-card clicks are fully handled in the card's own
    // pointerdown/up pair (activate / ctrl-toggle / drag) — never
    // double-handle them here. Without this guard the bubbled click
    // activates the workspace and closes the overlay, which breaks
    // ctrl+click multi-select (spec: toggle, don't activate, don't
    // close) and races the card's own activate_window on plain
    // clicks. Scoped to button cards so the inert "+N" overflow
    // badge still activates the column.
    if (
      e.target instanceof Element &&
      e.target.closest("button.window-card")
    ) {
      return;
    }
    // Swallow the click synthesized by the browser after a drag-drop.
    // 300ms is generous: a real user click lands within a few ms of
    // pointerup, a synthetic click after drag is even tighter.
    if (performance.now() - drag.lastDropTime < 300) return;
    activateWorkspace(id);
    closeOverlayAndCollapse();
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

  // ── Drag & Drop ──────────────────────────────────────────────────────────
  //
  // The pointer-gesture engine lives in `$lib/workspace/drag.svelte.ts`
  // (with the ghost in `dragGhost.ts`). The component hands it the
  // live per-output workspace view for drop resolution and the
  // collapsing close path a plain card click triggers; the template
  // forwards the card pointer handlers.
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
  {@const moveTargets = workspacesView.filter((ws) => ws.id !== currentWs)}

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
      <ContextMenu.Item onclick={restoreAllSelectedAndClose}>Restore All</ContextMenu.Item>
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
      <ContextMenu.Item onclick={() => tileSideBySideAndClose([sel[0], sel[1]])}>
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

{#if workspacesView.length > 0}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="ws-root" onmouseenter={onEnter} onmouseleave={onLeave}>
    {#if mode === "pills"}
      <div class="indicator" role="group" aria-label="Workspaces">
        {#each workspacesView as ws, i (ws.id)}
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
        {#each workspacesView as ws, i (ws.id)}
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
          {activeIndex >= 0 ? activeIndex + 1 : 1} / {workspacesView.length}
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
        {#each workspacesView as ws, i (ws.id)}
          {@const wsWindows = getWindowsForWorkspace(ws.id)}
          {@const { shown, overflow } = visibleSlice(wsWindows)}
          {@const isDropTarget =
            drag.dragState !== null && drag.dragState.sourceWs !== ws.id}
          {@const wsMinimized = $minimizedByWorkspace.get(ws.id) ?? []}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="ws-column"
            class:ws-column-active={ws.active}
            class:ws-column-drop-target={isDropTarget}
            class:ws-column-drop-hover={isDropTarget && drag.dragOverWs === ws.id}
            role="button"
            tabindex="0"
            aria-label={fullLabel(ws, i)}
            data-ws-id={ws.id}
            onclick={(e) => handleColumnClick(ws.id, e)}
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
              routing in `dropTargetAt` + `drag.onCardPointerUp`.
            -->
            <div
              class="ws-section ws-section-active"
              class:ws-section-drop-hover={isDropTarget
                && drag.dragOverWs === ws.id
                && drag.dragOverSection === "active"}
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
                            class:window-card-dragging={drag.dragState?.windowId ===
                              win.id}
                            class:window-card-keyboard-focus={kb.focusedWindowId ===
                              win.id}
                            class:window-card-selected={$selectedWindowIds.has(
                              win.id,
                            )}
                            onpointerdown={(e) =>
                              drag.onCardPointerDown(e, win.id, ws.id, "active")}
                            onpointermove={drag.onCardPointerMove}
                            onpointerup={drag.onCardPointerUp}
                            onpointercancel={drag.onCardPointerCancel}
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
              || (drag.dragState?.kind === "active"
                && drag.dragState.sourceWs === ws.id)}
              <div
                class="ws-section ws-section-minimized"
                class:ws-section-drop-hover={isDropTarget
                  && drag.dragOverWs === ws.id
                  && drag.dragOverSection === "minimized"}
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
                            class:window-card-dragging={drag.dragState?.windowId ===
                              m.windowId}
                            class:window-card-keyboard-focus={kb.focusedWindowId ===
                              m.windowId}
                            class:window-card-selected={$selectedWindowIds.has(
                              m.windowId,
                            )}
                            onpointerdown={(e) =>
                              drag.onCardPointerDown(e, m.windowId, ws.id, "minimized")}
                            onpointermove={drag.onCardPointerMove}
                            onpointerup={drag.onCardPointerUp}
                            onpointercancel={drag.onCardPointerCancel}
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
    border-radius: var(--radius-input);
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
    border-radius: var(--radius-input);
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
    border-radius: var(--radius-card);
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
    border-radius: var(--radius-input);
    border: 1px solid transparent;
    background: transparent;
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
    border-radius: var(--radius-input);
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
    border-radius: var(--radius-chip);
    pointer-events: none;
  }

  :global(.window-card-icon-fallback) {
    opacity: 0.5;
  }

  /* ── Drag ghost ──────────────────────────────────────────────────────
     The ghost is appended to `document.body`, outside the component's
     scoped DOM subtree, and JS owns its whole lifecycle — the float
     look is applied as inline styles in `applyGhostFloatStyle` (the
     only layer that beats the scoped `.window-card` rules the clone
     carries). Only the overflow badge, a fresh span with nothing to
     fight, is styled here. */
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
    /* The accent is a light monochrome in the default theme — plain
       `white` washes out on it; the inverse foreground keeps the
       count readable on any accent. */
    color: var(--color-fg-inverse);
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
    border-radius: var(--radius-card);
    border: none;
    font-size: 0.6875rem;
    font-weight: 500;
    line-height: 1;
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
