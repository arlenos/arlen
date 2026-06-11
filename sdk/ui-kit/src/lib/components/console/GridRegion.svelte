<script lang="ts">
  /// The Console archetype's cell-grid render area (terminal.md §4.2,
  /// §2.3): the DOM reserves a transparent region sized in terminal
  /// cell metrics and paints NOTHING — the compositor's grid
  /// subsurface shows through the hole. The UI never renders terminal
  /// text itself.
  ///
  /// `placeholder` exists for hosts running without the compositor
  /// (the screenshot loop, plain-browser dev): it paints a clearly
  /// labelled stand-in at the exact reserved size so proportions look
  /// real. Production hosts leave it unset.
  let {
    rows = 1,
    placeholder,
  }: {
    /// Number of terminal text rows to reserve. The region's height
    /// is rows times the cell height derived from the console mono
    /// font (1.5 line-height at 0.8125rem).
    rows?: number;
    /// When set, paint a labelled stand-in instead of the transparent
    /// hole; the string names what the grid would show.
    placeholder?: string;
  } = $props();
</script>

<div
  class="grid-region"
  class:mocked={!!placeholder}
  style:--grid-rows={Math.max(1, rows)}
  aria-hidden="true"
>
  {#if placeholder}
    <span class="grid-region-label">{placeholder}</span>
  {/if}
</div>

<style>
  .grid-region {
    /* One cell row = the console line box: 0.8125rem mono at 1.5. */
    --console-cell-h: calc(0.8125rem * 1.5);
    height: calc(var(--grid-rows) * var(--console-cell-h));
    background: transparent;
  }

  /* The labelled stand-in for compositor-less hosts. Dashed inset so
     it cannot be mistaken for real output. */
  .grid-region.mocked {
    display: flex;
    align-items: flex-start;
    border-radius: var(--radius-chip);
    outline: 1px dashed color-mix(in srgb, var(--foreground) 14%, transparent);
    outline-offset: -1px;
    background: color-mix(in srgb, var(--foreground) 2%, transparent);
    padding: 0 8px;
  }

  /* The label sits on the cell metric (one cell row high), so the
     stand-in previews the exact type the grid will paint. */
  .grid-region-label {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.8125rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
