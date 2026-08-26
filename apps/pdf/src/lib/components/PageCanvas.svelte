<script lang="ts">
  /// One page slot in the continuous flow. Renders lazily: a placeholder in
  /// the document's page ratio until the slot nears the viewport, then the
  /// worker's raster plus the transparent text layer over it. A page that will
  /// not render says so WHERE THE PAGE WOULD BE - its words below the sentence
  /// naming the substitution - because a blank frame cannot tell a reader
  /// whether the page or the renderer is blank.
  import { t } from "$lib/i18n/messages";
  import { fetchPage, type PageState } from "$lib/stores/pdf";

  let {
    page,
    scale,
    ratio,
    query,
    onmetrics,
  }: {
    page: number;
    /// The render scale; a change re-fetches once the slot is visible.
    scale: number;
    /// height/width of a page, from the first render (placeholder sizing).
    ratio: number;
    /// The active search text, for the honest line-level highlight.
    query: string;
    /// The first successful render reports its ratio and the page's width in
    /// PDF points back (width / scale), which the fit modes compute from.
    onmetrics: (ratio: number, ptWidth: number) => void;
  } = $props();

  let el = $state<HTMLElement | null>(null);
  let canvas = $state<HTMLCanvasElement | null>(null);
  let visible = $state(false);
  let stateNow = $state<PageState | null>(null);
  let fetching = false;

  $effect(() => {
    if (!el) return;
    const io = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) if (entry.isIntersecting) visible = true;
      },
      { rootMargin: "600px 0px" },
    );
    io.observe(el);
    return () => io.disconnect();
  });

  // Fetch when visible, and again when the scale changes under a visible slot.
  $effect(() => {
    const want = scale;
    if (!visible || fetching || (stateNow && stateNow.scale === want && !stateNow.failure)) return;
    fetching = true;
    void fetchPage(page, want).then((s) => {
      fetching = false;
      stateNow = s;
      if (s.image) onmetrics(s.image.height / s.image.width, s.image.width / want);
    });
  });

  $effect(() => {
    if (!canvas || !stateNow?.image) return;
    canvas.width = stateNow.image.width;
    canvas.height = stateNow.image.height;
    canvas.getContext("2d")?.putImageData(stateNow.image, 0, 0);
  });

  const q = $derived(query.trim().toLowerCase());
</script>

<div class="slot" bind:this={el} data-page={page}>
  {#if stateNow?.failure}
    <div class="fallback" data-selectable>
      <p class="quiet">
        {#if stateNow.failure === "no-renderer"}{$t("pdf.pageNoRenderer")}
        {:else if stateNow.failure === "lock-lost"}{$t("pdf.pageLockLost")}
        {:else}{$t("pdf.pageFailed")}{/if}
      </p>
      <!-- The words, when the picture cannot be had. Said to be the text and
           not the page, because it has none of the layout: a table comes back
           as its cells in reading order and a two-column article reads
           straight through. -->
      {#if stateNow.words}
        <p class="quiet">{$t("pdf.textInstead")}</p>
        <pre class="words">{stateNow.words}</pre>
      {/if}
    </div>
  {:else if stateNow?.image}
    <div class="sheet">
      <canvas bind:this={canvas} class="page-canvas"></canvas>
      <!-- The words, invisible, laid exactly over the ones in the picture:
           the canvas is pixels, so selection works on this transparent layer.
           `data-selectable` opts back into selection the app shell disables
           globally. A line holding the search text carries a quiet wash - the
           backend gives line boxes, not rects, and an honest line-level mark
           beats none. -->
      <div class="text-layer" data-selectable>
        {#each stateNow.lines as line, i (i)}
          <span
            class:hit={q !== "" && line.text.toLowerCase().includes(q)}
            style="left: {line.x}px; top: {line.y}px; width: {line.width}px; height: {line.height}px; font-size: {line.height * 0.8}px"
            >{line.text}</span
          >
        {/each}
      </div>
    </div>
  {:else}
    <div class="placeholder" style="aspect-ratio: {1 / ratio}" aria-hidden="true"></div>
  {/if}
</div>

<style>
  .slot {
    display: flex;
    justify-content: center;
  }
  .sheet {
    position: relative;
    box-shadow: var(--shadow-lg, 0 12px 32px rgb(0 0 0 / 0.35));
  }
  .page-canvas {
    display: block;
    background: #fff;
  }
  .placeholder {
    width: min(100%, 800px);
    border-radius: 2px;
    background: color-mix(in srgb, #ffffff 6%, transparent);
  }
  .text-layer {
    position: absolute;
    inset: 0;
    overflow: hidden;
  }
  .text-layer span {
    position: absolute;
    color: transparent;
    white-space: pre;
    transform-origin: 0 0;
    cursor: text;
    line-height: 1;
  }
  .text-layer span::selection {
    background: color-mix(in srgb, var(--color-accent) 40%, transparent);
  }
  .text-layer span.hit {
    background: color-mix(in srgb, var(--color-accent, #6366f1) 28%, transparent);
    border-radius: 2px;
  }
  .fallback {
    max-width: 62ch;
    text-align: start;
  }
  .quiet {
    font-size: 12px;
    color: var(--color-fg-secondary, #a3a3a3);
    margin: 8px 0 0;
  }
  .words {
    margin: 8px 0 0;
    padding: 12px 14px;
    border-radius: 6px;
    background: var(--color-bg-card, #171717);
    color: var(--color-fg-primary, #e5e5e5);
    font-size: 13px;
    line-height: 1.6;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
