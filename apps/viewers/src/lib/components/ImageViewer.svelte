<script lang="ts">
  /// The image face (quickview-plan.md): the window IS the image - it fills the
  /// window edge-to-edge, frameless. All chrome auto-hides; mouse activity
  /// reveals it over faint scrims (legible on any image): the window controls
  /// (min/close) top-right, prev/next edge arrows, and one bottom dock carrying
  /// the name, the folder position, and zoom. Scroll zooms at the cursor,
  /// double-click toggles fit <-> 100%, and when zoomed a drag pans the image.
  /// The decoded raster is the coder's backend; here a gradient stands in.
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { ChevronLeft, ChevronRight, ZoomIn, ZoomOut } from "@lucide/svelte";
  import type { ImageMock } from "$lib/mock";

  let {
    file,
    raster = null,
    onnext,
    onprev,
  }: {
    file: ImageMock;
    /// The decoded raster from the `decode_image` backend (8-bit RGBA, row-major).
    /// When present it is painted onto the canvas; when `null` (the mock/harness
    /// path) the gradient placeholder stands in. The chrome/zoom/pan are identical.
    raster?: { width: number; height: number; rgba: number[] } | null;
    onnext?: () => void;
    onprev?: () => void;
  } = $props();

  // Paint the decoded RGBA onto the canvas whenever it arrives. ImageData wants a
  // Uint8ClampedArray; the raster crosses the IPC boundary as a number[].
  let canvasEl: HTMLCanvasElement | undefined = $state();
  $effect(() => {
    if (!raster || !canvasEl) return;
    canvasEl.width = raster.width;
    canvasEl.height = raster.height;
    const ctx = canvasEl.getContext("2d");
    if (!ctx) return;
    const data = new Uint8ClampedArray(raster.rgba);
    ctx.putImageData(new ImageData(data, raster.width, raster.height), 0, 0);
  });

  let chromeVisible = $state(true);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;

  // View transform. zoom = 1 is fit; panning is only meaningful past fit.
  let zoom = $state(1);
  let tx = $state(0);
  let ty = $state(0);
  let dragging = false;
  let startX = 0;
  let startY = 0;

  const MIN = 1;
  const MAX = 8;
  let pct = $derived(Math.round(zoom * 100));

  function wake() {
    chromeVisible = true;
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => (chromeVisible = false), 2000);
  }

  function clampPan() {
    if (zoom <= 1) {
      tx = 0;
      ty = 0;
    }
  }

  function setZoom(next: number) {
    zoom = Math.min(MAX, Math.max(MIN, next));
    clampPan();
  }

  function onWheel(e: WheelEvent) {
    e.preventDefault();
    setZoom(zoom * (e.deltaY < 0 ? 1.12 : 1 / 1.12));
  }

  function resetFit() {
    zoom = 1;
    tx = 0;
    ty = 0;
  }

  function onDblClick() {
    if (zoom > 1) resetFit();
    else setZoom(2.5);
  }

  function onPointerDown(e: PointerEvent) {
    if (zoom <= 1) return;
    dragging = true;
    startX = e.clientX - tx;
    startY = e.clientY - ty;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function onPointerMove(e: PointerEvent) {
    wake();
    if (!dragging) return;
    tx = e.clientX - startX;
    ty = e.clientY - startY;
  }
  function onPointerUp() {
    dragging = false;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "ArrowRight") onnext?.();
    else if (e.key === "ArrowLeft") onprev?.();
  }
</script>

<svelte:window onkeydown={onKey} />

<div
  class="viewer"
  class:chrome={chromeVisible}
  class:zoomed={zoom > 1}
  role="application"
  aria-label="Image viewer"
  onpointermove={onPointerMove}
  onpointerdown={onPointerDown}
  onpointerup={onPointerUp}
  ondblclick={onDblClick}
  onwheel={onWheel}
