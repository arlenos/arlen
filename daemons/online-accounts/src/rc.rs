//! The rclone rc JSON-RPC client (online-accounts-plan.md OA-R3).
//!
//! For an account's `Files` service the daemon drives a CONFINED rclone (its own
//! `arlen-run` subprocess: Landlock + a per-host egress allowlist scoped to the
//! provider + a cgroup) over rclone's remote-control API: a POST of a method path
//! (`mount/mount`, `vfs/refresh`, `core/version`, ...) with a JSON params body,
//! returning JSON. This module is the typed protocol over a [`RcTransport`] seam,
//! so the method layer is tested with a mock; the real HTTP-over-socket transport
//! and the rclone subprocess management are the on-kernel integration on top.

use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

/// An error driving rclone over the rc API.
#[derive(Debug, thiserror::Error)]
pub enum RcError {
    /// The transport (socket / HTTP) failed.
    #[error("transport: {0}")]
    Transport(String),
    /// rclone returned an error result (`{error, status}` on an HTTP 4xx/5xx).
    #[error("rclone error ({status}): {message}")]
    Rclone {
        /// The rc HTTP status.
        status: u16,
        /// The error message rclone reported.
        message: String,
    },
    /// The response was not the shape the method expected.
    #[error("unexpected response: {0}")]
    Unexpected(String),
}

/// The transport that carries one rc method call: a POST of `path` with the JSON
/// `params`, yielding the decoded JSON result (or [`RcError::Rclone`] for an rc
/// error result, [`RcError::Transport`] for a socket failure). The real impl is
/// an HTTP client over the confined rclone's Unix socket; tests use a mock.
#[async_trait::async_trait]
pub trait RcTransport: Send + Sync {
    /// Call rc method `path` with `params`; return the decoded result JSON.
    async fn call(
        &self,
        path: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RcError>;
}

/// One active FUSE mount, as reported by `mount/listmounts`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MountInfo {
    /// The remote/fs string (`gdrive:`).
    #[serde(rename = "Fs")]
    pub fs: String,
    /// The local mount point.
    #[serde(rename = "MountPoint")]
    pub mount_point: String,
}

/// The typed rc client over a transport.
pub struct RcClient<T> {
    transport: T,
}

impl<T: RcTransport> RcClient<T> {
    /// A client driving rclone over `transport`.
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// `core/version` - a health probe; returns the rclone version string.
    pub async fn version(&self) -> Result<String, RcError> {
        let resp = self
            .transport
            .call("core/version", serde_json::json!({}))
            .await?;
        resp.get("version")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| RcError::Unexpected("no version field".into()))
    }

    /// `mount/mount` - FUSE-mount the remote `fs` at `mount_point` (the realised
    /// `Files` capability: the cloud drive becomes a local mount).
    pub async fn mount(&self, fs: &str, mount_point: &str) -> Result<(), RcError> {
        self.transport
            .call(
                "mount/mount",
                serde_json::json!({ "fs": fs, "mountPoint": mount_point }),
            )
            .await
            .map(|_| ())
    }

    /// `mount/unmount` - unmount `mount_point`.
    pub async fn unmount(&self, mount_point: &str) -> Result<(), RcError> {
        self.transport
            .call(
                "mount/unmount",
                serde_json::json!({ "mountPoint": mount_point }),
            )
            .await
            .map(|_| ())
    }

    /// `mount/listmounts` - the active FUSE mounts.
    pub async fn list_mounts(&self) -> Result<Vec<MountInfo>, RcError> {
        let resp = self
            .transport
            .call("mount/listmounts", serde_json::json!({}))
            .await?;
        let points = resp
            .get("mountPoints")
            .ok_or_else(|| RcError::Unexpected("no mountPoints field".into()))?;
        serde_json::from_value(points.clone())
            .map_err(|e| RcError::Unexpected(format!("mountPoints: {e}")))
    }

    /// `vfs/refresh` - refresh the VFS directory cache (so a change made out of
    /// band shows in the mount).
    pub async fn vfs_refresh(&self) -> Result<(), RcError> {
        self.transport
            .call("vfs/refresh", serde_json::json!({}))
            .await
            .map(|_| ())
    }
}

