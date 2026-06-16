<script lang="ts">
  /// The Console archetype's cell-grid render area (terminal.md §4.2,
  /// §2.3). Two modes:
  ///
  /// - `lines` set: paint the terminal screen text directly in the DOM,
  ///   one monospace row per string (terminal.md Option B, the portable
  ///   path - output shows without the compositor grid-subsurface).
  /// - `lines` unset: reserve a transparent region sized in cell metrics
  ///   for the compositor subsurface to show through, or, when
  ///   `placeholder` is set, paint a labelled stand-in at the reserved
  ///   size (plain-browser dev / the screenshot loop).
  let {
    rows = 1,
    lines = null,
    placeholder,
  }: {
    /// Number of terminal text rows to reserve when not painting text.
    /// The region's height is rows times the cell height derived from
    /// the console mono font (1.5 line-height at 0.8125rem).
    rows?: number;
    /// The screen text, one string per visible row. When set, the region
    /// paints these as monospace cells instead of reserving a hole.
    lines?: string[] | null;
    /// When set (and `lines` is not), paint a labelled stand-in instead
    /// of the transparent hole; the string names what the grid would show.
    placeholder?: string;
  } = $props();

  const painted = $derived(Array.isArray(lines));
</script>

<div
  class="grid-region"
  class:mocked={!!placeholder && !painted}
  class:painted
  style:--grid-rows={Math.max(1, rows)}
  aria-hidden={painted ? undefined : "true"}
>
  {#if painted}
    {#each lines ?? [] as line, i (i)}
      <div class="grid-line">{line}</div>
    {/each}
  {:else if placeholder}
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

  /* Painting the screen text directly (Option B): the height is the
     content's, not the reserved metric, and the rows are monospace with
     whitespace preserved so columns line up. */
  .grid-region.painted {
    height: auto;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.8125rem;
    line-height: 1.5;
    color: var(--foreground);
    overflow-x: auto;
  }
  .grid-line {
    white-space: pre;
    min-height: var(--console-cell-h);
    tab-size: 8;
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
