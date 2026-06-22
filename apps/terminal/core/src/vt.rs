//! The VT engine seam: the structured events the host consumes, and the parser
//! that turns a trusted shell's OSC marks into them (`terminal.md` §2.1, §4.1).
//!
//! The concrete engine (`wezterm-term` over a `portable-pty` shell) is a separate,
//! heavier crate so this contract core stays cheap to embed (the UI mock and the
//! file manager's terminal pane depend on it without pulling a VT state machine).
//! What lives here is the seam: [`VtEvent`] (the low-rate structured stream the
//! UI builds [`crate::Block`]s from, never the raw output firehose, §2.3) and
//! [`parse_osc_mark`] (the OSC 133/633/7 decode with the nonce forge-protection,
//! the security-load-bearing piece). The [`VtEngine`] trait is the control seam a
//! host drives, kept narrow so the post-1.0 engine swap (§2.1) stays clean.

use serde::{Deserialize, Serialize};

/// A structured event surfaced from the shell's OSC marks. Low-rate and
/// structured (block boundaries, the command line, exit/timing, cwd) - the raw
/// PTY output never travels as a `VtEvent` (§2.3: the grid renders through the
/// compositor subsurface). The host assembles a [`crate::Block`] from this stream:
/// [`VtEvent::PromptStart`] opens a block, [`VtEvent::CommandLine`] sets its
/// command, [`VtEvent::ExecStart`]/[`VtEvent::CommandEnd`] bound its timing, and
/// [`VtEvent::CwdChanged`] tracks the working directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VtEvent {
    /// A fresh prompt was drawn (OSC `133;A` / `633;A`): the boundary that starts
    /// a new block.
    PromptStart,
    /// The shell reported the exact command line about to run (OSC
    /// `633;E;<commandline>;<nonce>`), carried as explicit, nonce-verified data
    /// rather than scraped off the grid. Only emitted when the nonce matches the
    /// session's, so output cannot forge a command boundary.
    CommandLine {
        /// The decoded command line. It is trusted as to ORIGIN (only the
        /// nonce-holding shell produces it) but is arbitrary CONTENT: the OSC 633
        /// escaping can carry newlines and control bytes, so a command may be
        /// multi-line and non-printable. A consumer that renders it (the block
        /// header) or records it (the KG command node) must treat it as
        /// arbitrary-byte text, not single-line printable, and tag/sanitise per
        /// TM-R8 where it leaves the trust boundary.
        command: String,
    },
    /// The command began executing (OSC `133;C` / `633;C`): the start of the
    /// duration clock.
    ExecStart,
    /// The running command finished (OSC `133;D;<exit>` / `633;D;<exit>`).
    CommandEnd {
        /// The exit code, when the mark carried one.
        exit_code: Option<i32>,
    },
    /// The shell reported its working directory (OSC `7;file://<host>/<path>` or
    /// `633;P;Cwd=<path>`).
    CwdChanged {
        /// The absolute working directory path.
        cwd: String,
    },
    /// The shell set the window or tab title (OSC `0` / `2`).
    Title {
        /// The new title text.
        title: String,
    },
}

