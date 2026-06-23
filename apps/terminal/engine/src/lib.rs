//! The concrete VT engine: a shell (`zsh` by default) on a `portable-pty` PTY,
//! with a reader thread that streams the PTY output through the core
//! [`arlen_terminal_core::vt::OscScanner`] into [`VtEvent`]s. It implements the
//! core [`VtEngine`] seam, so a host (the terminal app, or the file manager's
//! embedded pane) drives it without depending on the pty backend.
//!
//! The grid STATE (a `wezterm-term`/`termwiz` screen model) is deliberately NOT
//! wired here. Its purpose is to feed the cosmic-comp grid-subsurface render,
//! which is the cross-repo compositor piece (terminal.md §2.2, "TM-R1
//! cross-repo"), deferred and not on this path; `wezterm-term` is also not
//! published to crates.io, so it would need a git dependency, taken when the
//! render work lands. What lives here is the non-cross-repo backend: spawn the
//! shell, surface its low-rate OSC marks as `VtEvent`s through the audited
//! scanner, and drive input/resize. That is complete and testable on its own.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use arlen_terminal_core::vt::{OscScanner, VtEngine, VtEvent};
use arlen_terminal_core::{CellColor, GridCell, GridSnapshot};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

/// The environment variable the engine sets to the per-session nonce. The shell
/// integration script (TM-R3) reads it so its OSC `633;E` command marks carry the
/// secret that proves they came from the trusted shell, not from forged output.
pub const NONCE_ENV: &str = "ARLEN_TERM_NONCE";

/// Env var naming the curated-zsh config dir to set as `ZDOTDIR` for a spawned
/// shell, so its `.zshrc` sources the TM-R3 integration script. The host or the
/// production install points it at the curated zsh directory; unset means the
/// shell uses its normal startup.
pub const ZDOTDIR_ENV: &str = "ARLEN_TERM_ZDOTDIR";

/// How many rows tall the per-command output capture parser is. A command's
/// output is fed into its own VT parser (the "grid inside the block") so it is
/// preserved in full, independent of the small visible screen; this caps a
/// pathological flood (output beyond this scrolls off the captured grid, the
/// same way a real terminal without unbounded scrollback drops the oldest rows).
const BLOCK_OUTPUT_ROWS: u16 = 600;

/// Env var the engine sets to the user's REAL config dir when it overrides
/// `ZDOTDIR` with the curated one. The curated config restores this (or `$HOME`)
/// and sources the user's own `.zshrc` before the integration, so the marks fire
/// without replacing the user's zsh setup. Must match the name the curated
/// `.zshenv`/`.zprofile`/`.zshrc` read.
pub const USER_ZDOTDIR_ENV: &str = "ARLEN_USER_ZDOTDIR";

/// Map any backend error into an `io::Error` for the [`VtEngine`] seam.
fn io_err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

/// A shell running on a PTY, surfacing its OSC marks as [`VtEvent`]s and taking
/// input/resize through the [`VtEngine`] seam.
pub struct PtyEngine {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    events: Arc<Mutex<Vec<VtEvent>>>,
    /// The visible-screen VT model, fed the same PTY byte stream the scanner
    /// reads. The host snapshots it ([`PtyEngine::screen_snapshot`]) to render
    /// output in the webview (terminal.md Option B).
    screen: Arc<Mutex<vt100::Parser>>,
    /// Whether a command is running (its `ExecStart` mark seen, `CommandEnd` not
    /// yet). Tracked from the same OSC-mark stream the scanner lifts, so the
    /// snapshot can tell an in-flight command from an idle prompt without the
    /// host's assembler. Set in the reader thread, read in `screen_snapshot`.
    running: Arc<AtomicBool>,
    /// The grid row where the running command's output begins: the cursor row at
    /// the moment its `ExecStart` mark fired (past the prompt and command echo).
    /// `None` at an idle prompt. Lets the renderer slice the live grid to the
    /// output region so the shell's prompt is not drawn under the composer.
    output_start_row: Arc<Mutex<Option<u16>>>,
    /// The grid row where the current prompt begins: the cursor row at the moment
    /// the `PromptStart` (OSC 133;A) mark fired, cleared at `ExecStart`. `None`
    /// while a command runs or before the first marked prompt. Lets the raw-PTY
    /// renderer show the live prompt + the line being typed as the interactive
    /// surface, while finished output above it stays in its blocks.
    prompt_start_row: Arc<Mutex<Option<u16>>>,
    /// Captured output grids of commands that have finished, in finish order. The
    /// reader thread feeds each command's output bytes (between its `ExecStart`
    /// and `CommandEnd` marks) into a dedicated VT parser and pushes the trimmed
    /// snapshot here on close; the host drains it ([`PtyEngine::take_finished_outputs`])
    /// and attaches each to its block so the block renders its own output.
    finished_outputs: Arc<Mutex<Vec<GridSnapshot>>>,
    reader: Option<JoinHandle<()>>,
}

