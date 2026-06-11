<script lang="ts">
  /// The horizontal workspace overview overlay (spec §2.2–2.4): one
  /// column per workspace on this output, hanging below the topbar
  /// indicator. Visibility, hover lifecycle and the engines live in
  /// the host (WorkspaceIndicator); this component owns the frame
  /// and the column loop.

  import { windows } from "$lib/stores/windows.js";
  import type { WindowInfo } from "$lib/stores/windows.js";
  import type { WorkspaceInfo } from "$lib/stores/workspaces.js";
  import type { DragEngine } from "$lib/workspace/drag.svelte.js";
  import type { KeyboardNav } from "$lib/workspace/keyboard.svelte.js";
  import WorkspaceColumn from "./WorkspaceColumn.svelte";

  let {
    el = $bindable(null),
    visible,
    workspaces,
    drag,
    kb,
    iconUrls,
    onOverlayEnter,
    onCardMenuOpenChange,
    hideOverlay,
    closeOverlayAndCollapse,
  }: {
    /// Bound back to the host so the compositor-event path can
    /// `.focus()` the overlay for keyboard input.
    el?: HTMLDivElement | null;
    visible: boolean;
    workspaces: WorkspaceInfo[];
    drag: DragEngine;
    kb: KeyboardNav;
    iconUrls: Record<string, string | null>;
    onOverlayEnter: () => void;
    onCardMenuOpenChange: (open: boolean) => void;
    hideOverlay: () => void;
    closeOverlayAndCollapse: () => void;
  } = $props();

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
</script>

<!-- No onmouseleave — closing is handled exclusively by the host's
     `.ws-root` mouseleave; see the comment on `onOverlayEnter` in
     WorkspaceIndicator. `tabindex="-1"` lets the host
     programmatically focus the div from the compositor-event path
     so the document-level keydown handler actually fires. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  bind:this={el}
  class="overlay"
  class:overlay-visible={visible}
  role="dialog"
  aria-label="Workspace overview"
  aria-modal="false"
  tabindex="-1"
  onmouseenter={onOverlayEnter}
>
  <div class="ws-columns">
    {#each workspaces as ws, i (ws.id)}
      <WorkspaceColumn
        {ws}
        index={i}
        windows={windowsByWorkspace.get(ws.id) ?? []}
        {workspaces}
        {drag}
        focusedWindowId={kb.focusedWindowId}
        {iconUrls}
        {onCardMenuOpenChange}
        {hideOverlay}
        {closeOverlayAndCollapse}
      />
    {/each}
  </div>
</div>

<style>
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
</style>
