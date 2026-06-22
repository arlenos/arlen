/// The rule for what the live region below the blocks shows, given the engine's
/// current screen snapshot. Pure so it is unit-testable without a PTY or a
/// render: this is the logic that fixes the double prompt (PR-2), so it carries
/// real regression risk and deserves a deterministic test rather than a flaky
/// live screenshot.
///
/// Three cases:
/// - A fullscreen / TUI app on the alternate screen owns the whole grid, so the
///   full grid is shown (the caller switches to fullscreen mode; no trimming).
/// - A running command: only its output, sliced from where the output begins
///   (excluding the shell's prompt and the echoed command line), so the shell's
///   prompt is never painted on top of the block-model composer.
/// - An idle prompt (no command running): nothing. The composer is the prompt,
///   and finished commands' output lives in their blocks.

import type { GridSnapshot, GridCell } from "$lib/contract";

/// The rows the live region paints for `grid` (empty when there is nothing to
/// show live). See the module doc for the three cases.
export function liveRegionCells(grid: GridSnapshot | null): GridCell[][] {
  if (!grid) return [];
  // A fullscreen TUI owns the whole grid; paint it all (no trimming would
  // corrupt its absolute layout).
  if (grid.alt_screen) return grid.cells;
  // Idle prompt: the composer is the prompt, finished output is in the blocks.
  if (!grid.running) return [];
  const start = grid.output_start_row ?? 0;
  // Trim trailing blank rows within the output region (but never below the
  // cursor), so the live region is the height of the streaming output.
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
