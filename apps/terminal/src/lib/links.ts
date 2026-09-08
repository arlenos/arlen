/// Clickable URLs in terminal output.
///
/// A URL printed by a command is text like any other text: the grid draws it
/// and nothing happens when you point at it. Every terminal a person has used
/// before this one makes that text clickable, so one that does not reads as
/// broken rather than as minimal.
///
/// Two kinds of link reach the grid and only one of them is here. A program that
/// speaks OSC 8 (`ls --hyperlink`, `gcc`, `systemd`) hands xterm the URL out of
/// band and xterm finds it on its own; that path needs a handler, not a scanner.
/// This file is the other kind: a bare URL sitting in the output of a program
/// that knows nothing about hyperlinks, which is most of them.
///
/// Kept pure and off the terminal object so it can be tested without a grid: a
/// string in, offsets out. The wrapping helper below takes the smallest shape of
/// a buffer it can, for the same reason.

/// One URL found in a run of text, as offsets into the string that was scanned.
///
/// Offsets rather than the substring alone because the caller has to point at
/// cells: a range in the grid is what puts the underline under the right
/// characters.
export interface LinkSpan {
  /// Offset of the first character of the URL.
  start: number;
  /// Offset one past the last character.
  end: number;
  /// The URL, already trimmed of the punctuation that ends the sentence rather
  /// than the address.
  url: string;
}

/// The schemes that may be clicked.
///
/// Kept to what `url.rs` accepts. A scheme this matches and the opener refuses
/// would be an underline that does nothing on click - the shape of surface this
/// tree keeps deleting - so the two lists move together or not at all.
const SCHEMES = ["https://", "http://", "mailto:"] as const;

/// A run of non-space characters starting at one of those schemes. Angle
/// brackets and quotes end it because a URL is routinely written inside them.
const URL_RE = /(?:https?:\/\/|mailto:)[^\s<>"']+/gi;

/// Characters that end a sentence, not an address.
const TRAILING = ".,;:!?*_";

/// Bracket pairs a URL is routinely written inside. A closing bracket is dropped
/// when the URL carries no opener for it, so `(see https://example.com/x)` keeps
/// the page and `https://en.wikipedia.org/wiki/Foo_(bar)` keeps its parenthesis.
const PAIRS: ReadonlyArray<readonly [string, string]> = [
  ["(", ")"],
  ["[", "]"],
  ["{", "}"],
];

/// How many times `ch` occurs in `s`.
function countOf(s: string, ch: string): number {
  let n = 0;
  for (const c of s) if (c === ch) n += 1;
  return n;
}

/// Drop the trailing characters that belong to the prose around the URL.
///
/// Runs to a fixed point because the two rules compose: `(https://x/y).` has to
/// lose the full stop before the unbalanced parenthesis is last.
function trimTail(url: string): string {
  let out = url;
  for (;;) {
    const before = out;
    while (out.length > 0 && TRAILING.includes(out[out.length - 1])) {
      out = out.slice(0, -1);
    }
    for (const [open, close] of PAIRS) {
      if (out.endsWith(close) && countOf(out, close) > countOf(out, open)) {
        out = out.slice(0, -1);
      }
    }
    if (out === before) return out;
  }
}

/// Whether anything follows the scheme.
///
/// A match that trims away to its scheme alone is not an address, and
/// underlining it would offer a click that opens nothing.
function hasBody(url: string): boolean {
  const lower = url.toLowerCase();
  for (const scheme of SCHEMES) {
    if (lower.startsWith(scheme)) return url.length > scheme.length;
  }
  return false;
}

/// Every clickable URL in `text`, in the order they appear.
export function findLinks(text: string): LinkSpan[] {
  const out: LinkSpan[] = [];
  URL_RE.lastIndex = 0;
  for (let m = URL_RE.exec(text); m !== null; m = URL_RE.exec(text)) {
    const url = trimTail(m[0]);
    if (!hasBody(url)) continue;
    out.push({ start: m.index, end: m.index + url.length, url });
  }
  return out;
}

/// The least of an xterm buffer line this file needs, so the wrapping helper can
/// be tested with two fields instead of a terminal.
export interface LineLike {
  /// Whether this row continues the one above it.
  isWrapped: boolean;
  /// The row's cells as a string. Called with `false`, so every row is padded to
  /// the full width and one row is exactly one column-count of characters.
  translateToString(trimRight?: boolean): string;
}

/// The least of an xterm buffer this file needs.
export interface BufferLike {
  /// Rows in the buffer, scrollback included.
  length: number;
  /// One row by its zero-based index.
  getLine(index: number): LineLike | undefined;
}

/// One logical line: the rows a wrapped line occupies, joined.
export interface LogicalLine {
  /// The joined rows, each padded to `cols`, so an offset maps to a cell by
  /// division alone.
  text: string;
  /// Zero-based index of the first row.
  firstRow: number;
  /// Columns per row.
  cols: number;
}

/// The whole logical line that row `row` belongs to.
///
/// A URL is long and a terminal is narrow, so the common case is a link split
/// across two rows. Scanning one row at a time would find the half that happens
/// to hold the scheme and offer a click on an address missing its end - worse
/// than no link at all, because it looks like it worked.
export function logicalLine(buffer: BufferLike, row: number): LogicalLine | null {
  if (!buffer.getLine(row)) return null;
  let first = row;
  while (first > 0) {
    const line = buffer.getLine(first);
    if (!line || !line.isWrapped) break;
    first -= 1;
  }
  let text = "";
  let cols = 0;
  for (let y = first; y < buffer.length; y += 1) {
    const line = buffer.getLine(y);
    if (!line) break;
    if (y > first && !line.isWrapped) break;
    const part = line.translateToString(false);
    if (cols === 0) cols = part.length;
    text += part;
  }
  if (cols === 0) return null;
  return { text, firstRow: first, cols };
}

/// A cell position in xterm's own coordinates: both axes one-based.
export interface CellPosition {
  /// Column, counting from one.
  x: number;
  /// Buffer row, counting from one.
  y: number;
}

/// Where offset `offset` of a logical line sits in the grid.
export function cellAt(line: LogicalLine, offset: number): CellPosition {
  return {
    x: (offset % line.cols) + 1,
    y: line.firstRow + Math.floor(offset / line.cols) + 1,
  };
}
