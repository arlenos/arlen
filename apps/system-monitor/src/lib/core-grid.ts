/// Layout for the per-core grid.
///
/// Lifted out of the component so the rule can be tested: its first version
/// was wrong in a way nothing but a screenshot could show, and a screenshot is
/// not something CI takes.

/// Columns for `n` cores in a `w` by `h` box: the count that makes the CELLS
/// closest to square, not the grid.
///
/// The first version used `ceil(sqrt(n))` and ignored the container. In a strip
/// 664 wide and 64 tall that put 16 cores into four columns of 166 by 13 - a
/// set of horizontal stripes rather than a grid, which is what the screenshot
/// showed. Folding the aspect in gives eight columns of 83 by 28, where a
/// stacked bar is actually readable.
///
/// Capped at 16 so a 128-thread machine does not draw slivers.
export function columnsFor(n: number, w: number, h: number): number {
  if (n <= 0) return 1;
  if (w <= 0 || h <= 0) return Math.min(16, n);
  // Cell aspect is (w/cols) / (h/rows); square when cols = sqrt(n * w / h).
  const ideal = Math.round(Math.sqrt((n * w) / h));
  return Math.max(1, Math.min(16, Math.min(n, ideal)));
}
