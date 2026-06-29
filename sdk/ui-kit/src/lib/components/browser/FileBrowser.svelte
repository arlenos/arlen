<script lang="ts" module>
  // Per-instance counter so each browser's row element ids are unique across
  // split panes (the aria-activedescendant target must resolve to one element).
  let instanceSeq = 0;
</script>

<script lang="ts">
  /// The shared file browser: one controller in, the listing with
  /// selection and activation out. Directory activation navigates
  /// internally (every host wants that); everything else calls
  /// `onactivate` and the host decides (the FM opens, the picker
  /// confirms). Hosted unchanged by the FM app and the confined xdg
  /// picker — nothing in here may assume ambient filesystem access
  /// or a particular window chrome.
  import type { Snippet } from "svelte";
  import type { BrowserState } from "./controller";
  import { type FileEntry, type ColumnSpec, DEFAULT_COLUMNS, joinPath } from "./types";
  import { Selection } from "./selection";
  import FileList from "./FileList.svelte";
  import FileGrid, {
    GRID_GAP_PX,
    GRID_PAD_PX,
    TILE_PX,
    type GridMetrics,
  } from "./FileGrid.svelte";
  import MillerColumns from "./MillerColumns.svelte";

  let {
    controller,
    onactivate,
    onselection,
    oncontextmenu,
    onrenamecommit,
    renamingName = $bindable(null),
    filter,
    now,
    columns = DEFAULT_COLUMNS,
    emptyLabel = "This folder is empty",
    icon,
  }: {
    /// The headless browser state; swapping it switches tabs.
    controller: BrowserState;
    /// A non-directory entry was activated (double-click or Enter).
    onactivate?: (entry: FileEntry, path: string) => void;
    /// The selection changed; entries are the selected rows.
    onselection?: (entries: FileEntry[]) => void;
    /// A row (or the empty area, entry null) asked for a context menu.
    oncontextmenu?: (entry: FileEntry | null, e: MouseEvent) => void;
    /// The inline rename committed with a changed name.
    onrenamecommit?: (entry: FileEntry, newName: string) => void;
    /// The entry name currently in inline rename (F2); bindable so
    /// the host can start a rename (e.g. right after New Folder).
    renamingName?: string | null;
    /// Host-side row filter (the picker's globs); directories always
    /// pass on the host side by convention.
    filter?: (entry: FileEntry) => boolean;
    /// Injectable clock for stable screenshots.
    now?: number;
    /// Which columns the list view shows (a virtual location swaps Size for
    /// Location and relabels the time column).
    columns?: ColumnSpec;
    /// The message shown when this location is empty (a virtual location speaks
    /// for itself: "Trash is empty", "No recent files").
    emptyLabel?: string;
    /// Icon seam for themed and KG-state icons.
    icon?: Snippet<[FileEntry]>;
  } = $props();

  // Each store ref re-derives when the controller prop swaps (a tab
  // switch), so the `$` subscriptions follow the active tab.
  const path = $derived(controller.path);
  const entries = $derived(controller.entries);
  const loading = $derived(controller.loading);
  const error = $derived(controller.error);
  const sortKey = $derived(controller.sortKey);
  const ascending = $derived(controller.ascending);
  const viewMode = $derived(controller.viewMode);
  const thumbnails = $derived(controller.thumbnails);
  const selectAllSignal = $derived(controller.selectAllSignal);

  const visible = $derived(filter ? $entries.filter(filter) : $entries);

  // The grid reports its measured geometry; keyboard stride, cursor
  // scrolling and the marquee compute from it (windowed tiles are
  // off-DOM, so rect reads would lie).
  let gridMetrics = $state<GridMetrics | null>(null);

  // Selection is synchronous view state (the documented exception to
  // the stores rule); it rebases whenever the listing identity
  // changes and is mirrored into a plain Set for the rows.
  const selection = new Selection(0);
  let selectedIndices = $state<ReadonlySet<number>>(new Set());
  let cursorIndex = $state<number | null>(null);
  let listedPath = $state("");

  // The id prefix for row/tile elements; the container points
  // aria-activedescendant at the cursored item so a screen reader announces it
  // as the user arrows through (list + grid views, which render flat indices).
  const idBase = `fb-${(instanceSeq += 1)}`;
  const activeDescendant = $derived(
    cursorIndex !== null && ($viewMode === "list" || $viewMode === "grid")
      ? `${idBase}-item-${cursorIndex}`
      : undefined,
  );

  $effect(() => {
    const p = $path;
    const count = visible.length;
    if (p !== listedPath) {
      listedPath = p;
      selection.rebase(count);
    } else if (count !== selection.size()) {
      selection.rebase(count);
    }
    publish();
  });

  function publish() {
    const set = new Set(selection.indices());
    selectedIndices = set;
    cursorIndex = selection.cursor();
    onselection?.([...set].map((i) => visible[i]).filter(Boolean));
  }

  // Apply a host "select all" command (the topbar Edit menu) to the
  // internal selection - the same path as Ctrl+A. The controller carries
  // a monotonic signal; we adopt its current value on mount and on a
  // controller swap (a tab switch is not a select-all), then select-all
  // on each later increment.
  let selectAllController: BrowserState | null = null;
  let selectAllSeen = 0;
  $effect(() => {
    const n = $selectAllSignal;
    if (controller !== selectAllController) {
      selectAllController = controller;
      selectAllSeen = n;
      return;
    }
    if (n !== selectAllSeen) {
      selectAllSeen = n;
      if (visible.length > 0) {
        selection.selectAll();
        publish();
      }
    }
  });

  function onrowevent(kind: "click" | "dblclick" | "contextmenu", i: number, e: MouseEvent) {
    if (kind === "click") {
      if (e.shiftKey) selection.rangeTo(i);
      else if (e.ctrlKey || e.metaKey) selection.toggle(i);
      else selection.click(i);
      publish();
      return;
    }
    if (kind === "contextmenu") {
      if (!selection.isSelected(i)) {
        selection.click(i);
        publish();
      }
      oncontextmenu?.(visible[i] ?? null, e);
      return;
    }
    // dblclick
    const entry = visible[i];
    if (!entry) return;
    activate(entry);
  }

  function activate(entry: FileEntry) {
    if (entry.kind === "directory") {
      void controller.navigate(joinPath($path, entry.name));
      return;
    }
    onactivate?.(entry, joinPath($path, entry.name));
  }

  // Type-ahead: printable keys accumulate a 1s prefix buffer and
  // jump the cursor to the next matching name. The vim keys hjkl and
  // the g chord take precedence over type-ahead by design (the
  // ranger trade-off); names starting with those letters are reached
  // by arrows or a longer prefix.
  let typeBuffer = "";
  let typeTimer: ReturnType<typeof setTimeout> | null = null;
  let pendingG = false;

  function typeAhead(ch: string) {
    typeBuffer += ch.toLowerCase();
    if (typeTimer) clearTimeout(typeTimer);
    typeTimer = setTimeout(() => (typeBuffer = ""), 1000);
    const start = (selection.cursor() ?? -1) + (typeBuffer.length === 1 ? 1 : 0);
    const n = visible.length;
    for (let off = 0; off < n; off++) {
      const i = (start + off) % n;
      if (visible[i]?.name.toLowerCase().startsWith(typeBuffer)) {
        selection.click(i);
        publish();
        scrollCursorIntoView();
        return;
      }
    }
  }

  /// The desktop keyboard grammar: arrows move the cursor (Shift
  /// extends), Home/End jump, Enter activates, Backspace goes up,
  /// Ctrl+A selects all, Escape clears, F2 renames the cursor entry.
  /// The vim layer: j/k down/up, h up-a-folder, l descend, gg/G ends.
  function onkeydown(e: KeyboardEvent) {
    if (renamingName !== null) return;
    let key = e.key;
    // g is a chord prefix: gg jumps to the first entry.
    if (pendingG) {
      pendingG = false;
      if (key === "g" && !e.ctrlKey && !e.metaKey) {
        key = "Home";
      }
    } else if (key === "g" && !e.ctrlKey && !e.metaKey && !e.altKey) {
      pendingG = true;
      setTimeout(() => (pendingG = false), 500);
      return;
    }
    if (!e.ctrlKey && !e.metaKey && !e.altKey) {
      if (key === "j") key = "ArrowDown";
      else if (key === "k") key = "ArrowUp";
      else if (key === "h") key = "Backspace";
      else if (key === "l") key = "Enter";
      else if (key === "G") key = "End";
    }
    if (key === "ArrowDown" || key === "ArrowUp") {
      e.preventDefault();
      const stride = $viewMode === "grid" ? gridColumns() : 1;
      selection.moveCursor(key === "ArrowDown" ? stride : -stride, e.shiftKey);
      publish();
      scrollCursorIntoView();
    } else if (($viewMode === "grid") && (key === "ArrowLeft" || key === "ArrowRight")) {
      e.preventDefault();
      selection.moveCursor(key === "ArrowRight" ? 1 : -1, e.shiftKey);
      publish();
      scrollCursorIntoView();
    } else if (key === "Home" || key === "End") {
      e.preventDefault();
      selection.moveCursor(key === "Home" ? -Infinity : Infinity, e.shiftKey);
      publish();
      scrollCursorIntoView();
    } else if (key === "Enter") {
      const i = selection.cursor();
      const entry = i !== null ? visible[i] : undefined;
      if (entry) {
        e.preventDefault();
        activate(entry);
      }
    } else if (key === "Backspace") {
      e.preventDefault();
      void controller.up();
    } else if (key === "a" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      selection.selectAll();
      publish();
    } else if (key === "Escape") {
      selection.clear();
      publish();
    } else if (key === "F2") {
      const i = selection.cursor();
      const entry = i !== null ? visible[i] : undefined;
      if (entry) {
        e.preventDefault();
        renamingName = entry.name;
      }
    } else if (
      key.length === 1 &&
      !e.ctrlKey &&
      !e.metaKey &&
      !e.altKey &&
      /[\x20-\x7e]/.test(key)
    ) {
      e.preventDefault();
      typeAhead(key);
    }
  }

  // ── rubber-band (marquee) selection ──────────────────────────────
  // Drag from empty space draws the rectangle; intersection selects
  // (Ctrl adds). The list intersects via the row metric, so windowed
  // rows off-DOM still select; the grid reads tile rects.
  let marquee = $state<{ x: number; y: number; w: number; h: number } | null>(null);
  let marqueeStart: { x: number; y: number; additive: boolean } | null = null;

  function contentPoint(e: PointerEvent): { x: number; y: number } {
    const r = rootEl!.getBoundingClientRect();
    return {
      x: e.clientX - r.left + rootEl!.scrollLeft,
      y: e.clientY - r.top + rootEl!.scrollTop,
    };
  }

  function onpointerdown(e: PointerEvent) {
    if (e.button !== 0 || !rootEl || $viewMode === "miller") return;
    const t = e.target as HTMLElement;
    if (t.closest(".file-row, .file-tile, button, input, .mc-row")) return;
    marqueeStart = { ...contentPoint(e), additive: e.ctrlKey || e.metaKey };
    rootEl.setPointerCapture(e.pointerId);
  }

  function onpointermove(e: PointerEvent) {
    if (!marqueeStart || !rootEl) return;
    const p = contentPoint(e);
    const x = Math.min(marqueeStart.x, p.x);
    const y = Math.min(marqueeStart.y, p.y);
    const w = Math.abs(p.x - marqueeStart.x);
    const h = Math.abs(p.y - marqueeStart.y);
    if (!marquee && w < 4 && h < 4) return;
    marquee = { x, y, w, h };
    applyMarquee();
  }

  function onpointerup() {
    if (!marqueeStart) return;
    const wasDrag = marquee !== null;
    marqueeStart = null;
    marquee = null;
    if (!wasDrag) {
      // A plain click on empty space clears the selection.
      selection.clear();
      publish();
    }
  }

  function applyMarquee() {
    if (!marquee || !rootEl) return;
    const hits: number[] = [];
    if ($viewMode === "list") {
      const headerPx = 28;
      const pad = 4;
      const rowPx = 32;
      const top = marquee.y - headerPx - pad;
      const bottom = top + marquee.h;
      const lo = Math.max(0, Math.floor(top / rowPx));
      const hi = Math.min(visible.length - 1, Math.floor(bottom / rowPx));
      for (let i = lo; i <= hi; i++) hits.push(i);
    } else {
      // The grid windows its tiles, so intersection runs on the same
      // metric the grid reported — scrolled-away tiles still select.
      const g = gridMetrics;
      if (!g) return;
      const m = marquee;
      const strideY = TILE_PX + GRID_GAP_PX;
      const strideX = g.colW + GRID_GAP_PX;
      const rowLo = Math.max(0, Math.floor((m.y - GRID_PAD_PX) / strideY));
      const rowHi = Math.floor((m.y + m.h - GRID_PAD_PX) / strideY);
      const colLo = Math.max(0, Math.floor((m.x - GRID_PAD_PX) / strideX));
      const colHi = Math.min(
        g.columns - 1,
        Math.floor((m.x + m.w - GRID_PAD_PX) / strideX),
      );
      for (let r = rowLo; r <= rowHi; r++) {
        const ry = GRID_PAD_PX + r * strideY;
        // A marquee living entirely in the gap band hits nothing.
        if (ry + TILE_PX <= m.y || ry >= m.y + m.h) continue;
        for (let c = colLo; c <= colHi; c++) {
          const cx = GRID_PAD_PX + c * strideX;
          if (cx + g.colW <= m.x || cx >= m.x + m.w) continue;
          const i = r * g.columns + c;
          if (i < visible.length) hits.push(i);
        }
      }
    }
    selection.setSelected(hits, marqueeStart?.additive ?? false);
    publish();
  }

  let rootEl = $state<HTMLDivElement | null>(null);
  function scrollCursorIntoView() {
    const i = selection.cursor();
    if (i === null || !rootEl) return;
    if ($viewMode === "list") {
      // The list windows its rows, so the target may not be in the
      // DOM; the row metric (2rem) makes the scroll math exact.
      const rowPx = 32;
      const headerPx = 28;
      const top = headerPx + i * rowPx;
      if (top < rootEl.scrollTop + headerPx) {
        rootEl.scrollTop = top - headerPx;
      } else if (top + rowPx > rootEl.scrollTop + rootEl.clientHeight) {
        rootEl.scrollTop = top + rowPx - rootEl.clientHeight;
      }
      return;
    }
    if ($viewMode === "grid") {
      // Same story as the list: the target tile may be windowed out,
      // so the scroll math runs on the grid metric.
      const cols = gridMetrics?.columns ?? 1;
      const top = GRID_PAD_PX + Math.floor(i / cols) * (TILE_PX + GRID_GAP_PX);
      if (top < rootEl.scrollTop) {
        rootEl.scrollTop = top;
      } else if (top + TILE_PX > rootEl.scrollTop + rootEl.clientHeight) {
        rootEl.scrollTop = top + TILE_PX - rootEl.clientHeight;
      }
      return;
    }
    rootEl
      .querySelectorAll(".file-row, .file-tile")
      [i]?.scrollIntoView({ block: "nearest" });
  }

  /// Tiles per grid row, from the grid's reported metric.
  function gridColumns(): number {
    return gridMetrics?.columns ?? 1;
  }
