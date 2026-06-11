<script lang="ts">
  /// The detail view: a sortable header row over FileRows. Sorting
  /// goes through the controller (the backend sorts, the view only
  /// asks). Rows are windowed against the scrolling host, so a
  /// hundred-thousand-entry folder costs the viewport, not the
  /// listing — two spacers keep the scrollbar honest.
  import { onMount } from "svelte";
  import type { Snippet } from "svelte";
  import { ChevronDown, ChevronUp } from "@lucide/svelte";
  import type { FileEntry, SortKey } from "./types";
  import FileRow from "./FileRow.svelte";

  let {
    entries,
    sortKey,
    ascending,
    selectedIndices,
    cursorIndex = null,
    now,
    icon,
    renamingName = null,
    onsort,
    onrowevent,
    onrename,
  }: {
    entries: FileEntry[];
    sortKey: SortKey;
    ascending: boolean;
    selectedIndices: ReadonlySet<number>;
    cursorIndex?: number | null;
    now?: number;
    icon?: Snippet<[FileEntry]>;
    /// The entry name in inline rename, or null.
    renamingName?: string | null;
    onsort?: (key: SortKey) => void;
    onrename?: (entry: FileEntry, newName: string) => void;
    onrowevent?: (
      kind: "click" | "dblclick" | "contextmenu",
      index: number,
      e: MouseEvent,
    ) => void;
  } = $props();

  const COLUMNS: { key: SortKey; label: string; align: "left" | "right" }[] = [
    { key: "name", label: "Name", align: "left" },
    { key: "size", label: "Size", align: "right" },
    { key: "modified", label: "Modified", align: "left" },
  ];

  // Windowing: the row height is the 2rem row box; the visible slice
  // follows the scrolling ancestor with a generous overscan.
  const ROW_PX = 32;
  const OVERSCAN = 24;
  let bodyEl = $state<HTMLDivElement | null>(null);
  let winStart = $state(0);
  let winEnd = $state(200);

  onMount(() => {
    const scroller = bodyEl?.closest(".file-browser");
    if (!(scroller instanceof HTMLElement)) return;
    const update = () => {
      const top = scroller.scrollTop;
      const height = scroller.clientHeight;
      winStart = Math.max(0, Math.floor(top / ROW_PX) - OVERSCAN);
      winEnd = Math.ceil((top + height) / ROW_PX) + OVERSCAN;
    };
    update();
    scroller.addEventListener("scroll", update, { passive: true });
    const observer = new ResizeObserver(update);
    observer.observe(scroller);
    return () => {
      scroller.removeEventListener("scroll", update);
      observer.disconnect();
    };
  });

  const sliceEnd = $derived(Math.min(winEnd, entries.length));
  const slice = $derived(entries.slice(winStart, sliceEnd));
  const padTop = $derived(winStart * ROW_PX);
  const padBottom = $derived(Math.max(0, entries.length - sliceEnd) * ROW_PX);
</script>

<div class="file-list" role="grid" aria-label="Files" aria-rowcount={entries.length}>
  <div class="fl-header" role="row">
    {#each COLUMNS as col (col.key)}
      <button
        class="fl-col"
        class:right={col.align === "right"}
        role="columnheader"
        onclick={() => onsort?.(col.key)}
        aria-sort={sortKey === col.key ? (ascending ? "ascending" : "descending") : undefined}
      >
        {col.label}
        {#if sortKey === col.key}
          {#if ascending}
            <ChevronUp size={12} strokeWidth={2} />
          {:else}
            <ChevronDown size={12} strokeWidth={2} />
          {/if}
        {/if}
      </button>
    {/each}
  </div>

  <div class="fl-body" bind:this={bodyEl}>
    <div style:height="{padTop}px"></div>
    {#each slice as entry, sliceIndex (entry.name)}
      {@const i = winStart + sliceIndex}
      <FileRow
        {entry}
        {now}
        {icon}
        selected={selectedIndices.has(i)}
        focused={cursorIndex === i}
        renaming={renamingName === entry.name}
        onrowclick={(e) => onrowevent?.("click", i, e)}
        onrowdblclick={(e) => onrowevent?.("dblclick", i, e)}
        onrowcontextmenu={(e) => onrowevent?.("contextmenu", i, e)}
        onrename={(newName) => onrename?.(entry, newName)}
      />
    {/each}
    <div style:height="{padBottom}px"></div>
  </div>
</div>

<style>
  .file-list {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .fl-header {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 6rem 9rem;
  }
  @container browser (max-width: 34rem) {
    .fl-header {
      grid-template-columns: minmax(0, 1fr);
    }
    .fl-col:not(:first-child) {
      display: none;
    }
  }
  .fl-header {
    gap: 8px;
    padding: 0 8px;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
    position: sticky;
    top: 0;
    background: var(--background);
    z-index: 1;
  }
  .fl-col {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: var(--height-control, 28px);
    padding: 0;
    border: none;
    background: transparent;
    font-size: 0.75rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    text-align: left;
  }
  .fl-col:hover {
    color: var(--foreground);
  }
  .fl-col.right {
    justify-content: flex-end;
  }
  /* The first column's text aligns with the row names (icon width +
     gap): 8px padding + 16px icon + 8px gap. */
  .fl-col:first-child {
    padding-left: 24px;
  }

  .fl-body {
    display: flex;
    flex-direction: column;
    padding: 4px 0;
  }
</style>
