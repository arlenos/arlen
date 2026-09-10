// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The quick-settings grid's keyboard model, which had no test: 270 lines
// deciding where the cursor goes for every arrow, every vim alias, a `gg`, a
// span-two tile and a slider that wants h/l for itself. All of it is invisible
// in a screenshot and none of it is reachable by the render probes - the sweeps
// can say a tile is drawn and ringed, never that Down from the tile above lands
// on it.
import { describe, it, expect, beforeEach } from "vitest";
import { attachFocusGrid, type GridCell } from "./focus_grid.js";

function grid(spans: (1 | 2)[]): { container: HTMLElement; cells: GridCell[] } {
  const container = document.createElement("div");
  const cells = spans.map((spanCols) => {
    const el = document.createElement("button");
    container.append(el);
    return { el, spanCols } as GridCell;
  });
  document.body.append(container);
  return { container, cells };
}

function press(container: HTMLElement, key: string): void {
  container.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
}

describe("attachFocusGrid", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });

  it("walks across a row and down a column", () => {
    // Two columns, three rows of single cells: 0 1 / 2 3 / 4 5
    const { container, cells } = grid([1, 1, 1, 1, 1, 1]);
    const api = attachFocusGrid(container, { cells: () => cells });
    api.focus(0);

    press(container, "ArrowRight");
    expect(document.activeElement).toBe(cells[1].el);
    press(container, "ArrowDown");
    expect(document.activeElement).toBe(cells[3].el);
    press(container, "ArrowLeft");
    expect(document.activeElement).toBe(cells[2].el);
    press(container, "ArrowUp");
    expect(document.activeElement).toBe(cells[0].el);
    api.destroy();
  });

  it("takes the vim aliases as the same keys", () => {
    const { container, cells } = grid([1, 1, 1, 1]);
    const api = attachFocusGrid(container, { cells: () => cells });
    api.focus(0);

    press(container, "l");
    expect(document.activeElement).toBe(cells[1].el);
    press(container, "j");
    expect(document.activeElement).toBe(cells[3].el);
    press(container, "h");
    expect(document.activeElement).toBe(cells[2].el);
    press(container, "k");
    expect(document.activeElement).toBe(cells[0].el);
    api.destroy();
  });

  it("stops at the edges rather than wrapping", () => {
    // A grid is not a list: falling off the left edge onto the previous row's
    // right-hand cell would move the cursor somewhere the eye did not go.
    const { container, cells } = grid([1, 1, 1, 1]);
    const api = attachFocusGrid(container, { cells: () => cells });
    api.focus(0);

    press(container, "ArrowLeft");
    expect(document.activeElement).toBe(cells[0].el);
    press(container, "ArrowUp");
    expect(document.activeElement).toBe(cells[0].el);

    api.focus(3);
    press(container, "ArrowRight");
    expect(document.activeElement).toBe(cells[3].el);
    press(container, "ArrowDown");
    expect(document.activeElement).toBe(cells[3].el);
    api.destroy();
  });

  it("treats a two-column tile as a whole row", () => {
    // 0 spans the row; 1 and 2 share the next one.
    const { container, cells } = grid([2, 1, 1]);
    const api = attachFocusGrid(container, { cells: () => cells });
    api.focus(0);

    // Nothing to the right of a tile that already owns both slots.
    press(container, "ArrowRight");
    expect(document.activeElement).toBe(cells[0].el);
    press(container, "ArrowDown");
    expect(document.activeElement).toBe(cells[1].el);
    press(container, "ArrowUp");
    expect(document.activeElement).toBe(cells[0].el);
    api.destroy();
  });

  it("jumps to the ends with Home and End", () => {
    const { container, cells } = grid([1, 1, 1, 1, 1]);
    const api = attachFocusGrid(container, { cells: () => cells });
    api.focus(2);

    press(container, "End");
    expect(document.activeElement).toBe(cells[4].el);
    press(container, "Home");
    expect(document.activeElement).toBe(cells[0].el);
    press(container, "G");
    expect(document.activeElement).toBe(cells[4].el);
    api.destroy();
  });

  it("needs two g presses to go to the top, and a lone g does nothing", () => {
    const { container, cells } = grid([1, 1, 1, 1]);
    const api = attachFocusGrid(container, { cells: () => cells });
    api.focus(3);

    press(container, "g");
    expect(document.activeElement).toBe(cells[3].el);
    press(container, "g");
    expect(document.activeElement).toBe(cells[0].el);
    api.destroy();
  });

  // A cell whose control is a range input: the grid reads what has focus rather
  // than being told, because the telling never happened. Before this, tabbing
  // onto the volume slider and pressing Right moved to the next tile.
  function sliderGrid(): { container: HTMLElement; cells: GridCell[]; slider: HTMLInputElement } {
    const container = document.createElement("div");
    const cells: GridCell[] = [];
    const group = document.createElement("div");
    const slider = document.createElement("input");
    slider.type = "range";
    group.append(slider);
    container.append(group);
    cells.push({ el: slider, group, spanCols: 1 });
    for (let i = 0; i < 3; i += 1) {
      const el = document.createElement("button");
      container.append(el);
      cells.push({ el, spanCols: 1 });
    }
    document.body.append(container);
    return { container, cells, slider };
  }

  it("hands h and l to a slider that has focus", () => {
    // Two rows, so there is somewhere to go DOWN to: a slider owns its own axis
    // and nothing else, which is only visible on a grid that has both.
    const { container, cells, slider } = sliderGrid();
    const api = attachFocusGrid(container, { cells: () => cells });
    api.focus(0);
    expect(document.activeElement).toBe(slider);

    press(container, "l");
    expect(document.activeElement).toBe(slider);
    press(container, "ArrowRight");
    expect(document.activeElement).toBe(slider);
    // Down still moves. The sliders in this kit are horizontal, so the grid
    // keeps the vertical axis - which is what the code does and half a sentence
    // more than its own doc used to say.
    press(container, "ArrowDown");
    expect(document.activeElement).toBe(cells[2].el);

    // And a cell that is not a slider keeps the horizontal axis.
    api.focus(2);
    press(container, "l");
    expect(document.activeElement).toBe(cells[3].el);
    api.destroy();
  });

  it("does not close the panel under Escape while a slider has focus", () => {
    const { container, cells } = sliderGrid();
    let escaped = 0;
    const api = attachFocusGrid(container, {
      cells: () => cells,
      onEscape: () => (escaped += 1),
    });
    api.focus(0);

    press(container, "Escape");
    expect(escaped).toBe(0);

    api.focus(1);
    press(container, "Escape");
    expect(escaped).toBe(1);
    api.destroy();
  });

  it("counts focus anywhere in a cell's group as that cell", () => {
    const container = document.createElement("div");
    const cells: GridCell[] = [];
    for (let i = 0; i < 2; i += 1) {
      const group = document.createElement("div");
      const main = document.createElement("button");
      const second = document.createElement("button");
      group.append(main, second);
      container.append(group);
      cells.push({ el: main, group, spanCols: 1 });
    }
    document.body.append(container);
    const api = attachFocusGrid(container, { cells: () => cells });

    (cells[0].group!.lastElementChild as HTMLElement).focus();
    press(container, "ArrowRight");
    expect(document.activeElement).toBe(cells[1].el);
    api.destroy();
  });

  it("asks for help on ? and answers nothing once detached", () => {
    const { container, cells } = grid([1, 1]);
    let helped = 0;
    const api = attachFocusGrid(container, { cells: () => cells, onHelp: () => (helped += 1) });
    api.focus(0);

    press(container, "?");
    expect(helped).toBe(1);

    api.destroy();
    press(container, "?");
    expect(helped).toBe(1);
  });

  it("says nothing about a grid with no cells", () => {
    const container = document.createElement("div");
    document.body.append(container);
    let escaped = 0;
    const api = attachFocusGrid(container, { cells: () => [], onEscape: () => (escaped += 1) });
    press(container, "Escape");
    press(container, "ArrowDown");
    expect(escaped).toBe(0);
    api.destroy();
  });
});
