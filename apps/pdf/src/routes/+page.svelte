<script lang="ts">
  import { clampPage, pageIntent } from "$lib/paging";
  /// The reader, two faces over one document. The reading face: the files
  /// chrome - contents/search in the rail, the h-10 bar with page and zoom
  /// clusters - around a continuous scroll of lazily rendered pages on the
  /// viewer family's dark ground. The document-only face (Tim's macOS
  /// reference): nothing but the pages; moving the mouse floats a small
  /// overlay that fades after a beat, the keyboard stays fully armed, and the
  /// depth lives in the shell's top-left app menu.
  ///
  /// Every empty-looking state is a DIFFERENT state and says so; a page that
  /// will not render says so where the page would be, with its words below.
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import {
    SidebarProvider,
    SidebarInset,
    SidebarTrigger,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { Separator } from "@arlen/ui-kit/components/ui/separator";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import { ChevronLeft, ChevronRight, FileText, Minus, Plus, PanelLeft, Scan, X } from "@lucide/svelte";
  import { tauriAvailable } from "$lib/tauri";
  import { t } from "$lib/i18n/messages";
  import { initAppMenu, menuAction } from "$lib/menu";
  import {
    doc,
    failure,
    launchFailure,
    openLaunched,
    search,
    type SearchOutcome,
  } from "$lib/stores/pdf";
  import PdfSidebar from "$lib/components/PdfSidebar.svelte";
  import PageCanvas from "$lib/components/PageCanvas.svelte";

  let current = $state(1);
  let query = $state("");
  let results = $state<SearchOutcome | null>(null);
  let clean = $state(false);

  // --- Zoom and fit --------------------------------------------------------
  // One scale for the raster AND the text layer, always: the boxes come back
  // in the raster's own pixel space (the coder's one-constant rule, now a
  // one-state rule). Fit modes derive it from the page's own point width.
  const ZOOM_STEPS = [50, 67, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400];
  const PT_TO_PX = 96 / 72;
  let fitMode = $state<"width" | "page" | "custom">("width");
  let percent = $state(100);
  let ratio = $state(842 / 595);
  let ptWidth = $state(595);
  let viewportW = $state(0);
  let viewportH = $state(0);

  const scale = $derived.by(() => {
    let s: number;
    if (fitMode === "width" && viewportW > 0) s = (viewportW - 96) / ptWidth;
    else if (fitMode === "page" && viewportH > 0)
      s = Math.min((viewportW - 96) / ptWidth, (viewportH - 48) / (ptWidth * ratio));
    else s = (percent / 100) * PT_TO_PX;
    return Math.min(8, Math.max(0.1, Math.round(s * 100) / 100));
  });
  const shownPercent = $derived(Math.round((scale / PT_TO_PX) * 100));

  function zoom(delta: number): void {
    const now = shownPercent;
    const next =
      delta > 0
        ? (ZOOM_STEPS.find((z) => z > now) ?? ZOOM_STEPS[ZOOM_STEPS.length - 1])
        : ([...ZOOM_STEPS].reverse().find((z) => z < now) ?? ZOOM_STEPS[0]);
    fitMode = "custom";
    percent = next;
  }
  function actualSize(): void {
    fitMode = "custom";
    percent = 100;
  }

  // --- Search (debounced; the core rescans the document per query) ---------
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    const q = query;
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => {
      void search(q)
        .then((r) => (results = r))
        .catch((e) => failure.set(String(e)));
    }, 200);
  });

  // --- Continuous scroll: sync and jumps -----------------------------------
  let scroller = $state<HTMLElement | null>(null);
  let suppressSync = 0;

  function goTo(page: number): void {
    if (!$doc) return;
    current = clampPage(page, 0, $doc.pages);
    const el = scroller?.querySelector(`[data-page="${current}"]`);
    if (el) {
      suppressSync = Date.now() + 600;
      el.scrollIntoView({ block: "start" });
    }
  }
  function step(delta: number): void {
    if (!$doc) return;
    goTo(clampPage(current, delta, $doc.pages));
  }

  function onScroll(): void {
    if (!scroller || !$doc || Date.now() < suppressSync) return;
    const top = scroller.getBoundingClientRect().top + 40;
    let best = current;
    for (const el of scroller.querySelectorAll("[data-page]")) {
      const r = el.getBoundingClientRect();
      if (r.top <= top && r.bottom > top) {
        best = Number((el as HTMLElement).dataset.page);
        break;
      }
    }
    current = best;
  }

  // --- Keyboard: the tested paging vocabulary, plus zoom and the mode ------
  function onKey(event: KeyboardEvent) {
    if (!$doc) return;
    const target = event.target as HTMLElement | null;
    const inInput = target?.tagName === "INPUT";
    const intent = pageIntent(event.key, event.shiftKey, inInput);
    if (intent) {
      event.preventDefault();
      if (intent.kind === "step") step(intent.delta);
      else if (intent.kind === "first") goTo(1);
      else goTo($doc.pages);
      return;
    }
    if (inInput) return;
    if (event.key === "+" || event.key === "=") zoom(1);
    else if (event.key === "-") zoom(-1);
    else if (event.key === "0") actualSize();
    else if (event.key === ".") clean = !clean;
    else if (event.key === "Escape" && clean) clean = false;
    else return;
    event.preventDefault();
  }

  function onWheel(event: WheelEvent): void {
    if (!event.ctrlKey) return;
    event.preventDefault();
    zoom(event.deltaY < 0 ? 1 : -1);
  }

  // --- The document-only overlay -------------------------------------------
  let overlayShown = $state(false);
  let overlayTimer: ReturnType<typeof setTimeout> | null = null;
  function wake(): void {
    if (!clean) return;
    overlayShown = true;
    if (overlayTimer) clearTimeout(overlayTimer);
    overlayTimer = setTimeout(() => (overlayShown = false), 2000);
  }

  async function winMin(): Promise<void> {
    try {
      await getCurrentWindow().minimize();
    } catch {
      /* standalone */
    }
  }
  async function winClose(): Promise<void> {
    try {
      await getCurrentWindow().close();
    } catch {
      /* standalone */
    }
  }

  // --- Menu actions from the shell topbar ----------------------------------
  $effect(() => {
    const a = $menuAction;
    if (!a) return;
    menuAction.set(null);
    if (a === "view.document-only") clean = !clean;
    else if (a === "view.contents") query = "";
    else if (a === "view.zoom-in") zoom(1);
    else if (a === "view.zoom-out") zoom(-1);
    else if (a === "view.actual-size") actualSize();
    else if (a === "view.fit-width") fitMode = "width";
    else if (a === "view.fit-page") fitMode = "page";
    else if (a === "go.next") step(1);
    else if (a === "go.previous") step(-1);
    else if (a === "go.first") goTo(1);
    else if (a === "go.last" && $doc) goTo($doc.pages);
  });

  onMount(() => {
    void openLaunched();
    void initAppMenu();
  });

  const title = $derived($doc ? $doc.path.split("/").pop() : null);

  function isInteractive(e: Event): boolean {
    const target = e.target as HTMLElement | null;
    return !!target?.closest("button, a, input, [role='button']");
  }
  async function startDrag(e: PointerEvent) {
    if (e.button !== 0 || e.pointerType !== "mouse") return;
    if (isInteractive(e)) return;
    try {
      await getCurrentWindow().startDragging();
    } catch {
      /* standalone (vite) has no toplevel to drag */
    }
  }
  async function toggleMax(e: MouseEvent) {
    if (isInteractive(e)) return;
    try {
      const w = getCurrentWindow();
      if (await w.isMaximized()) await w.unmaximize();
      else await w.maximize();
    } catch {
      /* no window in standalone */
    }
  }
