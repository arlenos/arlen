<script lang="ts">
  /// The reader: what is in this document, and where in it a word appears.
  ///
  /// Every empty-looking state here is a DIFFERENT state and says so. No host is
  /// not no document; a document with no contents page is not a document that
  /// failed to open; and a search that matched nothing is not a search that could
  /// not read half the pages. The last one is why `unsearchable` is carried all
  /// the way from the parser to this screen rather than dropped on the way.
  ///
  /// The page itself is drawn by a separate confined process and put on a canvas
  /// here. A page that will not render says so WHERE THE PAGE WOULD BE: a reader
  /// looking at a blank frame cannot tell a genuinely blank page from a renderer
  /// that refused, and those are different facts about their document.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { FileText, Search } from "@lucide/svelte";
  import { tauriAvailable } from "$lib/tauri";
  import { t } from "$lib/i18n/messages";

  type OutlineEntry = { title: string; depth: number; page: number | null };
  type Hit = { page: number; snippet: string };
  type SearchOutcome = { hits: Hit[]; unsearchable: number[] };
  type DocumentInfo = { path: string; pages: number; outline: OutlineEntry[] };
  type PageImage = { width: number; height: number; rgba: number[] };
  type TextLine = { text: string; x: number; y: number; width: number; height: number };

  let doc = $state<DocumentInfo | null>(null);
  let failure = $state<string | null>(null);
  let query = $state("");
  let results = $state<SearchOutcome | null>(null);
  let current = $state(1);
  let canvas = $state<HTMLCanvasElement | null>(null);
  let pageFailure = $state<string | null>(null);
  /// The page's text, shown only when the page itself could not be drawn.
  let pageWords = $state("");
  let lines = $state<TextLine[]>([]);

  /// The zoom the page and its words are both drawn at.
  ///
  /// One constant for both on purpose: the boxes come back in the raster's own
  /// pixel space, so a page rendered at one scale and a text layer fetched at
  /// another would put every box beside its word instead of on it.
  const SCALE = 1.5;

  /// Draw the current page.
  ///
  /// A page that will not render is reported where the page would be, not
  /// swallowed: a reader looking at a blank frame has no way to tell a scanned
  /// blank page from a renderer that refused.
  async function drawPage() {
    if (!doc || !canvas) return;
    try {
      const img = await invoke<PageImage>("pdf_page_image", { page: current, scale: SCALE });
      canvas.width = img.width;
      canvas.height = img.height;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      const data = new ImageData(new Uint8ClampedArray(img.rgba), img.width, img.height);
      ctx.putImageData(data, 0, 0);
      pageFailure = null;
      pageWords = "";
      // Selectable text is a nicety over a drawn page, so a text layer that
      // will not come back leaves the page shown rather than taking it down
      // with it. A scan has no text and that is not a failure either.
      lines = await invoke<TextLine[]>("pdf_text_layer", { page: current, scale: SCALE })
        .catch(() => []);
    } catch (e) {
      pageFailure = String(e);
      lines = [];
      // The page could not be drawn, so ask for its words instead. This read is
      // `lopdf` in the host and needs no engine, which is the whole point: it is
      // the path that still works on a machine with no rasteriser.
      pageWords = await invoke<string>("pdf_page_text", { page: current }).catch(() => "");
    }
  }

  // Redrawn when the page changes or a document arrives, which is also what a
  // contents-entry click and a search hit do.
  $effect(() => {
    void current;
    void doc;
    void drawPage();
  });

  onMount(() => {
    if (!tauriAvailable) return;
    void (async () => {
      const launched = await invoke<string | null>("launch_file").catch(() => null);
      if (!launched) return;
      try {
        doc = await invoke<DocumentInfo>("pdf_open", { path: launched });
        failure = null;
      } catch (e) {
        failure = String(e);
      }
    })();
  });

  async function run() {
    // An empty box is not a query. The core says the same thing, and saying it
    // here too keeps the surface from flashing a "nothing found" for a search
    // nobody made.
    if (!query.trim()) {
      results = null;
      return;
    }
    try {
      results = await invoke<SearchOutcome>("pdf_search", { query });
    } catch (e) {
      failure = String(e);
    }
  }

  const title = $derived(doc ? doc.path.split("/").pop() : null);

  /// Move by `delta` pages, stopping at the ends.
  ///
  /// Clamped rather than wrapping: a reader who presses Right on the last page
  /// of a report has reached the end of it, and jumping back to page one reads
  /// as the document having restarted.
  function step(delta: number) {
    if (!doc) return;
    current = Math.min(Math.max(current + delta, 1), doc.pages);
  }

  /// Keyboard first, as the viewer conventions have it.
  ///
  /// Ignored while the search box has focus, because there Space and the arrows
  /// belong to the text being typed - a reader mid-word does not expect the page
  /// to turn under them.
  function onKey(event: KeyboardEvent) {
    if (!doc) return;
    const target = event.target as HTMLElement | null;
    if (target?.tagName === "INPUT") return;
    const map: Record<string, () => void> = {
      ArrowRight: () => step(1),
      ArrowDown: () => step(1),
      PageDown: () => step(1),
      " ": () => step(event.shiftKey ? -1 : 1),
      ArrowLeft: () => step(-1),
      ArrowUp: () => step(-1),
      PageUp: () => step(-1),
      Home: () => (current = 1),
      End: () => (current = doc ? doc.pages : 1),
    };
    const act = map[event.key];
    if (act) {
      event.preventDefault();
      act();
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="pdf-app">
  <header class="pdf-bar" data-tauri-drag-region>
    <span class="pdf-title">{title ?? $t("pdf.app.title")}</span>
    <WindowButtons />
  </header>

  <div class="pdf-body">
    {#if !tauriAvailable}
      <p class="quiet">{$t("pdf.hostAbsent")}</p>
    {:else if failure}
      <p class="quiet">{$t("pdf.failed", { reason: failure })}</p>
    {:else if !doc}
      <p class="quiet">{$t("pdf.nothingOpen")}</p>
    {:else}
      <aside class="pdf-side">
        <div class="pdf-count">
          {doc.pages === 1 ? $t("pdf.onePage") : $t("pdf.pages", { count: doc.pages })}
        </div>

        <label class="pdf-search">
          <Search size={14} aria-hidden="true" />
          <input
            type="search"
            bind:value={query}
            oninput={run}
            aria-label={$t("pdf.search.label")}
            placeholder={$t("pdf.search.label")}
          />
        </label>

        {#if results}
          {#if results.hits.length === 0}
            <p class="quiet">{$t("pdf.search.none")}</p>
          {:else}
            <ul class="pdf-hits">
              {#each results.hits as hit (hit.page)}
                <li>
                  <button type="button" onclick={() => (current = hit.page)}>
                    <span class="pdf-hit-page">{$t("pdf.page", { number: hit.page })}</span>
                    <span class="pdf-hit-text">{hit.snippet}</span>
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
          <!-- Said whether or not there were hits: pages nobody could read are
               missing from BOTH answers, and the empty one is where a reader is
               most likely to conclude the word is not in the document. -->
          {#if results.unsearchable.length > 0}
            <p class="quiet">
              {$t("pdf.search.unsearchable", { count: results.unsearchable.length })}
            </p>
          {/if}
        {:else}
          <div class="pdf-contents">
            <h2>{$t("pdf.contents")}</h2>
            {#if doc.outline.length === 0}
              <p class="quiet">{$t("pdf.noContents")}</p>
            {:else}
              <ul>
                {#each doc.outline as entry, i (i)}
                  <li style="padding-inline-start: {entry.depth * 12}px">
                    <!-- An entry whose target this reader could not resolve is
                         still shown, with the jump disabled. Hiding it would
                         lose a heading the document plainly has. -->
                    <button
                      type="button"
                      disabled={entry.page === null}
                      onclick={() => entry.page && (current = entry.page)}
                    >
                      {entry.title}
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          </div>
        {/if}
      </aside>

      <main class="pdf-page">
        {#if pageFailure}
          <FileText size={28} aria-hidden="true" />
          <p class="quiet">{$t("pdf.pageFailed", { reason: pageFailure })}</p>
          <!-- The words, when the picture cannot be had. Said to be the text and
               not the page, because it has none of the layout: a table comes
               back as its cells in reading order and a two-column article reads
               straight through. Better than an empty sheet, and only if the
               reader is told which one they are looking at. -->
          {#if pageWords}
            <p class="quiet">{$t("pdf.textInstead")}</p>
            <pre class="pdf-words" data-selectable>{pageWords}</pre>
          {/if}
        {/if}
        <div class="pdf-sheet" class:hidden={pageFailure !== null}>
          <canvas bind:this={canvas} class="pdf-canvas"></canvas>
          <!-- The words, invisible, laid exactly over the ones in the picture.
               This is what makes a rendered page selectable at all: the canvas
               is pixels and carries no text a browser can reach, so the page's
               own lines are placed over it as transparent text and the ordinary
               selection works on those. -->
          <!-- `data-selectable` because the app shell turns selection OFF for
               everything by default - dragging across a button label reads as a
               bug - and opts back in through this attribute. The text layer was
               positioned correctly and selected nothing until it carried it,
               which is the difference between a layer that exists and one that
               works. -->
          <div class="pdf-text-layer" data-selectable>
            {#each lines as line, i (i)}
              <span
                style="left: {line.x}px; top: {line.y}px; width: {line.width}px; height: {line.height}px; font-size: {line.height * 0.8}px"
              >{line.text}</span>
            {/each}
          </div>
        </div>
        <p class="pdf-page-number">
          {$t("pdf.pageOf", { number: current, total: doc.pages })}
        </p>
      </main>
    {/if}
  </div>
</div>

<style>
  .pdf-words {
    max-width: 62ch;
    margin: 8px auto 0;
    padding: 12px 14px;
    border-radius: 6px;
    background: var(--color-bg-card, #171717);
    color: var(--color-fg-primary, #e5e5e5);
    font-size: 13px;
    line-height: 1.6;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    text-align: left;
  }
  .pdf-app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--color-bg-app);
    color: var(--color-fg-primary);
  }
  .pdf-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 0 8px 0 12px;
    height: 36px;
    border-bottom: 1px solid var(--color-border-default);
  }
  .pdf-title {
    font-size: 13px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .pdf-body {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .pdf-side {
    width: 280px;
    flex-shrink: 0;
    border-inline-end: 1px solid var(--color-border-default);
    padding: 12px;
    overflow-y: auto;
  }
  .pdf-count {
    font-size: 12px;
    color: var(--color-fg-secondary);
    margin-bottom: 8px;
  }
  .pdf-search {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border: 1px solid var(--color-border-default);
    border-radius: 6px;
    margin-bottom: 12px;
  }
  .pdf-search input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font-size: 13px;
    outline: none;
  }
  .pdf-contents h2 {
    font-size: 12px;
    font-weight: 600;
    color: var(--color-fg-secondary);
    margin: 0 0 6px;
  }
  .pdf-contents ul,
  .pdf-hits {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .pdf-contents button,
  .pdf-hits button {
    display: block;
    width: 100%;
    text-align: start;
    background: transparent;
    border: 0;
    color: inherit;
    font: inherit;
    padding: 4px 6px;
    border-radius: 4px;
    cursor: pointer;
  }
  .pdf-contents button:hover:not(:disabled),
  .pdf-hits button:hover {
    background: var(--color-bg-card);
  }
  .pdf-contents button:disabled {
    cursor: default;
    color: var(--color-fg-disabled);
  }
  .pdf-hit-page {
    display: block;
    font-size: 11px;
    color: var(--color-fg-secondary);
  }
  .pdf-hit-text {
    display: block;
    font-size: 12px;
  }
  .pdf-sheet {
    position: relative;
    max-width: 100%;
    max-height: calc(100vh - 100px);
    box-shadow: var(--shadow-lg);
  }
  .pdf-canvas {
    display: block;
    max-width: 100%;
    max-height: calc(100vh - 100px);
    background: #fff;
  }
  /* Transparent, but selectable: `color: transparent` keeps the glyphs in the
     document for selection and search while the canvas underneath is what a
     reader actually sees. Sized per line rather than per glyph, so the text is
     stretched to its box - the selection highlight then follows the words
     closely enough to read as theirs. */
  .pdf-text-layer {
    position: absolute;
    inset: 0;
    overflow: hidden;
  }
  .pdf-text-layer span {
    position: absolute;
    color: transparent;
    white-space: pre;
    transform-origin: 0 0;
    cursor: text;
    line-height: 1;
  }
  .pdf-text-layer span::selection {
    background: color-mix(in srgb, var(--color-accent) 40%, transparent);
  }
  .hidden {
    display: none;
  }
  .pdf-page {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 24px;
    text-align: center;
    color: var(--color-fg-secondary);
  }
  .pdf-page-number {
    font-size: 13px;
    margin: 0;
  }
  .quiet {
    font-size: 12px;
    color: var(--color-fg-secondary);
    max-width: 42ch;
    margin: 8px 0;
  }
</style>
