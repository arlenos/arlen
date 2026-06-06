<script lang="ts">
  /// Visual layout map for the Display panel.
  ///
  /// Renders one `<button>` per monitor at its current position,
  /// scaled to fit the available canvas. Drag-and-drop updates the
  /// position in the bound `drafts` store; snap-to-edges aligns
  /// boxes that are within 32 px (canvas-space) of each other.
  ///
  /// The map intentionally does *not* talk to the wayland thread
  /// directly; the parent page collects the drafts, calls
  /// `applyConfig`, and re-renders from the live `monitors` store
  /// once the compositor confirms.

  import type { Monitor, MonitorConfig, Position } from "$lib/stores/displays";
  import { monitors as monitorsStore } from "$lib/stores/displays";
  import { get } from "svelte/store";
  import { Star } from "lucide-svelte";

  interface Props {
    /** Per-connector draft config the panel is currently editing. */
    drafts: Record<string, MonitorConfig>;
    selected: string | null;
    onSelect: (connector: string) => void;
    onPositionChange: (connector: string, pos: Position) => void;
  }

  let { drafts, selected, onSelect, onPositionChange }: Props = $props();

  // The monitors writable store is read via `$monitorsStore` ONLY in
  // the template below — neither `$derived($monitorsStore)` nor
  // `$effect` observe Tauri-IPC-driven `writable.set` updates
  // (Svelte 5 scheduler limitation, CLAUDE.md). Template
  // auto-subscribe is the only reliable read path. Event handlers
  // pull live state via `get(monitorsStore)`.

  // Snap distance in canvas-space pixels.
  const SNAP_PX = 32;

  let canvasEl = $state<HTMLDivElement | null>(null);
  let canvasW = $state(800);
  let canvasH = $state(360);

  // Resize-Observer keeps the canvas dimensions reactive without
  // having to layout-thrash on every parent prop change.
  $effect(() => {
    if (!canvasEl) return;
    const ro = new ResizeObserver((entries) => {
      const r = entries[0]?.contentRect;
      if (r) {
        canvasW = r.width;
        canvasH = r.height;
      }
    });
    ro.observe(canvasEl);
    return () => ro.disconnect();
  });

  function computeLayout(
    mons: Monitor[],
    drs: Record<string, MonitorConfig>,
    cw: number,
    ch: number,
  ): { scale: number; offsetX: number; offsetY: number } {
    if (mons.length === 0) return { scale: 1, offsetX: cw / 2, offsetY: ch / 2 };
    let minX = Infinity,
      minY = Infinity,
      maxX = -Infinity,
      maxY = -Infinity;
    for (const m of mons) {
      const draft = drs[m.connector];
      const pos = draft?.position ?? m.position;
      const mode = m.modes[m.currentMode ?? 0];
      const w = mode?.width ?? 1920;
      const h = mode?.height ?? 1080;
      // The compositor's "logical" size is post-scale; the layout
      // map mirrors that, so a 4K @ 2x reads as 1920 × 1080 here.
      const scale = draft?.scale ?? m.scale;
      const lw = w / scale;
      const lh = h / scale;
      minX = Math.min(minX, pos.x);
      minY = Math.min(minY, pos.y);
      maxX = Math.max(maxX, pos.x + lw);
      maxY = Math.max(maxY, pos.y + lh);
    }
    const margin = 32;
    const totalW = maxX - minX;
    const totalH = maxY - minY;
    const scale = Math.min(
      (cw - 2 * margin) / Math.max(totalW, 1),
      (ch - 2 * margin) / Math.max(totalH, 1),
      0.2, // never blow up tiny setups beyond 20 % real-pixel size
    );
    const offsetX = (cw - totalW * scale) / 2 - minX * scale;
    const offsetY = (ch - totalH * scale) / 2 - minY * scale;
    return { scale, offsetX, offsetY };
  }

  function boxRect(
    m: Monitor,
    layout: { scale: number; offsetX: number; offsetY: number },
  ) {
    const draft = drafts[m.connector];
    const pos = draft?.position ?? m.position;
    const mode = m.modes[m.currentMode ?? 0];
    const cfgScale = draft?.scale ?? m.scale;
    const w = (mode?.width ?? 1920) / cfgScale;
    const h = (mode?.height ?? 1080) / cfgScale;
    return {
      left: pos.x * layout.scale + layout.offsetX,
      top: pos.y * layout.scale + layout.offsetY,
      width: w * layout.scale,
      height: h * layout.scale,
    };
  }

  // ── Drag state ──────────────────────────────────────────────
  let dragging = $state<string | null>(null);
  let dragStart = $state({ pointerX: 0, pointerY: 0, monX: 0, monY: 0 });
  // rAF coalescing: pointermove fires at ~120Hz on modern devices,
  // and each call recomputes the layout + snap candidates + spreads
  // a fresh `drafts` object. Coalesce to one update per animation
  // frame so the drag stays smooth instead of jittering at the
  // browser's repaint cadence.
  let pendingFrame: number | null = null;
  let pendingPointer: { clientX: number; clientY: number } | null = null;

  function onPointerDown(e: PointerEvent, m: Monitor) {
    if (e.button !== 0) return;
    onSelect(m.connector);
    const draft = drafts[m.connector];
    const pos = draft?.position ?? m.position;
    dragging = m.connector;
    dragStart = {
      pointerX: e.clientX,
      pointerY: e.clientY,
      monX: pos.x,
      monY: pos.y,
    };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function onPointerMove(e: PointerEvent) {
    if (!dragging) return;
    // Stash the latest pointer coords; the rAF-coalesced flush picks
    // them up so we do at most one snap+layout calculation per frame.
    pendingPointer = { clientX: e.clientX, clientY: e.clientY };
    if (pendingFrame !== null) return;
    pendingFrame = requestAnimationFrame(flushDrag);
  }

  function flushDrag() {
    pendingFrame = null;
    if (!dragging || !pendingPointer) return;
    const { clientX, clientY } = pendingPointer;
    pendingPointer = null;

    // Read the live store value imperatively. We can't close over a
    // reactive `monitors` here because Svelte 5 effects don't track
    // Tauri-IPC-driven store updates.
    const monitors = get(monitorsStore);
    const layout = computeLayout(monitors, drafts, canvasW, canvasH);

    // Convert pointer delta back into compositor coordinates.
    const dx = (clientX - dragStart.pointerX) / layout.scale;
    const dy = (clientY - dragStart.pointerY) / layout.scale;
    let newX = dragStart.monX + dx;
    let newY = dragStart.monY + dy;

    // Snap to other monitors' edges. Compare the dragged monitor's
    // four edges against every other monitor's four edges in
    // compositor-space (not screen-space).
    const me = monitors.find((m) => m.connector === dragging);
    if (me) {
      const meRect = monitorRect(me, drafts[me.connector], { x: newX, y: newY });
      for (const other of monitors) {
        if (other.connector === me.connector) continue;
        const o = monitorRect(other, drafts[other.connector], null);
        const candidates: { axis: "x" | "y"; from: number; to: number }[] = [
          { axis: "x", from: meRect.right, to: o.left },
          { axis: "x", from: meRect.left, to: o.right },
          { axis: "x", from: meRect.left, to: o.left },
          { axis: "x", from: meRect.right, to: o.right },
          { axis: "y", from: meRect.bottom, to: o.top },
          { axis: "y", from: meRect.top, to: o.bottom },
          { axis: "y", from: meRect.top, to: o.top },
          { axis: "y", from: meRect.bottom, to: o.bottom },
        ];
        for (const c of candidates) {
          if (Math.abs(c.from - c.to) < SNAP_PX / layout.scale) {
            if (c.axis === "x") newX += c.to - c.from;
            else newY += c.to - c.from;
          }
        }
      }
    }

    onPositionChange(dragging, { x: Math.round(newX), y: Math.round(newY) });
  }

  function onPointerUp(e: PointerEvent) {
    if (dragging && e.currentTarget) {
      try {
        (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
      } catch {}
    }
    if (pendingFrame !== null) {
      cancelAnimationFrame(pendingFrame);
      pendingFrame = null;
    }
    pendingPointer = null;
    dragging = null;
  }

  function monitorRect(
    m: Monitor,
    draft: MonitorConfig | undefined,
    pos: Position | null,
  ): { left: number; right: number; top: number; bottom: number } {
    const p = pos ?? draft?.position ?? m.position;
    const mode = m.modes[m.currentMode ?? 0];
    const cfgScale = draft?.scale ?? m.scale;
    const w = (mode?.width ?? 1920) / cfgScale;
    const h = (mode?.height ?? 1080) / cfgScale;
    return { left: p.x, right: p.x + w, top: p.y, bottom: p.y + h };
  }
</script>

<div
  bind:this={canvasEl}
  class="canvas"
  role="region"
  aria-label="Monitor arrangement"
>
  {#if $monitorsStore.length === 0}
    <div class="empty">
      No displays detected. Are you running under a Wayland compositor
      that implements <code>wlr-output-management</code>?
    </div>
  {:else}
    {@const layout = computeLayout($monitorsStore, drafts, canvasW, canvasH)}
    {#each $monitorsStore as m (m.connector)}
      {@const r = boxRect(m, layout)}
      {@const isSel = selected === m.connector}
      <button
        type="button"
        class="box"
        class:selected={isSel}
        class:disabled={!m.enabled}
        style="left: {r.left}px; top: {r.top}px; width: {r.width}px; height: {r.height}px;"
        onpointerdown={(e) => onPointerDown(e, m)}
        onpointermove={onPointerMove}
        onpointerup={onPointerUp}
        onpointercancel={onPointerUp}
      >
        <div class="label">
          <span class="name">{m.connector}</span>
          {#if m.primary}
            <Star size={12} strokeWidth={2.5} />
          {/if}
        </div>
        <div class="model">
          {m.make} {m.model}
        </div>
      </button>
    {/each}
  {/if}
</div>

<style>
  .canvas {
    position: relative;
    width: 100%;
    height: 360px;
    background: color-mix(in srgb, var(--color-fg-app) 5%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-fg-app) 10%, transparent);
    border-radius: var(--radius-input);
    overflow: hidden;
    user-select: none;
  }

  .empty {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
    text-align: center;
    color: color-mix(in srgb, var(--color-fg-app) 60%, transparent);
    font-size: 0.875rem;
    line-height: 1.4;
  }

  .empty code {
    background: color-mix(in srgb, var(--color-fg-app) 10%, transparent);
    padding: 1px 6px;
    border-radius: 4px;
    font-family: ui-monospace, monospace;
    font-size: 0.8em;
  }

  .box {
    position: absolute;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    padding: 8px;
    background: var(--color-bg-card);
    border: 2px solid color-mix(in srgb, var(--color-fg-app) 15%, transparent);
    border-radius: var(--radius-input);
    color: var(--color-fg-app);
    cursor: grab;
    transition:
      border-color 100ms ease,
      box-shadow 100ms ease;
    text-align: center;
    font: inherit;
  }

  .box:active {
    cursor: grabbing;
  }

  .box.selected {
    border-color: var(--color-accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--color-accent) 30%, transparent);
  }

  .box.disabled {
    opacity: 0.4;
  }

  .label {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 0.85rem;
    font-weight: 500;
  }

  .model {
    font-size: 0.7rem;
    color: color-mix(in srgb, var(--color-fg-app) 55%, transparent);
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
