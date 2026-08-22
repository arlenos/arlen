/// Which page a key press means, and where a page number may land.
///
/// Both rules lived inside `+page.svelte`. They are the two places this reader
/// can be off by one - the ends of a document, and what Shift does to the space
/// bar - and neither could be exercised without rendering the component and
/// dispatching real key events at it. The window's look is another lane's to
/// redo; these are not.

/// What a key press asks the reader to do.
export type PageIntent =
  | { kind: "step"; delta: number }
  | { kind: "first" }
  | { kind: "last" }
  | null;

/// Read a key press as a paging intent, or `null` when it is not one.
///
/// `inInput` is the caller's answer to "is the search box focused", and it wins
/// over everything: there Space and the arrows belong to the text being typed,
/// and a reader mid-word does not expect the page to turn under them.
export function pageIntent(key: string, shiftKey: boolean, inInput: boolean): PageIntent {
  if (inInput) return null;
  switch (key) {
    case "ArrowRight":
    case "ArrowDown":
    case "PageDown":
      return { kind: "step", delta: 1 };
    case "ArrowLeft":
    case "ArrowUp":
    case "PageUp":
      return { kind: "step", delta: -1 };
    case " ":
      // Space forward, Shift+Space back: the convention every reader shares.
      return { kind: "step", delta: shiftKey ? -1 : 1 };
    case "Home":
      return { kind: "first" };
    case "End":
      return { kind: "last" };
    default:
      return null;
  }
}

/// Where a move lands, clamped to the document.
///
/// Clamped rather than wrapping: a reader who presses Right on the last page of
/// a report has reached the end of it, and jumping back to page one reads as the
/// document having restarted.
export function clampPage(current: number, delta: number, pages: number): number {
  return Math.min(Math.max(current + delta, 1), pages);
}
