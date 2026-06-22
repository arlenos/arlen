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
        let reader_handle = std::thread::Builder::new()
            .name("arlen-pty-reader".into())
            .spawn(move || {
                let mut scanner = OscScanner::new(nonce);
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        // 0 = clean EOF; Err = the master closed (Linux returns
                        // EIO when the slave goes away). Either ends the loop.
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if let Ok(mut p) = screen_sink.lock() {
                                p.process(&buf[..n]);
                            }
                            let evs = scanner.feed(&buf[..n]);
                            if !evs.is_empty() {
                                if let Ok(mut q) = sink.lock() {
                                    q.extend(evs);
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
            reader: Some(reader_handle),
        })
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
        self.screen
            .lock()
            .map(|p| snapshot_of(&p))
            .unwrap_or_default()
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
            row.push(match screen.cell(r, c) {
                Some(cell) => GridCell {
                    text: cell.contents(),
                    fg: conv_color(cell.fgcolor()),
                    bg: conv_color(cell.bgcolor()),
                    bold: cell.bold(),
                    italic: cell.italic(),
                    underline: cell.underline(),
                    inverse: cell.inverse(),
                    wide: cell.is_wide(),
                },
                None => GridCell::default(),
            });
        }
        cells.push(row);
    }
    let (cursor_row, cursor_col) = screen.cursor_position();
    GridSnapshot {
        cols,
        rows,
        cells,
        cursor_row,
        cursor_col,
    }
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
