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
/// grid has focus. When a SLIDER has focus the HORIZONTAL keys - h, l,
/// Left, Right - flow through to it instead of moving the cursor. The
/// vertical ones keep moving between rows, because the sliders here are
/// horizontal and a person on j still means "the tile below". (This
/// paragraph said "h/j/k/l" until a test asked what Down does, and said
/// the cell opts in until it turned out nothing ever did.)

import type { Action } from "svelte/action";

export interface GridCell {
  /// HTML element to focus when this cell is selected.
  el: HTMLElement;
  /// Column span: 1 (default) or 2 (full row).
  spanCols?: 1 | 2;
  /// The subtree that counts as being "in" this cell, when that is wider than
  /// the element the arrows focus. A quick-settings tile is a container holding
  /// two controls (design-system.md 6.13): the arrows land on the primary one,
  /// and focus sitting on the detail control beside it is still this cell, so
  /// the next arrow moves to the neighbouring tile rather than doing nothing.
  /// Defaults to `el`.
  group?: HTMLElement;
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
  /// Detach handlers; called on component unmount.
  destroy: () => void;
}

/// Whether the horizontal keys belong to what has focus rather than to the grid.
///
/// This was an out-of-band flag a slider tile was supposed to set on focus, and
/// nothing in the tree ever called it - so a person who tabbed onto the volume
/// slider pressed Right and moved to the next tile instead of turning the volume
/// up, because the grid absorbed the key on the way past. Reading the focused
/// element answers the same question and cannot be forgotten by a caller.
function focusOwnsHorizontal(): boolean {
  const active = document.activeElement as HTMLElement | null;
  if (!active) return false;
  if (active.getAttribute("role") === "slider") return true;
  return (
    active.tagName === "INPUT" &&
    (active as HTMLInputElement).type === "range"
  );
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
  return cells.findIndex((c) => {
    const region = c.group ?? c.el;
    return region === active || region.contains(active);
  });
}

/// Attach grid keyboard handling to a container. Returns an API that
/// callers can use to imperatively focus or set slider-mode.
export function attachFocusGrid(
  container: HTMLElement,
  options: FocusGridOptions,
): FocusGridApi {
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

    const sliderMode = focusOwnsHorizontal();

    // Escape: caller-controlled; never absorbed while a slider has focus.
    if (e.key === "Escape" && !sliderMode) {
      options.onEscape?.();
      return;
    }

    // While a slider has focus, h/l and the horizontal arrows flow to it rather
    // than moving the cursor. The vertical ones keep moving between rows,
    // because these sliders are horizontal and a person on j still means "the
    // tile below". Tab still moves focus.
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
        // ONLY when somebody is listening. The grid used to swallow both keys
        // whether or not `onActivate` was wired, and the quick-settings panel -
        // the caller this whole file exists for - wires cells, Escape and help
        // and nothing else. So a person could walk every tile with the arrows
        // and never switch one on: the preventDefault stopped the button
        // underneath from turning the key into a click. With no listener the
        // key belongs to the control that has focus.
        const focused = cells[cur]?.el;
        if (focused && options.onActivate) {
          e.preventDefault();
          options.onActivate(focused);
        }
        break;
      }
    }
  };

  container.addEventListener("keydown", onKey);

  return {
    focus,
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
