//! The terminal screen model on a PROVEN VT core (`alacritty_terminal`).
//!
//! The hand-rolled vt100-based screen kept getting the hard parts wrong on metal
//! (alt-screen, cursor, line-wrap, resize-reflow). This wraps `alacritty_terminal`
//! - the reference embeddable Rust VT engine - so those are correct BY
//! CONSTRUCTION: PTY bytes are fed through its `vte::ansi::Processor` into a
//! `Term`, and the host reads back the visible grid, cursor, and alt-screen state
//! as the contract [`GridSnapshot`]. Resize reflows the grid. This is built and
//! tested in isolation alongside the current path; the engine swaps onto it next.

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, TermMode};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Processor};
use alacritty_terminal::Term;
use arlen_terminal_core::{CellColor, GridCell, GridSnapshot};

/// The Term needs an event listener; the host drives I/O itself (PTY read/write
/// and resize), so terminal-originated events (bell, title, clipboard) are
/// dropped - the default `send_event` is a no-op.
struct NoEvents;
impl EventListener for NoEvents {
    fn send_event(&self, _event: Event) {}
}

/// A screen size as the [`Dimensions`] the Term + resize take. Defined here
/// rather than using the crate's test-only `TermSize` so it is available in a
/// release build.
#[derive(Clone, Copy)]
struct Size {
    cols: usize,
    lines: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// The VT screen: a real terminal grid driven by `alacritty_terminal`.
pub struct Screen {
    term: Term<NoEvents>,
    parser: Processor,
}

impl Screen {
    /// A fresh screen of `cols` x `rows` (both clamped to at least 1).
    pub fn new(cols: u16, rows: u16) -> Self {
        let size = Size { cols: cols.max(1) as usize, lines: rows.max(1) as usize };
        Self { term: Term::new(Config::default(), &size, NoEvents), parser: Processor::new() }
    }

    /// Feed PTY output bytes into the parser, advancing the screen.
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    /// Resize the grid to `cols` x `rows`; `alacritty_terminal` reflows wrapped
    /// lines and clamps the cursor (the resize-reflow that broke before).
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(Size { cols: cols.max(1) as usize, lines: rows.max(1) as usize });
    }

    /// Whether a fullscreen TUI holds the alternate screen.
    pub fn alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    /// The visible screen as the contract [`GridSnapshot`]. Fills the grid +
    /// geometry + cursor + alt-screen; the OSC133 overlay fields (`running`,
    /// `output_start_row`, `prompt_start_row`) are the engine's to set when it
    /// wraps this, so they default here.
    pub fn snapshot(&self) -> GridSnapshot {
        let grid = self.term.grid();
        let cols = grid.columns();
        let rows = grid.screen_lines();
        let mut cells = Vec::with_capacity(rows);
        for r in 0..rows {
            let row = &grid[Line(r as i32)];
            let mut line = Vec::with_capacity(cols);
            for c in 0..cols {
                let cell = &row[Column(c)];
                let flags = cell.flags;
                // The trailing half of a wide glyph is a spacer: emit nothing so
                // the glyph occupies its two columns once (the lead cell is `wide`).
                let text = if flags.contains(Flags::WIDE_CHAR_SPACER) {
                    String::new()
                } else {
                    cell.c.to_string()
                };
                line.push(GridCell {
                    text,
                    fg: conv_color(cell.fg),
                    bg: conv_color(cell.bg),
                    bold: flags.contains(Flags::BOLD),
                    italic: flags.contains(Flags::ITALIC),
                    underline: flags.contains(Flags::UNDERLINE),
                    inverse: flags.contains(Flags::INVERSE),
                    wide: flags.contains(Flags::WIDE_CHAR),
                });
            }
            cells.push(line);
        }
        let cursor = grid.cursor.point;
        GridSnapshot {
            cols: cols as u16,
            rows: rows as u16,
            cells,
            alt_screen: self.term.mode().contains(TermMode::ALT_SCREEN),
            cursor_row: cursor.line.0.max(0) as u16,
            cursor_col: cursor.column.0 as u16,
            running: false,
            output_start_row: None,
            prompt_start_row: None,
        }
    }
}

