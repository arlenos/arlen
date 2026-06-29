<script module lang="ts">
  /// The grid's layout metric, shared with FileBrowser's keyboard
  /// stride, cursor scrolling and marquee: windowed tiles are
  /// off-DOM, so geometry must come from arithmetic, not rects.
  /// TILE_PX is the FileTile constant height (fixed media box +
  /// fixed two-line name; see FileTile.svelte).
  export const TILE_PX = 130;
  export const GRID_GAP_PX = 4;
  export const GRID_PAD_PX = 8;
  export const MIN_TILE_W_PX = 112;

  /// What the grid measured: tiles per row and the rendered
  /// (stretched) column width.
  export interface GridMetrics {
    columns: number;
    colW: number;
  }
</script>

<script lang="ts">
  /// The icon view: responsive tile grid over the same entries and
  /// the same event/selection contract as the list, windowed like
  /// FileList (only the visible rows of tiles are in the DOM, two
  /// spacers keep the scrollbar honest). Thumbnails are requested
  /// per rendered tile — the windowing is what makes them lazy.
  import { onMount, type Snippet } from "svelte";
  import type { FileEntry } from "./types";
  import FileTile from "./FileTile.svelte";

  let {
    entries,
    selectedIndices,
    cursorIndex = null,
    idBase,
    icon,
    onrowevent,
    thumbnails,
    thumbKey,
    requestThumbnail,
    onmetrics,
  }: {
    entries: FileEntry[];
    selectedIndices: ReadonlySet<number>;
    cursorIndex?: number | null;
    /// Per-instance id prefix for tile element ids (the grid's
    /// aria-activedescendant target).
    idBase?: string;
    icon?: Snippet<[FileEntry]>;
    onrowevent?: (
      kind: "click" | "dblclick" | "contextmenu",
      index: number,
      e: MouseEvent,
    ) => void;
    /// Resolved thumbnail URLs from the controller (grid view only).
    thumbnails?: ReadonlyMap<string, string | null>;
    /// The controller's key for an entry in `thumbnails`.
    thumbKey?: (entry: FileEntry) => string;
    /// Ask the controller for an entry's thumbnail (deduped there).
    requestThumbnail?: (entry: FileEntry) => void;
    /// The measured grid geometry, for the browser's metric math.
    onmetrics?: (m: GridMetrics) => void;
  } = $props();

  const ROW_STRIDE = TILE_PX + GRID_GAP_PX;
  const OVERSCAN_ROWS = 3;

  let gridEl = $state<HTMLDivElement | null>(null);
  let cols = $state(1);
  let winRowStart = $state(0);
  let winRowEnd = $state(60);

  onMount(() => {
    const scroller = gridEl?.closest(".file-browser");
    if (!(scroller instanceof HTMLElement) || !gridEl) return;
    const update = () => {
      // The same column count CSS derives from
      // repeat(auto-fill, minmax(MIN_TILE_W, 1fr)).
      const contentW = gridEl!.clientWidth - 2 * GRID_PAD_PX;
      cols = Math.max(
        1,
        Math.floor((contentW + GRID_GAP_PX) / (MIN_TILE_W_PX + GRID_GAP_PX)),
      );
      const top = scroller.scrollTop - GRID_PAD_PX;
      winRowStart = Math.max(0, Math.floor(top / ROW_STRIDE) - OVERSCAN_ROWS);
      winRowEnd =
        Math.ceil((top + scroller.clientHeight) / ROW_STRIDE) + OVERSCAN_ROWS;
      onmetrics?.({
        columns: cols,
        colW: (contentW - (cols - 1) * GRID_GAP_PX) / cols,
      });
    };
    update();
    scroller.addEventListener("scroll", update, { passive: true });
    const observer = new ResizeObserver(update);
    observer.observe(scroller);
    observer.observe(gridEl);
    return () => {
      scroller.removeEventListener("scroll", update);
      observer.disconnect();
    };
  });

  const start = $derived(winRowStart * cols);
  const end = $derived(Math.min(entries.length, winRowEnd * cols));
  const slice = $derived(entries.slice(start, end));
  // A spacer replaces N rows plus all but one of their row gaps (the
  // grid still puts one gap between the spacer row and the content).
  const padTop = $derived(winRowStart > 0 ? winRowStart * ROW_STRIDE - GRID_GAP_PX : 0);
  const rowsBelow = $derived(Math.max(0, Math.ceil((entries.length - end) / cols)));
  const padBottom = $derived(rowsBelow > 0 ? rowsBelow * ROW_STRIDE - GRID_GAP_PX : 0);

  // Rendered tiles ask for their thumbnail; the controller dedupes
  // and resolves into its store, so this is cheap to re-run per
  // window move.
  $effect(() => {
    if (!requestThumbnail) return;
    for (const e of slice) {
      if (e.kind === "file") requestThumbnail(e);
    }
  });

  /// The corner badge says what a thumbnail hides: the filetype.
  /// Only paired with a thumbnail URL — an icon tile's corner stays
  /// empty (the icon already speaks).
  function extBadge(e: FileEntry): string | null {
    const i = e.name.lastIndexOf(".");
    if (i <= 0) return null;
    const ext = e.name.slice(i + 1).toUpperCase();
    return ext.length <= 4 ? ext : null;
  }
</script>

<div class="file-grid" role="grid" aria-label="Files" bind:this={gridEl}>
  {#if padTop > 0}
    <div class="fg-spacer" style:height="{padTop}px"></div>
  {/if}
  {#each slice as entry, sliceIndex (entry.name)}
    {@const i = start + sliceIndex}
    {@const url = thumbnails?.get(thumbKey?.(entry) ?? "") ?? null}
    <FileTile
      id={idBase ? `${idBase}-item-${i}` : undefined}
      {entry}
      {icon}
      thumbnail={url}
      badge={url ? extBadge(entry) : null}
      selected={selectedIndices.has(i)}
      focused={cursorIndex === i}
      ontileclick={(e) => onrowevent?.("click", i, e)}
      ontiledblclick={(e) => onrowevent?.("dblclick", i, e)}
      ontilecontextmenu={(e) => onrowevent?.("contextmenu", i, e)}
    />
  {/each}
  {#if padBottom > 0}
    <div class="fg-spacer" style:height="{padBottom}px"></div>
  {/if}
</div>

<style>
  .file-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(7rem, 1fr));
    grid-auto-rows: max-content;
    gap: 4px;
    padding: 8px;
    align-content: start;
  }
  .fg-spacer {
    grid-column: 1 / -1;
  }
</style>
