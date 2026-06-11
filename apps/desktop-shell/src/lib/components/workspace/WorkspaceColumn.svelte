<script lang="ts">
  /// One workspace column in the overview overlay: number, project
  /// label, the active-windows section and (when present or needed
  /// as a drop hint) the minimized section. The column is the drop
  /// target for card drags and activates its workspace on click.

  import type { WindowInfo } from "$lib/stores/windows.js";
  import type { WorkspaceInfo } from "$lib/stores/workspaces.js";
  import { activateWorkspace } from "$lib/stores/workspaces.js";
  import { minimizedByWorkspace } from "$lib/stores/minimizedWindows.js";
  import { projectPerWorkspace } from "$lib/stores/workspaceProjects.js";
  import { selectedWindowIds } from "$lib/stores/overlaySelection.js";
  import type { DragEngine } from "$lib/workspace/drag.svelte.js";
  import { fullLabel, visibleSlice } from "$lib/workspace/format.js";
  import WindowCard from "./WindowCard.svelte";

  let {
    ws,
    index,
    windows,
    workspaces,
    drag,
    focusedWindowId,
    iconUrls,
    onCardMenuOpenChange,
    hideOverlay,
    closeOverlayAndCollapse,
  }: {
    ws: WorkspaceInfo;
    /// 0-based position in the strip; the header speaks 1-based.
    index: number;
    /// The non-minimized windows of this workspace (pre-bucketed by
    /// the overlay so the per-column filter stays O(windows) total).
    windows: WindowInfo[];
    /// The full per-output view, for the card menus' move targets.
    workspaces: WorkspaceInfo[];
    drag: DragEngine;
    focusedWindowId: string | null;
    iconUrls: Record<string, string | null>;
    /// Hover-engine tracker. Wired to the active cards only — the
    /// minimized cards never had it; asymmetry conserved from the
    /// monolith (see WindowCard).
    onCardMenuOpenChange: (open: boolean) => void;
    hideOverlay: () => void;
    closeOverlayAndCollapse: () => void;
  } = $props();

  const slice = $derived(visibleSlice(windows));
  const isDropTarget = $derived(
    drag.dragState !== null && drag.dragState.sourceWs !== ws.id,
  );
  const wsMinimized = $derived($minimizedByWorkspace.get(ws.id) ?? []);

  function handleColumnClick(e: MouseEvent) {
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
    activateWorkspace(ws.id);
    closeOverlayAndCollapse();
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="ws-column"
  class:ws-column-active={ws.active}
  class:ws-column-drop-target={isDropTarget}
  class:ws-column-drop-hover={isDropTarget && drag.dragOverWs === ws.id}
  role="button"
  tabindex="0"
  aria-label={fullLabel(ws, index)}
  data-ws-id={ws.id}
  onclick={handleColumnClick}
>
  <div class="ws-number">{index + 1}</div>
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
    Active-windows section: top ~75% of each workspace card. Drop
    target for minimized windows (restores them on drop).
    `data-ws-section` drives the drop-target routing in the drag
    engine's hit-testing.
  -->
  <div
    class="ws-section ws-section-active"
    class:ws-section-drop-hover={isDropTarget
      && drag.dragOverWs === ws.id
      && drag.dragOverSection === "active"}
    data-ws-section="active"
  >
    {#if slice.shown.length === 0}
      <div class="ws-empty">No open windows</div>
    {:else}
      <div class="ws-cards">
        {#each slice.shown as win (win.id)}
          <WindowCard
            windowId={win.id}
            wsId={ws.id}
            wsIndex={index}
            title={win.title}
            appId={win.app_id}
            iconUrl={iconUrls[win.app_id]}
            selected={$selectedWindowIds.has(win.id)}
            keyboardFocus={focusedWindowId === win.id}
            dragging={drag.dragState?.windowId === win.id}
            {drag}
            {workspaces}
            {hideOverlay}
            onMenuOpenChange={onCardMenuOpenChange}
          />
        {/each}
        {#if slice.overflow > 0}
          <div class="overflow-badge" aria-hidden="true">
            +{slice.overflow}
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
          <WindowCard
            windowId={m.windowId}
            wsId={ws.id}
            wsIndex={index}
            title={m.title}
            appId={m.appId}
            minimized
            iconUrl={iconUrls[m.appId]}
            selected={$selectedWindowIds.has(m.windowId)}
            keyboardFocus={focusedWindowId === m.windowId}
            dragging={drag.dragState?.windowId === m.windowId}
            {drag}
            {workspaces}
            {hideOverlay}
          />
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
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

  /* Inert "+N more windows" tile. Shares the window-card format
     register (60×56, radius-input, the 6% surface tint — the
     register itself lives in WindowCard.svelte) without being an
     interactive card. */
  .overflow-badge {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 60px;
    height: 56px;
    padding: 8px 4px;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--color-fg-shell) 6%, transparent);
    border: 1px solid
      color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    font-size: 11px;
    font-weight: 600;
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
    cursor: default;
  }
</style>
