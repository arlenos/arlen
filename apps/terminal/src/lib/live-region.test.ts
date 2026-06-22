import { describe, it, expect } from "vitest";
import { liveRegionCells, isAltScreenActive } from "./live-region";
import type { GridSnapshot, GridCell } from "$lib/contract";

const def = { kind: "default" as const };
function cell(text: string): GridCell {
  return { text, fg: def, bg: def, bold: false, italic: false, underline: false, inverse: false, wide: false };
}
function row(text: string): GridCell[] {
  return [...text].map(cell);
}
function grid(over: Partial<GridSnapshot>): GridSnapshot {
  return {
    cols: 20,
    rows: 4,
    cells: [],
    alt_screen: false,
    cursor_row: 0,
    cursor_col: 0,
    running: false,
    output_start_row: null,
    ...over,
  };
}

describe("liveRegionCells (the double-prompt gate)", () => {
  it("shows nothing at an idle prompt", () => {
    // The composer is the prompt and finished output is in the blocks, so the
    // shell's own prompt screen is NOT painted live. A naive impl that returns
    // the visible screen would here paint the shell prompt under the composer
    // (the double prompt).
    const g = grid({ running: false, cells: [row("~/proj ❯ "), row("")] });
    expect(liveRegionCells(g)).toEqual([]);
  });

  it("shows only the running command's output, excluding the prompt and echo", () => {
    // Rows 0-1 are the prompt + the echoed command; output begins at row 2.
    const cells = [row("~/proj ❯ neofetch"), row(""), row("line-one"), row("line-two")];
    const g = grid({ running: true, output_start_row: 2, cursor_row: 3, cells });
    const live = liveRegionCells(g);
    const text = live.map((r) => r.map((c) => c.text).join("").trimEnd());
    expect(text).toEqual(["line-one", "line-two"]);
  });

  it("paints the whole grid for a fullscreen alternate-screen app", () => {
    // A TUI owns the screen; every row is kept (no trimming would corrupt its
    // absolute layout), even blank ones.
    const cells = [row("btop header"), row(""), row("cpu 12%"), row("")];
    const g = grid({ alt_screen: true, cells });
    expect(liveRegionCells(g)).toHaveLength(4);
  });

  it("returns nothing for a null snapshot", () => {
    expect(liveRegionCells(null)).toEqual([]);
  });

  it("trims trailing blank rows within a running command's output region", () => {
    const cells = [row("~ ❯ cmd"), row("only-line"), row(""), row("")];
    const g = grid({ running: true, output_start_row: 1, cursor_row: 1, cells });
    expect(liveRegionCells(g)).toHaveLength(1);
  });
});

describe("isAltScreenActive (switch to fullscreen, block UI off)", () => {
  it("is true only when the alternate screen is held and has content", () => {
    const g = grid({ alt_screen: true, cells: [row("x")] });
    expect(isAltScreenActive(g, liveRegionCells(g))).toBe(true);
  });
  it("is false for a normal running command", () => {
    const g = grid({ running: true, output_start_row: 0, cells: [row("out")] });
    expect(isAltScreenActive(g, liveRegionCells(g))).toBe(false);
  });
  it("is false at an idle prompt", () => {
    const g = grid({ running: false, cells: [row("~ ❯ ")] });
    expect(isAltScreenActive(g, liveRegionCells(g))).toBe(false);
  });
});
