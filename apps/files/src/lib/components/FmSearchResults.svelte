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
    <div class="sr-header" role="row">
      {#each COLUMNS as col (col.key)}
        <button
          class="sr-col sr-col-{col.key}"
          role="columnheader"
          onclick={() => setSearchSort(col.key)}
          aria-sort={$searchSortKey === col.key
            ? $searchAscending
              ? "ascending"
              : "descending"
            : undefined}
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
      <span class="sr-empty-title">{$t("f.results.emptyTitle")}</span>
      <span class="sr-empty-hint">{$t("f.results.emptyHint")}</span>
    </div>
  {/if}
  {#each sorted ?? [] as hit (hit.rel_path)}
    {@const Icon = entryIcon(hit.entry)}
    <button class="sr-row" ondblclick={() => jump(hit.rel_path)}>
      <span class="sr-name-cell">
        <span class="sr-icon"><Icon size={16} strokeWidth={1.75} /></span>
        <span class="sr-name">{hit.entry.name}</span>
      </span>
      <span class="sr-dir">{dirOf(hit.rel_path)}</span>
      <span class="sr-meta">{formatModified(hit.entry.modified_unix)}</span>
    </button>
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
    height: var(--height-control, 28px);
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
    padding-left: 24px;
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
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
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
