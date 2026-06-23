/// The rule for what the live region shows, given the engine's current screen
/// snapshot. Pure so it is unit-testable without a PTY or a render: this is the
/// raw-PTY re-root's render logic, so it carries real regression risk and
/// deserves a deterministic test rather than a flaky live screenshot.
///
/// The live region is the interactive surface (the shell runs in it, the user
/// types into it; there is no composer textbox). Three cases:
/// - A fullscreen / TUI app on the alternate screen owns the whole grid, so the
///   full grid is shown (the caller switches to fullscreen mode; no trimming).
/// - A running command: from where its output begins (`output_start_row`, past
///   the prompt + echoed command line) to the cursor.
/// - An idle prompt: from where the prompt begins (`prompt_start_row`) to the
///   cursor, so the shell's prompt and the line being typed (with zle
///   syntax-highlighting) are shown. Finished commands ABOVE this row stay in
///   their blocks, so they are not painted twice.
/// In both non-alt cases a missing start row (no OSC marks yet) shows nothing.

import type { GridSnapshot, GridCell } from "$lib/contract";

/// The rows the live region paints for `grid` (empty when there is nothing to
/// show live). See the module doc for the three cases.
export function liveRegionCells(grid: GridSnapshot | null): GridCell[][] {
  if (!grid) return [];
  // A fullscreen TUI owns the whole grid; paint it all (no trimming would
  // corrupt its absolute layout).
  if (grid.alt_screen) return grid.cells;
  // The region begins at the current activity: a running command's output, or
  // the prompt the shell drew when idle (so the prompt + typed line show). A
  // null start means no marks have fired yet - nothing to paint live.
  const start = grid.running ? grid.output_start_row : grid.prompt_start_row;
  if (start == null) return [];
  // Trim trailing blank rows (but never below the cursor), so the live region is
  // the height of the real content.
  let last = start - 1;
  for (let i = start; i < grid.cells.length; i++) {
    if (grid.cells[i].some((cell) => cell.text.trim() !== "")) last = i;
  }
  last = Math.max(last, grid.cursor_row);
  if (last < start) return [];
  return grid.cells.slice(start, last + 1);
}

/// Whether the caller should switch to fullscreen (block UI off): a fullscreen
/// TUI holds the alternate screen and has painted something. Mirrors the live
/// cells so an empty alternate screen does not blank the UI.
export function isAltScreenActive(
  grid: GridSnapshot | null,
  liveCells: GridCell[][],
): boolean {
  return (grid?.alt_screen ?? false) && liveCells.length > 0;
}
