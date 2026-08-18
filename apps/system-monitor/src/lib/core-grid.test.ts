import { describe, expect, it } from "vitest";
import { columnsFor } from "./core-grid";

describe("columnsFor", () => {
  /// The defect, as it rendered: 16 cores in a 664x64 strip came out as four
  /// columns of 166x13 - horizontal stripes, not a grid. The rule has to read
  /// the container, not just the count.
  it("fills a wide short strip across rather than down", () => {
    const cols = columnsFor(16, 664, 64);
    expect(cols).toBeGreaterThan(8);
    const cellW = 664 / cols;
    const cellH = 64 / Math.ceil(16 / cols);
    expect(cellW / cellH).toBeLessThan(3);
  });

  it("fills a tall narrow box down rather than across", () => {
    const cols = columnsFor(16, 100, 600);
    expect(cols).toBeLessThanOrEqual(3);
  });

  it("gives a square box a squarish grid", () => {
    expect(columnsFor(16, 400, 400)).toBe(4);
  });

  it("never asks for more columns than there are cores", () => {
    expect(columnsFor(3, 1000, 40)).toBeLessThanOrEqual(3);
  });

  it("caps the width so a many-thread machine does not draw slivers", () => {
    expect(columnsFor(128, 1200, 60)).toBeLessThanOrEqual(16);
  });

  it("survives a container that has not been laid out yet", () => {
    expect(columnsFor(8, 0, 0)).toBeGreaterThanOrEqual(1);
    expect(columnsFor(0, 100, 100)).toBe(1);
  });
});
