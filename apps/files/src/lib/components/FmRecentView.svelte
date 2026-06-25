<script lang="ts">
  /// The Recent view: the KG's most-recently-accessed files as a flat list,
  /// each opening with the system handler on click. A virtual view (not a
  /// browsed folder); backed by `files_recent`. The shared chrome lives in
  /// `FmVirtualView`; this supplies the rows.
  import { Clock } from "lucide-svelte";
  import { recentItems, closeRecent, type RecentFile } from "$lib/stores/recent";
  import { openPath } from "$lib/adapter";
  import FmVirtualView from "./FmVirtualView.svelte";

  /// Micros-since-epoch to a short locale timestamp; empty for an absent (0) time.
  function when(micros: number): string {
    if (!micros) return "";
    return new Date(micros / 1000).toLocaleString();
  }

  function open(item: RecentFile): void {
    void openPath(item.path);
  }
</script>

<FmVirtualView
  title="Recent"
  onClose={() => closeRecent()}
  loading={$recentItems === null}
  empty={$recentItems !== null && $recentItems.length === 0}
  emptyLabel="No recent files"
>
  <ul class="rv-list">
    {#each $recentItems ?? [] as item (item.path)}
      <li>
        <button class="rv-row" onclick={() => open(item)}>
          <Clock size={14} strokeWidth={2} class="rv-icon" />
          <span class="rv-info">
            <span class="rv-name">{item.name}</span>
            <span class="rv-meta">{item.path}</span>
          </span>
          {#if when(item.accessed)}
            <span class="rv-when">{when(item.accessed)}</span>
          {/if}
        </button>
      </li>
    {/each}
  </ul>
</FmVirtualView>

<style>
  .rv-list {
    list-style: none;
    margin: 0;
    padding: 4px 0;
  }
  .rv-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 12px;
    border: none;
    background: transparent;
    text-align: left;
    color: var(--foreground);
    cursor: pointer;
  }
  .rv-row:hover {
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
  }
  .rv-row :global(.rv-icon) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .rv-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .rv-name {
    font-size: 0.8125rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rv-meta {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rv-when {
    flex-shrink: 0;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    font-variant-numeric: tabular-nums;
  }
</style>
