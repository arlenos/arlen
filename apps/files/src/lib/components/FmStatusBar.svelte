<script lang="ts">
  /// The status line: item count and, when something is selected, the
  /// selection count with its total size. Chrome voice, one quiet
  /// row; success is silent, so an empty selection says nothing.
  import {
    formatSize,
    type FileEntry,
  } from "@arlen/ui-kit/components/browser";
  import { t } from "$lib/i18n/messages";

  let {
    entries,
    selected,
    resultsCount = null,
    errored = false,
  }: {
    entries: FileEntry[];
    selected: FileEntry[];
    /// Search is showing: the line counts results, not folder items.
    resultsCount?: number | null;
    /// The listing failed: the bar stays silent (it cannot know a
    /// count it never saw).
    errored?: boolean;
  } = $props();

  const itemsLine = $derived.by(() => {
    if (errored) return null;
    if (resultsCount !== null) {
      return $t("f.status.results", { count: resultsCount });
    }
    return $t("f.status.items", { count: entries.length });
  });

  const selectionLine = $derived.by(() => {
    if (selected.length === 0) return null;
    const bytes = selected.reduce((sum, e) => sum + (e.size ?? 0), 0);
    const count = $t("f.status.selected", { count: selected.length });
    return bytes > 0 ? `${count}, ${formatSize(bytes)}` : count;
  });
</script>

<div class="status-bar">
  {#if itemsLine}
    <span>{itemsLine}</span>
  {/if}
  {#if selectionLine && !errored && resultsCount === null}
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
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    flex-shrink: 0;
  }
</style>
