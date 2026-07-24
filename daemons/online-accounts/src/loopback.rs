//! OA-R2: the RFC-8252 §7.3 loopback redirect receiver.
//!
//! After the daemon opens the authorization URL in the system browser, the
//! provider redirects to `http://127.0.0.1:<port>/?code=...&state=...`. This
//! binds an ephemeral loopback port, accepts the single redirect request,
//! extracts the authorization code, checks the CSRF `state` against the one the
//! daemon generated, replies with a small close-this-window page, and returns
//! the code. Binding to `127.0.0.1` means only a loopback client can reach it.
//!
//! The request-line parsing is pure + unit-tested; the socket receive is driven
//! by a real client in the module's own tests (no browser needed).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use percent_encoding::percent_decode_str;

/// The largest request head (line + trailing CRLF) read before giving up: an
/// OAuth redirect line is short, so this bounds a slow/hostile client.
const MAX_REQUEST_LINE: usize = 8 * 1024;

/// A close-this-window page returned to the browser on success.
const SUCCESS_BODY: &str =
    "<!doctype html><title>Signed in</title><p>Sign-in complete. You can close this window.</p>";
/// The page returned when the redirect is rejected (bad state / provider error).
const FAILURE_BODY: &str =
    "<!doctype html><title>Sign-in failed</title><p>Sign-in could not be completed.</p>";

/// What the redirect request line carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectOutcome {
    /// The authorization code + the echoed `state` (checked against expected).
    Code {
        /// The authorization code to exchange for tokens.
        code: String,
        /// The `state` the provider echoed back.
        state: String,
    },
    /// The provider denied/failed the request (RFC 6749 §4.1.2.1).
    Denied {
        /// The `error` code (e.g. `access_denied`).
        error: String,
    },
    /// Not a recognisable OAuth redirect (no `code` and no `error`).
    Unrecognised,
}

/// A loopback receive failure.
#[derive(Debug, thiserror::Error)]
pub enum RecvError {
    /// A socket error accepting/reading/writing.
    #[error("loopback io: {0}")]
    Io(#[from] std::io::Error),
    /// The echoed `state` did not match the expected one (CSRF guard).
    #[error("state mismatch (possible CSRF)")]
    StateMismatch,
    /// The provider returned an error on the redirect.
    #[error("authorization denied: {0}")]
    Denied(String),
    /// The request was not a recognisable OAuth redirect.
    #[error("unrecognised redirect request")]
    Unrecognised,
}

/// Extract the request target (`/path?query`) from an HTTP request line
/// (`GET /path?query HTTP/1.1`). `None` if it is not a GET request line.
pub fn request_target(line: &str) -> Option<&str> {
    let mut parts = line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    parts.next()
}

/// Parse an OAuth redirect from an HTTP request line. Reads the `code`/`state`
/// or `error` from the target's query, percent-decoding values.
pub fn parse_redirect_line(line: &str) -> RedirectOutcome {
    let Some(target) = request_target(line) else {
        return RedirectOutcome::Unrecognised;
    };
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for pair in query.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let value = decode(v);
        match k {
            "code" => code = Some(value),
            "state" => state = Some(value),
            "error" => error = Some(value),
            _ => {}
        }
    }
    match (code, error) {
        (Some(code), _) => RedirectOutcome::Code {
            code,
            state: state.unwrap_or_default(),
        },
        (None, Some(error)) => RedirectOutcome::Denied { error },
        (None, None) => RedirectOutcome::Unrecognised,
    }
}

/// Percent-decode a query value (lossy on invalid UTF-8, which a real code/
/// state never is).
fn decode(v: &str) -> String {
    percent_decode_str(v).decode_utf8_lossy().into_owned()
}

/// A bound loopback redirect receiver.
pub struct LoopbackReceiver {
    listener: TcpListener,
}

impl LoopbackReceiver {
    /// Bind an ephemeral loopback port (`127.0.0.1:0`).
    pub fn bind() -> std::io::Result<Self> {
        Ok(Self {
            listener: TcpListener::bind(("127.0.0.1", 0))?,
        })
    }

    /// The bound port (for the `redirect_uri`).
    pub fn port(&self) -> std::io::Result<u16> {
        Ok(self.listener.local_addr()?.port())
    }

    /// The loopback redirect URI to register in the authorization request.
    pub fn redirect_uri(&self) -> std::io::Result<String> {
        Ok(format!("http://127.0.0.1:{}/", self.port()?))
    }

