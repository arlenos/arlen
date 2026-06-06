<script lang="ts">
  /// A responsive grid of read-only `StatTile`s. Tiles reflow to fill the
  /// available width (each stays readable); used for status/metric sections.
  import type { Snippet } from "svelte";

  let { children }: { children?: Snippet } = $props();
</script>

<div class="stat-grid">
  {@render children?.()}
</div>

<style>
  .stat-grid {
    display: grid;
    gap: 0.5rem;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 9rem), 1fr));
    width: 100%;
  }
</style>
