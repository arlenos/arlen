<script lang="ts">
  /// The status line: item count and, when something is selected, the
  /// selection count with its total size. Chrome voice, one quiet
  /// row; success is silent, so an empty selection says nothing.
  import {
    formatSize,
    type FileEntry,
  } from "@arlen/ui-kit/components/browser";

  let {
    entries,
    selected,
  }: {
    entries: FileEntry[];
    selected: FileEntry[];
  } = $props();

  const itemsLine = $derived(
    entries.length === 1 ? "1 item" : `${entries.length} items`,
  );

  const selectionLine = $derived.by(() => {
    if (selected.length === 0) return null;
    const bytes = selected.reduce((sum, e) => sum + (e.size ?? 0), 0);
    const count =
      selected.length === 1 ? "1 selected" : `${selected.length} selected`;
    return bytes > 0 ? `${count}, ${formatSize(bytes)}` : count;
  });
</script>

<div class="status-bar">
  <span>{itemsLine}</span>
  {#if selectionLine}
    <span>{selectionLine}</span>
  {/if}
</div>

<style>
  .status-bar {
    display: flex;
    align-items: center;
    gap: 16px;
    height: var(--height-control, 28px);
    padding: 0 16px;
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
    font-size: 0.75rem;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    flex-shrink: 0;
  }
</style>
