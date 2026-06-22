import { describe, expect, it } from "vitest";
import {
  type GridCell,
  cellStyle,
  colorOf,
  paletteHex,
  trimTrailingPerLine,
} from "./cell-style";

describe("trimTrailingPerLine (clean terminal copy)", () => {
  it("drops the trailing space padding the grid adds to each row", () => {
    // The grid pads "OS: Arlen OS" out to the full column width; the copy must
    // not carry that. A no-op impl would keep the trailing spaces.
    expect(trimTrailingPerLine("OS:   Arlen OS      ")).toBe("OS:   Arlen OS");
  });
  it("preserves interior whitespace and the line structure", () => {
    expect(trimTrailingPerLine("OS:   Arlen OS  \nKernel:  7.0  ")).toBe(
      "OS:   Arlen OS\nKernel:  7.0",
    );
  });
  it("keeps a blank line blank, not collapsed away", () => {
    expect(trimTrailingPerLine("a  \n   \nb")).toBe("a\n\nb");
  });
  it("trims tabs as well as spaces at the line end", () => {
    expect(trimTrailingPerLine("path\t \t")).toBe("path");
  });
});

const cell = (over: Partial<GridCell>): GridCell => ({
  text: "x",
  fg: { kind: "default" },
  bg: { kind: "default" },
  bold: false,
  italic: false,
  underline: false,
  inverse: false,
  wide: false,
  ...over,
});

describe("paletteHex (the standard xterm 256 palette)", () => {
  it("maps the 16 base ANSI colours", () => {
    expect(paletteHex(0)).toBe("#000000");
    expect(paletteHex(1)).toBe("#cd0000");
    expect(paletteHex(15)).toBe("#ffffff");
  });
  it("maps the 6x6x6 colour cube (16-231)", () => {
    expect(paletteHex(16)).toBe("#000000"); // cube origin
    expect(paletteHex(21)).toBe("#0000ff"); // pure blue corner
    expect(paletteHex(196)).toBe("#ff0000"); // pure red corner
    expect(paletteHex(231)).toBe("#ffffff"); // cube white corner
  });
  it("maps the 24-step greyscale ramp (232-255)", () => {
    expect(paletteHex(232)).toBe("#080808");
    expect(paletteHex(255)).toBe("#eeeeee");
  });
});

describe("colorOf", () => {
  it("returns null for the theme default (so the cell inherits)", () => {
    expect(colorOf({ kind: "default" })).toBeNull();
  });
  it("emits an indexed colour as a themeable var with the palette fallback", () => {
    expect(colorOf({ kind: "indexed", value: 1 })).toBe(
      "var(--term-ansi-1, #cd0000)",
    );
  });
  it("emits a direct RGB triple as a CSS rgb()", () => {
    expect(colorOf({ kind: "rgb", value: [10, 20, 30] })).toBe("rgb(10 20 30)");
  });
});

describe("cellStyle", () => {
  it("paints a foreground colour", () => {
    expect(cellStyle(cell({ fg: { kind: "indexed", value: 1 } }))).toBe(
      "color:var(--term-ansi-1, #cd0000)",
    );
  });
  it("emits nothing for a plain default cell", () => {
    expect(cellStyle(cell({}))).toBe("");
  });
  it("adds SGR weight, slant and underline", () => {
    expect(cellStyle(cell({ bold: true }))).toContain("font-weight:600");
    expect(cellStyle(cell({ italic: true }))).toContain("font-style:italic");
    expect(cellStyle(cell({ underline: true }))).toContain(
      "text-decoration:underline",
    );
  });
  it("swaps foreground and background under inverse", () => {
    // fg=red, bg=default -> after inverse: text takes the (default) background,
    // background takes red. A non-swapping impl would keep color:red.
    const s = cellStyle(cell({ fg: { kind: "indexed", value: 1 }, inverse: true }));
    expect(s).toContain("background:var(--term-ansi-1, #cd0000)");
    expect(s).toContain("color:var(--background, transparent)");
  });
});