/// Parse one OSC payload (the bytes between `ESC ]` and the terminator, already
/// extracted by the escape parser) into a [`VtEvent`], or `None` for a sequence
/// this seam does not surface or that fails validation.
///
/// `session_nonce` is the per-session secret the shell integration script mints
/// at startup and keeps out of the byte stream (§4.1). A `633;E` command-line
/// mark is accepted ONLY when its trailing nonce matches, so terminal output
/// (an attacker echoing `633;E;rm -rf /;guess`) cannot forge a command and slip
/// a fabricated command into the block record. An empty `session_nonce` rejects
/// every `633;E` (fail-closed: no nonce configured means no trusted command
/// marks).
pub fn parse_osc_mark(payload: &str, session_nonce: &str) -> Option<VtEvent> {
    let (code, rest) = split_first_field(payload);
    match code {
        // OSC 7: "7;file://<host>/<path>" - the working directory as a file URI.
        "7" => cwd_from_file_uri(rest).map(|cwd| VtEvent::CwdChanged { cwd }),
        // OSC 0 (icon+title) / OSC 2 (title): "2;<title>".
        "0" | "2" => Some(VtEvent::Title {
            title: rest.to_string(),
        }),
        // OSC 133: the bare semantic-prompt family (no command line, no nonce).
        "133" => {
            let (sub, tail) = split_first_field(rest);
            match sub {
                "A" => Some(VtEvent::PromptStart),
                "C" => Some(VtEvent::ExecStart),
                "D" => Some(VtEvent::CommandEnd {
                    exit_code: parse_exit(tail),
                }),
                _ => None,
            }
        }
        // OSC 633: the VS Code superset that carries the command line + nonce.
        "633" => {
            let (sub, tail) = split_first_field(rest);
            match sub {
                "A" => Some(VtEvent::PromptStart),
                "C" => Some(VtEvent::ExecStart),
                "D" => Some(VtEvent::CommandEnd {
                    exit_code: parse_exit(tail),
                }),
                // "E;<escaped-commandline>;<nonce>": the nonce is the last field,
                // so the command line keeps its meaning even if it (escaped)
                // contained no separators. rsplit isolates the nonce; the command
                // is everything before it.
                "E" => {
                    let (escaped_cmd, provided_nonce) = tail.rsplit_once(';')?;
                    if session_nonce.is_empty() || !nonce_matches(provided_nonce, session_nonce) {
                        return None;
                    }
                    Some(VtEvent::CommandLine {
                        command: unescape_633(escaped_cmd),
                    })
                }
                // "P;Cwd=<path>" (and other properties this seam ignores). After
                // the "P" field, the remainder is the single "<key>=<value>"
                // property, not a further `;`-split list.
                "P" => tail.strip_prefix("Cwd=").map(|cwd| VtEvent::CwdChanged {
                    cwd: cwd.to_string(),
                }),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Split `s` at the first `;` into the leading field and the remainder (without
/// the `;`). With no `;`, the whole string is the field and the remainder empty.
fn split_first_field(s: &str) -> (&str, &str) {
    match s.split_once(';') {
        Some((head, tail)) => (head, tail),
        None => (s, ""),
    }
}

/// Parse the exit code carried after `133;D` / `633;D`. The field may be absent
/// (a clean prompt with no prior command) or non-numeric, in which case there is
/// no exit code rather than a fabricated zero.
fn parse_exit(tail: &str) -> Option<i32> {
    let (field, _) = split_first_field(tail);
    if field.is_empty() {
        return None;
    }
    field.parse::<i32>().ok()
}

/// Extract the path from an OSC 7 `file://<host>/<path>` URI, percent-decoded.
/// The host segment is dropped (a local cwd is always on this host). Returns
/// `None` for a non-`file://` value.
fn cwd_from_file_uri(uri: &str) -> Option<String> {
    let after = uri.strip_prefix("file://")?;
    // Everything from the first '/' is the absolute path; the authority (host)
    // before it is ignored.
    let path = match after.find('/') {
        Some(i) => &after[i..],
        None => "/",
    };
    Some(percent_decode(path))
}

/// Decode `%HH` percent-escapes (OSC 7 paths are percent-encoded). Invalid or
/// truncated escapes are left verbatim rather than dropped.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(b) = hex2(bytes[i + 1], bytes[i + 2]) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Decode the VS Code OSC 633 command-line escaping (`\xHH` for the control and
/// separator bytes the protocol escapes: `;` `\n` `\r` `\`). An incomplete or
/// invalid escape is left verbatim.
fn unescape_633(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // "\xHH" -> the byte 0xHH. Needs bytes[i+1]='x' and the two hex digits
        // bytes[i+2], bytes[i+3] in range (so i + 3 must be a valid index).
        if bytes[i] == b'\\' && i + 3 < bytes.len() && bytes[i + 1] == b'x' {
            if let Some(b) = hex2(bytes[i + 2], bytes[i + 3]) {
                out.push(b);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Compare a forged-or-trusted nonce against the session secret in constant time
/// over equal-length inputs, so accepting/rejecting a `633;E` mark leaks no
/// timing signal about how many leading bytes of the secret a forged payload
/// matched. The length branch is not secret (the nonce format is fixed-length),
/// and the forge-protection itself rests on the byte equality, not the timing;
/// this is defence-in-depth that removes the residual rather than re-arguing it.
fn nonce_matches(provided: &str, session: &str) -> bool {
    let (a, b) = (provided.as_bytes(), session.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Two ASCII hex digits to a byte, or `None` if either is not hex.
fn hex2(hi: u8, lo: u8) -> Option<u8> {
    Some(hex_digit(hi)? << 4 | hex_digit(lo)?)
}

/// One ASCII hex digit to its value.
fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Maximum OSC payload bytes the scanner buffers. A `633;E` command line is far
/// under this; an unterminated or pathological OSC beyond it is dropped (the
/// scanner keeps resyncing on the terminator) so untrusted output cannot grow
/// memory without bound.
const MAX_OSC_PAYLOAD: usize = 8192;

/// The ECMA-48 escape/string framing state the scanner tracks. Only enough of it
/// to frame OSC correctly in the presence of the other string sequences; the
/// full grid emulation stays in the VT engine.
#[derive(PartialEq, Eq)]
enum ScanState {
    /// Ordinary stream, outside any escape.
    Ground,
    /// Saw `ESC`; the next byte selects the sequence kind.
    Escape,
    /// Inside an OSC (`ESC ]`), accumulating the payload.
    Osc,
    /// Saw `ESC` inside an OSC: `ESC \` is the ST terminator, anything else aborts.
    OscEsc,
    /// Inside a DCS/SOS/PM/APC string sequence: consumed to ST, never parsed (so
    /// a payload containing OSC-looking bytes cannot forge a mark).
    StringSeq,
    /// Saw `ESC` inside a string sequence (awaiting the `\` of ST).
    StringEsc,
}

/// A streaming extractor that pulls OSC marks out of a raw PTY byte stream and
/// turns them into [`VtEvent`]s.
///
/// It tracks the ECMA-48 string-sequence framing, so a DCS/APC/PM/SOS payload
/// that contains OSC-looking bytes is consumed rather than mis-parsed as a mark;
/// it bounds the buffered payload ([`MAX_OSC_PAYLOAD`]) and keeps resyncing on the
/// terminator, so untrusted output cannot grow memory without bound; and it
/// carries state across calls, so a mark split over two `feed` chunks is still
/// found. Only OSC is decoded (via [`parse_osc_mark`] with the session nonce);
/// ordinary output bytes are ignored here (the grid renders them, §2.3). Keeping
/// the security-relevant mark path in this audited scanner, not the grid engine's
/// internal OSC handling, is deliberate.
///
/// It frames the 7-bit `ESC`-introduced forms (`ESC ]`, `ESC \`, `ESC P`/`X`/`^`/
/// `_`), which is what a UTF-8 PTY stream emits: the 8-bit C1 single-byte controls
/// (OSC 0x9d, ST 0x9c, the C1 string introducers) are illegal standalone bytes in
/// UTF-8, so a sane shell never sends them. Ignoring a bare C1 control only DROPS a
/// mark, never fabricates one, so the omission is fail-safe.
pub struct OscScanner {
    nonce: String,
    state: ScanState,
    buf: Vec<u8>,
    /// Set once the current OSC overflowed the cap: keep scanning to the
    /// terminator but drop the payload (no event).
    overflowed: bool,
}

impl OscScanner {
    /// A scanner for a session whose shell integration mints `nonce` (the secret
    /// gating `633;E` command marks; an empty nonce rejects all of them).
    pub fn new(nonce: impl Into<String>) -> Self {
        Self {
            nonce: nonce.into(),
            state: ScanState::Ground,
            buf: Vec::new(),
            overflowed: false,
        }
    }

    /// Feed a chunk of raw PTY bytes; return the marks found in it. Bytes that
    /// are not part of an OSC mark are ignored (the grid consumes them).
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<VtEvent> {
        self.feed_positioned(bytes)
            .into_iter()
            .map(|(_, ev)| ev)
            .collect()
    }

    /// Like [`feed`](Self::feed), but each mark is paired with its end offset:
    /// the index in `bytes` just PAST the mark's terminator. The engine processes
    /// the screen up to that offset before sampling the cursor, so a mark that
    /// shares a read buffer with the command output it precedes (133;C followed by
    /// output in one chunk) still resolves the output-start row correctly instead
    /// of sampling after the output has scrolled the cursor on.
    pub fn feed_positioned(&mut self, bytes: &[u8]) -> Vec<(usize, VtEvent)> {
        const ESC: u8 = 0x1b;
        const BEL: u8 = 0x07;
        let mut events = Vec::new();
        for (i, &b) in bytes.iter().enumerate() {
            match self.state {
                ScanState::Ground => {
                    if b == ESC {
                        self.state = ScanState::Escape;
                    }
                }
                ScanState::Escape => self.dispatch_after_esc(b),
                ScanState::Osc => match b {
                    BEL => {
                        self.finish_osc(i + 1, &mut events);
                    }
                    ESC => self.state = ScanState::OscEsc,
                    _ => self.push_osc(b),
                },
                ScanState::OscEsc => {
                    if b == b'\\' {
                        // ST terminator (ESC \).
                        self.finish_osc(i + 1, &mut events);
                    } else {
                        // The OSC was aborted by a new escape; drop it and treat
                        // this byte as the one following a fresh ESC.
                        self.reset_buf();
                        self.dispatch_after_esc(b);
                    }
                }
                ScanState::StringSeq => {
                    // A DCS/SOS/PM/APC string is consumed until ST (ESC \) ONLY -
                    // not BEL (that ends an OSC, not these). It is never parsed and
                    // never buffered, so its payload can neither forge a mark nor
                    // grow memory, however much OSC-looking content it carries.
                    if b == ESC {
                        self.state = ScanState::StringEsc;
                    }
                }
                ScanState::StringEsc => {
                    self.state = match b {
                        b'\\' => ScanState::Ground, // ST terminates the string
                        ESC => ScanState::StringEsc, // another ESC: keep awaiting ST
                        // Not ST: the ESC was part of the consumed string, so stay
                        // inside it (do NOT re-dispatch, which could let a string
                        // body's `ESC ]` open an OSC).
                        _ => ScanState::StringSeq,
                    };
                }
            }
        }
        events
    }

    /// Handle the byte after an `ESC`: `]` opens an OSC, the string-sequence
    /// introducers open a consumed string, everything else is a short escape we
    /// ignore (its bytes contain no `ESC`, so Ground is safe).
    fn dispatch_after_esc(&mut self, b: u8) {
        match b {
            b']' => {
                self.reset_buf();
                self.state = ScanState::Osc;
            }
            // DCS (P), SOS (X), PM (^), APC (_): framed strings we consume, never
            // parse, so their payloads cannot forge an OSC mark.
            b'P' | b'X' | b'^' | b'_' => self.state = ScanState::StringSeq,
            _ => self.state = ScanState::Ground,
        }
    }

    /// Append a payload byte, honouring the cap (over-cap bytes are dropped but
    /// the scanner stays in `Osc` to resync on the terminator).
    fn push_osc(&mut self, b: u8) {
        if self.buf.len() < MAX_OSC_PAYLOAD {
            self.buf.push(b);
        } else {
            self.overflowed = true;
        }
        self.state = ScanState::Osc;
    }

    /// Terminate the current OSC: parse it (unless it overflowed) and return to
    /// Ground. `end` is the offset in the fed buffer just past the terminator, so
    /// the caller can correlate the mark with a position in the byte stream.
    fn finish_osc(&mut self, end: usize, events: &mut Vec<(usize, VtEvent)>) {
        if !self.overflowed {
            let payload = String::from_utf8_lossy(&self.buf);
            if let Some(ev) = parse_osc_mark(&payload, &self.nonce) {
                events.push((end, ev));
            }
        }
        self.reset_buf();
        self.state = ScanState::Ground;
    }

    /// Clear the payload buffer and the overflow flag.
    fn reset_buf(&mut self) {
        self.buf.clear();
        self.overflowed = false;
    }
}

/// The control seam over the concrete VT engine, so a host (the terminal app or
/// an embedded pane) drives the shell without depending on `wezterm-term` /
/// `portable-pty` (the §2.1 post-1.0 engine swap stays a one-impl change). The
/// concrete engine lives in a separate crate; this trait is its contract.
pub trait VtEngine {
    /// Send user input (keystrokes, paste) to the shell's PTY.
    fn send_input(&mut self, bytes: &[u8]) -> std::io::Result<()>;

    /// Resize the terminal grid to `cols` x `rows` cells.
    fn resize(&mut self, cols: u16, rows: u16) -> std::io::Result<()>;

    /// Take the structured events surfaced since the last drain. The host polls
    /// this on the low-rate `VtEvent` channel (§2.3), never the byte firehose.
    fn drain_events(&mut self) -> Vec<VtEvent>;

    /// A snapshot of the visible screen as text, for the webview to render
    /// (terminal.md Option B). The default is an empty grid, so an engine that
    /// has no screen model (a mock, or the OSC-only path) is unaffected; the
    /// concrete PTY engine overrides it from its VT parser.
    fn screen_snapshot(&self) -> crate::GridSnapshot {
        crate::GridSnapshot::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONCE: &str = "s3cr3t-nonce";

    #[test]
    fn prompt_and_exec_boundaries() {
        assert_eq!(parse_osc_mark("133;A", NONCE), Some(VtEvent::PromptStart));
        assert_eq!(parse_osc_mark("633;A", NONCE), Some(VtEvent::PromptStart));
        assert_eq!(parse_osc_mark("133;C", NONCE), Some(VtEvent::ExecStart));
        assert_eq!(parse_osc_mark("633;C", NONCE), Some(VtEvent::ExecStart));
    }

    #[test]
    fn command_end_carries_the_exit_code_or_none() {
        assert_eq!(
            parse_osc_mark("133;D;0", NONCE),
            Some(VtEvent::CommandEnd { exit_code: Some(0) })
        );
        assert_eq!(
            parse_osc_mark("633;D;130", NONCE),
            Some(VtEvent::CommandEnd {
                exit_code: Some(130)
            })
        );
        // A bare D (clean prompt, no prior command) carries no fabricated zero.
        assert_eq!(
            parse_osc_mark("133;D", NONCE),
            Some(VtEvent::CommandEnd { exit_code: None })
        );
        assert_eq!(
            parse_osc_mark("133;D;junk", NONCE),
            Some(VtEvent::CommandEnd { exit_code: None })
        );
    }

    #[test]
    fn command_line_requires_a_matching_nonce() {
        // The trusted shell's mark, with the right nonce, yields the command.
        assert_eq!(
            parse_osc_mark("633;E;ls -la;s3cr3t-nonce", NONCE),
            Some(VtEvent::CommandLine {
                command: "ls -la".into()
            })
        );
        // Output forging a command boundary with a wrong/guessed nonce is rejected.
        assert_eq!(parse_osc_mark("633;E;rm -rf /;guessed", NONCE), None);
        // No configured nonce fails closed (no trusted command marks at all).
        assert_eq!(parse_osc_mark("633;E;ls;anything", ""), None);
    }

    #[test]
    fn command_line_decodes_escapes_and_keeps_separators() {
        // VS Code escapes ';' as \x3b and newline as \x0a inside the command.
        assert_eq!(
            parse_osc_mark("633;E;echo a\\x3b echo b;s3cr3t-nonce", NONCE),
            Some(VtEvent::CommandLine {
                command: "echo a; echo b".into()
            })
        );
        assert_eq!(
            parse_osc_mark("633;E;grep \\x5cd file;s3cr3t-nonce", NONCE),
            Some(VtEvent::CommandLine {
                command: "grep \\d file".into()
            })
        );
    }

    #[test]
    fn cwd_from_osc_7_and_633_property() {
        assert_eq!(
            parse_osc_mark("7;file://host/home/x/arlen", NONCE),
            Some(VtEvent::CwdChanged {
                cwd: "/home/x/arlen".into()
            })
        );
        // Percent-encoded path (a space).
        assert_eq!(
            parse_osc_mark("7;file://host/home/x/my%20dir", NONCE),
            Some(VtEvent::CwdChanged {
                cwd: "/home/x/my dir".into()
            })
        );
        // The 633 property form.
        assert_eq!(
            parse_osc_mark("633;P;Cwd=/srv/work", NONCE),
            Some(VtEvent::CwdChanged {
                cwd: "/srv/work".into()
            })
        );
    }

    #[test]
    fn title_and_unknown_sequences() {
        assert_eq!(
            parse_osc_mark("2;my title", NONCE),
            Some(VtEvent::Title {
                title: "my title".into()
            })
        );
        assert_eq!(parse_osc_mark("9;notification", NONCE), None);
        assert_eq!(parse_osc_mark("633;Z;weird", NONCE), None);
    }

    // ESC and the ST terminator (ESC \) as byte literals for scanner fixtures.
    const ESC: &[u8] = b"\x1b";

    #[test]
    fn scanner_extracts_a_bel_terminated_mark() {
        let mut sc = OscScanner::new(NONCE);
        // "ls\n" output, then ESC ] 133;A BEL, then more output.
        let mut input = b"ls\r\n".to_vec();
        input.extend_from_slice(ESC);
        input.extend_from_slice(b"]133;A\x07");
        input.extend_from_slice(b"$ ");
        assert_eq!(sc.feed(&input), vec![VtEvent::PromptStart]);
    }

    #[test]
    fn feed_positioned_reports_the_offset_just_past_a_mark() {
        // The engine processes the screen up to a mark's offset before sampling
        // the cursor, so the offset must land exactly after the terminator and
        // before any output that shares the buffer. Build prefix + mark + output
        // in one chunk and assert the offset splits them.
        let mut sc = OscScanner::new(NONCE);
        let prefix = b"mycmd\r\n";
        let mark = b"\x1b]133;C\x07"; // exec start, BEL-terminated
        let output = b"OUTPUT\r\n";
        let mut input = prefix.to_vec();
        input.extend_from_slice(mark);
        input.extend_from_slice(output);

        let got = sc.feed_positioned(&input);
        assert_eq!(got.len(), 1, "exactly the one exec-start mark");
        let (off, ev) = &got[0];
        assert_eq!(*ev, VtEvent::ExecStart);
        assert_eq!(*off, prefix.len() + mark.len(), "offset is just past the terminator");
        assert_eq!(&input[*off..], output, "the bytes after the offset are the output, not the mark");
    }

    #[test]
    fn scanner_extracts_an_st_terminated_command_with_the_nonce() {
        let mut sc = OscScanner::new(NONCE);
        let mut input = Vec::new();
        input.extend_from_slice(ESC);
        input.extend_from_slice(b"]633;E;git status;s3cr3t-nonce");
        input.extend_from_slice(ESC);
        input.extend_from_slice(b"\\"); // ST
        assert_eq!(
            sc.feed(&input),
            vec![VtEvent::CommandLine {
                command: "git status".into()
            }]
        );
    }

    #[test]
    fn scanner_resyncs_a_mark_split_across_feed_chunks() {
        let mut sc = OscScanner::new(NONCE);
        // The mark is cut in the middle of the payload between two reads.
        assert_eq!(sc.feed(b"\x1b]633;D;"), Vec::new());
        assert_eq!(
            sc.feed(b"0\x07"),
            vec![VtEvent::CommandEnd { exit_code: Some(0) }]
        );
    }

    #[test]
    fn a_dcs_payload_containing_osc_bytes_is_not_a_mark() {
        let mut sc = OscScanner::new(NONCE);
        // ESC P (DCS) whose body literally embeds a full `ESC ] 633;E;...;<nonce>
        // BEL` - even WITH the correct nonce - plus a stray BEL. A DCS is consumed
        // to ST (ESC \) only and is never parsed, so neither the embedded OSC nor
        // the BEL can forge or terminate anything: no command escapes the string.
        let mut input = Vec::new();
        input.extend_from_slice(ESC);
        input.extend_from_slice(b"P start-dcs ");
        input.extend_from_slice(ESC);
        input.extend_from_slice(b"]633;E;evil --right-nonce;s3cr3t-nonce\x07 still-dcs ");
        input.extend_from_slice(ESC);
        input.extend_from_slice(b"\\"); // ST finally ends the DCS
        // Only the real mark AFTER the DCS is found; the embedded one never fires.
        input.extend_from_slice(ESC);
        input.extend_from_slice(b"]133;A\x07");
        assert_eq!(sc.feed(&input), vec![VtEvent::PromptStart]);
    }

    #[test]
    fn an_oversized_unterminated_osc_is_dropped_and_resyncs() {
        let mut sc = OscScanner::new(NONCE);
        let mut input = Vec::new();
        input.extend_from_slice(ESC);
        input.extend_from_slice(b"]133;");
        input.extend(std::iter::repeat_n(b'A', MAX_OSC_PAYLOAD + 1000)); // over the cap
        input.extend_from_slice(b"\x07"); // terminates, but it overflowed -> dropped
        // No event from the runaway OSC, and the next real mark is still found.
        input.extend_from_slice(ESC);
        input.extend_from_slice(b"]633;A\x07");
        let events = sc.feed(&input);
        assert_eq!(events, vec![VtEvent::PromptStart]);
        // The buffer did not grow unbounded.
        assert!(sc.buf.len() <= MAX_OSC_PAYLOAD);
    }

    #[test]
    fn an_osc_aborted_by_a_new_escape_yields_no_spurious_event() {
        let mut sc = OscScanner::new(NONCE);
        // ESC ] 133;A  then ESC [ (a CSI) aborts the OSC before any terminator.
        let mut input = Vec::new();
        input.extend_from_slice(ESC);
        input.extend_from_slice(b"]133;A");
        input.extend_from_slice(ESC);
        input.extend_from_slice(b"[0m"); // CSI SGR reset
        assert_eq!(sc.feed(&input), Vec::new());
        // A clean mark afterwards still works.
        let mut next = Vec::new();
        next.extend_from_slice(ESC);
        next.extend_from_slice(b"]133;A\x07");
        assert_eq!(sc.feed(&next), vec![VtEvent::PromptStart]);
    }

    #[test]
    fn plain_output_yields_no_marks() {
        let mut sc = OscScanner::new(NONCE);
        assert!(sc
            .feed(b"total 12\r\ndrwxr-xr-x 1 user user 0 main.rs\r\n")
            .is_empty());
    }

    #[test]
    fn multiple_marks_in_one_chunk() {
        let mut sc = OscScanner::new(NONCE);
        let mut input = Vec::new();
        input.extend_from_slice(ESC);
        input.extend_from_slice(b"]133;A\x07");
        input.extend_from_slice(ESC);
        input.extend_from_slice(b"]633;E;ls;s3cr3t-nonce\x07");
        input.extend_from_slice(ESC);
        input.extend_from_slice(b"]133;C\x07");
        input.extend_from_slice(ESC);
        input.extend_from_slice(b"]133;D;0\x07");
        assert_eq!(
            sc.feed(&input),
            vec![
                VtEvent::PromptStart,
                VtEvent::CommandLine { command: "ls".into() },
                VtEvent::ExecStart,
                VtEvent::CommandEnd { exit_code: Some(0) },
            ]
        );
    }

    #[test]
    fn vt_event_serializes_tagged_for_the_ui() {
        let v = serde_json::to_value(VtEvent::CommandEnd { exit_code: Some(1) }).unwrap();
        assert_eq!(v, serde_json::json!({ "kind": "command_end", "exit_code": 1 }));
        let v = serde_json::to_value(VtEvent::CommandLine {
            command: "ls".into(),
        })
        .unwrap();
        assert_eq!(v, serde_json::json!({ "kind": "command_line", "command": "ls" }));
    }
}