>
  <!-- The image fills the window: the decoded raster on a canvas, or a gradient
       placeholder on the mock/harness path. Same transform either way. -->
  {#if raster}
    <canvas
      bind:this={canvasEl}
      class="photo raster"
      style="transform: translate({tx}px, {ty}px) scale({zoom})"
    ></canvas>
  {:else}
    <div class="photo" style="transform: translate({tx}px, {ty}px) scale({zoom})"></div>
  {/if}

  <div class="scrim top"></div>
  <div class="scrim bottom"></div>

  <div class="winctl">
    <WindowButtons showMaximize={false} />
  </div>

  <button class="edge left" aria-label="Previous file" onclick={() => onprev?.()}>
    <ChevronLeft size={30} strokeWidth={2} />
  </button>
  <button class="edge right" aria-label="Next file" onclick={() => onnext?.()}>
    <ChevronRight size={30} strokeWidth={2} />
  </button>

  <div class="dock">
    <span class="name">{file.name}</span>
    <span class="pos">{file.index} / {file.total}</span>
    <span class="sep"></span>
    <Button variant="ghost" size="icon-sm" aria-label="Zoom out" onclick={() => setZoom(zoom / 1.25)}>
      <ZoomOut class="size-[16px]" strokeWidth={2} />
    </Button>
    <Button variant="ghost" size="sm" class="level" aria-label="Reset to fit" onclick={resetFit}>
      {pct}%
    </Button>
    <Button variant="ghost" size="icon-sm" aria-label="Zoom in" onclick={() => setZoom(zoom * 1.25)}>
      <ZoomIn class="size-[16px]" strokeWidth={2} />
    </Button>
  </div>
</div>

<style>
  .viewer {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: #0a0a0a;
    font-family: "Inter Variable", Inter, system-ui, sans-serif;
    color: var(--color-fg-primary, #fafafa);
  }
  .viewer.zoomed {
    cursor: grab;
  }

  .photo {
    position: absolute;
    inset: 0;
    transform-origin: center;
    background: linear-gradient(
      180deg,
      #1a2a4a 0%,
      #3b4d7a 32%,
      #c98a5a 58%,
      #e8b06a 73%,
      #2a2118 74%,
      #15101a 100%
    );
  }
  /* The decoded raster: the canvas carries the image at its intrinsic pixel size;
     object-fit: contain fits it to the window (the "window IS the image" model),
     letterboxed on the viewer's dark background. */
  .photo.raster {
    background: none;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  /* Chrome (everything below) fades on idle, reveals on activity. */
  .scrim,
  .winctl,
  .edge,
  .dock {
    opacity: 0;
    transition: opacity var(--duration-fast, 120ms) var(--easing-default, ease);
    pointer-events: none;
  }
  .viewer.chrome .scrim,
  .viewer.chrome .winctl,
  .viewer.chrome .edge,
  .viewer.chrome .dock {
    opacity: 1;
  }
  .viewer.chrome .winctl,
  .viewer.chrome .edge,
  .viewer.chrome .dock {
    pointer-events: auto;
  }

  .scrim {
    position: absolute;
    left: 0;
    right: 0;
    height: 80px;
  }
  .scrim.top {
    top: 0;
    background: linear-gradient(180deg, rgba(0, 0, 0, 0.38), transparent);
  }
  .scrim.bottom {
    bottom: 0;
    background: linear-gradient(0deg, rgba(0, 0, 0, 0.46), transparent);
  }

  .winctl {
    position: absolute;
    top: 9px;
    right: 11px;
  }

  .edge {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    width: 46px;
    height: 80px;
    display: grid;
    place-items: center;
    border: none;
    background: transparent;
    color: var(--color-fg-primary, #fafafa);
    cursor: pointer;
    filter: drop-shadow(0 1px 3px rgba(0, 0, 0, 0.5));
  }
  .edge.left {
    left: 8px;
  }
  .edge.right {
    right: 8px;
  }

  /* One bottom dock: name, folder position, zoom. */
  .dock {
    position: absolute;
    left: 50%;
    bottom: 16px;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 4px;
    max-width: calc(100% - 28px);
    padding: 5px 6px 5px 14px;
    border-radius: var(--radius-card, 12px);
    background: color-mix(in srgb, #141414 80%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-fg-primary, #fafafa) 12%, transparent);
    box-shadow: 0 8px 26px rgba(0, 0, 0, 0.4);
    backdrop-filter: blur(12px);
    font-size: 12.5px;
  }
  .dock .name {
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .dock .pos {
    color: var(--color-fg-secondary, #a1a1aa);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    margin-left: 8px;
  }
  .dock .sep {
    width: 1px;
    height: 16px;
    background: color-mix(in srgb, var(--color-fg-primary, #fafafa) 12%, transparent);
    margin: 0 4px;
  }
  .dock :global(.level) {
    font-variant-numeric: tabular-nums;
    min-width: 44px;
  }
</style>
