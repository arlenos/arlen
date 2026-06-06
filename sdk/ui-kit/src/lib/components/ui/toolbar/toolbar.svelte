<script lang="ts">
  /// Predictable action row: `start` content (left, grows + truncates) and
  /// `end` content (right, fixed) with consistent spacing — so page/section
  /// headers (title + search + buttons) stop being hand-rolled flex markup.
  /// Falls back to plain `children` when the start/end split is not needed.
  import type { Snippet } from "svelte";

  let {
    start,
    end,
    children,
    class: className,
  }: {
    start?: Snippet;
    end?: Snippet;
    children?: Snippet;
    class?: string;
  } = $props();
</script>

<div class="toolbar {className ?? ''}">
  {#if start || end}
    <div class="toolbar-start">{@render start?.()}</div>
    <div class="toolbar-end">{@render end?.()}</div>
  {:else}
    {@render children?.()}
  {/if}
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .toolbar-start {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .toolbar-end {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
</style>