</script>

<!-- A custom keyboard-driven composite: role="application" hands all keys to
     the grid's own navigation (Arrow/Enter/Home/End/type-ahead), tabindex makes
     it the single tab stop, and aria-activedescendant tracks the cursored item
     for screen readers. The lint treats application as non-interactive, which is
     wrong for this widget. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  class="file-browser"
  bind:this={rootEl}
  role="application"
  aria-label="File browser"
  aria-activedescendant={activeDescendant}
  tabindex="0"
  onkeydown={onkeydown}
  onpointerdown={onpointerdown}
  onpointermove={onpointermove}
  onpointerup={onpointerup}
  oncontextmenu={(e) => {
    if (!(e.target as HTMLElement).closest(".file-row")) {
      oncontextmenu?.(null, e);
    }
  }}
>
  {#if $error}
    <div class="fb-state">
      <span class="fb-state-title">Can't open this folder</span>
      <span class="fb-state-hint">
        {#if /permission denied/i.test($error)}
          You don't have permission to see what's inside.
        {:else if /not connected/i.test($error)}
          This place is not connected right now.
        {:else if /no such directory/i.test($error)}
          This folder does not exist anymore.
        {:else}
          {$error}
        {/if}
      </span>
    </div>
  {:else if !$loading && visible.length === 0}
    <div class="fb-state">
      <span class="fb-state-title">{emptyLabel}</span>
    </div>
  {:else if $viewMode === "grid"}
    <!-- Thumbnails ride the grid only (the doc's default; "on-demand
         in list view" has no specified control yet — flagged, not
         invented). -->
    <FileGrid
      entries={visible}
      {selectedIndices}
      {cursorIndex}
      {idBase}
      {icon}
      {onrowevent}
      thumbnails={controller.hasThumbnails ? $thumbnails : undefined}
      thumbKey={controller.hasThumbnails
        ? (e) => controller.thumbnailKeyFor(e)
        : undefined}
      requestThumbnail={controller.hasThumbnails
        ? (e) => controller.requestThumbnail(e)
        : undefined}
      onmetrics={(m) => (gridMetrics = m)}
    />
  {:else if $viewMode === "miller"}
    <MillerColumns
      {controller}
      {selectedIndices}
      {cursorIndex}
      {onrowevent}
    />
  {:else}
    <FileList
      entries={visible}
      sortKey={$sortKey}
      ascending={$ascending}
      {selectedIndices}
      {cursorIndex}
      {idBase}
      {now}
      {columns}
      {icon}
      {renamingName}
      thumbnails={controller.hasThumbnails ? $thumbnails : undefined}
      thumbKey={controller.hasThumbnails ? (e) => controller.thumbnailKeyFor(e) : undefined}
      requestThumbnail={controller.hasThumbnails ? (e) => controller.requestThumbnail(e) : undefined}
      onsort={(key) => controller.setSort(key)}
      {onrowevent}
      onrename={(entry, newName) => {
        renamingName = null;
        if (newName !== entry.name) onrenamecommit?.(entry, newName);
      }}
    />
  {/if}
  {#if marquee}
    <div
      class="fb-marquee"
      style="left: {marquee.x}px; top: {marquee.y}px; width: {marquee.w}px; height: {marquee.h}px;"
    ></div>
  {/if}
</div>

<style>
  .file-browser {
    position: relative;
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    outline: none;
    /* Children query this width: a narrow pane (dual + info open)
       drops the metadata columns instead of crushing the names. */
    container-type: inline-size;
    container-name: browser;
  }

  .fb-marquee {
    position: absolute;
    z-index: 2;
    pointer-events: none;
    border: 1px solid color-mix(in srgb, var(--color-accent, var(--primary)) 45%, transparent);
    background: color-mix(in srgb, var(--color-accent, var(--primary)) 10%, transparent);
    border-radius: var(--radius-chip);
  }

  .fb-state {
    margin: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    text-align: center;
    padding: 2rem;
  }
  .fb-state-title {
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .fb-state-hint {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    max-width: 36ch;
  }
</style>