/// Map an `alacritty_terminal` cell colour onto the contract [`CellColor`].
/// A spec is a direct RGB; an indexed is the 256-palette index; a named colour
/// is the default fg/bg (so the theme paints it) or, for the 16 ANSI names, the
/// matching palette index.
fn conv_color(c: AnsiColor) -> CellColor {
    match c {
        AnsiColor::Spec(rgb) => CellColor::Rgb([rgb.r, rgb.g, rgb.b]),
        AnsiColor::Indexed(i) => CellColor::Indexed(i),
        AnsiColor::Named(named) => match named {
            NamedColor::Foreground | NamedColor::Background => CellColor::Default,
            other => {
                // NamedColor's first 16 discriminants are the ANSI palette
                // (Black=0 .. BrightWhite=15); anything past that (Dim*, etc.)
                // falls back to the theme default rather than a wrong index.
                let idx = other as usize;
                if idx < 16 {
                    CellColor::Indexed(idx as u8)
                } else {
                    CellColor::Default
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_text(snap: &GridSnapshot, r: usize) -> String {
        snap.cells[r].iter().map(|c| c.text.as_str()).collect::<String>().trim_end().to_string()
    }

    #[test]
    fn plain_text_lands_on_the_grid() {
        let mut s = Screen::new(20, 4);
        s.process(b"hello");
        let snap = s.snapshot();
        assert_eq!(snap.cols, 20);
        assert_eq!(snap.rows, 4);
        assert_eq!(row_text(&snap, 0), "hello");
    }

    #[test]
    fn a_newline_advances_the_row_and_cursor() {
        let mut s = Screen::new(20, 4);
        s.process(b"one\r\ntwo");
        let snap = s.snapshot();
        assert_eq!(row_text(&snap, 0), "one");
        assert_eq!(row_text(&snap, 1), "two");
        assert_eq!(snap.cursor_row, 1, "the cursor is on the second row");
        assert_eq!(snap.cursor_col, 3, "after 'two'");
    }

    #[test]
    fn sgr_colour_and_attributes_flow_through() {
        let mut s = Screen::new(20, 2);
        // bold red 'X' then reset.
        s.process(b"\x1b[1;31mX\x1b[0m");
        let snap = s.snapshot();
        let cell = &snap.cells[0][0];
        assert_eq!(cell.text, "X");
        assert!(cell.bold, "SGR 1 is bold");
        assert_eq!(cell.fg, CellColor::Indexed(1), "SGR 31 is ANSI red (index 1)");
    }

    #[test]
    fn truecolor_spec_maps_to_rgb() {
        let mut s = Screen::new(10, 1);
        s.process(b"\x1b[38;2;10;20;30mZ");
        assert_eq!(snapshot_fg(&s), CellColor::Rgb([10, 20, 30]));
    }
    fn snapshot_fg(s: &Screen) -> CellColor {
        s.snapshot().cells[0][0].fg
    }

    #[test]
    fn the_alternate_screen_is_detected_and_restored() {
        let mut s = Screen::new(20, 4);
        assert!(!s.alt_screen(), "primary screen is not alternate");
        s.process(b"\x1b[?1049h");
        assert!(s.alt_screen(), "DECSET 1049 enters the alternate screen");
        s.process(b"\x1b[?1049l");
        assert!(!s.alt_screen(), "DECRST 1049 restores the primary screen");
    }

    #[test]
    fn resize_changes_the_geometry() {
        let mut s = Screen::new(80, 24);
        let before = s.snapshot();
        assert_eq!((before.cols, before.rows), (80, 24));
        s.resize(100, 40);
        let after = s.snapshot();
        assert_eq!((after.cols, after.rows), (100, 40), "the grid tracks the resize");
    }

    #[test]
    fn a_wide_glyph_occupies_two_columns_once() {
        let mut s = Screen::new(10, 1);
        s.process("\u{5b57}x".as_bytes()); // 字 (width 2) then x
        let snap = s.snapshot();
        assert_eq!(snap.cells[0][0].text, "\u{5b57}");
        assert!(snap.cells[0][0].wide, "the lead cell is wide");
        assert_eq!(snap.cells[0][1].text, "", "the spacer half emits nothing");
        assert_eq!(snap.cells[0][2].text, "x", "the next glyph is at column 2");
    }
}
