<script lang="ts">
  /// A generic drag-to-reorder list. Each item renders through the `item`
  /// snippet and is reordered by dragging its handle (any element the snippet
  /// marks `data-sortable-handle`). On drop, `onReorder` receives the new id
  /// order.
  ///
  /// Pointer-event based on purpose, NOT the HTML5 drag API: the HTML5 drag API
  /// (dragstart/dragover + setDragImage) froze WebKitGTK - Arlen's webview - when
  /// combined with a custom ghost. So this captures the pointer, trails a cloned
  /// `pointer-events:none` ghost on `document.body`, and hit-tests with
  /// `elementFromPoint` over `data-sortable-id` (rAF-throttled). The list reflows
  /// live as the pointer crosses rows; the commit is on pointerup.
  import type { Snippet } from "svelte";

  let {
    ids,
    item,
    onReorder,
    disabled = false,
  }: {
    /// The item ids, in display order.
    ids: string[];
    /// Renders one item, given its id. Mark the drag affordance inside it with
    /// `data-sortable-handle`.
    item: Snippet<[string]>;
    /// The new id order, on drop (only when it actually changed).
    onReorder: (ids: string[]) => void;
    /// Suppress dragging (e.g. while a row is mid-edit).
    disabled?: boolean;
  } = $props();

  // The working order: mirrors `ids` when idle (synced by the effect, which runs
  // on mount since dragId starts null), reflows live during a drag.
  let order = $state<string[]>([]);
  let dragId = $state<string | null>(null);
  $effect(() => {
    if (dragId === null) order = [...ids];
  });

  const THRESHOLD = 5;
  let activeId: string | null = null;
  let sourceRow: HTMLElement | null = null;
  let started = false;
  let startX = 0;
  let startY = 0;
  let lastX = 0;
  let lastY = 0;
  let grabX = 0;
  let grabY = 0;
  let ghost: HTMLElement | null = null;
  let raf = 0;

  function rowIdFromPoint(x: number, y: number): string | null {
    const el = document.elementFromPoint(x, y) as HTMLElement | null;
    return el?.closest<HTMLElement>("[data-sortable-id]")?.dataset.sortableId ?? null;
  }

  function buildGhost() {
    if (!sourceRow) return;
    const rect = sourceRow.getBoundingClientRect();
    grabX = startX - rect.left;
    grabY = startY - rect.top;
    const clone = sourceRow.cloneNode(true) as HTMLElement;
    clone.style.cssText = [
      "position:fixed",
      "top:0",
      "left:0",
      `width:${rect.width}px`,
      `height:${rect.height}px`,
      "margin:0",
      "pointer-events:none",
      "z-index:10001",
      "opacity:0.95",
      "box-shadow:0 12px 32px rgba(0,0,0,.35), 0 4px 8px rgba(0,0,0,.2)",
      "transition:none",
      "cursor:grabbing",
    ].join(";");
    document.body.appendChild(clone);
    ghost = clone;
    dragId = activeId;
  }

  function positionGhost() {
    if (ghost) ghost.style.transform = `translate3d(${lastX - grabX}px, ${lastY - grabY}px, 0)`;
  }

  function reorderTo(overId: string | null) {
    if (!overId || overId === activeId || activeId === null) return;
    const from = order.indexOf(activeId);
    const to = order.indexOf(overId);
    if (from < 0 || to < 0) return;
    const next = [...order];
    next.splice(from, 1);
    next.splice(to, 0, activeId);
    order = next;
  }

  function onPointerDown(event: PointerEvent) {
    if (disabled || event.button !== 0) return;
    const target = event.target as HTMLElement;
    if (!target.closest("[data-sortable-handle]")) return;
    const row = target.closest<HTMLElement>("[data-sortable-id]");
    if (!row) return;
    activeId = row.dataset.sortableId ?? null;
    sourceRow = row;
    started = false;
    startX = lastX = event.clientX;
    startY = lastY = event.clientY;
    target.setPointerCapture?.(event.pointerId);
    event.preventDefault();
  }

  function onPointerMove(event: PointerEvent) {
    if (activeId === null) return;
    lastX = event.clientX;
    lastY = event.clientY;
    if (!started) {
      if (Math.hypot(lastX - startX, lastY - startY) < THRESHOLD) return;
      started = true;
      buildGhost();
    }
    positionGhost();
    if (raf) return;
    raf = requestAnimationFrame(() => {
      raf = 0;
      reorderTo(rowIdFromPoint(lastX, lastY));
    });
  }

  function finish(commit: boolean) {
    if (raf) {
      cancelAnimationFrame(raf);
      raf = 0;
    }
    if (ghost) {
      ghost.remove();
      ghost = null;
    }
    if (commit && started && order.some((id, i) => id !== ids[i])) {
      onReorder([...order]);
    }
    activeId = null;
    sourceRow = null;
    started = false;
    dragId = null;
  }
</script>

<svelte:window
  onpointermove={onPointerMove}
  onpointerup={() => finish(true)}
  onpointercancel={() => finish(false)}
/>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="sortable" onpointerdown={onPointerDown}>
  {#each order as id (id)}
    <div class="sortable-row" data-sortable-id={id} class:dragging={dragId === id}>
      {@render item(id)}
    </div>
  {/each}
</div>

<style>
  .sortable {
    display: flex;
    flex-direction: column;
  }
  /* The source row stays in place as a quiet gap while its ghost is carried. */
  .sortable-row.dragging {
    opacity: 0.35;
  }
</style>
