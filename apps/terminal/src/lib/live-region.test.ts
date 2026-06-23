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
    prompt_start_row: null,
    ...over,
  };
}

describe("liveRegionCells (the interactive surface)", () => {
  it("shows the prompt and the typed line at an idle prompt", () => {
    // The grid IS the prompt now (no composer): from prompt_start_row down, so
    // the shell's prompt + the line being typed are the interactive surface. The
    // finished output on row 0 is in its block, above this row, not repainted.
    const cells = [row("(finished output)"), row("~/proj ❯ ec"), row("")];
    const g = grid({ running: false, prompt_start_row: 1, cursor_row: 1, cells });
    const text = liveRegionCells(g).map((r) => r.map((c) => c.text).join("").trimEnd());
    expect(text).toEqual(["~/proj ❯ ec"]);
  });

  it("shows nothing at a prompt before any mark has fired", () => {
    // No prompt_start_row yet (the shell integration has not emitted 133;A), so
    // there is no known interactive region to paint.
    const g = grid({ running: false, prompt_start_row: null, cells: [row("~ ❯ ")] });
    expect(liveRegionCells(g)).toEqual([]);
  });

  it("shows only the running command's output, from output_start_row", () => {
    // Rows 0-1 are the prompt + echoed command; output begins at row 2.
    const cells = [row("~/proj ❯ neofetch"), row(""), row("line-one"), row("line-two")];
    const g = grid({ running: true, output_start_row: 2, cursor_row: 3, cells });
    const text = liveRegionCells(g).map((r) => r.map((c) => c.text).join("").trimEnd());
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

  it("trims trailing blank rows below the active region", () => {
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
    const g = grid({ running: false, prompt_start_row: 0, cells: [row("~ ❯ ")] });
    expect(isAltScreenActive(g, liveRegionCells(g))).toBe(false);
  });
});
