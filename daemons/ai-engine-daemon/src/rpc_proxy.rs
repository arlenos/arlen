//! The Phase-2-A drive-channel relay (`pi-agent-adoption.md` §A: "pi RPC over
//! stdio = the shell-facing drive channel; the daemon proxies shell <-> pi").
//!
//! A bidirectional JSONL relay between the harness shell and the confined pi
//! sidecar's RPC stdio: conversational commands (`{"type":"prompt",...}`, steer,
//! follow_up, abort, get_last_assistant_text) flow shell -> pi's stdin, and pi's
//! agent events flow pi's stdout -> shell. The pi -> shell direction is a faithful
//! byte-for-byte pass-through (pi's events are trusted output; reshaping them would
//! be a prompt-injection surface). The shell -> pi direction is ALLOWLISTED to the
//! conversational verbs: pi's operator RPC surface also carries `bash` (a raw local
//! shell exec that never fires a gated `tool_call`), `switch_session`,
//! `get_messages`, `export_html`, ... and forwarding those verbatim would let a
//! drive-socket peer run un-gated code or read another session, so the relay drops
//! every non-conversational command (fail-closed). The gate for what an admitted
//! prompt can then DO is still the separate contract socket (Authorize / Report).
//!
//! Framing is pi's strict JSONL: LF (`\n`) is the only record delimiter
//! (`rpc.md`). Each record is read up to and including its newline and written
//! out verbatim, so an embedded `\r`, `U+2028` or `U+2029` inside a JSON string
//! is never mistaken for a record boundary (the bug pi's docs warn generic line
//! readers hit). The relay ends when EITHER side closes: a shell disconnect
//! drops pi's stdin sink (pi sees EOF and winds down its session), and pi
//! exiting closes its stdout (the shell sees the stream end).
//!
//! This is the MECHANISM, exercised here over in-memory streams. The
//! shell-facing drive socket (A2) and the sidecar handing over pi's piped
//! stdin/stdout (A3) are the wiring that feeds it.

use std::io;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use arlen_permissions::connection_auth::ConnectionAuth;

/// The largest single JSONL record the relay will buffer, in either direction.
/// Generous enough for a `prompt` carrying an inline base64 image, but bounded so
/// a peer that streams bytes without ever sending `\n` cannot grow the daemon's
/// memory without limit (the same hazard `wire.rs` bounds with `MAX_FRAME`). A
/// record over this cap ends the relay rather than allocating unboundedly.
const MAX_RECORD: usize = 16 * 1024 * 1024;

/// Forward one LF-delimited record at a time from `from` to `to` until `from`
/// reaches EOF, flushing after each so a streamed token reaches the peer
/// promptly rather than sitting in a buffer.
async fn pump<R, W>(from: R, to: W) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    pump_capped(from, to, MAX_RECORD).await
}

/// The pi RPC commands the drive channel permits (shell -> pi). pi's operator RPC
/// surface ALSO includes `bash` (a raw local shell exec that never fires a
/// `tool_call`), `switch_session` / `export_html` / `get_messages` / `fork` /
/// `set_model` / `get_state` / ... - none of which are gated `tool_call`s, so
/// forwarding them verbatim would let any drive-socket peer run un-gated,
/// un-audited code, load an arbitrary session file, or read another principal's
/// conversation. The relay therefore ALLOWLISTS the conversational verbs the
/// harness legitimately sends and DROPS every other command (fail-closed): the
/// un-gated operator surface is unreachable over the drive channel.
const ALLOWED_DRIVE_COMMANDS: &[&str] =
    &["prompt", "steer", "follow_up", "abort", "get_last_assistant_text"];

/// Whether a shell -> pi JSONL record is an allowed drive command. A record that
/// is not valid JSON, carries no string `type`, or whose `type` is not on the
/// allowlist is REJECTED, so a malformed or operator command never reaches pi.
fn is_allowed_drive_command(record: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(record)
        .ok()
        .as_ref()
        .and_then(|v| v.get("type"))
        .and_then(|t| t.as_str())
        .is_some_and(|t| ALLOWED_DRIVE_COMMANDS.contains(&t))
}

/// [`pump`] with an explicit per-record byte cap (so the bound is unit-testable
/// without allocating megabytes). A record that reaches `max` bytes without a
/// terminating `\n` is rejected as a protocol error rather than buffered further.
async fn pump_capped<R, W>(from: R, to: W, max: usize) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    // The bare pump forwards every record (used pi -> shell, where pi's events
    // are trusted output, not commands).
    pump_filtered(from, to, max, |_| true).await
}

