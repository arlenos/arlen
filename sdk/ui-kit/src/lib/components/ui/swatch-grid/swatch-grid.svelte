<script lang="ts">
  /// A grid of things a person picks one of by look: themes, wallpapers,
  /// palette colours. The grid only lays the tiles out; each tile is a
  /// `Swatch`. Tiles are as wide as `min` allows and the grid fills the row
  /// with as many as fit, so a set of five leaves an even gap and not a hole.
  import type { Snippet } from "svelte";

  let {
    min = "11rem",
    dense = false,
    label,
    children,
  }: {
    /// The narrowest a tile may be before the grid drops a column.
    min?: string;
    /// Compact tiles (a palette): tighter gap, no foot on the tile.
    dense?: boolean;
    /// The group's accessible name.
    label?: string;
    children?: Snippet;
  } = $props();
</script>

<div class="swatch-grid" class:dense role="group" aria-label={label} style={`--swatch-min:${min}`}>
  {@render children?.()}
</div>

<style>
  .swatch-grid {
    /* Full width of whatever holds it, so a flex column that aligns its
       children to the start does not squeeze the grid to one tile. */
    width: 100%;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(var(--swatch-min), 1fr));
    gap: 0.75rem;
  }
  .swatch-grid.dense {
    gap: 0.5rem;
  }
</style>
