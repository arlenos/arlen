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

  let doc = $state<DocumentInfo | null>(null);
  let failure = $state<string | null>(null);
  let query = $state("");
  let results = $state<SearchOutcome | null>(null);
  let current = $state(1);
  let canvas = $state<HTMLCanvasElement | null>(null);
  let pageFailure = $state<string | null>(null);

  /// Draw the current page.
  ///
  /// A page that will not render is reported where the page would be, not
  /// swallowed: a reader looking at a blank frame has no way to tell a scanned
  /// blank page from a renderer that refused.
  async function drawPage() {
    if (!doc || !canvas) return;
    try {
      const img = await invoke<PageImage>("pdf_page_image", { page: current, scale: 1.5 });
      canvas.width = img.width;
      canvas.height = img.height;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      const data = new ImageData(new Uint8ClampedArray(img.rgba), img.width, img.height);
      ctx.putImageData(data, 0, 0);
      pageFailure = null;
    } catch (e) {
      pageFailure = String(e);
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
</script>

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
                  <li style="padding-left: {entry.depth * 12}px">
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
        {/if}
        <canvas bind:this={canvas} class="pdf-canvas" class:hidden={pageFailure !== null}></canvas>
        <p class="pdf-page-number">{$t("pdf.page", { number: current })}</p>
      </main>
    {/if}
  </div>
</div>

<style>
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
    border-right: 1px solid var(--color-border-default);
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
    text-align: left;
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
  .pdf-canvas {
    max-width: 100%;
    max-height: calc(100vh - 100px);
    object-fit: contain;
    background: #fff;
    box-shadow: var(--shadow-lg);
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
