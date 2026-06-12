/// The file-browser selection + keyboard-cursor model: the TS port of
/// `apps/files/core/src/selection.rs`, normative for every UI host
/// (per-keystroke IPC against the Rust model would lag and race).
/// The conformance suite in `selection.test.ts` ports the Rust tests
/// verbatim; behavior changes happen in the Rust model first.
///
/// One deliberate divergence from the Rust model: `remap` carries the
/// selection across a re-sort or hidden-toggle by entry name instead
/// of clearing (Ctrl+H must not eat the user's selection). Real
/// navigation still clears via `rebase`.

export class Selection {
  private count = 0;
  private selected = new Set<number>();
  /// The fixed end of a shift-range (set by the last plain click /
  /// cursor move).
  private anchor: number | null = null;
  /// The keyboard-focused index.
  private cursorIndex: number | null = null;

  constructor(count = 0) {
    this.count = count;
  }

  /// Re-base onto a freshly listed directory of `count` entries: the
  /// previous indices no longer map to the same files, so selection,
  /// anchor and cursor clear. Call on every navigation.
  rebase(count: number): void {
    this.count = count;
    this.selected.clear();
    this.anchor = null;
    this.cursorIndex = null;
  }

  /// Carry the selection across a re-list of the SAME directory
  /// (re-sort, hidden toggle): indices remap through entry names;
  /// names that vanished drop out.
  remap(prevNames: string[], nextNames: string[]): void {
    const index = new Map(nextNames.map((n, i) => [n, i] as const));
    const mapped = new Set<number>();
    for (const i of this.selected) {
      const next = index.get(prevNames[i] ?? "");
      if (next !== undefined) mapped.add(next);
    }
    const mapOne = (i: number | null) =>
      i === null ? null : (index.get(prevNames[i] ?? "") ?? null);
    this.anchor = mapOne(this.anchor);
    this.cursorIndex = mapOne(this.cursorIndex);
    this.selected = mapped;
    this.count = nextNames.length;
  }

  size(): number {
    return this.count;
  }

  isSelected(i: number): boolean {
    return this.selected.has(i);
  }

  /// The selected indices, ascending.
  indices(): number[] {
    return [...this.selected].sort((a, b) => a - b);
  }

  cursor(): number | null {
    return this.cursorIndex;
  }

  /// Plain click: select only `i`, anchor and cursor there. Out-of-
  /// range indices are ignored.
  click(i: number): void {
    if (i >= this.count) return;
    this.selected.clear();
    this.selected.add(i);
    this.anchor = i;
    this.cursorIndex = i;
  }

  /// Ctrl-click: toggle `i` keeping the rest; anchor and cursor move
  /// here so a following shift-range starts at `i`.
  toggle(i: number): void {
    if (i >= this.count) return;
    if (this.selected.has(i)) {
      this.selected.delete(i);
    } else {
      this.selected.add(i);
    }
    this.anchor = i;
    this.cursorIndex = i;
  }

  /// Shift-click: replace the selection with the contiguous range
  /// from the anchor to `i` (inclusive). With no anchor yet, `i`
  /// becomes the anchor. The anchor is preserved across successive
  /// shift-clicks; the cursor follows `i`.
  rangeTo(i: number): void {
    if (i >= this.count) return;
    const anchor = this.anchor ?? i;
    this.anchor = anchor;
    this.cursorIndex = i;
    this.selectRange(anchor, i);
  }

  /// Move the keyboard cursor by `delta` (clamped). With `extend`,
  /// grow the shift-range from the anchor; without, collapse to a
  /// single selection at the new cursor and re-anchor. Hosts pass ±1
  /// (list), ±columns (grid), or ±Infinity for Home/End.
  moveCursor(delta: number, extend: boolean): void {
    if (this.count === 0) return;
    const max = this.count - 1;
    const from = this.cursorIndex ?? 0;
    const next = Math.min(Math.max(from + delta, 0), max);
    this.cursorIndex = next;
    if (extend) {
      const anchor = this.anchor ?? next;
      this.anchor = anchor;
      this.selectRange(anchor, next);
    } else {
      this.selected.clear();
      this.selected.add(next);
      this.anchor = next;
    }
  }

  /// Replace the selection with an arbitrary index set (the marquee:
  /// a grid rectangle is non-contiguous in index space). An extension
  /// over the Rust model — flagged, not silently divergent. `additive`
  /// keeps the existing selection.
  setSelected(indices: number[], additive = false): void {
    if (!additive) this.selected.clear();
    let last: number | null = null;
    for (const i of indices) {
      if (i >= 0 && i < this.count) {
        this.selected.add(i);
        last = i;
      }
    }
    if (last !== null) {
      this.anchor = last;
      this.cursorIndex = last;
    }
  }

  /// Select every entry; anchor at 0, cursor at the last.
  selectAll(): void {
    this.selected = new Set(Array.from({ length: this.count }, (_, i) => i));
    if (this.count > 0) {
      this.anchor = 0;
      this.cursorIndex = this.count - 1;
    }
  }

  /// Clear the selection and the anchor; the cursor stays, so
  /// keyboard focus is not lost on Escape.
  clear(): void {
    this.selected.clear();
    this.anchor = null;
  }

  private selectRange(a: number, b: number): void {
    const [lo, hi] = a <= b ? [a, b] : [b, a];
    this.selected = new Set();
    for (let i = lo; i <= hi && i < this.count; i++) {
      this.selected.add(i);
    }
  }
}
