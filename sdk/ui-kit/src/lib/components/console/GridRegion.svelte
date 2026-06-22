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

  /// A terminal cell's colour (mirrors the core `CellColor`).
  type CellColor =
    | { kind: "default" }
    | { kind: "indexed"; value: number }
    | { kind: "rgb"; value: [number, number, number] };

  /// One visible terminal cell (mirrors the core `GridCell`).
  type GridCell = {
    text: string;
    fg: CellColor;
    bg: CellColor;
    bold: boolean;
    italic: boolean;
    underline: boolean;
    inverse: boolean;
    wide: boolean;
  };

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

  // The 16 base ANSI colours; 16-255 are computed from the xterm 6x6x6
  // colour cube and the 24-step greyscale ramp. Emitted as the fallback of
  // a `--term-ansi-N` custom property so a theme can override the palette.
  const ANSI16 = [
    "#000000", "#cd0000", "#00cd00", "#cdcd00", "#0000ee", "#cd00cd",
    "#00cdcd", "#e5e5e5", "#7f7f7f", "#ff0000", "#00ff00", "#ffff00",
    "#5c5cff", "#ff00ff", "#00ffff", "#ffffff",
  ];

  function paletteHex(n: number): string {
    if (n < 16) return ANSI16[n] ?? "#ffffff";
    if (n < 232) {
      const i = n - 16;
      const r = Math.floor(i / 36);
      const g = Math.floor(i / 6) % 6;
      const b = i % 6;
      const ch = (v: number) => (v === 0 ? 0 : 55 + v * 40);
      const hh = (v: number) => ch(v).toString(16).padStart(2, "0");
      return `#${hh(r)}${hh(g)}${hh(b)}`;
    }
    const v = 8 + (n - 232) * 10;
    const h = v.toString(16).padStart(2, "0");
    return `#${h}${h}${h}`;
  }

  function colorOf(c: CellColor): string | null {
    if (c.kind === "rgb") return `rgb(${c.value[0]} ${c.value[1]} ${c.value[2]})`;
    if (c.kind === "indexed") return `var(--term-ansi-${c.value}, ${paletteHex(c.value)})`;
    return null;
  }

  function cellStyle(cell: GridCell): string {
    let fg = colorOf(cell.fg);
    let bg = colorOf(cell.bg);
    if (cell.inverse) {
      const f = fg ?? "var(--foreground)";
      const b = bg ?? "var(--background, transparent)";
      fg = b;
      bg = f;
    }
    const parts: string[] = [];
    if (fg) parts.push(`color:${fg}`);
    if (bg) parts.push(`background:${bg}`);
    if (cell.bold) parts.push("font-weight:600");
    if (cell.italic) parts.push("font-style:italic");
    if (cell.underline) parts.push("text-decoration:underline");
    return parts.join(";");
  }
</script>

<div
  class="grid-region"
  class:mocked={!!placeholder && !painted}
  class:painted
  style:--grid-rows={Math.max(1, rows)}
  aria-hidden={painted ? undefined : "true"}
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
     cells so columns line up exactly. */
  .grid-region.painted {
    height: auto;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.8125rem;
    line-height: 1.5;
    color: var(--foreground);
    overflow-x: auto;
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