</script>

<svelte:window onkeydown={onKey} />

{#if clean && $doc}
  <!-- The document-only face: the window is the content. The overlay is a
       courtesy that appears under a moving mouse and leaves; the keyboard and
       the shell app menu carry everything else. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="clean" onpointermove={wake} onwheel={onWheel}>
    <div
      class="pages clean-pages"
      bind:this={scroller}
      bind:clientWidth={viewportW}
      bind:clientHeight={viewportH}
      onscroll={onScroll}
    >
      {#each Array.from({ length: $doc.pages }, (_, i) => i + 1) as page (page)}
        <PageCanvas {page} {scale} {ratio} {query} onmetrics={(r, w) => ((ratio = r), (ptWidth = w))} />
      {/each}
    </div>
    <div class="overlay" class:shown={overlayShown}>
      <div class="ov-top">
        <IconAction label={$t("pdf.readingView")} size="control" onclick={() => (clean = false)}>
          <PanelLeft size={15} strokeWidth={1.75} />
        </IconAction>
        <span class="ov-spacer"></span>
        <IconAction label={$t("pdf.minimize")} size="control" onclick={winMin}>
          <Minus size={15} strokeWidth={1.75} />
        </IconAction>
        <IconAction label={$t("pdf.close")} size="control" onclick={winClose}>
          <X size={15} strokeWidth={1.75} />
        </IconAction>
      </div>
      <span class="ov-pill">{$t("pdf.pageOf", { number: current, total: $doc.pages })}</span>
    </div>
  </div>
{:else}
  <SidebarProvider class="h-screen min-h-0 overflow-hidden">
    {#if $doc}
      <PdfSidebar doc={$doc} bind:query {results} {current} onjump={goTo} />
    {/if}

    <SidebarInset class="h-svh min-h-0">
      <!-- The header is a drag surface (a non-keyboard pointer interaction);
           its actual controls are the accessible buttons inside it, so the
           static-interaction lint is a false positive here. -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <header
        onpointerdown={startDrag}
        ondblclick={toggleMax}
        class="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-background px-2"
      >
        {#if $doc}
          <SidebarTrigger class="-ml-1" />
          <Separator orientation="vertical" class="me-1 h-4" />
        {/if}
        <span class="select-none truncate text-sm font-medium text-foreground">{title ?? $t("pdf.app.title")}</span>
        {#if $doc}
          <IconAction label={$t("pdf.prevPage")} size="control" onclick={() => step(-1)}>
            <ChevronLeft size={15} strokeWidth={1.75} />
          </IconAction>
          <span class="page-of">{$t("pdf.pageOf", { number: current, total: $doc.pages })}</span>
          <IconAction label={$t("pdf.nextPage")} size="control" onclick={() => step(1)}>
            <ChevronRight size={15} strokeWidth={1.75} />
          </IconAction>
        {/if}
        <div class="flex-1"></div>
        {#if $doc}
          <IconAction label={$t("pdf.zoomOut")} size="control" onclick={() => zoom(-1)}>
            <Minus size={15} strokeWidth={1.75} />
          </IconAction>
          <button type="button" class="zoom-pct" onclick={actualSize}>{shownPercent}%</button>
          <IconAction label={$t("pdf.zoomIn")} size="control" onclick={() => zoom(1)}>
            <Plus size={15} strokeWidth={1.75} />
          </IconAction>
          <IconAction label={$t("pdf.fitWidth")} size="control" active={fitMode === "width"} onclick={() => (fitMode = "width")}>
            <Scan size={15} strokeWidth={1.75} />
          </IconAction>
          <IconAction label={$t("pdf.documentOnly")} size="control" onclick={() => (clean = true)}>
            <FileText size={15} strokeWidth={1.75} />
          </IconAction>
        {/if}
        <WindowButtons />
      </header>

      <div class="content">
        {#if !tauriAvailable && !$doc}
          <p class="center-note">{$t("pdf.hostAbsent")}</p>
        {:else if $launchFailure}
          <p class="center-note">{$t("pdf.launchUnknown", { reason: $launchFailure })}</p>
        {:else if $failure === "locked"}
          <!-- The host sends a token for this one, so the sentence is written
               here and reaches a German reader in German. -->
          <p class="center-note">{$t("pdf.locked")}</p>
        {:else if $failure}
          <p class="center-note">{$t("pdf.failed", { reason: $failure })}</p>
        {:else if !$doc}
          <div class="empty">
            <FileText size={28} strokeWidth={1.5} aria-hidden="true" />
            <p class="center-note">{$t("pdf.nothingOpen")}</p>
          </div>
        {:else}
          <div
            class="pages"
            bind:this={scroller}
            bind:clientWidth={viewportW}
            bind:clientHeight={viewportH}
            onscroll={onScroll}
            onwheel={onWheel}
          >
            {#each Array.from({ length: $doc.pages }, (_, i) => i + 1) as page (page)}
              <PageCanvas {page} {scale} {ratio} {query} onmetrics={(r, w) => ((ratio = r), (ptWidth = w))} />
            {/each}
          </div>
        {/if}
      </div>
    </SidebarInset>
  </SidebarProvider>
{/if}

<style>
  .content {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  /* The letterbox ground of the viewer family. */
  .pages {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 24px;
    padding: 24px;
    background: #0a0a0a;
    scroll-padding-top: 12px;
  }
  .page-of {
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .zoom-pct {
    min-width: 3.2rem;
    padding: 0.2rem 0.3rem;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    font: inherit;
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }
  .zoom-pct:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .center-note {
    margin: auto;
    max-width: 46ch;
    padding: 24px;
    text-align: center;
    font-size: 13px;
    line-height: 1.5;
    color: var(--color-fg-secondary, #a3a3a3);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
    margin: auto;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
  .empty .center-note {
    margin: 0;
  }

  /* The document-only face. */
  .clean {
    position: fixed;
    inset: 0;
    display: flex;
    background: #0a0a0a;
  }
  .clean-pages {
    flex: 1;
  }
  .overlay {
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    padding: 10px 12px 14px;
    pointer-events: none;
    opacity: 0;
    transition: opacity 180ms ease;
  }
  .overlay.shown {
    opacity: 1;
  }
  @media (prefers-reduced-motion: reduce) {
    .overlay {
      transition: none;
    }
  }
  .ov-top {
    display: flex;
    gap: 4px;
    pointer-events: auto;
    align-self: stretch;
  }
  .ov-top :global(.ia) {
    background: color-mix(in srgb, #0a0a0a 70%, transparent);
    -webkit-backdrop-filter: blur(6px);
    backdrop-filter: blur(6px);
  }
  .ov-spacer {
    flex: 1;
    pointer-events: none;
  }
  .ov-pill {
    align-self: center;
    padding: 4px 12px;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, #0a0a0a 70%, transparent);
    -webkit-backdrop-filter: blur(6px);
    backdrop-filter: blur(6px);
    font-size: var(--text-xs, 12px);
    color: #e5e5e5;
    font-variant-numeric: tabular-nums;
    pointer-events: none;
  }
</style>
