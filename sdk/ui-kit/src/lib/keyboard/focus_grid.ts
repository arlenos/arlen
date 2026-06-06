/// Two-column logical grid keyboard navigation.
///
/// Wraps a container element + a list of focusable cell elements. The
/// grid traversal is row-first / column-second:
///   row N: [cells[N*2], cells[N*2+1]]
/// Cells that span two columns (`spanCols = 2`) consume both slots in
/// their row; their right slot is unreachable and j/k jumps to the
/// next single-cell row.
///
/// Bindings:
///   ArrowLeft / h   → previous cell in row
///   ArrowRight / l  → next cell in row
///   ArrowUp / k     → cell directly above
///   ArrowDown / j   → cell directly below
///   Home / g g      → first cell
///   End / G         → last cell
///   Enter / Space   → activate (caller decides, hook only forwards focus)
///   ? (Shift+/)     → toggle help overlay (caller subscribes)
///   Escape          → close (caller subscribes)
///
/// Vim aliases are aliases, not modal: hjkl always navigate when the
/// grid has focus. Slider cells that want to consume h/j/k/l for value
/// adjustment opt in via `setSliderMode(true)` while focused; the hook
/// then treats arrow/vim keys as range-input adjustments instead of
/// grid traversals.

import type { Action } from "svelte/action";

export interface GridCell {
  /// HTML element to focus when this cell is selected.
  el: HTMLElement;
  /// Column span: 1 (default) or 2 (full row).
  spanCols?: 1 | 2;
}

export interface FocusGridOptions {
  /// Cells in render order. The hook re-snapshots this on every key
  /// press so dynamic visibility changes don't force re-init.
  cells: () => GridCell[];
  /// Number of columns (only 2 is supported today; keep the param for
  /// forward compat).
  columns?: 2;
  /// Called when the user presses `?`. Caller toggles help overlay.
  onHelp?: () => void;
  /// Called when the user presses Escape and no slider is in slider-
  /// mode. Caller closes the panel.
  onEscape?: () => void;
  /// Called when the user presses Enter or Space on a cell. Receives
  /// the focused cell's element so the caller can dispatch its click
  /// behaviour.
  onActivate?: (el: HTMLElement) => void;
}

export interface FocusGridApi {
  /// Move keyboard focus to cell `i`. No-op for out-of-range indices.
  focus: (i: number) => void;
  /// Tell the grid that the focused cell wants h/j/k/l to flow through
  /// to its own range input. Slider tiles call this on focusin.
  setSliderMode: (active: boolean) => void;
  /// Detach handlers; called on component unmount.
  destroy: () => void;
}

/// Compute (row, col) for a cell index given the cell list. Two-column
/// cells consume both slots so the next cell starts a new row.
function position(cells: GridCell[], index: number): { row: number; col: number } {
  let row = 0;
  let col = 0;
  for (let i = 0; i < index; i += 1) {
    const span = cells[i]?.spanCols ?? 1;
    col += span;
    if (col >= 2) {
      col = 0;
      row += 1;
    }
  }
  return { row, col };
}

/// Index of the first cell in `row`, or `null` if the row has none.
function firstInRow(cells: GridCell[], row: number): number | null {
  let r = 0;
  let col = 0;
  for (let i = 0; i < cells.length; i += 1) {
    if (r === row) return i;
    const span = cells[i]?.spanCols ?? 1;
    col += span;
    if (col >= 2) {
      col = 0;
      r += 1;
    }
  }
  return null;
}

/// Index of the cell at logical (row, col), or null when no cell sits
/// there. Two-column cells "occupy" col 0 for purposes of vertical
/// traversal — pressing k from a single-cell at col 1 above a 2-col
/// row lands on the 2-col cell.
function indexAt(cells: GridCell[], row: number, col: number): number | null {
  let r = 0;
  let c = 0;
  for (let i = 0; i < cells.length; i += 1) {
    const span = cells[i]?.spanCols ?? 1;
    if (r === row && c <= col && col < c + span) return i;
    c += span;
    if (c >= 2) {
      c = 0;
      r += 1;
    }
  }
  return null;
}

