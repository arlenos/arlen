<script lang="ts">
  /// Search results in place of the listing: the hit's name with its
  /// folder as quiet context, double-click jumps to that folder. The
  /// header row sorts the columns the way the file list does (client
  /// side; the backend walk has no order contract).
  import { ChevronDown, ChevronUp } from "lucide-svelte";
  import {
    entryIcon,
    formatModified,
    joinPath,
  } from "@arlen/ui-kit/components/browser";
  import { t } from "$lib/i18n/messages";
  import {
    closeSearch,
    searchAscending,
    searchResults,
    searchFailed,
    searchUnavailable,
    searchSortKey,
    searchTruncated,
    setSearchSort,
    sortHits,
    type SearchSortKey,
  } from "$lib/stores/search";

  let {
    basePath,
    onjump,
  }: {
    /// The location the search ran under; rel paths resolve from it.
    basePath: string;
    /// Navigate to the hit's folder.
    onjump?: (dirPath: string) => void;
  } = $props();

  const COLUMNS: { key: SearchSortKey; labelKey: string }[] = [
    { key: "name", labelKey: "f.results.name" },
    { key: "folder", labelKey: "f.results.folder" },
    { key: "modified", labelKey: "f.results.modified" },
  ];

  const sorted = $derived(
    $searchResults === null
      ? null
      : sortHits($searchResults, $searchSortKey, $searchAscending),
  );

  function dirOf(relPath: string): string {
    const parts = relPath.split("/");
    parts.pop();
    return parts.join("/");
  }

  function jump(relPath: string) {
    const dir = dirOf(relPath);
    onjump?.(dir ? joinPath(basePath, dir) : basePath);
    closeSearch();
  }
</script>

<div class="search-results" role="list" aria-label={$t("f.results.aria")}>
  {#if sorted && sorted.length > 0}
    <!-- A LIST WITH SORT CONTROLS, not a table. These were
         `<button role="columnheader" aria-sort=…>` inside a `role="row"` inside a
         `role="list"`: a button cannot be a columnheader and a row cannot be in a
         list, so the markup claimed two structures at once and was neither.
         The system-monitor process table went the other way on 8 September - it
         became a real grid - and that was right for IT: selection, context menus,
         a tree to expand. This pane is three columns of results you open, with no
         grid navigation to promise, so it stays a list and the sort state moves
         into each button's name where a list can carry it. -->
    <div class="sr-header">
      {#each COLUMNS as col (col.key)}
        <button
          class="sr-col sr-col-{col.key}"
          aria-label={$searchSortKey === col.key
            ? $searchAscending
              ? $t("f.results.sortAsc", { col: $t(col.labelKey) })
              : $t("f.results.sortDesc", { col: $t(col.labelKey) })
            : $t("f.results.sortNone", { col: $t(col.labelKey) })}
          onclick={() => setSearchSort(col.key)}
        >
          {$t(col.labelKey)}
          {#if $searchSortKey === col.key}
            {#if $searchAscending}
              <ChevronUp size={12} strokeWidth={2} />
            {:else}
              <ChevronDown size={12} strokeWidth={2} />
            {/if}
          {/if}
        </button>
      {/each}
    </div>
  {/if}
  {#if sorted && sorted.length === 0}
    <div class="sr-empty">
      <span class="sr-empty-title">{$searchUnavailable
          ? $t("f.results.noHostTitle")
          : $searchFailed
            ? $t("f.results.failedTitle")
            : $t("f.results.emptyTitle")}</span>
      <span class="sr-empty-hint">{$searchUnavailable
          ? $t("f.results.noHostHint")
          : $searchFailed
            ? $t("f.results.failedHint")
            : $t("f.results.emptyHint")}</span>
    </div>
  {/if}
  {#each sorted ?? [] as hit (hit.rel_path)}
    {@const Icon = entryIcon(hit.entry)}
    <!-- ENTER OPENS IT, which it did not until 6 September. The only handler was
         `ondblclick`, so a keyboard could tab to every result and open none of
         them - Enter on a button fires `click`, never `dblclick`. The listitem
         wrapper is `display: contents`, so the row keeps its own grid columns. -->
    <div class="sr-item" role="listitem">
    <button
      class="sr-row"
      ondblclick={() => jump(hit.rel_path)}
      onkeydown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          jump(hit.rel_path);
        }
      }}
    >
      <span class="sr-name-cell">
        <span class="sr-icon"><Icon size={16} strokeWidth={1.75} /></span>
        <span class="sr-name">{hit.entry.name}</span>
      </span>
      <span class="sr-dir">{dirOf(hit.rel_path)}</span>
      <span class="sr-meta">{formatModified(hit.entry.modified_unix)}</span>
    </button>
    </div>
  {/each}
  {#if $searchTruncated}
    <div class="sr-more">{$t("f.results.truncated")}</div>
  {/if}
</div>

<style>
  .search-results {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 8px 4px;
  }

  .sr-header {
    display: grid;
    grid-template-columns: minmax(0, 2fr) minmax(0, 2fr) 9rem;
    gap: 8px;
    padding: 0 8px;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
    position: sticky;
    top: 0;
    background: var(--background);
    z-index: 1;
  }
  .sr-col {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: var(--height-control, 30px);
    padding: 0;
    border: none;
    background: transparent;
    font-size: var(--text-xs);
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    text-align: start;
  }
  .sr-col:hover {
    color: var(--foreground);
  }
  /* The first column's text aligns with the row names (icon + gap). */
  .sr-col-name {
    padding-inline-start: 24px;
  }

  /* `display: contents`, so the listitem wrapper carries the role and the button
     below it keeps being the grid item. Same trick the process table uses for its
     column headers, used the other way round. */
  .sr-item {
    display: contents;
  }
  .sr-row {
    display: grid;
    grid-template-columns: minmax(0, 2fr) minmax(0, 2fr) 9rem;
    align-items: center;
    gap: 8px;
    height: 2rem;
    padding: 0 8px;
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    text-align: start;
    transition: background-color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .sr-row:hover {
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
  }

  .sr-name-cell {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .sr-icon {
    display: inline-flex;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .sr-name {
    font-size: var(--text-sm);
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sr-dir {
    min-width: 0;
    font-size: var(--text-xs);
    color: var(--color-fg-secondary, #a1a1aa);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sr-meta {
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .sr-empty {
    margin: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 2rem;
  }
  .sr-empty-title {
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--foreground);
  }
  .sr-empty-hint {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .sr-more {
    padding: 8px;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