    /// Accept the single redirect, check `expected_state`, reply to the browser,
    /// and return the authorization code. Blocks on accept; the caller enforces
    /// any overall wall-clock deadline (dropping the receiver closes the port).
    pub fn recv(&self, expected_state: &str) -> Result<String, RecvError> {
        let (mut stream, _) = self.listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let line = read_request_line(&mut stream)?;
        let (ok, result) = match parse_redirect_line(&line) {
            RedirectOutcome::Code { code, state } if state == expected_state => (true, Ok(code)),
            RedirectOutcome::Code { .. } => (false, Err(RecvError::StateMismatch)),
            RedirectOutcome::Denied { error } => (false, Err(RecvError::Denied(error))),
            RedirectOutcome::Unrecognised => (false, Err(RecvError::Unrecognised)),
        };
        let body = if ok { SUCCESS_BODY } else { FAILURE_BODY };
        let _ = write_response(&mut stream, body);
        result
    }
}

/// Read the request, returning its first line. Consumes the whole request head
/// (up to the `\r\n\r\n` header terminator, or EOF, or [`MAX_REQUEST_LINE`]) so
/// that the client's bytes are drained before the response - closing with
/// unread data in the socket sends an RST that resets the browser's read.
fn read_request_line<R: Read>(reader: &mut R) -> std::io::Result<String> {
    let mut buf = Vec::new();
    let mut first_line_end = None;
    let mut byte = [0u8; 1];
    while buf.len() < MAX_REQUEST_LINE {
        let n = reader.read(&mut byte)?;
        if n == 0 {
            break;
        }
        buf.push(byte[0]);
        if first_line_end.is_none() && byte[0] == b'\n' {
            first_line_end = Some(buf.len());
        }
        if buf.ends_with(b"\r\n\r\n") || buf.ends_with(b"\n\n") {
            break;
        }
    }
    let line = match first_line_end {
        Some(end) => &buf[..end],
        None => &buf[..],
    };
    Ok(String::from_utf8_lossy(line).trim_end().to_string())
}

/// Write a minimal HTTP/1.1 200 response with `body` and close.
fn write_response<W: Write>(writer: &mut W, body: &str) -> std::io::Result<()> {
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    writer.write_all(resp.as_bytes())?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::net::TcpStream;

    #[test]
    fn request_target_and_parse() {
        assert_eq!(
            request_target("GET /?code=abc&state=xyz HTTP/1.1"),
            Some("/?code=abc&state=xyz")
        );
        assert_eq!(request_target("POST / HTTP/1.1"), None);

        assert_eq!(
            parse_redirect_line("GET /?code=the%2Dcode&state=st HTTP/1.1"),
            RedirectOutcome::Code {
                code: "the-code".into(),
                state: "st".into()
            }
        );
        assert_eq!(
            parse_redirect_line("GET /?error=access_denied&state=st HTTP/1.1"),
            RedirectOutcome::Denied {
                error: "access_denied".into()
            }
        );
        assert_eq!(
            parse_redirect_line("GET /favicon.ico HTTP/1.1"),
            RedirectOutcome::Unrecognised
        );
    }

    #[test]
    fn redirect_uri_is_loopback() {
        let r = LoopbackReceiver::bind().unwrap();
        let uri = r.redirect_uri().unwrap();
        assert!(uri.starts_with("http://127.0.0.1:"));
        assert!(uri.ends_with('/'));
    }

    /// End-to-end over a real loopback socket: a client sends the redirect GET,
    /// recv returns the code + replies to the browser.
    #[test]
    fn recv_returns_the_code_and_replies() {
        let r = LoopbackReceiver::bind().unwrap();
        let port = r.port().unwrap();
        // Queue the redirect request, then accept it (single-threaded: the tiny
        // request buffers in the socket for recv to read).
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .write_all(b"GET /?code=auth-code-1&state=state-1 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .unwrap();

        let code = r.recv("state-1").unwrap();
        assert_eq!(code, "auth-code-1");

        // The browser gets a 200 close-window response.
        let mut resp = String::new();
        client.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 200 OK"));
        assert!(resp.contains("close this window"));
    }

    #[test]
    fn recv_rejects_a_state_mismatch() {
        let r = LoopbackReceiver::bind().unwrap();
        let port = r.port().unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .write_all(b"GET /?code=c&state=WRONG HTTP/1.1\r\n\r\n")
            .unwrap();
        match r.recv("expected") {
            Err(RecvError::StateMismatch) => {}
            other => panic!("expected StateMismatch, got {other:?}"),
        }
    }

    #[test]
    fn recv_surfaces_a_denial() {
        let r = LoopbackReceiver::bind().unwrap();
        let port = r.port().unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .write_all(b"GET /?error=access_denied&state=st HTTP/1.1\r\n\r\n")
            .unwrap();
        match r.recv("st") {
            Err(RecvError::Denied(e)) => assert_eq!(e, "access_denied"),
            other => panic!("expected Denied, got {other:?}"),
        }
    }
}
