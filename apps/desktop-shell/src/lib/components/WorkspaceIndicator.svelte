<script lang="ts">
  // Value and type imports are split — inline mixed form trips a
  // Tailwind Vite plugin bug (CSS-parses the script block). See
  // top-level CLAUDE.md.
  import { primaryWorkspaces, activateWorkspace } from "$lib/stores/workspaces.js";
  import type { WorkspaceInfo } from "$lib/stores/workspaces.js";
  import { windows } from "$lib/stores/windows.js";
  import type { WindowInfo } from "$lib/stores/windows.js";
  import { resolveAppIcon } from "$lib/stores/appIcons.js";
  import { invoke } from "@tauri-apps/api/core";
  import { AppWindow } from "lucide-svelte";

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

  function scheduleClose() {
    if (hoverTimer) clearTimeout(hoverTimer);
    hoverTimer = setTimeout(() => {
      overlayVisible = false;
      invoke("set_popover_input_region", { expanded: false }).catch(
        () => {},
      );
      hoverTimer = null;
    }, 300);
  }

  function onEnter() {
    if (hoverTimer) clearTimeout(hoverTimer);
    hoverTimer = setTimeout(() => {
      openOverlay();
      hoverTimer = null;
    }, 50);
  }

  function onLeave() {
    scheduleClose();
  }

  function onOverlayEnter() {
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

  function getWindowsForWorkspace(wsId: string): WindowInfo[] {
    return $windows.filter((w) => w.workspace_ids.includes(wsId));
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

  let dragState = $state<{ windowId: string; sourceWs: string } | null>(
    null,
  );
  let dragOverWs = $state<string | null>(null);

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
  let pointerDrag: {
    pointerId: number;
    startX: number;
    startY: number;
    windowId: string;
    sourceWs: string;
    card: HTMLElement;
    dragging: boolean;
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

  /// Finds the workspace column under (x, y) via elementFromPoint,
  /// returning its workspace id (from `data-ws-id`) or null if the
  /// pointer isn't over any column. Used for live hover highlight
  /// during the drag and for drop-target resolution.
  function columnIdAt(clientX: number, clientY: number): string | null {
    const el = document.elementFromPoint(clientX, clientY) as HTMLElement | null;
    if (!el) return null;
    const column = el.closest("[data-ws-id]") as HTMLElement | null;
    return column?.dataset.wsId ?? null;
  }

  function resetDragUI() {
    dragState = null;
    dragOverWs = null;
    removeGhost();
  }

  function onCardPointerDown(
    e: PointerEvent,
    win: WindowInfo,
    sourceWs: string,
  ) {
    if (e.button !== 0) return; // left mouse / primary touch only
    const card = e.currentTarget as HTMLElement;
    // Pointer capture: all subsequent move/up events for this
    // pointerId route to `card`, even when the pointer leaves
    // the card's bounds. Drops the need for document-level
    // fallback listeners.
    try {
      card.setPointerCapture(e.pointerId);
    } catch {
      /* capture not supported → we'll still get events on the card */
    }
    // Capture the offset from card-top-left to pointer NOW while
    // the card is still in its starting position. Used later to
    // position the ghost so the pointer stays on the same point
    // of the card.
    const rect = card.getBoundingClientRect();
    ghostOffsetX = e.clientX - rect.left;
    ghostOffsetY = e.clientY - rect.top;

    pointerDrag = {
      pointerId: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      windowId: win.id,
      sourceWs,
      card,
      dragging: false,
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
        const rect = pointerDrag.card.getBoundingClientRect();
        const clone = pointerDrag.card.cloneNode(true) as HTMLElement;
        clone.removeAttribute("draggable");
        clone.classList.add("drag-ghost");
        clone.style.width = `${rect.width}px`;
        clone.style.height = `${rect.height}px`;
        document.body.appendChild(clone);
        ghostEl = clone;
        // Seed the tilt tracker with the current X so the very first
        // positionGhost call sees deltaX ≈ 0 (neutral tilt) instead
        // of an artificial jump from 0 → clientX.
        ghostLastX = e.clientX;
        ghostRotation = 0;
      } catch (err) {
        console.error("drag-ghost setup failed", err);
        removeGhost();
      }

      dragState = {
        windowId: pointerDrag.windowId,
        sourceWs: pointerDrag.sourceWs,
      };

      // Backstop in case pointerup/cancel never fire (OS-level
      // grab loss, WebKitGTK quirk). Forces cleanup after 8s.
      dragWatchdog = setTimeout(resetDragUI, 8000);
    }

    positionGhost(e.clientX, e.clientY);
    dragOverWs = columnIdAt(e.clientX, e.clientY);
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
      const targetWs = columnIdAt(e.clientX, e.clientY);
      // Mark the drop BEFORE resetDragUI so the synthetic click that
      // follows pointerup (see `lastDropTime` comment) is inside the
      // 300ms suppression window.
      lastDropTime = performance.now();
      resetDragUI();
      if (targetWs && targetWs !== captured.sourceWs) {
        invoke("window_move_to_workspace", {
          windowId: captured.windowId,
          targetWorkspaceId: targetWs,
        }).catch((err) =>
          console.error("window_move_to_workspace failed", err),
        );
      }
    } else {
      // Pointer never moved past the threshold — treat as a click.
      // Unifies focus-on-click with drag so we don't need a
      // separate `onclick` that would fire AFTER pointerup and
      // double-trigger.
      invoke("activate_window", { id: captured.windowId }).catch(() => {});
      overlayVisible = false;
      invoke("set_popover_input_region", { expanded: false }).catch(
        () => {},
      );
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
    return () => {
      if (hoverTimer) clearTimeout(hoverTimer);
      pointerDrag = null;
      resetDragUI();
    };
  });
</script>

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
         script for why. -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="overlay"
      class:overlay-visible={overlayVisible}
      onmouseenter={onOverlayEnter}
    >
      <div class="ws-columns">
        {#each $primaryWorkspaces as ws, i (ws.id)}
          {@const wsWindows = getWindowsForWorkspace(ws.id)}
          {@const { shown, overflow } = visibleSlice(wsWindows)}
          {@const isDropTarget =
            dragState !== null && dragState.sourceWs !== ws.id}
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
            <div class="ws-project"></div>

            {#if shown.length === 0}
              <div class="ws-empty">Empty</div>
            {:else}
              <div class="ws-cards">
                {#each shown as win (win.id)}
                  <!-- svelte-ignore a11y_click_events_have_key_events -->
                  <button
                    class="window-card"
                    class:window-card-dragging={dragState?.windowId ===
                      win.id}
                    onpointerdown={(e) => onCardPointerDown(e, win, ws.id)}
                    onpointermove={onCardPointerMove}
                    onpointerup={onCardPointerUp}
                    onpointercancel={onCardPointerCancel}
                    title={win.title || win.app_id}
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
                {/each}
                {#if overflow > 0}
                  <div class="window-card overflow-badge" aria-hidden="true">
                    +{overflow}
                  </div>
                {/if}
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

  /* ── Overlay ────────────────────────────────────────────────────────── */

  .overlay {
    position: absolute;
    top: 100%;
    left: 50%;
    transform: translateX(-50%) translateY(-4px);
    z-index: 50;
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
    padding: 12px;
    border-radius: var(--radius-md);
    border: 1px solid transparent;
    background: transparent;
    cursor: pointer;
    transition:
      background-color 120ms ease,
      border-color 120ms ease;
    color: var(--color-fg-shell);
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