impl PtyEngine {
    /// Spawn `zsh` on a fresh PTY of `cols` x `rows`, in `cwd` (or the inherited
    /// directory). See [`PtyEngine::spawn`].
    pub fn spawn_zsh(cwd: Option<&str>, cols: u16, rows: u16) -> std::io::Result<Self> {
        Self::spawn("zsh", &[], cwd, cols, rows)
    }

    /// Spawn `program` with `args` on a fresh PTY sized `cols` x `rows`, in `cwd`.
    ///
    /// A fresh CSPRNG nonce is minted, exported as [`NONCE_ENV`] for the shell
    /// integration to stamp into its command marks, and used to gate the scanner -
    /// so the nonce never leaves this process except into the child shell's
    /// environment (the host that drives the engine never sees it and so cannot
    /// leak it). `TERM=xterm-256color` (terminal.md §2.1) is set so a remote host
    /// without an Arlen terminfo still works.
    pub fn spawn(
        program: &str,
        args: &[&str],
        cwd: Option<&str>,
        cols: u16,
        rows: u16,
    ) -> std::io::Result<Self> {
        let nonce = mint_nonce()?;
        let pty = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(io_err)?;

        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        cmd.env("TERM", "xterm-256color");
        cmd.env(NONCE_ENV, &nonce);
        // Point the spawned shell at the curated zsh config dir when one is
        // provided: its `.zshrc` sources the user's own `.zshrc` and then the
        // TM-R3 integration script, which (seeing the nonce above) emits the OSC
        // 133/633 block marks this engine scans. The user's real config dir is
        // forwarded as `ARLEN_USER_ZDOTDIR` so the curated config restores it
        // (the marks fire without dropping the user's setup). Without the override
        // the shell uses its normal startup (in production the system-installed
        // curated zshrc sources the integration); the engine stays silent rather
        // than guessing a path.
        if let Some(zdotdir) = std::env::var_os(ZDOTDIR_ENV) {
            if let Some(user_zdotdir) = std::env::var_os("ZDOTDIR") {
                cmd.env(USER_ZDOTDIR_ENV, user_zdotdir);
            }
            cmd.env("ZDOTDIR", zdotdir);
        }
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }
        let child = pty.slave.spawn_command(cmd).map_err(io_err)?;
        // Drop our extra slave handle: once the child holds the slave, closing
        // this one means the master reader sees EOF exactly when the child exits.
        drop(pty.slave);