/// [`pump_capped`] that forwards a record only when `allow(record)` holds; a
/// rejected record is DROPPED and the relay keeps running (fail-closed). Used
/// shell -> pi with [`is_allowed_drive_command`] so only conversational verbs
/// reach pi.
async fn pump_filtered<R, W, F>(from: R, mut to: W, max: usize, allow: F) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    F: Fn(&[u8]) -> bool,
{
    let mut from = BufReader::new(from);
    let mut record = Vec::new();
    loop {
        record.clear();
        // LF-only framing, bounded: read through the next `\n` (kept) or to EOF,
        // but never more than `max + 1` bytes, so an unterminated record cannot
        // grow `record` without limit.
        let n = (&mut from)
            .take((max as u64).saturating_add(1))
            .read_until(b'\n', &mut record)
            .await?;
        if n == 0 {
            return Ok(()); // EOF: this direction is done.
        }
        if record.len() > max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "rpc record exceeds the maximum length",
            ));
        }
        if allow(&record) {
            to.write_all(&record).await?;
            to.flush().await?;
        }
        // else: drop the disallowed command silently (fail-closed); the relay
        // stays up, the peer's rejected command is simply a no-op.
    }
}

/// Bridge the harness shell to the pi RPC sidecar. Commands read from
/// `shell_read` are written to `pi_stdin`; events read from `pi_stdout` are
/// written to `shell_write`. Returns when either direction ends (the session is
/// over). The first direction to finish drops the other's sink, signalling EOF
/// to that peer.
pub async fn relay<SR, SW, PR, PW>(
    shell_read: SR,
    shell_write: SW,
    mut pi_stdin: PW,
    mut pi_stdout: PR,
) -> io::Result<()>
where
    SR: AsyncRead + Unpin,
    SW: AsyncWrite + Unpin,
    PR: AsyncRead + Unpin,
    PW: AsyncWrite + Unpin,
{
    relay_borrowed(shell_read, shell_write, &mut pi_stdin, &mut pi_stdout).await
}

/// [`relay`] over BORROWED pi stdio, so the drive server can run one relay per
/// shell connection while RETAINING pi's stdin/stdout across reconnects within a
/// single pi instance (a shell disconnect must not close pi's stdin).
async fn relay_borrowed<SR, SW, PR, PW>(
    shell_read: SR,
    shell_write: SW,
    pi_stdin: &mut PW,
    pi_stdout: &mut PR,
) -> io::Result<()>
where
    SR: AsyncRead + Unpin,
    SW: AsyncWrite + Unpin,
    PR: AsyncRead + Unpin,
    PW: AsyncWrite + Unpin,
{
    // shell -> pi: ALLOWLISTED to conversational verbs (drops pi's un-gated
    // operator commands like `bash`/`switch_session`/`get_messages`).
    let commands = pump_filtered(shell_read, pi_stdin, MAX_RECORD, is_allowed_drive_command);
    let events = pump(pi_stdout, shell_write); // pi -> shell (trusted output, verbatim)
    tokio::select! {
        r = commands => r,
        r = events => r,
    }
}

/// Serve shell drive connections on `listener` for one pi instance's lifetime,
/// relaying each accepted connection against the instance's RPC stdio.
///
/// The drive socket's 0600 permissions (set at bind) are the same-uid boundary -
/// only the owning user can `connect()` a 0600 socket - so no per-connection
/// auth is done here: the channel carries prompts to the user's OWN pi, and the
/// security gate for what those prompts can DO is the separate contract socket
/// (Authorize/Report).
///
/// Reconnect-capable: pi's stdin/stdout are retained across connections, so a
/// shell disconnect does NOT close pi's stdin (the session survives) and the
/// next shell can reconnect to the same pi. A per-connection relay error is
/// logged and the next connection awaited. This loops until `accept` fails (a
/// broken listener) or the caller cancels it (the supervisor cancels it when pi
/// exits).
/// The uid the daemon runs as; a drive peer must match it (cross-uid rejected).
fn current_uid() -> u32 {
    // SAFETY: getuid is always safe; it reads the real uid and never fails.
    unsafe { libc::getuid() }
}

