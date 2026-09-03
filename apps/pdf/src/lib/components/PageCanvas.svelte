<script lang="ts">
  /// One page slot in the continuous flow. Renders lazily: a placeholder in
  /// the document's page ratio until the slot nears the viewport, then the
  /// worker's raster plus the transparent text layer over it. On a machine
  /// with nothing to draw with - every machine today - the slot is a TEXT
  /// SHEET instead: the page's words on paper at the page's own width, so
  /// zoom, fit, jumps and search work on it the way they work on a picture.
  /// A page the renderer had and would not draw says so WHERE THE PAGE WOULD
  /// BE, with its words below, because a blank frame cannot tell a reader
  /// whether the page or the renderer is blank.
  import { t } from "$lib/i18n/messages";
  import { fetchPage, type PageState } from "$lib/stores/pdf";

  let {
    page,
    scale,
    ratio,
    ptWidth,
    query,
    onmetrics,
  }: {
    page: number;
    /// The render scale; a change re-fetches once the slot is visible.
    scale: number;
    /// height/width of a page, from the first render (placeholder sizing).
    ratio: number;
    /// The page's width in PDF points; the text sheet takes it as its own.
    ptWidth: number;
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
  // A text sheet is scale-free (the words are the same at every zoom), so it
  // is fetched once and never again.
  $effect(() => {
    const want = scale;
    if (!visible || fetching) return;
    if (stateNow && (stateNow.failure === "no-renderer" || (stateNow.scale === want && !stateNow.failure))) return;
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

  /// The words cut at every occurrence of the query, so the sheet can mark
  /// each one: the text face can be exact where the raster can only wash a
  /// line.
  const pieces = $derived.by(() => {
    const words = stateNow?.words ?? "";
    if (!q) return [{ text: words, hit: false }];
    const out: { text: string; hit: boolean }[] = [];
    const lower = words.toLowerCase();
    let at = 0;
    for (;;) {
      const i = lower.indexOf(q, at);
      if (i < 0) break;
      if (i > at) out.push({ text: words.slice(at, i), hit: false });
      out.push({ text: words.slice(i, i + q.length), hit: true });
      at = i + q.length;
    }
    if (at < words.length) out.push({ text: words.slice(at), hit: false });
    return out;
  });
</script>

<div class="slot" bind:this={el} data-page={page}>
  {#if stateNow?.failure && (stateNow.failure === "no-renderer" || stateNow.words.trim())}
    <!-- The sheet is a page: the page's own width at the current scale, its
         margins scaled with it, the type at a book's size scaled with it, so
         the zoom cluster means the same thing here as on a picture. It stands
         in for ANY page without a picture; when the renderer was there and
         would not draw this one, the sheet says so on itself, once. -->
    <div
      class="sheet text-sheet"
      data-selectable
      style="width: {ptWidth * scale}px; padding: {50 * scale}px {56 * scale}px; font-size: {11.5 * scale}px"
    >
      <span class="page-mark" style="top: {22 * scale}px; right: {56 * scale}px; font-size: {8.5 * scale}px"
        >{$t("pdf.page", { number: page })}</span
      >
      {#if stateNow.failure !== "no-renderer"}
        <p class="sheet-note">
          {stateNow.failure === "lock-lost" ? $t("pdf.pageLockLost") : $t("pdf.pageTextInstead")}
        </p>
      {/if}
      {#if stateNow.words.trim()}
        <p class="text">{#each pieces as piece, i (i)}{#if piece.hit}<mark>{piece.text}</mark>{:else}{piece.text}{/if}{/each}</p>
      {:else}
        <p class="no-text">{$t("pdf.pageNoText")}</p>
      {/if}
    </div>
  {:else if stateNow?.failure}
    <!-- No picture and no words either: the sentence is all there is. -->
    <div class="fallback">
      <p class="quiet">
        {#if stateNow.failure === "lock-lost"}{$t("pdf.pageLockLost")}{:else}{$t("pdf.pageFailed")}{/if}
      </p>
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

  /* Paper, not a card: the same ground a drawn page has, a reading face on
     it, and no minimum height - a sheet is as long as its words. */
  .text-sheet {
    box-sizing: border-box;
    max-width: 100%;
    background: #fbfaf7;
    color: #1c1b19;
    font-family: Georgia, "Noto Serif", "Liberation Serif", serif;
    line-height: 1.55;
    cursor: text;
  }
  .page-mark {
    position: absolute;
    color: #1c1b19;
    opacity: 0.4;
    font-family: var(--font-sans, system-ui, sans-serif);
    letter-spacing: 0.04em;
    font-variant-numeric: tabular-nums;
  }
  .text {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  /* A highlighter on paper: the accent is the shell's ink and vanishes on a
     cream sheet, so the mark is the one colour a page has always worn. */
  .text mark {
    background: rgb(255 214 10 / 0.45);
    color: inherit;
    border-radius: 2px;
  }
  .text-sheet ::selection {
    background: rgb(90 130 255 / 0.3);
  }
  .no-text,
  .sheet-note {
    margin: 0;
    opacity: 0.5;
    font-family: var(--font-sans, system-ui, sans-serif);
    font-size: 0.9em;
  }
  .sheet-note {
    margin-bottom: 1.4em;
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
</style>