/// The largest rc response we will buffer. rc results are small JSON (a version
/// string, a handful of mount points), so a response past this cap is a buggy or
/// hostile rcd, not a real result; refuse it rather than let it grow the heap.
const MAX_RESPONSE: usize = 8 * 1024 * 1024;

/// The whole-exchange deadline (connect + write + read). A stalled rcd must not
/// pin the daemon; the caller sees a [`RcError::Transport`] timeout instead.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// The real transport: HTTP/1.1 over the confined rclone's rc Unix socket.
///
/// One request per connection - the request carries `Connection: close`, so the
/// response is read to EOF and validated against its `Content-Length` (rclone's
/// rc API is always `Content-Length`-framed, never chunked, and aligns the HTTP
/// status with the JSON `status` field, so a non-2xx body carries `{error}`).
///
/// There is no SSRF surface: this connects ONLY to a local AF_UNIX path the
/// daemon itself spawned the confined rclone on. The socket path is the trust
/// boundary, never a caller-supplied host, so the method/params a caller drives
/// can reach exactly that one rcd and nothing else.
pub struct UnixRcTransport {
    socket_path: PathBuf,
}

impl UnixRcTransport {
    /// A transport to the confined rclone rc API listening on `socket_path`.
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    /// The unbounded-time call body; [`RcTransport::call`] wraps it in the
    /// whole-exchange deadline.
    async fn do_call(
        &self,
        path: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RcError> {
        // The method is a hardcoded rc method string from RcClient, never caller
        // input, but reject anything outside the rc method charset so a method can
        // never carry a CRLF or otherwise break out of the request line.
        if !is_safe_method(path) {
            return Err(RcError::Transport(format!("invalid rc method {path:?}")));
        }
        let body = serde_json::to_vec(&params)
            .map_err(|e| RcError::Transport(format!("serialize params: {e}")))?;
        // The HTTP request target is the rc method path with a single leading
        // slash; the method names carry no query/fragment, so no escaping is due.
        let target = format!("/{}", path.trim_start_matches('/'));
        let head = format!(
            "POST {target} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let mut req = Vec::with_capacity(head.len() + body.len());
        req.extend_from_slice(head.as_bytes());
        req.extend_from_slice(&body);

        let mut stream = UnixStream::connect(&self.socket_path).await.map_err(|e| {
            RcError::Transport(format!("connect {}: {e}", self.socket_path.display()))
        })?;
        stream
            .write_all(&req)
            .await
            .map_err(|e| RcError::Transport(format!("write: {e}")))?;
        stream
            .flush()
            .await
            .map_err(|e| RcError::Transport(format!("flush: {e}")))?;

        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let n = stream
                .read(&mut chunk)
                .await
                .map_err(|e| RcError::Transport(format!("read: {e}")))?;
            if n == 0 {
                break;
            }
            if buf.len() + n > MAX_RESPONSE {
                return Err(RcError::Transport("rc response exceeds cap".into()));
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        parse_response(&buf)
    }
}

#[async_trait::async_trait]
impl RcTransport for UnixRcTransport {
    async fn call(
        &self,
        path: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RcError> {
        match tokio::time::timeout(CALL_TIMEOUT, self.do_call(path, params)).await {
            Ok(result) => result,
            Err(_) => Err(RcError::Transport("rc call timed out".into())),
        }
    }
}

/// Parse a complete raw HTTP/1.1 response (status line + headers + body) into the
/// decoded rc result, mapping a non-2xx status to [`RcError::Rclone`]. Strict by
/// design: a missing header terminator, a chunked body, or a `Content-Length`
/// that disagrees with the bytes read is a framing fault, not a result.
fn parse_response(raw: &[u8]) -> Result<serde_json::Value, RcError> {
    let sep = find_subslice(raw, b"\r\n\r\n")
        .ok_or_else(|| RcError::Transport("malformed response: no header terminator".into()))?;
    let body = &raw[sep + 4..];
    let head = std::str::from_utf8(&raw[..sep])
        .map_err(|_| RcError::Transport("non-utf8 response headers".into()))?;

    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| RcError::Transport("empty response".into()))?;
    let status = parse_status(status_line)?;

    let mut content_length: Option<usize> = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        match name.as_str() {
            // rclone never chunk-encodes; a chunked body would mean a different
            // server in front of the socket - refuse rather than mis-frame it.
            "transfer-encoding" => {
                return Err(RcError::Transport(format!(
                    "unsupported transfer-encoding: {value}"
                )));
            }
            // A Content-Length is authoritative-or-fatal (RFC 9112 6.3): a
            // duplicate, an unparseable, or a non-digit value is a framing fault,
            // never silently dropped - else a "good then bad" pair would disable
            // the cross-check below and let a peer smuggle trailing bytes.
            "content-length" => {
                if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(RcError::Transport(format!("bad content-length: {value}")));
                }
                let cl = value
                    .parse::<usize>()
                    .map_err(|_| RcError::Transport(format!("content-length overflow: {value}")))?;
                if content_length.replace(cl).is_some() {
                    return Err(RcError::Transport("duplicate content-length".into()));
                }
            }
            _ => {}
        }
    }

    // Connection: close means the body is exactly the bytes after the headers; a
    // declared length that disagrees is truncation or trailing junk, both faults.
    if let Some(cl) = content_length {
        if cl != body.len() {
            return Err(RcError::Transport(format!(
                "framing mismatch: content-length {cl}, body {}",
                body.len()
            )));
        }
    }

    if !(200..300).contains(&status) {
        let message = serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
            .unwrap_or_else(|| String::from_utf8_lossy(body).trim().chars().take(200).collect());
        return Err(RcError::Rclone { status, message });
    }

    if body.is_empty() {
        return Ok(serde_json::json!({}));
    }
    serde_json::from_slice(body).map_err(|e| RcError::Transport(format!("decode body: {e}")))
}

/// Parse `HTTP/1.1 200 OK` into the numeric status, rejecting anything that is
/// not an HTTP status line.
fn parse_status(line: &str) -> Result<u16, RcError> {
    let mut parts = line.split_whitespace();
    let version = parts.next().unwrap_or_default();
    if !version.starts_with("HTTP/") {
        return Err(RcError::Transport(format!("not an HTTP response: {line}")));
    }
    let code = parts
        .next()
        .ok_or_else(|| RcError::Transport(format!("no status code: {line}")))?;
    code.parse::<u16>()
        .map_err(|_| RcError::Transport(format!("bad status code: {code}")))
}

/// An rc method path is `[A-Za-z0-9_/.-]+`; reject anything else (control chars,
/// CRLF, whitespace) so a method string can never inject into the request line.
fn is_safe_method(path: &str) -> bool {
    !path.is_empty()
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'/' | b'.' | b'-'))
}