        let mut reader = pty.master.try_clone_reader().map_err(io_err)?;
        let writer = pty.master.take_writer().map_err(io_err)?;

        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        // The screen model gets the SAME byte stream the scanner does: the
        // scanner lifts the low-rate OSC marks, the parser builds the visible
        // grid the webview renders. No scrollback for now (the visible screen
        // is what shows); scrollback is a later addition.
        let screen = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 0)));
        let screen_sink = Arc::clone(&screen);
        let running = Arc::new(AtomicBool::new(false));
        let running_sink = Arc::clone(&running);
        let output_start_row = Arc::new(Mutex::new(None));
        let output_start_sink = Arc::clone(&output_start_row);
        let prompt_start_row = Arc::new(Mutex::new(None));
        let prompt_start_sink = Arc::clone(&prompt_start_row);
        let finished_outputs = Arc::new(Mutex::new(Vec::new()));
        let finished_sink = Arc::clone(&finished_outputs);
        let reader_handle = std::thread::Builder::new()
            .name("arlen-pty-reader".into())
            .spawn(move || {
                let mut scanner = OscScanner::new(nonce);
                let mut buf = [0u8; 4096];
                // The active command's output parser: created at `ExecStart`, fed
                // the output bytes that follow, finalized at `CommandEnd` (or at
                // the next prompt, for a command that emitted no end mark).
                let mut block_parser: Option<vt100::Parser> = None;
                loop {
                    match reader.read(&mut buf) {
                        // 0 = clean EOF; Err = the master closed (Linux returns
                        // EIO when the slave goes away). Either ends the loop.
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            // Process the screen in segments split at each OSC
                            // mark, so the command-output region is resolved
                            // precisely even when a mark shares a read buffer with
                            // the output it precedes. `ExecStart` captures the
                            // cursor row at that point (output begins there, past
                            // the prompt + echoed command); `PromptStart` clears it
                            // (idle); `CommandEnd` ends the run. Sampling between
                            // segments means the cursor reflects bytes up to the
                            // mark, never the not-yet-processed output after it.
                            let positioned = scanner.feed_positioned(&buf[..n]);
                            if let Ok(mut p) = screen_sink.lock() {
                                let mut at = 0usize;
                                for (off, ev) in &positioned {
                                    let seg = &buf[at..*off];
                                    p.process(seg);
                                    // Output bytes before this mark belong to the
                                    // running command (the parser is `Some` only
                                    // between its ExecStart and CommandEnd, so the
                                    // prompt + echo segment is never captured).
                                    if let Some(bp) = block_parser.as_mut() {
                                        bp.process(seg);
                                    }
                                    at = *off;
                                    match ev {
                                        VtEvent::ExecStart => {
                                            running_sink.store(true, Ordering::Relaxed);
                                            if let Ok(mut o) = output_start_sink.lock() {
                                                *o = Some(p.screen().cursor_position().0);
                                            }
                                            // The prompt is done; output begins.
                                            if let Ok(mut ps) = prompt_start_sink.lock() {
                                                *ps = None;
                                            }
                                            // A fresh, tall parser captures this
                                            // command's output in full; cols match
                                            // the screen so wrapping is identical.
                                            let cols = p.screen().size().1;
                                            block_parser = Some(vt100::Parser::new(
                                                BLOCK_OUTPUT_ROWS,
                                                cols,
                                                0,
                                            ));
                                        }
                                        VtEvent::CommandEnd { .. } => {
                                            running_sink.store(false, Ordering::Relaxed);
                                            if let Some(bp) = block_parser.take() {
                                                if let Ok(mut outs) = finished_sink.lock() {
                                                    outs.push(trim_trailing_blank_rows(snapshot_of(
                                                        &bp,
                                                    )));
                                                }
                                            }
                                        }
                                        VtEvent::PromptStart => {
                                            running_sink.store(false, Ordering::Relaxed);
                                            if let Ok(mut o) = output_start_sink.lock() {
                                                *o = None;
                                            }
                                            // The new prompt begins at the cursor's
                                            // current row; the live region renders
                                            // from here (prompt + typed line).
                                            if let Ok(mut ps) = prompt_start_sink.lock() {
                                                *ps = Some(p.screen().cursor_position().0);
                                            }
                                            // A command that reached the next prompt
                                            // without an end mark still has its
                                            // captured output preserved.
                                            if let Some(bp) = block_parser.take() {
                                                if let Ok(mut outs) = finished_sink.lock() {
                                                    outs.push(trim_trailing_blank_rows(snapshot_of(
                                                        &bp,
                                                    )));
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                let tail = &buf[at..n];
                                p.process(tail);
                                if let Some(bp) = block_parser.as_mut() {
                                    bp.process(tail);
                                }
                            }
                            if !positioned.is_empty() {
                                if let Ok(mut q) = sink.lock() {
                                    q.extend(positioned.into_iter().map(|(_, ev)| ev));
                                }
                            }
                        }
                    }
                }
            })?;

        Ok(Self {
            master: pty.master,
            writer,
            child,
            events,
            screen,
            running,
            output_start_row,
            prompt_start_row,
            finished_outputs,
            reader: Some(reader_handle),
        })
    }

    /// Drain the captured output grids of commands that finished since the last
    /// call, in finish order. The host attaches each to the matching block so the
    /// block renders its own output (the grid-inside-the-block), decoupled from
    /// the small live screen. Not part of the [`VtEngine`] seam: it is specific to
    /// a parser-backed engine, and a mock has no output to capture.
    pub fn take_finished_outputs(&self) -> Vec<GridSnapshot> {
        self.finished_outputs
            .lock()
            .map(|mut v| std::mem::take(&mut *v))
            .unwrap_or_default()
    }
}

impl VtEngine for PtyEngine {
    fn send_input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    fn resize(&mut self, cols: u16, rows: u16) -> std::io::Result<()> {
        // Keep the screen model's geometry in step with the PTY so wrapping and
        // the cursor stay correct after a resize.
        if let Ok(mut p) = self.screen.lock() {
            p.set_size(rows, cols);
        }
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(io_err)
    }

    fn drain_events(&mut self) -> Vec<VtEvent> {
        self.events
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default()
    }

    fn screen_snapshot(&self) -> GridSnapshot {
        let mut snap = self
            .screen
            .lock()
            .map(|p| snapshot_of(&p))
            .unwrap_or_default();
        // Overlay the mark-derived command-output region the free `snapshot_of`
        // cannot know (it sees only the screen, not the OSC-mark stream).
        snap.running = self.running.load(Ordering::Relaxed);
        snap.output_start_row = self.output_start_row.lock().ok().and_then(|o| *o);
        snap.prompt_start_row = self.prompt_start_row.lock().ok().and_then(|o| *o);
        snap
    }
}

/// Read a VT parser's visible screen into a [`GridSnapshot`]: one styled
/// [`GridCell`] per column so the webview paints colour and a fixed-width grid.
/// vt100 already tracks the alternate-screen buffer, so a fullscreen app's
/// screen reads back here too. Free so the snapshot shape is unit-testable
/// without a PTY.
fn snapshot_of(parser: &vt100::Parser) -> GridSnapshot {
    let screen = parser.screen();
    let (rows, cols) = screen.size();
    let mut cells = Vec::with_capacity(rows as usize);
    for r in 0..rows {
        let mut row = Vec::with_capacity(cols as usize);
        for c in 0..cols {
            match screen.cell(r, c) {
                // The trailing column of a wide (double-width) character: the wide
                // cell itself already spans both columns (it renders two cells
                // wide), so emitting a cell here would push the row a column too
                // wide and break the monospace alignment for every wide glyph.
                Some(cell) if cell.is_wide_continuation() => {}
                Some(cell) => row.push(GridCell {
                    text: cell.contents(),
                    fg: conv_color(cell.fgcolor()),
                    bg: conv_color(cell.bgcolor()),
                    bold: cell.bold(),
                    italic: cell.italic(),
                    underline: cell.underline(),
                    inverse: cell.inverse(),
                    wide: cell.is_wide(),
                }),
                None => row.push(GridCell::default()),
            }
        }
        cells.push(row);
    }
    let (cursor_row, cursor_col) = screen.cursor_position();
    GridSnapshot {
        cols,
        rows,
        cells,
        alt_screen: screen.alternate_screen(),
        cursor_row,
        cursor_col,
        // The free fn sees only the screen; the engine overlays the mark-derived
        // command-output region in `screen_snapshot`.
        running: false,
        output_start_row: None,
        prompt_start_row: None,
    }
}

/// Drop trailing all-blank rows from a captured snapshot so a block's stored
/// output is the height of the real output, not the tall capture parser. Keeps
/// at least one row (an empty-output command still has a body row), and updates
/// `rows` to match. Used only for the per-command capture, never the live screen
/// (whose trailing-row handling lives in the renderer).
fn trim_trailing_blank_rows(mut snap: GridSnapshot) -> GridSnapshot {
    let mut last = 0usize;
    for (i, row) in snap.cells.iter().enumerate() {
        if row.iter().any(|c| !c.text.trim().is_empty()) {
            last = i;
        }
    }
    snap.cells.truncate(last + 1);
    snap.rows = snap.cells.len() as u16;
    snap
}

/// Map a vt100 colour to the contract's [`CellColor`].
fn conv_color(c: vt100::Color) -> CellColor {
    match c {
        vt100::Color::Default => CellColor::Default,
        vt100::Color::Idx(i) => CellColor::Indexed(i),
        vt100::Color::Rgb(r, g, b) => CellColor::Rgb([r, g, b]),
    }
}

impl Drop for PtyEngine {
    fn drop(&mut self) {
        // Kill the shell; its slave fds close, so the reader sees EOF and exits,
        // then the join completes promptly (no detached thread, no hang).
        let _ = self.child.kill();
        if let Some(h) = self.reader.take() {
            let _ = h.join();
        }
    }
}

/// Mint a 128-bit CSPRNG nonce as lowercase hex, the secret the shell integration
/// stamps into its command marks (§4.1).
fn mint_nonce() -> std::io::Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(io_err)?;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    Ok(hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PR-2 #3 double-prompt repro (throwaway, `#[ignore]`d): replay a captured
    /// real zsh+p10k+arlen-integration byte stream through the REAL reader by
    /// spawning `cat <file>` on the PTY (its 133;C/133;D/633;A marks drive the
    /// same output_start_row/running logic as a live shell; only the nonce-gated
    /// 633;E text is dropped). Prints the resulting live snapshot + finished block
    /// grids so the prompt-leak can be SEEN. Set ARLEN_REPLAY_FILE to the capture.
    #[test]
    #[ignore]
    fn replay_capture_for_double_prompt() {
        let file = std::env::var("ARLEN_REPLAY_FILE").expect("set ARLEN_REPLAY_FILE");
        let cmd = format!("cat {file}; sleep 1");
        let eng = PtyEngine::spawn("sh", &["-c", &cmd], None, 80, 24).expect("spawn");
        std::thread::sleep(std::time::Duration::from_millis(1600));
        let snap = eng.screen_snapshot();
        let row_text = |row: &[GridCell]| -> String {
            row.iter()
                .map(|c| if c.text.is_empty() { " ".to_string() } else { c.text.clone() })
                .collect::<String>()
                .trim_end()
                .to_string()
        };
        eprintln!(
            "LIVE snapshot: running={} output_start_row={:?} alt_screen={} rows={}",
            snap.running, snap.output_start_row, snap.alt_screen, snap.cells.len()
        );
        for (i, row) in snap.cells.iter().enumerate() {
            let t = row_text(row);
            if !t.is_empty() {
                eprintln!("  live[{i}] {t}");
            }
        }
        let finished = eng.take_finished_outputs();
        eprintln!("FINISHED blocks: {}", finished.len());
        for (bi, b) in finished.iter().enumerate() {
            eprintln!("  block {bi} ({} rows):", b.cells.len());
            for row in &b.cells {
                let t = row_text(row);
                if !t.is_empty() {
                    eprintln!("    | {t}");
                }
            }
        }
    }

    #[test]
    fn mint_nonce_is_128_bit_hex_and_unique() {
        let a = mint_nonce().unwrap();
        let b = mint_nonce().unwrap();
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b, "each session gets a fresh nonce");
    }

    #[test]
    fn snapshot_reads_processed_output_as_styled_cells() {
        // The snapshot is what the webview renders (Option B): a fixed-width grid
        // of styled cells. Feed ordinary output plus an SGR colour and read the
        // visible grid back as cells + cursor.
        let mut parser = vt100::Parser::new(4, 20, 0);
        parser.process(b"hello\r\n\x1b[31mworld\x1b[0m");
        let snap = snapshot_of(&parser);
        assert_eq!(snap.rows, 4);
        assert_eq!(snap.cols, 20);
        assert_eq!(snap.cells.len(), 4);
        // Every row carries exactly `cols` cells, so the monospace grid aligns.
        assert!(snap.cells.iter().all(|r| r.len() == snap.cols as usize));
        let row_text = |r: usize| -> String {
            snap.cells[r].iter().map(|c| c.text.as_str()).collect::<String>()
        };
        assert_eq!(row_text(0).trim_end(), "hello");
        assert_eq!(row_text(1).trim_end(), "world");
        // Colour is captured, not flattened away: "world" was written under
        // SGR 31 (ANSI red, index 1); the first row keeps the default colour.
        assert_eq!(snap.cells[1][0].fg, CellColor::Indexed(1));
        assert_eq!(snap.cells[0][0].fg, CellColor::Default);
        // The cursor sits just after "world" on the second row.
        assert_eq!(snap.cursor_row, 1);
        assert_eq!(snap.cursor_col, 5);
    }

    #[test]
    fn a_wide_character_does_not_emit_a_phantom_continuation_cell() {
        // A double-width glyph (CJK) occupies two terminal columns: the wide cell
        // renders two columns wide on its own, so the trailing continuation column
        // must NOT become its own cell - otherwise the row runs a column too wide
        // and every wide glyph shifts the rest of the line. Feed a wide char then
        // an ASCII char and assert the ASCII follows the wide cell directly.
        let mut parser = vt100::Parser::new(2, 20, 0);
        parser.process("\u{5b57}x".as_bytes()); // 字 (width 2) then x
        let snap = snapshot_of(&parser);
        assert_eq!(snap.cells[0][0].text, "\u{5b57}");
        assert!(snap.cells[0][0].wide, "the CJK glyph is marked wide");
        assert_eq!(
            snap.cells[0][1].text, "x",
            "the next column's content follows the wide cell, not a phantom continuation"
        );
    }

    #[test]
    fn snapshot_flags_the_alternate_screen() {
        // A fullscreen / TUI app (vim, less) switches to the alternate screen
        // via DECSET 1049; the renderer needs that flag so it stops trimming
        // trailing rows and paints the full grid the app owns.
        let mut parser = vt100::Parser::new(4, 20, 0);
        assert!(!snapshot_of(&parser).alt_screen, "primary screen is not alternate");
        parser.process(b"\x1b[?1049h");
        assert!(snapshot_of(&parser).alt_screen, "DECSET 1049 enters the alternate screen");
        parser.process(b"\x1b[?1049l");
        assert!(!snapshot_of(&parser).alt_screen, "DECRST 1049 restores the primary screen");
    }

    /// On-host (needs a PTY + `/bin/sh`): a program that emits an OSC 133;A mark
    /// is read off the PTY, framed by the scanner, and surfaced as a VtEvent end
    /// to end. `#[ignore]`d so CI (which need not have a usable PTY) skips it; run
    /// with `--ignored`.
    #[test]
    #[ignore]
    fn pty_surfaces_an_emitted_osc_mark_end_to_end() {
        let mut eng = PtyEngine::spawn(
            "/bin/sh",
            &["-c", "printf '\\033]133;A\\007'"],
            None,
            80,
            24,
        )
        .unwrap();

        // Poll for the event the child emitted before exiting (give the reader
        // thread time to read + process; the mark is not nonce-gated).
        let mut found = Vec::new();
        for _ in 0..50 {
            found.extend(eng.drain_events());
            if !found.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            found.contains(&VtEvent::PromptStart),
            "the emitted OSC 133;A surfaced as PromptStart, got {found:?}"
        );
    }

    /// On-host (needs a PTY + `/bin/sh`): the LIVE alternate-screen path end to
    /// end. A real program enters the alternate screen (DECSET 1049) and writes a
    /// marker, then stays alive; the engine's reader thread must feed that byte
    /// stream into the screen model so `screen_snapshot()` reports `alt_screen`
    /// AND the alt-screen content. This is the seam the frontend's fullscreen
    /// mode-switch depends on (a TUI like btop owns the whole grid): it proves the
    /// flag + content reach the snapshot off a real PTY, not only a synthetic
    /// parser. `#[ignore]`d (needs a usable PTY); run with `--ignored`.
    #[test]
    #[ignore]
    fn the_live_pty_path_carries_alt_screen_and_its_content() {
        // Enter the alternate screen, paint a marker, and hold the screen open so
        // the snapshot is taken while the app still owns the alt screen (exiting
        // would restore the primary screen and drop the marker).
        let eng = PtyEngine::spawn(
            "/bin/sh",
            &["-c", "printf '\\033[?1049hALTHELLO'; sleep 5"],
            None,
            80,
            24,
        )
        .unwrap();

        let mut snap = eng.screen_snapshot();
        for _ in 0..100 {
            snap = eng.screen_snapshot();
            if snap.alt_screen
                && snap
                    .cells
                    .iter()
                    .any(|row| row.iter().map(|c| c.text.as_str()).collect::<String>().contains("ALTHELLO"))
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(snap.alt_screen, "the live PTY path flags the alternate screen");
        let text: String = snap
            .cells
            .iter()
            .map(|row| row.iter().map(|c| c.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("ALTHELLO"),
            "alt-screen content reaches the snapshot off a real PTY, got:\n{text}"
        );
    }

    /// On-host (needs the user's real zsh + zsh-syntax-highlighting): the PR-2
    /// re-root premise. When keystrokes reach the PTY PER CHARACTER (not batched
    /// on Enter, the way the old composer sent them), the user's real shell runs
    /// its line editor (zsh `zle`) interactively, so the command line is echoed
    /// and syntax-highlighted as it is typed. Tim reports highlighting "renders to
    /// null" today precisely because the composer never sends per-keystroke; this
    /// proves the ENGINE + real shell deliver it, so the remaining fix is the
    /// frontend input path (the grid taking keystrokes, not a textbox). `#[ignore]`d
    /// (needs an interactive zsh with the user's config); run with `--ignored`.
    #[test]
    #[ignore]
    fn the_real_shell_echoes_and_colours_per_keystroke_input() {
        let mut eng = PtyEngine::spawn("zsh", &["-i"], None, 80, 24).expect("spawn zsh");
        // Let the shell + prompt + plugins finish loading before typing.
        std::thread::sleep(std::time::Duration::from_millis(2000));
        // Type a valid command ONE BYTE AT A TIME, pausing so zle processes each
        // key (echo + re-highlight) - the way raw-PTY input will deliver them.
        for ch in "echo hello".bytes() {
            eng.send_input(&[ch]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        let snap = eng.screen_snapshot();
        let text: String = snap
            .cells
            .iter()
            .map(|row| row.iter().map(|c| c.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        // The per-keystroke input was echoed by the shell's line editor: the
        // command line shows in the grid (it would be absent if zle never ran).
        assert!(
            text.contains("echo hello"),
            "the per-keystroke command line is echoed into the grid by zle, got:\n{text}"
        );
        // The live shell rendered with colour (a coloured prompt and/or the
        // syntax-highlighted command): the grid captures non-default foreground
        // cells, so highlighting does NOT render to null when input is interactive.
        let coloured = snap
            .cells
            .iter()
            .flatten()
            .any(|c| c.text.trim() != "" && !matches!(c.fg, CellColor::Default));
        assert!(
            coloured,
            "the live shell's colour (prompt / syntax-highlighting) reaches the grid, got:\n{text}"
        );
    }

    /// On-host (needs a PTY + `/bin/sh`): the engine resolves the command-output
    /// REGION from the OSC marks, which is what lets the renderer paint only a
    /// command's output and never the shell's own prompt (the double-prompt). A
    /// scripted sequence draws a prompt at row 0, marks the prompt (133;A), echoes
    /// the command (cursor to row 1), marks exec-start (133;C), prints output, and
    /// pauses on `read` so each phase is a stable steady state the test observes by
    /// driving the PTY. Asserts: while running, `output_start_row` is row 1 (PAST
    /// the row-0 prompt + echo, so the prompt is excluded) and `running` is true;
    /// after the 133;D end mark, `running` is false. `#[ignore]`d (needs a PTY);
    /// run with `--ignored`.
    #[test]
    #[ignore]
    fn the_live_pty_path_tracks_the_command_output_region() {
        // `read` pauses gate the phases (released by send_input), so the snapshot
        // is read in a known steady state rather than racing the byte stream.
        let script = concat!(
            "printf 'prompt$ ';",
            "printf '\\033]133;A\\007';", // prompt start -> output_start_row = None
            "printf 'mycmd\\r\\n';",      // echoed command -> cursor to row 1
            "printf '\\033]133;C\\007';", // exec start -> output_start_row = 1, running
            "printf 'OUT\\r\\n';",        // command output
            "read x;",                    // PHASE 1: running, output_start_row = 1
            "printf '\\033]133;D;0\\007';", // command end -> running = false
            "read y",                     // PHASE 2: not running, output_start_row = 1
        );
        let mut eng = PtyEngine::spawn("/bin/sh", &["-c", script], None, 80, 24).unwrap();

        // PHASE 1: the command is "running"; output starts at row 1, past the
        // row-0 prompt and the echoed command line.
        let mut snap = eng.screen_snapshot();
        for _ in 0..100 {
            snap = eng.screen_snapshot();
            if snap.running {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(snap.running, "ExecStart (133;C) marks the command as running");
        assert_eq!(
            snap.output_start_row,
            Some(1),
            "output begins at row 1, excluding the row-0 prompt + echoed command"
        );

        // Release the first `read`, advancing past the 133;D end mark.
        eng.send_input(b"\n").unwrap();

        // PHASE 2: the end mark cleared the running state; the output-start row
        // stays put (only a new prompt clears it).
        for _ in 0..100 {
            snap = eng.screen_snapshot();
            if !snap.running {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!snap.running, "CommandEnd (133;D) clears the running state");
        assert_eq!(snap.output_start_row, Some(1), "the output-start row persists past the end mark");
    }

    /// On-host (needs a PTY + `/bin/sh`): `PromptStart` (OSC 133;A) records the
    /// row where the current prompt begins, and `ExecStart` clears it. This is the
    /// raw-PTY re-root's enabler: at an idle prompt the renderer shows the live
    /// grid from `prompt_start_row` (the prompt + the line being typed, the
    /// interactive surface) while finished output above stays in its blocks; a
    /// running command's region is `output_start_row` instead. `#[ignore]`d (needs
    /// a PTY); run with `--ignored`.
    #[test]
    #[ignore]
    fn the_prompt_start_row_marks_the_prompt_and_clears_on_exec() {
        let script = concat!(
            "printf '\\033]133;A\\007';", // prompt start at row 0
            "printf 'prompt$ ';",         // the prompt is drawn (cursor stays row 0)
            "read a;",                    // PHASE 1: idle at the prompt
            "printf '\\033]133;C\\007';", // exec start
            "read b",                     // PHASE 2: running
        );
        let mut eng = PtyEngine::spawn("/bin/sh", &["-c", script], None, 80, 24).unwrap();

        // PHASE 1: at the prompt, prompt_start_row is set to the prompt's row and
        // no command is running.
        let mut snap = eng.screen_snapshot();
        for _ in 0..100 {
            snap = eng.screen_snapshot();
            if snap.prompt_start_row.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(snap.prompt_start_row, Some(0), "PromptStart marks the prompt's row");
        assert!(!snap.running, "no command runs at the prompt");

        // Release the first `read`, advancing past the 133;C exec mark.
        eng.send_input(b"\n").unwrap();

        // PHASE 2: ExecStart cleared the prompt-start row and marked running.
        for _ in 0..100 {
            snap = eng.screen_snapshot();
            if snap.running {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(snap.running, "ExecStart marks the command running");
        assert_eq!(snap.prompt_start_row, None, "ExecStart clears the prompt-start row");
    }

    /// On-host (needs a PTY): a resize reaches both the VT parser and the master
    /// PTY. The snapshot geometry tracks the new size (the parser), and a shell
    /// that reports its own width sees the change (SIGWINCH on the master), so a
    /// running command and a TUI reflow. Before the resize fix the PTY kept its
    /// initial 80x24 no matter the window size. `#[ignore]`d (needs a PTY); run
    /// with `--ignored`.
    #[test]
    #[ignore]
    fn resizing_tracks_the_new_geometry_in_the_snapshot() {
        let mut eng = PtyEngine::spawn("/bin/sh", &["-c", "sleep 5"], None, 80, 24).unwrap();
        let before = eng.screen_snapshot();
        assert_eq!(before.cols, 80, "the screen starts at the spawn width");
        assert_eq!(before.rows, 24, "the screen starts at the spawn height");

        eng.resize(100, 40).unwrap();
        let after = eng.screen_snapshot();
        assert_eq!(after.cols, 100, "the screen width tracks the resize");
        assert_eq!(after.rows, 40, "the screen height tracks the resize");
    }

    /// On-host (needs a PTY + `/bin/sh`): a command's output is captured into its
    /// own block grid, in full, with the prompt and echoed command line excluded.
    /// This is the "VT grid inside the block": the renderer paints a block's own
    /// output rather than slicing the small shared live screen, so multi-line
    /// output (neofetch, a build log) is preserved instead of truncated. The
    /// scripted command emits three output lines between its marks; the capture
    /// must hold exactly those lines and neither the prompt nor the echo.
    /// `#[ignore]`d (needs a PTY); run with `--ignored`.
    #[test]
    #[ignore]
    fn a_commands_output_is_captured_into_its_own_block_grid() {
        let script = concat!(
            "printf 'prompt$ ';",
            "printf '\\033]133;A\\007';",   // prompt start
            "printf 'mycmd\\r\\n';",        // echoed command (NOT output)
            "printf '\\033]133;C\\007';",   // exec start
            "printf 'line-one\\r\\n';",     // \
            "printf 'line-two\\r\\n';",     //  > the command's output
            "printf 'line-three\\r\\n';",   // /
            "printf '\\033]133;D;0\\007';", // command end -> capture finalized
            "sleep 5",
        );
        let eng = PtyEngine::spawn("/bin/sh", &["-c", script], None, 80, 24).unwrap();

        let mut outputs = Vec::new();
        for _ in 0..100 {
            outputs.extend(eng.take_finished_outputs());
            if !outputs.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(outputs.len(), 1, "one finished command was captured");
        let grid = &outputs[0];
        let lines: Vec<String> = grid
            .cells
            .iter()
            .map(|row| {
                row.iter()
                    .map(|c| c.text.as_str())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();
        assert_eq!(
            lines,
            vec!["line-one", "line-two", "line-three"],
            "the block holds exactly the command output, trimmed, in order"
        );
        let joined = lines.join("\n");
        assert!(!joined.contains("mycmd"), "the echoed command is excluded from the block output");
        assert!(!joined.contains("prompt$"), "the prompt is excluded from the block output");
    }

    /// On-host (needs zsh + a PTY): the FULL mark loop. The engine mints the
    /// nonce and exports it; the TM-R3 integration script (sourced in the spawned
    /// zsh) reads it, escapes a command line containing a `;`, and emits the
    /// nonced 633;E mark; the engine's scanner - holding the SAME nonce - accepts
    /// it and decodes the command. Proves the producer and consumer agree on the
    /// nonce, the OSC framing and the escaping. `#[ignore]`d (on-host).
    #[test]
    #[ignore]
    fn the_integration_script_emits_a_nonced_command_mark_the_engine_decodes() {
        let script = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../integration/arlen-shell-integration.zsh"
        );
        // Source the script (defines the emitters using the engine-set nonce),
        // then invoke the preexec emitter directly with a command that contains a
        // `;` (the escaping + nonce-field separation is the thing under test).
        let inner = format!("source {script}; _arlen_term_preexec 'ls -la; echo hi'");
        let mut eng = PtyEngine::spawn("zsh", &["-c", &inner], None, 80, 24).unwrap();

        let mut found = Vec::new();
        for _ in 0..100 {
            found.extend(eng.drain_events());
            if found
                .iter()
                .any(|e| matches!(e, VtEvent::CommandLine { .. }))
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            found.contains(&VtEvent::CommandLine {
                command: "ls -la; echo hi".into()
            }),
            "the script's nonced 633;E decoded to the exact command line, got {found:?}"
        );
    }

    /// On-host (needs zsh + a PTY): the CURATED ZDOTDIR injection. The engine is
    /// pointed at the curated config dir (`ARLEN_TERM_ZDOTDIR`) with a throwaway
    /// user config dir (`ZDOTDIR`); the curated `.zshrc` must source the user's
    /// own `.zshrc` (proven by a marker file it writes) AND the integration
    /// script (proven by the interactive shell's first precmd emitting a
    /// `CwdChanged`/`PromptStart` mark). Confirms marks fire through the curated
    /// dir without dropping the user's setup. `#[ignore]`d (on-host); run with
    /// `--ignored --test-threads=1` (it mutates process env).
    #[test]
    #[ignore]
    fn the_curated_zdotdir_sources_the_user_rc_and_the_integration() {
        let curated = concat!(env!("CARGO_MANIFEST_DIR"), "/../integration/zdotdir");
        let tmp = std::env::temp_dir().join(format!("arlen-term-zdotdir-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let marker = tmp.join("user-rc-ran");
        let _ = std::fs::remove_file(&marker);
        // The throwaway "user" .zshrc: prove it ran by touching a marker, and be
        // a no-op interactive shell otherwise (no prompt framework to hang on).
        std::fs::write(tmp.join(".zshrc"), format!("touch {}\n", marker.display())).unwrap();

        let prev_zdotdir = std::env::var_os("ZDOTDIR");
        let prev_curated = std::env::var_os(ZDOTDIR_ENV);
        std::env::set_var("ZDOTDIR", &tmp);
        std::env::set_var(ZDOTDIR_ENV, curated);

        let mut eng = PtyEngine::spawn("zsh", &["-i"], None, 80, 24).unwrap();

        // The first interactive precmd emits 633;A (PromptStart) + OSC 7
        // (CwdChanged) - only if the curated .zshrc sourced the integration.
        let mut found = Vec::new();
        for _ in 0..100 {
            found.extend(eng.drain_events());
            let integration_ran = found.iter().any(|e| {
                matches!(e, VtEvent::PromptStart | VtEvent::CwdChanged { .. })
            });
            if integration_ran && marker.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        drop(eng);

        // Restore process env before asserting (so a failure does not leak state).
        match prev_zdotdir {
            Some(v) => std::env::set_var("ZDOTDIR", v),
            None => std::env::remove_var("ZDOTDIR"),
        }
        match prev_curated {
            Some(v) => std::env::set_var(ZDOTDIR_ENV, v),
            None => std::env::remove_var(ZDOTDIR_ENV),
        }
        let user_ran = marker.exists();
        let _ = std::fs::remove_dir_all(&tmp);

        assert!(
            user_ran,
            "the curated .zshrc sourced the user's .zshrc (marker written)"
        );
        assert!(
            found
                .iter()
                .any(|e| matches!(e, VtEvent::PromptStart | VtEvent::CwdChanged { .. })),
            "the curated .zshrc sourced the integration (a prompt/cwd mark fired), got {found:?}"
        );
    }

    /// On-host: a command line containing a BEL (which would otherwise terminate
    /// the OSC early) is escaped by the script, survives the framing, and the BEL
    /// byte round-trips back into the decoded command. Guards the framing-safety
    /// escaping. `#[ignore]`d (on-host).
    #[test]
    #[ignore]
    fn a_command_with_a_control_byte_survives_the_osc_framing() {
        let script = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../integration/arlen-shell-integration.zsh"
        );
        // zsh $'...' turns \a into a literal BEL inside the command line.
        let inner = format!("source {script}; _arlen_term_preexec $'echo \\a hi'");
        let mut eng = PtyEngine::spawn("zsh", &["-c", &inner], None, 80, 24).unwrap();
        let mut found = Vec::new();
        for _ in 0..100 {
            found.extend(eng.drain_events());
            if found
                .iter()
                .any(|e| matches!(e, VtEvent::CommandLine { .. }))
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            found.contains(&VtEvent::CommandLine {
                command: "echo \u{7} hi".into()
            }),
            "the BEL-bearing command survived framing and round-tripped, got {found:?}"
        );
    }
}
