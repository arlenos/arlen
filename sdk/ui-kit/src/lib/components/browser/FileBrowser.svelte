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
  import type { FileEntry } from "./types";
  import { joinPath } from "./types";
  import { Selection } from "./selection";
  import FileList from "./FileList.svelte";
  import FileGrid from "./FileGrid.svelte";
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

  const visible = $derived(filter ? $entries.filter(filter) : $entries);

  // Selection is synchronous view state (the documented exception to
  // the stores rule); it rebases whenever the listing identity
  // changes and is mirrored into a plain Set for the rows.
  const selection = new Selection(0);
  let selectedIndices = $state<ReadonlySet<number>>(new Set());
  let cursorIndex = $state<number | null>(null);
  let listedPath = $state("");

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

  /// The desktop keyboard grammar: arrows move the cursor (Shift
  /// extends), Home/End jump, Enter activates, Backspace goes up,
  /// Ctrl+A selects all, Escape clears, F2 renames the cursor entry.
  function onkeydown(e: KeyboardEvent) {
    if (renamingName !== null) return;
    const key = e.key;
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
    }
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
    rootEl
      .querySelectorAll(".file-row, .file-tile")
      [i]?.scrollIntoView({ block: "nearest" });
  }

  /// Tiles per grid row, measured from layout (the first tile whose
  /// top differs from the first marks the wrap).
  function gridColumns(): number {
    const tiles = rootEl?.querySelectorAll(".file-tile");
    if (!tiles || tiles.length < 2) return 1;
    const top = (tiles[0] as HTMLElement).offsetTop;
    for (let i = 1; i < tiles.length; i++) {
      if ((tiles[i] as HTMLElement).offsetTop !== top) return i;
    }
    return tiles.length;
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="file-browser"
  bind:this={rootEl}
  role="application"
  aria-label="File browser"
  tabindex="0"
  onkeydown={onkeydown}
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
      <span class="fb-state-title">This folder is empty</span>
    </div>
  {:else if $viewMode === "grid"}
    <FileGrid
      entries={visible}
      {selectedIndices}
      {cursorIndex}
      {icon}
      {onrowevent}
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
      {now}
      {icon}
      {renamingName}
      onsort={(key) => controller.setSort(key)}
      {onrowevent}
      onrename={(entry, newName) => {
        renamingName = null;
        if (newName !== entry.name) onrenamecommit?.(entry, newName);
      }}
    />
  {/if}
</div>

<style>
  .file-browser {
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
