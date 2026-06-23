<script lang="ts">
  /// Headless render harness for the terminal grid (coder-jobs PR-2: the render
  /// bugs are frontend, verify by rendering GridRegion with an injected snapshot
  /// in a headless browser + screenshotting - no wry/PTY needed). Mounts
  /// GridRegion with a representative neofetch-like fixture (coloured SGR cells,
  /// a full ANSI palette row, aligned columns) so colour + fixed-width alignment
  /// are directly visible. Not shipped in any nav; a dev/test route only.
  import { GridRegion } from "@arlen/ui-kit/components/console";
  import StreamBlock from "$lib/components/StreamBlock.svelte";
  import type { Block } from "$lib/contract";

  type CellColor =
    | { kind: "default" }
    | { kind: "indexed"; value: number }
    | { kind: "rgb"; value: [number, number, number] };
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

  const COLS = 72;
  const def: CellColor = { kind: "default" };
  const idx = (n: number): CellColor => ({ kind: "indexed", value: n });

  function blank(): GridCell {
    return { text: "", fg: def, bg: def, bold: false, italic: false, underline: false, inverse: false, wide: false };
  }
  function put(row: GridCell[], at: number, ch: string, fg: CellColor, bold = false, bg: CellColor = def) {
    for (let i = 0; i < ch.length && at + i < COLS; i++) {
      row[at + i] = { text: ch[i], fg, bg, bold, italic: false, underline: false, inverse: false, wide: false };
    }
  }
  function emptyRow(): GridCell[] {
    return Array.from({ length: COLS }, blank);
  }
  // A "key: value" line: key in default, value in a colour - tests that colour
  // starts mid-row and columns to the right stay aligned.
  function kv(key: string, value: string, vcol: CellColor): GridCell[] {
    const r = emptyRow();
    put(r, 0, key, def, true);
    put(r, 12, value, vcol);
    return r;
  }
  // The 16 base ANSI colours as solid blocks, to eyeball the palette mapping.
  function paletteRow(): GridCell[] {
    const r = emptyRow();
    for (let n = 0; n < 16; n++) put(r, n * 3, "██", idx(n));
    return r;
  }

  const cells: GridCell[][] = (() => {
    const rows: GridCell[][] = [];
    const title = emptyRow();
    put(title, 0, "tim@arlen", idx(2), true);
    rows.push(title);
    const rule = emptyRow();
    put(rule, 0, "---------", idx(8));
    rows.push(rule);
    rows.push(kv("OS:", "Arlen OS", idx(4)));
    rows.push(kv("Kernel:", "7.0.11-arch1-1", idx(4)));
    rows.push(kv("Shell:", "zsh 5.9", idx(4)));
    rows.push(kv("Terminal:", "arlen-terminal", idx(4)));
    rows.push(kv("Colours:", "red green yellow blue", idx(1)));
    rows.push(emptyRow());
    rows.push(paletteRow());
    rows.push(emptyRow());
    // Alignment ruler: every column boundary must line up under the digits.
    const ruler = emptyRow();
    put(ruler, 0, "0123456789 0123456789 0123456789 0123456789", idx(6));
    rows.push(ruler);
    const bars = emptyRow();
    put(bars, 0, "|....|....|....|....|....|....|....|....|", idx(3));
    rows.push(bars);
    rows.push(emptyRow());
    // Wide (double-width / CJK) glyphs: each is one `wide` cell that must render
    // two columns wide, so the trailing ASCII lines up under the ruler above.
    // This mirrors the engine snapshot, which emits one wide cell per glyph and
    // skips the continuation column (the wide-glyph alignment fix).
    const cjk: GridCell[] = [];
    for (const ch of "日本語ＡＢ") {
      cjk.push({ text: ch, fg: idx(2), bg: def, bold: false, italic: false, underline: false, inverse: false, wide: true });
    }
    for (const ch of " <- 5 wide glyphs end at col 10") {
      cjk.push({ text: ch, fg: idx(7), bg: def, bold: false, italic: false, underline: false, inverse: false, wide: false });
    }
    while (cjk.length < COLS) cjk.push(blank());
    rows.push(cjk);
    return rows;
  })();

  // A finished block carrying the same fixture as its captured output, to verify
  // the "grid inside the block" path: the block frame (command, exit chip, time)
  // plus the per-cell output grid rendered inside it.
  const block: Block = {
    id: "b1",
    command: "neofetch",
    exit_code: 0,
    duration_ms: 42,
    cwd: "/home/tim",
    git: null,
    origin: "you",
    body_kind: "grid",
    body: { cells, rows: cells.length },
  };
</script>

<div style="background:#0a0a0a;padding:8px;min-height:100vh;">
  <GridRegion {cells} />
  <div style="margin-top:16px;max-width:760px;">
    <StreamBlock {block} />
  </div>
</div>
