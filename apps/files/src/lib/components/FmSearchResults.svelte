<script lang="ts">
  /// Search results in place of the listing: the hit's name with its
  /// folder as quiet context, double-click jumps to that folder.
  import {
    entryIcon,
    formatModified,
    joinPath,
  } from "@arlen/ui-kit/components/browser";
  import { searchResults, searchTruncated, closeSearch } from "$lib/stores/search";

  let {
    basePath,
    onjump,
  }: {
    /// The location the search ran under; rel paths resolve from it.
    basePath: string;
    /// Navigate to the hit's folder.
    onjump?: (dirPath: string) => void;
  } = $props();

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

<div class="search-results" role="list" aria-label="Search results">
  {#if $searchResults && $searchResults.length === 0}
    <div class="sr-empty">
      <span class="sr-empty-title">Nothing matches</span>
      <span class="sr-empty-hint">Try fewer letters or different filters.</span>
    </div>
  {/if}
  {#each $searchResults ?? [] as hit (hit.rel_path)}
    {@const Icon = entryIcon(hit.entry)}
    <button class="sr-row" ondblclick={() => jump(hit.rel_path)}>
      <span class="sr-icon"><Icon size={16} strokeWidth={1.75} /></span>
      <span class="sr-name">{hit.entry.name}</span>
      {#if dirOf(hit.rel_path)}
        <span class="sr-dir">{dirOf(hit.rel_path)}</span>
      {/if}
      <span class="sr-meta">{formatModified(hit.entry.modified_unix)}</span>
    </button>
  {/each}
  {#if $searchTruncated}
    <div class="sr-more">Showing the first matches only. Narrow the search.</div>
  {/if}
</div>

<style>
  .search-results {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 8px;
  }

  .sr-row {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 2rem;
    padding: 0 8px;
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    text-align: left;
    transition: background-color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .sr-row:hover {
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
  }

  .sr-icon {
    display: inline-flex;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .sr-name {
    font-size: 0.8125rem;
    color: var(--foreground);
    white-space: nowrap;
  }
  .sr-dir {
    flex: 1;
    min-width: 0;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sr-meta {
    flex-shrink: 0;
    margin-left: auto;
    font-size: 0.75rem;
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
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .sr-empty-hint {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .sr-more {
    padding: 8px;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
