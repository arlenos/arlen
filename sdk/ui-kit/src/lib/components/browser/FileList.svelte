<script lang="ts">
  /// The detail view: a sortable header row over FileRows. Sorting
  /// goes through the controller (the backend sorts, the view only
  /// asks); rows render with content-visibility so huge folders stay
  /// scrollable without windowing machinery.
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
    onsort,
    onrowevent,
  }: {
    entries: FileEntry[];
    sortKey: SortKey;
    ascending: boolean;
    selectedIndices: ReadonlySet<number>;
    cursorIndex?: number | null;
    now?: number;
    icon?: Snippet<[FileEntry]>;
    onsort?: (key: SortKey) => void;
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
</script>

<div class="file-list" role="grid" aria-label="Files">
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

  <div class="fl-body">
    {#each entries as entry, i (entry.name)}
      <div class="fl-rowslot">
        <FileRow
          {entry}
          {now}
          {icon}
          selected={selectedIndices.has(i)}
          focused={cursorIndex === i}
          onrowclick={(e) => onrowevent?.("click", i, e)}
          onrowdblclick={(e) => onrowevent?.("dblclick", i, e)}
          onrowcontextmenu={(e) => onrowevent?.("contextmenu", i, e)}
        />
      </div>
    {/each}
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
  /* Huge folders: rows outside the viewport render lazily; the fixed
     height keeps the scrollbar honest. */
  .fl-rowslot {
    content-visibility: auto;
    contain-intrinsic-size: auto 2rem;
  }
</style>
