<script lang="ts">
  /// The Trash view: the home trash contents as a flat list, each entry
  /// restorable to its recorded original location, plus a guarded Empty Trash.
  /// A virtual view (not a browsed folder); the backend trash trio (list /
  /// restore / empty) does the work. The polished surface is an arlen-ui pass;
  /// this is the coder's functional wiring over the built+tested backend.
  import { Trash2, RotateCcw, X } from "lucide-svelte";
  import {
    trashItems,
    closeTrash,
    restoreTrashItem,
    emptyTrash,
    type TrashedItem,
  } from "$lib/stores/trash";

  let confirming = $state(false);
  let busy = $state(false);

  const baseName = (p: string): string => p.split("/").filter(Boolean).pop() ?? p;

  async function restore(item: TrashedItem): Promise<void> {
    busy = true;
    try {
      await restoreTrashItem(item);
    } finally {
      busy = false;
    }
  }

  async function doEmpty(): Promise<void> {
    if (!confirming) {
      confirming = true;
      return;
    }
    confirming = false;
    busy = true;
    try {
      await emptyTrash();
    } finally {
      busy = false;
    }
  }
</script>

<div class="trash-view">
  <div class="tv-head">
    <span class="tv-title">Trash</span>
    <div class="tv-actions">
      <button
        class="tv-empty"
        class:confirming
        disabled={busy || ($trashItems?.length ?? 0) === 0}
        onclick={() => void doEmpty()}
      >
        <Trash2 size={14} strokeWidth={2} />
        {confirming ? "Click to confirm" : "Empty Trash"}
      </button>
      <button class="tv-close" aria-label="Close trash" onclick={() => closeTrash()}>
        <X size={14} strokeWidth={2} />
      </button>
    </div>
  </div>

  {#if $trashItems === null}
    <div class="tv-empty-state">Loading…</div>
  {:else if $trashItems.length === 0}
    <div class="tv-empty-state">Trash is empty</div>
  {:else}
    <ul class="tv-list">
      {#each $trashItems as item (item.trashed_name)}
        <li class="tv-row">
          <div class="tv-info">
            <span class="tv-name">{baseName(item.original_path)}</span>
            <span class="tv-meta">{item.original_path}</span>
            <span class="tv-meta">Deleted {item.deletion_date}</span>
          </div>
          <button
            class="tv-restore"
            disabled={busy}
            onclick={() => void restore(item)}
          >
            <RotateCcw size={13} strokeWidth={2} />
            Restore
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .trash-view {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
  }

  .tv-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .tv-title {
    flex: 1;
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .tv-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .tv-empty {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: var(--height-control, 28px);
    padding: 0 10px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-input);
    background: var(--control-bg);
    color: var(--foreground);
    font-size: 0.75rem;
    font-weight: 500;
  }
  .tv-empty:hover:not(:disabled) {
    background: var(--control-bg-hover);
  }
  .tv-empty:disabled {
    opacity: 0.5;
  }
  .tv-empty.confirming {
    border-color: var(--color-error, #e5484d);
    color: var(--color-error, #e5484d);
  }
  .tv-close {
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
  .tv-close:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }

  .tv-empty-state {
    margin: auto;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .tv-list {
    list-style: none;
    margin: 0;
    padding: 4px 0;
  }
  .tv-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
  }
  .tv-row:hover {
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
  }
  .tv-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .tv-name {
    font-size: 0.8125rem;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tv-meta {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tv-restore {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    flex-shrink: 0;
    height: var(--height-control-compact, 24px);
    padding: 0 9px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-input);
    background: var(--control-bg);
    color: var(--foreground);
    font-size: 0.6875rem;
    font-weight: 500;
  }
  .tv-restore:hover:not(:disabled) {
    background: var(--control-bg-hover);
  }
  .tv-restore:disabled {
    opacity: 0.5;
  }
</style>