/// Last valid cell index for End / G.
function lastIndex(cells: GridCell[]): number {
  return Math.max(0, cells.length - 1);
}

/// Find the index of the currently-focused cell (or -1).
function activeIndex(cells: GridCell[]): number {
  const active = document.activeElement;
  if (!active) return -1;
  return cells.findIndex((c) => c.el === active || c.el.contains(active));
}

/// Attach grid keyboard handling to a container. Returns an API that
/// callers can use to imperatively focus or set slider-mode.
export function attachFocusGrid(
  container: HTMLElement,
  options: FocusGridOptions,
): FocusGridApi {
  let sliderMode = false;
  let lastG = 0;

  const focus = (i: number) => {
    const cells = options.cells();
    if (i < 0 || i >= cells.length) return;
    cells[i].el.focus();
  };

  const onKey = (e: KeyboardEvent) => {
    const cells = options.cells();
    if (cells.length === 0) return;

    // ?-help: only fire on Shift+/ (printable "?").
    if (e.key === "?") {
      e.preventDefault();
      options.onHelp?.();
      return;
    }

    // Escape: caller-controlled; never absorbed when in slider-mode.
    if (e.key === "Escape" && !sliderMode) {
      options.onEscape?.();
      return;
    }

    // While slider-mode is active, h/j/k/l + arrows flow to the slider
    // input, not the grid. Tab still moves focus.
    if (sliderMode && (e.key === "ArrowLeft" || e.key === "ArrowRight" || e.key === "h" || e.key === "l")) {
      return;
    }

    const cur = activeIndex(cells);
    if (cur < 0) return;
    const { row, col } = position(cells, cur);

    switch (e.key) {
      case "ArrowLeft":
      case "h": {
        e.preventDefault();
        if (col > 0) {
          const target = indexAt(cells, row, col - 1);
          if (target !== null) focus(target);
        }
        break;
      }
      case "ArrowRight":
      case "l": {
        e.preventDefault();
        const target = indexAt(cells, row, col + 1);
        if (target !== null) focus(target);
        break;
      }
      case "ArrowUp":
      case "k": {
        e.preventDefault();
        if (row > 0) {
          const target = indexAt(cells, row - 1, col);
          if (target !== null) focus(target);
        }
        break;
      }
      case "ArrowDown":
      case "j": {
        e.preventDefault();
        const target = indexAt(cells, row + 1, col);
        if (target !== null) focus(target);
        break;
      }
      case "Home": {
        e.preventDefault();
        focus(0);
        break;
      }
      case "End":
      case "G": {
        e.preventDefault();
        focus(lastIndex(cells));
        break;
      }
      case "g": {
        const now = Date.now();
        if (now - lastG < 400) {
          e.preventDefault();
          focus(0);
          lastG = 0;
        } else {
          lastG = now;
        }
        break;
      }
      case "Enter":
      case " ": {
        const focused = cells[cur]?.el;
        if (focused) {
          e.preventDefault();
          options.onActivate?.(focused);
        }
        break;
      }
    }
  };

  container.addEventListener("keydown", onKey);

  return {
    focus,
    setSliderMode: (active) => {
      sliderMode = active;
    },
    destroy: () => {
      container.removeEventListener("keydown", onKey);
    },
  };
}

/// Svelte action wrapper for the common case: bind to a container,
/// re-fetch cells on every key, no init required.
///
/// ```svelte
/// <div use:focusGrid={{ cells: () => myCells, onEscape: closePanel }}>
///   ...
/// </div>
/// ```
export const focusGrid: Action<HTMLElement, FocusGridOptions> = (
  node,
  options,
) => {
  const api = attachFocusGrid(node, options);
  return {
    update(next: FocusGridOptions) {
      options = next;
    },
    destroy() {
      api.destroy();
    },
  };
};