/// Quote one rclone connection-string parameter value. rclone's parser unquotes a
/// double-quoted value and treats a doubled `""` as a literal quote, so a value that
/// could otherwise be read as structure (a `,` starting a new parameter, a `:` ending
/// the parameter section, a `"`, surrounding whitespace, or an empty value) is wrapped
/// in `"..."` with inner quotes doubled. A value of only safe characters is emitted
/// raw (rclone accepts it unquoted). This is what stops a hostile or odd credential /
/// host value from breaking out of its slot into another parameter.
fn quote_conn_value(value: &str) -> String {
    let needs_quote = value.is_empty()
        || value.starts_with(' ')
        || value.ends_with(' ')
        || value.contains([',', '"', ':']);
    if !needs_quote {
        return value.to_string();
    }
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

/// Build an rclone INLINE connection string `:backend,key=value,...:path` (CONN-R9,
/// §8.1). The broker injects the credential into this string at mount time and hands
/// it to [`RcClient::mount`] as the `fs`, so the secret lives ONLY in the broker and
/// is NEVER written to rclone's reversible on-disk config (the §8.1 invariant: take
/// rclone's breadth, reject its credential store). `params` preserve their given order
/// (deterministic output) and every value is quoted per [`quote_conn_value`] so a
/// credential or host value cannot inject an extra parameter. `path` is the remote
/// path within the backend (often empty for a mount root).
pub fn rclone_connection_string(backend: &str, params: &[(&str, &str)], path: &str) -> String {
    let mut out = String::from(":");
    out.push_str(backend);
    for (key, value) in params {
        out.push(',');
        out.push_str(key);
        out.push('=');
        out.push_str(&quote_conn_value(value));
    }
    out.push(':');
    out.push_str(path);
    out
}

/// The index of the first occurrence of `needle` in `haystack`, or `None`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod conn_tests {
    use super::rclone_connection_string;

    #[test]
    fn a_plain_sftp_connection_string_is_unquoted() {
        let s = rclone_connection_string(
            "sftp",
            &[("host", "example.com"), ("user", "bob"), ("pass", "OBSCURED")],
            "/data",
        );
        assert_eq!(s, ":sftp,host=example.com,user=bob,pass=OBSCURED:/data");
    }

    #[test]
    fn an_empty_path_keeps_the_trailing_colon() {
        let s = rclone_connection_string("s3", &[("provider", "AWS")], "");
        assert_eq!(s, ":s3,provider=AWS:");
    }

    #[test]
    fn a_value_with_a_comma_or_colon_is_quoted() {
        // A comma would otherwise start a new parameter; a colon would end the
        // parameter section. Both are quoted so they stay inside the value.
        let s = rclone_connection_string("webdav", &[("url", "http://h:8080/dav,x")], "");
        assert_eq!(s, ":webdav,url=\"http://h:8080/dav,x\":");
    }

    #[test]
    fn an_inner_quote_is_doubled() {
        let s = rclone_connection_string("sftp", &[("pass", "a\"b")], "");
        assert_eq!(s, ":sftp,pass=\"a\"\"b\":");
    }

    #[test]
    fn a_credential_cannot_inject_an_extra_parameter() {
        // A hostile pass value carrying its own `,key=value` is quoted whole, so it
        // stays one parameter value rather than smuggling a second parameter.
        let s = rclone_connection_string(
            "sftp",
            &[("host", "h"), ("pass", "x,user=attacker")],
            "",
        );
        assert_eq!(s, ":sftp,host=h,pass=\"x,user=attacker\":");
    }

    #[test]
    fn surrounding_whitespace_and_empty_values_are_quoted() {
        assert_eq!(rclone_connection_string("b", &[("k", " v")], ""), ":b,k=\" v\":");
        assert_eq!(rclone_connection_string("b", &[("k", "")], ""), ":b,k=\"\":");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// A transport that returns a canned result per path, or a canned rc error,
    /// and records the params it was called with.
    #[derive(Default)]
    struct MockTransport {
        results: HashMap<String, serde_json::Value>,
        error: Option<(u16, String)>,
        calls: Mutex<Vec<(String, serde_json::Value)>>,
    }

    #[async_trait::async_trait]
    impl RcTransport for MockTransport {
        async fn call(
            &self,
            path: &str,
            params: serde_json::Value,
        ) -> Result<serde_json::Value, RcError> {
            self.calls
                .lock()
                .unwrap()
                .push((path.to_string(), params.clone()));
            if let Some((status, message)) = &self.error {
                return Err(RcError::Rclone {
                    status: *status,
                    message: message.clone(),
                });
            }
            Ok(self.results.get(path).cloned().unwrap_or(serde_json::json!({})))
        }
    }

    #[tokio::test]
    async fn version_extracts_the_version_field() {
        let mut t = MockTransport::default();
        t.results.insert(
            "core/version".into(),
            serde_json::json!({ "version": "v1.66.0" }),
        );
        let rc = RcClient::new(t);
        assert_eq!(rc.version().await.unwrap(), "v1.66.0");
    }

    #[tokio::test]
    async fn mount_posts_the_fs_and_mountpoint() {
        let t = MockTransport::default();
        let rc = RcClient::new(t);
        rc.mount("gdrive:", "/home/x/Drive").await.unwrap();
        let calls = rc.transport.calls.lock().unwrap();
        assert_eq!(calls[0].0, "mount/mount");
        assert_eq!(calls[0].1["fs"], "gdrive:");
        assert_eq!(calls[0].1["mountPoint"], "/home/x/Drive");
    }

    #[tokio::test]
    async fn list_mounts_parses_the_pascal_case_fields() {
        let mut t = MockTransport::default();
        t.results.insert(
            "mount/listmounts".into(),
            serde_json::json!({
                "mountPoints": [
                    { "Fs": "gdrive:", "MountPoint": "/home/x/Drive", "MountedOn": "2026-06-11T00:00:00Z" }
                ]
            }),
        );
        let rc = RcClient::new(t);
        let mounts = rc.list_mounts().await.unwrap();
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].fs, "gdrive:");
        assert_eq!(mounts[0].mount_point, "/home/x/Drive");
    }

    #[tokio::test]
    async fn an_rc_error_propagates() {
        // Real rclone returns 400/404 for client-side faults (bad method, bad
        // params, bad JSON); 500 only for a genuine server-side op failure. The
        // client surfaces whatever status the transport reports.
        let t = MockTransport {
            error: Some((400, "Didn't find key \"mountPoint\" in input".into())),
            ..Default::default()
        };
        let rc = RcClient::new(t);
        let err = rc.mount("gdrive:", "/bad").await.unwrap_err();
        assert!(matches!(err, RcError::Rclone { status: 400, .. }), "got {err:?}");
    }

    // --- UnixRcTransport: the real HTTP-over-socket transport ---
    //
    // These drive the transport against a mock UDS that speaks the exact rc wire
    // shape probed from a live `rclone rcd` (status line + arbitrary headers +
    // a Content-Length-framed JSON body, Connection: close).

    /// A canned HTTP/1.1 response with a correct `Content-Length` for `body`.
    fn http_response(status_line: &str, body: &str) -> Vec<u8> {
        format!(
            "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    /// Accept one connection on `listener`, drain the request (headers +
    /// Content-Length body), then write `response` verbatim and close, so the
    /// client reads to EOF exactly as it would against a real `Connection: close`
    /// rcd.
    async fn serve_once(listener: tokio::net::UnixListener, response: Vec<u8>) {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let mut req = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            if let Some(pos) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&req[..pos]);
                let cl: usize = head
                    .lines()
                    .find_map(|l| {
                        let (n, v) = l.split_once(':')?;
                        n.trim()
                            .eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if req.len() - (pos + 4) >= cl {
                    break;
                }
            }
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => req.extend_from_slice(&chunk[..n]),
            }
        }
        let _ = stream.write_all(&response).await;
        let _ = stream.flush().await;
    }

    /// Bind a mock rcd socket under a fresh tempdir and start serving `response`.
    fn mock_rcd(response: Vec<u8>) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("rcd.sock");
        let listener = tokio::net::UnixListener::bind(&sock).unwrap();
        tokio::spawn(serve_once(listener, response));
        (dir, sock)
    }

    #[tokio::test]
    async fn version_over_a_mock_socket_parses_the_framing() {
        let (_dir, sock) = mock_rcd(http_response("HTTP/1.1 200 OK", r#"{"version":"v1.74.3"}"#));
        let rc = RcClient::new(UnixRcTransport::new(&sock));
        assert_eq!(rc.version().await.unwrap(), "v1.74.3");
    }

    #[tokio::test]
    async fn a_non_2xx_status_maps_to_rclone_error() {
        let (_dir, sock) = mock_rcd(http_response(
            "HTTP/1.1 404 Not Found",
            r#"{"error":"couldn't find method","status":404}"#,
        ));
        let rc = RcClient::new(UnixRcTransport::new(&sock));
        let err = rc.mount("gdrive:", "/x").await.unwrap_err();
        match err {
            RcError::Rclone { status, message } => {
                assert_eq!(status, 404);
                assert_eq!(message, "couldn't find method");
            }
            other => panic!("expected rclone error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_content_length_mismatch_is_a_transport_error() {
        // Declared 999 but a short body: truncation/trailing-junk is a framing fault.
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 999\r\nConnection: close\r\n\r\n{}".to_vec();
        let (_dir, sock) = mock_rcd(raw);
        let rc = RcClient::new(UnixRcTransport::new(&sock));
        let err = rc.version().await.unwrap_err();
        assert!(matches!(err, RcError::Transport(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn chunked_transfer_encoding_is_rejected() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nhello\r\n0\r\n\r\n".to_vec();
        let (_dir, sock) = mock_rcd(raw);
        let rc = RcClient::new(UnixRcTransport::new(&sock));
        let err = rc.version().await.unwrap_err();
        assert!(matches!(err, RcError::Transport(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn a_malformed_status_line_is_a_transport_error() {
        let raw = b"NOT-HTTP garbage\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_vec();
        let (_dir, sock) = mock_rcd(raw);
        let t = UnixRcTransport::new(&sock);
        let err = t.call("core/version", serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, RcError::Transport(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn an_empty_2xx_body_is_an_empty_object() {
        let (_dir, sock) = mock_rcd(http_response("HTTP/1.1 200 OK", ""));
        let t = UnixRcTransport::new(&sock);
        let v = t.call("vfs/refresh", serde_json::json!({})).await.unwrap();
        assert_eq!(v, serde_json::json!({}));
    }

    #[tokio::test]
    async fn a_method_outside_the_rc_charset_is_refused_before_connecting() {
        // A method carrying CRLF, a space, a tab, or any non-rc-charset byte (or an
        // empty method) must never reach the request line; the guard rejects it
        // with the distinct "invalid rc method" fault, BEFORE any socket connect.
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("unused.sock");
        let t = UnixRcTransport::new(&sock);
        for bad in ["core/version\r\nX: y", "core version", "", "core\tversion", "core?x=1"] {
            let err = t.call(bad, serde_json::json!({})).await.unwrap_err();
            match err {
                RcError::Transport(m) => assert!(
                    m.contains("invalid rc method"),
                    "{bad:?} should be guard-rejected, got {m:?}"
                ),
                other => panic!("{bad:?} -> {other:?}"),
            }
        }
        // A legitimate method passes the guard (and then fails only on the dead
        // socket - a DISTINCT "connect" fault), proving the guard does not
        // over-reject real rc paths.
        let err = t.call("mount/listmounts", serde_json::json!({})).await.unwrap_err();
        assert!(
            matches!(&err, RcError::Transport(m) if m.contains("connect")),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn a_duplicate_content_length_is_a_transport_error() {
        // Two Content-Length headers (the request-smuggling shape): a framing fault,
        // never a silently-disabled cross-check.
        let raw =
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}"
                .to_vec();
        let (_dir, sock) = mock_rcd(raw);
        let t = UnixRcTransport::new(&sock);
        let err = t.call("core/version", serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, RcError::Transport(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn an_unparseable_content_length_is_a_transport_error() {
        let raw =
            b"HTTP/1.1 200 OK\r\nContent-Length: xyz\r\nConnection: close\r\n\r\n{}".to_vec();
        let (_dir, sock) = mock_rcd(raw);
        let t = UnixRcTransport::new(&sock);
        let err = t.call("core/version", serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, RcError::Transport(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn a_dead_socket_path_is_a_transport_error() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("absent.sock");
        let t = UnixRcTransport::new(&sock);
        let err = t.call("core/version", serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, RcError::Transport(_)), "got {err:?}");
    }

    /// Drives the real `rclone rcd` over a Unix socket end-to-end: spawn it on a
    /// temp socket, probe `core/version`, assert a real version string. Ignored
    /// in normal CI (needs rclone on PATH); run with `--ignored`.
    #[tokio::test]
    #[ignore = "needs rclone on PATH; run with --ignored"]
    async fn real_rclone_version_over_the_rc_socket() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("rcd.sock");
        let mut child = std::process::Command::new("rclone")
            .args(["rcd", "--rc-no-auth"])
            .arg(format!("--rc-addr=unix://{}", sock.display()))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn rclone rcd");

        let mut ready = false;
        for _ in 0..50 {
            if sock.exists() {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(ready, "rclone rc socket did not appear");

        let rc = RcClient::new(UnixRcTransport::new(&sock));
        let result = rc.version().await;

        let _ = child.kill();
        let _ = child.wait();

        let version = result.expect("core/version");
        assert!(version.starts_with('v'), "unexpected version {version}");
    }
}
