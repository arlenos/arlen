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

  // Hover overlay state. Timings per spec §3.1: 600ms open, 300ms grace.
  let overlayVisible = $state(false);
  let hoverTimer: ReturnType<typeof setTimeout> | null = null;

  function onEnter() {
    if (hoverTimer) clearTimeout(hoverTimer);
    hoverTimer = setTimeout(() => {
      overlayVisible = true;
      invoke("set_popover_input_region", { expanded: true }).catch(() => {});
    }, 600);
  }

  function onLeave() {
    if (hoverTimer) {
      clearTimeout(hoverTimer);
      hoverTimer = null;
    }
    hoverTimer = setTimeout(() => {
      overlayVisible = false;
      invoke("set_popover_input_region", { expanded: false }).catch(
        () => {},
      );
    }, 300);
  }

  function onOverlayEnter() {
    if (hoverTimer) {
      clearTimeout(hoverTimer);
      hoverTimer = null;
    }
  }

  function onOverlayLeave() {
    overlayVisible = false;
    invoke("set_popover_input_region", { expanded: false }).catch(() => {});
  }

  function pillLabel(_ws: WorkspaceInfo, i: number): string {
    return String(i + 1);
  }

  function fullLabel(ws: WorkspaceInfo, i: number): string {
    return ws.name.trim() || `Workspace ${i + 1}`;
  }

  function handlePillClick(id: string) {
    activateWorkspace(id);
  }

  function handleColumnClick(id: string) {
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

  function onDragStart(e: DragEvent, win: WindowInfo, sourceWs: string) {
    if (!e.dataTransfer) return;
    dragState = { windowId: win.id, sourceWs };
    e.dataTransfer.setData("text/plain", win.id);
    e.dataTransfer.effectAllowed = "move";
  }

  function onDragEnd() {
    dragState = null;
    dragOverWs = null;
  }

  function onDragEnter(wsId: string) {
    if (dragState) dragOverWs = wsId;
  }

  function onDragLeave(wsId: string) {
    if (dragOverWs === wsId) dragOverWs = null;
  }

  function onDragOver(e: DragEvent) {
    if (!dragState) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
  }

  function onDrop(e: DragEvent, targetWs: string) {
    e.preventDefault();
    const state = dragState;
    dragState = null;
    dragOverWs = null;
    if (!state) return;
    // Drop on source workspace → no-op, saves a compositor round-trip.
    if (state.sourceWs === targetWs) return;
    invoke("window_move_to_workspace", {
      windowId: state.windowId,
      targetWorkspaceId: targetWs,
    }).catch((err) => {
      console.error("window_move_to_workspace failed", err);
    });
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

    <!-- Horizontal workspace overview overlay (spec §2.2–2.4). -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="overlay"
      class:overlay-visible={overlayVisible}
      onmouseenter={onOverlayEnter}
      onmouseleave={onOverlayLeave}
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
            onclick={() => handleColumnClick(ws.id)}
            ondragenter={() => onDragEnter(ws.id)}
            ondragleave={() => onDragLeave(ws.id)}
            ondragover={onDragOver}
            ondrop={(e) => onDrop(e, ws.id)}
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
                    draggable="true"
                    onclick={(e) => handleWindowClick(win, e)}
                    ondragstart={(e) => onDragStart(e, win, ws.id)}
                    ondragend={onDragEnd}
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
    opacity: 0.5;
    transform: scale(0.98);
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
