<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// The image face (quickview-plan.md): the window IS the image - it fills the
  /// window edge-to-edge, frameless. All chrome auto-hides; mouse activity
  /// reveals it over faint scrims (legible on any image): the window controls
  /// (min/close) top-right, prev/next edge arrows, and one bottom dock carrying
  /// the name, the folder position, and zoom. Scroll zooms at the cursor,
  /// double-click toggles fit <-> 2.5x, and when zoomed a drag pans the image.
  ///
  /// It said "fit <-> 100%" until 16 August, which described a viewer this is not:
  /// `zoom = 1` IS fit here, so 100% and fit are the same state and the toggle
  /// would have been a no-op. There is no 1:1-with-the-file mode at all - every
  /// figure in the dock is a multiple of the fitted size.
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
    quarters = 0,
  }: {
    file: ImageMock;
    /// The decoded raster from the `decode_image` backend (8-bit RGBA, row-major).
    /// When present it is painted onto the canvas; when `null` (the mock/harness
    /// path) the gradient placeholder stands in. The chrome/zoom/pan are identical.
    raster?: { width: number; height: number; rgba: number[] } | null;
    onnext?: () => void;
    onprev?: () => void;
    /// Quarter turns applied to the view, 0-3, owned by the window so a new file
    /// arrives upright rather than wearing the last one's rotation.
    ///
    /// VIEW ONLY - nothing is written. A picture stored sideways is still stored
    /// sideways after this, which is the honest scope for an app that has never
    /// written a file: saving a rotation means re-encoding (or lossless jpegtran
    /// for the one format that has it), and that is an edit feature with its own
    /// decisions rather than a side effect of looking.
    quarters?: number;
  } = $props();

  // Paint the decoded RGBA onto the canvas whenever it arrives. ImageData wants a
  // Uint8ClampedArray; the raster crosses the IPC boundary as a number[].
  //
  // The ROTATION IS PAINTED, not applied as a CSS transform on the canvas box. The
  // canvas is fitted by `object-fit: contain`, so a CSS rotate would turn the
  // already-fitted rectangle and push a landscape picture off both sides of the
  // window. Painting into a canvas whose dimensions are swapped for a quarter turn
  // keeps `contain` doing the fitting, and leaves zoom and pan untouched.
  let canvasEl: HTMLCanvasElement | undefined = $state();
  $effect(() => {
    if (!raster || !canvasEl) return;
    const turned = quarters % 2 === 1;
    canvasEl.width = turned ? raster.height : raster.width;
    canvasEl.height = turned ? raster.width : raster.height;
    const ctx = canvasEl.getContext("2d");
    if (!ctx) return;
    const data = new Uint8ClampedArray(raster.rgba);
    const image = new ImageData(data, raster.width, raster.height);
    if (quarters === 0) {
      ctx.putImageData(image, 0, 0);
      return;
    }
    // `putImageData` ignores the context transform by definition, so the pixels go
    // onto an offscreen canvas first and are then drawn through the rotation.
    const off = document.createElement("canvas");
    off.width = raster.width;
    off.height = raster.height;
    const offCtx = off.getContext("2d");
    if (!offCtx) return;
    offCtx.putImageData(image, 0, 0);
    ctx.save();
    ctx.translate(canvasEl.width / 2, canvasEl.height / 2);
    ctx.rotate((quarters * Math.PI) / 2);
    ctx.drawImage(off, -raster.width / 2, -raster.height / 2);
    ctx.restore();
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
  // The dock's zoom face. At fit it says so in a word rather than "100%".
  //
  // Every figure here is a multiple of the FITTED size, not of the file, so a
  // 1280-wide picture in a 960-wide window sits at zoom 1 and used to be labelled
  // "100%" while occupying about three quarters of its own pixels. In every other
  // viewer 100% means one image pixel per screen pixel, so the label was making a
  // claim about scale that was not true - and the button's own accessible name
  // already said "reset to fit", so the two halves of one control disagreed.
  let pct = $derived(Math.round(zoom * 100));
  let zoomFace = $derived(zoom === 1 ? $t("v.fit") : `${pct}%`);

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

  // There is nowhere to go when the folder holds one file, or when it was never
  // read - and `index`/`total` already say which, because the position readout
  // below refuses to print a number it does not have. The arrows were drawn
  // enabled regardless: with `1 / 1` on screen both were live and neither did
  // anything. A control that cannot act says so rather than accepting the click.
  const canPrev = $derived(!!file.total && !!file.index && file.index > 1);
  const canNext = $derived(!!file.total && !!file.index && file.index < file.total);

  function onKey(e: KeyboardEvent) {
    // Same bound as the arrows: a key that silently does nothing is the same
    // defect without the pixels.
    if (e.key === "ArrowRight") {
      if (canNext) onnext?.();
    } else if (e.key === "ArrowLeft") {
      if (canPrev) onprev?.();
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<div
  class="viewer"
  class:chrome={chromeVisible}
  class:zoomed={zoom > 1}
  role="application"
  aria-label={$t("v.imageViewer")}
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

  <button class="edge left" aria-label={$t("v.prevFile")} disabled={!canPrev} onclick={() => onprev?.()}>
    <ChevronLeft size={30} strokeWidth={2} />
  </button>
  <button class="edge right" aria-label={$t("v.nextFile")} disabled={!canNext} onclick={() => onnext?.()}>
    <ChevronRight size={30} strokeWidth={2} />
  </button>

  <div class="dock">
    <span class="name">{file.name}</span>
    <!-- Only when the folder has actually been read. A viewer that always
         prints a position prints a wrong one whenever it does not have one. -->
    {#if file.index && file.total}
      <span class="pos">{file.index} / {file.total}</span>
    {/if}
    <span class="sep"></span>
    <Button variant="ghost" size="icon-sm" aria-label={$t("v.zoomOut")} onclick={() => setZoom(zoom / 1.25)}>
      <ZoomOut class="size-[16px]" strokeWidth={2} />
    </Button>
    <Button variant="ghost" size="sm" class="level" aria-label={$t("v.resetToFit")} onclick={resetFit}>
      {zoomFace}
    </Button>
    <Button variant="ghost" size="icon-sm" aria-label={$t("v.zoomIn")} onclick={() => setZoom(zoom * 1.25)}>
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

  /* A disabled arrow must not look like a live one. `disabled` already stops the
     click, but the chevron kept full contrast, so at `2 / 2` both edges read as
     available and only one did anything - the half-fix that looks like no fix. */
  .viewer.chrome .edge:disabled {
    opacity: 0.25;
    pointer-events: none;
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
    margin-inline-start: 8px;
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
