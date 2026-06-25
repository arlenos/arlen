<script lang="ts">
  /// The Recent view: the KG's most-recently-accessed files as a flat list,
  /// each opening with the system handler on click. A virtual view (not a
  /// browsed folder); backed by `files_recent`. The polished surface is an
  /// arlen-ui pass; this is the coder's functional wiring over the KG read.
  import { Clock, X } from "lucide-svelte";
  import { recentItems, closeRecent, type RecentFile } from "$lib/stores/recent";
  import { openPath } from "$lib/adapter";

  /// Micros-since-epoch to a short locale timestamp; empty for an absent (0) time.
  function when(micros: number): string {
    if (!micros) return "";
    return new Date(micros / 1000).toLocaleString();
  }

  function open(item: RecentFile): void {
    void openPath(item.path);
  }
</script>

<div class="recent-view">
  <div class="rv-head">
    <span class="rv-title">Recent</span>
    <button class="rv-close" aria-label="Close recent" onclick={() => closeRecent()}>
      <X size={14} strokeWidth={2} />
    </button>
  </div>

  {#if $recentItems === null}
    <div class="rv-empty-state">Loading…</div>
  {:else if $recentItems.length === 0}
    <div class="rv-empty-state">No recent files</div>
  {:else}
    <ul class="rv-list">
      {#each $recentItems as item (item.path)}
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
  {/if}
</div>

<style>
  .recent-view {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
  }

  .rv-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .rv-title {
    flex: 1;
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .rv-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control-compact, 24px);
    height: var(--height-control-compact, 24px);
    border: none;
    border-radius: var(--radius-chip);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .rv-close:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }

  .rv-empty-state {
    margin: auto;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

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