pub async fn serve_drive<PW, PR>(
    listener: &UnixListener,
    mut pi_stdin: PW,
    mut pi_stdout: PR,
) -> io::Result<()>
where
    PW: AsyncWrite + Unpin,
    PR: AsyncRead + Unpin,
{
    loop {
        let (stream, _addr) = listener.accept().await?;
        // Peer-attest the shell before relaying, bringing the drive socket to the
        // SAME posture as the contract socket (SO_PEERCRED, cross-uid rejected, the
        // pid attested) rather than resting on the 0600 bind alone. A connection
        // whose credentials cannot be read, or that is cross-uid, is DROPPED and we
        // re-accept. The residual same-uid boundary (any same-uid process may still
        // connect) is the Arlen-wide one closed by F3/AppArmor, identical to the
        // contract socket; the drive channel is no longer the weaker sibling.
        if let Err(e) = ConnectionAuth::extract_from(&stream, current_uid()) {
            tracing::warn!(error = %e, "rejecting an unauthenticated drive connection");
            continue;
        }
        let (read_half, write_half) = stream.into_split();
        if let Err(e) =
            relay_borrowed(read_half, write_half, &mut pi_stdin, &mut pi_stdout).await
        {
            tracing::warn!(error = %e, "drive relay session ended with an error; awaiting the next shell");
        }
        // The shell disconnected (or its session errored); pi's stdio is retained,
        // so re-accept the next shell connection for this pi instance.
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn allows_only_conversational_verbs() {
        use super::is_allowed_drive_command;
        // Conversational verbs the harness sends are allowed.
        assert!(is_allowed_drive_command(br#"{"type":"prompt","message":"hi"}"#));
        assert!(is_allowed_drive_command(br#"{"type":"get_last_assistant_text","id":"x"}"#));
        assert!(is_allowed_drive_command(br#"{"type":"abort"}"#));
        // pi's un-gated operator surface is rejected.
        assert!(!is_allowed_drive_command(br#"{"type":"bash","command":"rm -rf ~"}"#));
        assert!(!is_allowed_drive_command(br#"{"type":"switch_session","sessionPath":"/etc/x"}"#));
        assert!(!is_allowed_drive_command(br#"{"type":"get_messages"}"#));
        assert!(!is_allowed_drive_command(br#"{"type":"export_html","outputPath":"/x"}"#));
        assert!(!is_allowed_drive_command(br#"{"type":"get_state"}"#));
        // Malformed / typeless records fail closed.
        assert!(!is_allowed_drive_command(b"not json"));
        assert!(!is_allowed_drive_command(br#"{"no":"type"}"#));
        assert!(!is_allowed_drive_command(br#"{"type":123}"#));
    }

    #[tokio::test]
    async fn the_command_pump_drops_disallowed_records_and_forwards_allowed_ones() {
        use super::{is_allowed_drive_command, pump_filtered, MAX_RECORD};
        // A bash command then a prompt: only the prompt reaches pi.
        let input = b"{\"type\":\"bash\",\"command\":\"id\"}\n{\"type\":\"prompt\",\"message\":\"hi\"}\n";
        let mut out = Vec::new();
        pump_filtered(&input[..], &mut out, MAX_RECORD, is_allowed_drive_command).await.unwrap();
        let forwarded = String::from_utf8(out).unwrap();
        assert!(!forwarded.contains("bash"), "the bash command must be dropped");
        assert!(forwarded.contains("prompt"), "the prompt must be forwarded");
    }

    use super::*;

    /// Read one LF-terminated record (as a String) from `r`. Reads byte-by-byte
    /// rather than via a `BufReader`, which would over-read the next record into
    /// its buffer and then discard it when dropped - leaving a second
    /// `read_record` on the same stream blocked forever (the bug a buffered
    /// reader hides in a multi-record test).
    async fn read_record<R: AsyncRead + Unpin>(r: &mut R) -> String {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            if r.read(&mut byte).await.unwrap() == 0 {
                break; // EOF
            }
            buf.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
        String::from_utf8(buf).unwrap()
    }

    #[tokio::test]
    async fn a_shell_command_reaches_pi_stdin_verbatim() {
        let (mut shell_in, shell_read) = tokio::io::duplex(4096);
        let (pi_stdin, mut pi_in) = tokio::io::duplex(4096);
        let (_pi_out, pi_stdout) = tokio::io::duplex(4096);
        let (shell_write, _shell_out) = tokio::io::duplex(4096);

        let relay = tokio::spawn(relay(shell_read, shell_write, pi_stdin, pi_stdout));

        // A command with an embedded escaped newline inside the JSON string must
        // NOT be split: only the trailing LF terminates the record.
        let cmd = "{\"type\":\"prompt\",\"message\":\"a\\nb\"}\n";
        shell_in.write_all(cmd.as_bytes()).await.unwrap();
        assert_eq!(read_record(&mut pi_in).await, cmd);

        drop(shell_in); // shell disconnects -> relay winds down
        relay.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_pi_event_reaches_the_shell_verbatim() {
        let (_shell_in, shell_read) = tokio::io::duplex(4096);
        let (pi_stdin, _pi_in) = tokio::io::duplex(4096);
        let (mut pi_out, pi_stdout) = tokio::io::duplex(4096);
        let (shell_write, mut shell_out) = tokio::io::duplex(4096);

        let relay = tokio::spawn(relay(shell_read, shell_write, pi_stdin, pi_stdout));

        let event = "{\"type\":\"assistantMessageEvent\",\"delta\":\"hi\"}\n";
        pi_out.write_all(event.as_bytes()).await.unwrap();
        assert_eq!(read_record(&mut shell_out).await, event);

        drop(pi_out); // pi exits -> relay ends
        relay.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn multiple_records_relay_in_order() {
        let (mut shell_in, shell_read) = tokio::io::duplex(4096);
        let (pi_stdin, mut pi_in) = tokio::io::duplex(4096);
        let (_pi_out, pi_stdout) = tokio::io::duplex(4096);
        let (shell_write, _shell_out) = tokio::io::duplex(4096);

        let relay = tokio::spawn(relay(shell_read, shell_write, pi_stdin, pi_stdout));

        shell_in.write_all(b"{\"type\":\"abort\",\"id\":\"1\"}\n").await.unwrap();
        shell_in.write_all(b"{\"type\":\"abort\",\"id\":\"2\"}\n").await.unwrap();
        assert_eq!(read_record(&mut pi_in).await, "{\"type\":\"abort\",\"id\":\"1\"}\n");
        assert_eq!(read_record(&mut pi_in).await, "{\"type\":\"abort\",\"id\":\"2\"}\n");

        drop(shell_in);
        relay.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_shell_disconnect_ends_the_relay() {
        let (shell_in, shell_read) = tokio::io::duplex(4096);
        let (pi_stdin, _pi_in) = tokio::io::duplex(4096);
        let (_pi_out, pi_stdout) = tokio::io::duplex(4096);
        let (shell_write, _shell_out) = tokio::io::duplex(4096);

        let handle = tokio::spawn(relay(shell_read, shell_write, pi_stdin, pi_stdout));
        drop(shell_in); // immediate EOF on the command direction
        // The relay resolves rather than hanging.
        handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn pump_rejects_a_record_over_the_cap() {
        // A record that reaches the cap without a terminating newline is a
        // protocol error, not an unbounded allocation (H1).
        let over = pump_capped(&b"123456789"[..], tokio::io::sink(), 8).await;
        assert!(over.is_err(), "an over-cap LF-less record must be rejected");
        // A within-cap, newline-terminated record relays fine, then EOF -> Ok.
        let ok = pump_capped(&b"12345\n"[..], tokio::io::sink(), 8).await;
        assert!(ok.is_ok());
    }

    fn drive_sock(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("arlen-drive-test-{}-{}.sock", std::process::id(), name));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[tokio::test]
    async fn serve_drive_bridges_a_connected_shell_to_pi() {
        use tokio::net::UnixStream;

        let path = drive_sock("bridge");
        let listener = UnixListener::bind(&path).unwrap();
        let (pi_stdin, mut pi_in) = tokio::io::duplex(4096);
        let (mut pi_out, pi_stdout) = tokio::io::duplex(4096);

        let server = tokio::spawn(async move { serve_drive(&listener, pi_stdin, pi_stdout).await });

        let mut client = UnixStream::connect(&path).await.unwrap();
        // A command from the shell reaches pi's stdin verbatim.
        client.write_all(b"{\"type\":\"prompt\",\"message\":\"hi\"}\n").await.unwrap();
        assert_eq!(read_record(&mut pi_in).await, "{\"type\":\"prompt\",\"message\":\"hi\"}\n");
        // A pi event reaches the shell verbatim.
        pi_out.write_all(b"{\"type\":\"event\"}\n").await.unwrap();
        assert_eq!(read_record(&mut client).await, "{\"type\":\"event\"}\n");

        drop(client); // the shell disconnects; serve_drive loops to re-accept
        server.abort();
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn serve_drive_accepts_a_second_shell_after_the_first_disconnects() {
        use tokio::net::UnixStream;

        let path = drive_sock("reconnect");
        let listener = UnixListener::bind(&path).unwrap();
        // pi's stdin is retained across connections; both shells write to it.
        let (pi_stdin, mut pi_in) = tokio::io::duplex(4096);
        let (_pi_out, pi_stdout) = tokio::io::duplex(4096);
        let server = tokio::spawn(async move {
            let _ = serve_drive(&listener, pi_stdin, pi_stdout).await;
        });

        // First shell session.
        let mut c1 = UnixStream::connect(&path).await.unwrap();
        c1.write_all(b"{\"type\":\"abort\",\"id\":\"1\"}\n").await.unwrap();
        assert_eq!(read_record(&mut pi_in).await, "{\"type\":\"abort\",\"id\":\"1\"}\n");
        drop(c1); // disconnect -> serve_drive re-accepts, pi stdin NOT closed

        // A second shell reconnects to the SAME pi instance and drives it.
        let mut c2 = UnixStream::connect(&path).await.unwrap();
        c2.write_all(b"{\"type\":\"abort\",\"id\":\"2\"}\n").await.unwrap();
        assert_eq!(read_record(&mut pi_in).await, "{\"type\":\"abort\",\"id\":\"2\"}\n");

        server.abort();
        let _ = std::fs::remove_file(&path);
    }
}
