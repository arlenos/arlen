<script lang="ts">
  /// The Console archetype's cell-grid render area (terminal.md §4.2,
  /// §2.3). Two modes:
  ///
  /// - `cells` set: paint the terminal screen directly in the DOM, one
  ///   fixed-width styled cell per column (terminal.md Option B, the
  ///   portable path - output shows with colour and alignment, without the
  ///   compositor grid-subsurface).
  /// - `cells` unset: reserve a transparent region sized in cell metrics
  ///   for the compositor subsurface to show through, or, when
  ///   `placeholder` is set, paint a labelled stand-in at the reserved
  ///   size (plain-browser dev / the screenshot loop).

  import { type GridCell, cellStyle, trimTrailingPerLine } from "./cell-style";

  let {
    rows = 1,
    cells = null,
    placeholder,
  }: {
    /// Number of terminal text rows to reserve when not painting cells.
    /// The region's height is rows times the cell height derived from
    /// the console mono font (1.5 line-height at 0.8125rem).
    rows?: number;
    /// The screen grid, one inner array per visible row, each holding one
    /// styled cell per column. When set, the region paints these cells.
    cells?: GridCell[][] | null;
    /// When set (and `cells` is not), paint a labelled stand-in instead
    /// of the transparent hole; the string names what the grid would show.
    placeholder?: string;
  } = $props();

  const painted = $derived(Array.isArray(cells));

  /// On copy from the painted grid, strip the per-row trailing-space padding
  /// the grid adds for alignment, so the clipboard carries clean terminal text
  /// (a one-word line does not paste with dozens of trailing spaces). Only fires
  /// when the selection is within this grid; a selection that also spans other
  /// elements bubbles its copy event past this handler and keeps the default.
  function onCopy(event: ClipboardEvent) {
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed || !event.clipboardData) return;
    event.clipboardData.setData("text/plain", trimTrailingPerLine(sel.toString()));
    event.preventDefault();
  }
</script>

<div
  class="grid-region"
  class:mocked={!!placeholder && !painted}
  class:painted
  style:--grid-rows={Math.max(1, rows)}
  aria-hidden={painted ? undefined : "true"}
  data-selectable={painted ? "" : undefined}
  oncopy={painted ? onCopy : undefined}
>
  {#if painted}
    {#each cells ?? [] as row, r (r)}
      <div class="grid-line">{#each row as cell, c (c)}<span
            class="cell"
            class:wide={cell.wide}
            style={cellStyle(cell)}>{cell.text === "" ? " " : cell.text}</span>{/each}</div>
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

  /* Painting the screen directly (Option B): the height is the content's,
     not the reserved metric, and each row is a run of fixed-width monospace
     cells so columns line up exactly. No per-region scroll container: a
     terminal wraps long lines at the grid width (the shell wraps at the PTY
     column count, which the window resize keeps equal to the visible width),
     and the whole view scrolls as scrollback (the stream's own overflow). So
     the grid is exactly as wide as its columns; `overflow-x: hidden` only
     clips a transient over-width before a resize settles, never a scrollbar. */
  .grid-region.painted {
    height: auto;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.8125rem;
    line-height: 1.5;
    color: var(--foreground);
    overflow-x: hidden;
  }
  .grid-line {
    white-space: nowrap;
    min-height: var(--console-cell-h);
  }
  /* Each cell is exactly one character wide (two for a double-width glyph),
     so the grid aligns regardless of glyph advance. */
  .cell {
    display: inline-block;
    width: 1ch;
    height: var(--console-cell-h);
    overflow: hidden;
    vertical-align: top;
    /* Preserve the cell's whitespace so a blank or space cell contributes a
       real space to a text selection. Without this, the browser drops the
       whitespace of a space-only inline-block, and copying terminal output
       loses every space (paths and alignment break). Visually unchanged: the
       cell is still one character wide. */
    white-space: pre;
  }
  .cell.wide {
    width: 2ch;
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
