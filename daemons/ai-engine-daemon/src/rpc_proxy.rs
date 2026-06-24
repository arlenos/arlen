//! The Phase-2-A drive-channel relay (`pi-agent-adoption.md` §A: "pi RPC over
//! stdio = the shell-facing drive channel; the daemon proxies shell <-> pi").
//!
//! A bidirectional JSONL relay between the harness shell and the confined pi
//! sidecar's RPC stdio: commands (`{"type":"prompt",...}`, steer, interrupt)
//! flow shell -> pi's stdin, and pi's agent events flow pi's stdout -> shell.
//! It is a FAITHFUL pass-through - records are forwarded byte-for-byte, never
//! reshaped (reshaping the stream would be a prompt-injection surface; the
//! security boundary is the separate contract socket where pi calls Authorize /
//! Report, not this drive channel).
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

/// [`pump`] with an explicit per-record byte cap (so the bound is unit-testable
/// without allocating megabytes). A record that reaches `max` bytes without a
/// terminating `\n` is rejected as a protocol error rather than buffered further.
async fn pump_capped<R, W>(from: R, mut to: W, max: usize) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
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
        to.write_all(&record).await?;
        to.flush().await?;
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
    let commands = pump(shell_read, pi_stdin); // shell -> pi
    let events = pump(pi_stdout, shell_write); // pi -> shell
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

        shell_in.write_all(b"{\"id\":\"1\"}\n").await.unwrap();
        shell_in.write_all(b"{\"id\":\"2\"}\n").await.unwrap();
        assert_eq!(read_record(&mut pi_in).await, "{\"id\":\"1\"}\n");
        assert_eq!(read_record(&mut pi_in).await, "{\"id\":\"2\"}\n");

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
        c1.write_all(b"{\"id\":\"1\"}\n").await.unwrap();
        assert_eq!(read_record(&mut pi_in).await, "{\"id\":\"1\"}\n");
        drop(c1); // disconnect -> serve_drive re-accepts, pi stdin NOT closed

        // A second shell reconnects to the SAME pi instance and drives it.
        let mut c2 = UnixStream::connect(&path).await.unwrap();
        c2.write_all(b"{\"id\":\"2\"}\n").await.unwrap();
        assert_eq!(read_record(&mut pi_in).await, "{\"id\":\"2\"}\n");

        server.abort();
        let _ = std::fs::remove_file(&path);
    }
}
