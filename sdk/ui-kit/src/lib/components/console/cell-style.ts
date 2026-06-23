/// Pure colour-mapping for the Console GridRegion: turns a terminal cell's
/// `CellColor` and SGR flags into the CSS the DOM paint applies. Extracted from
/// `GridRegion.svelte` so the mapping (the bug-prone part) is unit-testable
/// headlessly, independent of a full-app screenshot.

/// A terminal cell's colour (mirrors the core `CellColor`).
export type CellColor =
  | { kind: "default" }
  | { kind: "indexed"; value: number }
  | { kind: "rgb"; value: [number, number, number] };

/// One visible terminal cell (mirrors the core `GridCell`).
export type GridCell = {
  text: string;
  fg: CellColor;
  bg: CellColor;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  inverse: boolean;
  wide: boolean;
};

/// The 16 base ANSI colours: the Arlen terminal palette. Desaturated and
/// soft for the flat house style, good contrast on the near-black surface
/// without the harsh pure xterm primaries. A theme overrides any slot via
/// the `--term-ansi-N` custom properties; these are the shipped defaults.
const ANSI16 = [
  "#15161b", "#c96a6a", "#8fae74", "#d4b483", "#7d9cc4", "#b08bc4",
  "#83b3b1", "#c8c9cf", "#54565e", "#d98585", "#a6c98a", "#e3c99a",
  "#97b5da", "#c4a0d6", "#9bcac8", "#f2f3f7",
];

/// The hex for a 256-palette index: 0-15 the base ANSI colours, 16-231 the
/// 6x6x6 colour cube, 232-255 the 24-step greyscale ramp (the standard xterm
/// palette). Out-of-range indices fall back to white.
export function paletteHex(n: number): string {
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

/// CSS colour for a `CellColor`, or null for the theme default (so the caller
/// leaves the cell at the inherited foreground/background). An indexed colour
/// is emitted as a `--term-ansi-N` custom property with the xterm palette as
/// fallback, so a theme can override the palette.
export function colorOf(c: CellColor): string | null {
  if (c.kind === "rgb") return `rgb(${c.value[0]} ${c.value[1]} ${c.value[2]})`;
  if (c.kind === "indexed") return `var(--term-ansi-${c.value}, ${paletteHex(c.value)})`;
  return null;
}

/// Trim trailing spaces and tabs from every line, the clean-terminal-copy
/// convention. The grid pads each row to the full column width with space cells
/// so the monospace columns line up; that padding must not travel into the
/// clipboard (it turns a one-word line into a line with dozens of trailing
/// spaces and breaks paste into editors). Interior whitespace and the line
/// structure are preserved (only the run at each line end is dropped).
export function trimTrailingPerLine(text: string): string {
  return text
    .split("\n")
    .map((line) => line.replace(/[ \t]+$/, ""))
    .join("\n");
}

/// The inline `style` string for a cell: foreground, background, and the SGR
/// weight / slant / underline, with `inverse` swapping foreground and
/// background (falling back to the theme colours when a side is default).
export function cellStyle(cell: GridCell): string {
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
