<script lang="ts">
  /// WYSIWYG preview of the QuickSettings panel.
  ///
  /// 380px frame matching the real shell panel; 2-col tile grid
  /// of `PreviewTile`s. Pinned tiles render below the grid in a
  /// non-draggable footer slot — matches the shell's user-row
  /// behaviour (always at the bottom, never customisable).
  ///
  /// Drag-and-drop uses pointer events (pointerdown/pointermove/
  /// pointerup) rather than the HTML5 drag API. WebKitGTK (Tauri's
  /// embedded webview) freezes when HTML5 drag is combined with
  /// custom ghost rendering — the WorkspaceIndicator's
  /// 2026-04-19 debug session captured this and the same finding
  /// applies here. Pointer events give us full control:
  ///   pointerdown  → capture pointer, stash start position
  ///   pointermove  → once moved past 5px, mark as dragging;
  ///                  hit-test the cell under the cursor and
  ///                  highlight as drop-target
  ///   pointerup    → if dragging, fire onReorder with target
  ///                  cell index; if not, no-op (right-click
  ///                  goes through ContextMenu's oncontextmenu)
  ///   pointercancel→ cleanup (browser/OS abort)
  import PreviewTile from "./PreviewTile.svelte";

  type WireSize = "one_by_one" | "two_by_one" | "two_by_two";

  interface VisibleTile {
    id: string;
    label: string;
    iconName: string;
    size: WireSize;
    allowedSizes: WireSize[];
    bodyVariant: "status" | "slider" | "chart" | "footer";
    pinned?: boolean;
  }

  let {
    tiles,
    onReorder,
    onResize,
    onHide,
    readOnly = false,
  }: {
    tiles: VisibleTile[];
    onReorder: (fromId: string, toIndex: number) => void;
    onResize: (id: string, size: WireSize) => void;
    onHide: (id: string) => void;
    readOnly?: boolean;
  } = $props();

  const draggableTiles = $derived(tiles.filter((t) => !t.pinned));
  const pinnedTiles = $derived(tiles.filter((t) => t.pinned));

  const DRAG_THRESHOLD_PX = 5;

  let dragState = $state<{
    pointerId: number;
    sourceId: string;
    startX: number;
    startY: number;
    started: boolean;
  } | null>(null);
  let dragOverIndex = $state<number | null>(null);

  function handlePointerDown(e: PointerEvent, id: string) {
    if (readOnly) return;
    if (e.button !== 0) return; // only left-button initiates drag
    e.preventDefault();
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
    dragState = {
      pointerId: e.pointerId,
      sourceId: id,
      startX: e.clientX,
      startY: e.clientY,
      started: false,
    };
  }

  function handlePointerMove(e: PointerEvent) {
    if (!dragState || e.pointerId !== dragState.pointerId) return;
    const dx = e.clientX - dragState.startX;
    const dy = e.clientY - dragState.startY;
    const dist = Math.hypot(dx, dy);
    if (!dragState.started) {
      if (dist < DRAG_THRESHOLD_PX) return;
      dragState.started = true;
    }
    // Hit-test which cell the pointer is over.
    const hit = document.elementFromPoint(e.clientX, e.clientY);
    if (!hit) {
      dragOverIndex = null;
      return;
    }
    const cell = (hit as HTMLElement).closest<HTMLElement>(
      "[data-cell-index]",
    );
    if (cell) {
      const idx = parseInt(cell.getAttribute("data-cell-index") ?? "");
      dragOverIndex = isNaN(idx) ? null : idx;
    } else {
      dragOverIndex = null;
    }
  }

  function handlePointerUp(e: PointerEvent) {
    if (!dragState || e.pointerId !== dragState.pointerId) return;
    if (dragState.started && dragOverIndex !== null) {
      onReorder(dragState.sourceId, dragOverIndex);
    }
    dragState = null;
    dragOverIndex = null;
  }

  function handlePointerCancel(e: PointerEvent) {
    if (!dragState || e.pointerId !== dragState.pointerId) return;
    dragState = null;
    dragOverIndex = null;
  }
</script>

<svelte:window
  onpointermove={handlePointerMove}
  onpointerup={handlePointerUp}
  onpointercancel={handlePointerCancel}
/>

<div class="qs-preview-frame">
  <div class="qs-preview-grid">
    {#each draggableTiles as tile, index (tile.id)}
      <div
        class="qs-preview-cell"
        class:drop-target={dragOverIndex === index &&
          dragState?.sourceId !== tile.id}
        data-cell-index={index}
      >
        <PreviewTile
          id={tile.id}
          label={tile.label}
          iconName={tile.iconName}
          size={tile.size}
          allowedSizes={tile.allowedSizes}
          bodyVariant={tile.bodyVariant}
          dragging={dragState?.sourceId === tile.id && dragState.started}
          {readOnly}
          onPointerDown={handlePointerDown}
          {onResize}
          {onHide}
        />
      </div>
    {/each}
    <!-- Trailing drop-zone so users can drop "at end" without
         needing to overlap the last tile. -->
    <div
      class="qs-preview-tail"
      class:drop-target={dragOverIndex === draggableTiles.length}
      data-cell-index={draggableTiles.length}
    ></div>
  </div>

  {#if pinnedTiles.length > 0}
    <div class="qs-preview-pinned">
      {#each pinnedTiles as tile (tile.id)}
        <PreviewTile
          id={tile.id}
          label={tile.label}
          iconName={tile.iconName}
          size={tile.size}
          allowedSizes={tile.allowedSizes}
          bodyVariant={tile.bodyVariant}
          {readOnly}
          pinned
        />
      {/each}
    </div>
  {/if}
</div>

<style>
  .qs-preview-frame {
    width: 380px;
    max-width: 100%;
    padding: 12px;
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 16%, transparent);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-fg-shell) 3%, transparent);
    box-shadow: var(--shadow-md);
  }
  .qs-preview-grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 8px;
    min-height: 80px;
  }
  .qs-preview-cell {
    display: flex;
    min-width: 0;
  }
  .qs-preview-cell > :global(*) {
    width: 100%;
  }
  .qs-preview-cell:has(> :global(.size-2x1)) {
    grid-column: span 2;
  }
  .qs-preview-cell:has(> :global(.size-2x2)) {
    grid-column: span 2;
    grid-row: span 2;
  }

  .qs-preview-cell.drop-target,
  .qs-preview-tail.drop-target {
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--color-accent) 50%, transparent);
    border-radius: var(--radius-card);
  }
  .qs-preview-tail {
    grid-column: span 2;
    height: 24px;
  }

  .qs-preview-pinned {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 0;
  }
  .qs-preview-pinned > :global(*) {
    width: 100%;
  }
</style>
