//! The pi model-completion egress (Option B, daemon-mediated).
//!
//! pi's provider (LLM) calls are privileged egress: by the two-class rule pi must
//! not hold the credential or make the raw call (`pi-agent-adoption.md` §67-76).
//! pi runs `--unshare-net`, so its ONLY channel out is a bound Unix socket. Its
//! undici global dispatcher is overridden to dial THIS socket for every provider
//! request; the daemon then authenticates the caller and forwards the raw body
//! through the governed `ai-proxy` (`forward_completion`), which the daemon is
//! already trusted to reach (it is in ai-proxy's `PeerAuthMap`). So pi never sees
//! a key and never dials the model, and the egress trust boundary is unchanged
//! (the daemon is the caller, not pi) - the reason this is daemon-mediated rather
//! than pi hitting ai-proxy directly (pi's `/proc/pid/exe` is the generic node
//! binary, which ai-proxy's exe-based peer-auth cannot distinguish from any other
//! node process).
//!
//! Auth: SO_PEERCRED (same-uid, cross-uid rejected) via `ConnectionAuth`, plus the
//! session token pi presents as its provider API key (`Authorization: Bearer`),
//! verified against the same `SessionStore` the contract socket uses - so only the
//! attested, session-bound pi may transit.
//!
//! Transport: a deliberately minimal HTTP/1.1 read-body / write-response over the
//! Unix socket (pi's undici client speaks HTTP; one fixed route, no dependency on
//! a full HTTP server). The raw request body is forwarded verbatim (Ollama's
//! OpenAI-compatible endpoint is the catalogued `ollama-default` provider), so no
//! request-shape translation happens here.

use std::sync::Arc;
use std::time::Duration;

use arlen_ai_core::proxied::ProxyAIClient;
use arlen_permissions::connection_auth::peer_credentials;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Semaphore;
use tracing::{debug, warn};

use crate::dispatch::SessionVerifier;
use crate::session::SessionToken;

/// The catalogued provider the raw body is forwarded through. Ollama's
/// OpenAI-compatible endpoint; the daemon never lets pi name the provider or the
/// URL - the endpoint comes from ai-proxy's trusted catalog.
const PROVIDER_NAME: &str = "ollama-default";

/// Cap on the request head (request line + headers) before the body. A client
/// that never terminates the head is dropped rather than buffered unbounded.
const MAX_HEAD: usize = 16 * 1024;

/// Cap on the request body. LLM chat requests are small; a larger body is a
/// misbehaving or hostile client, refused before any egress.
const MAX_BODY: usize = 4 * 1024 * 1024;

/// Max time to read a full request (head + body) before the connection is
/// dropped. A same-uid client that connects and dribbles bytes, or never
/// terminates the head, is reaped rather than parking a task and fd indefinitely.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Max concurrent completion connections served at once. Bounds a same-uid client
/// that opens many slow connections; further connections wait for a permit.
const MAX_CONNECTIONS: usize = 16;

/// What a parsed request head yields: the bearer token (the session token pi
/// presents as its API key) and the declared body length.
struct RequestHead {
    bearer: Option<String>,
    content_length: usize,
}

/// Parse the HTTP/1.1 request head (everything up to the blank line). Extracts
/// `Authorization: Bearer <token>` and `Content-Length`; every other header is
/// ignored (this endpoint serves exactly one POST route). Header names are
/// matched case-insensitively per RFC 7230. Pure, so it is unit-tested.
fn parse_head(head: &str) -> RequestHead {
    let mut bearer = None;
    let mut content_length = 0usize;
    // Skip the request line (line 0); parse header lines.
    for line in head.split("\r\n").skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        match name.as_str() {
            "authorization" => {
                if let Some(tok) = value.strip_prefix("Bearer ").or_else(|| value.strip_prefix("bearer ")) {
                    bearer = Some(tok.trim().to_string());
                }
            }
            "content-length" => {
                content_length = value.parse().unwrap_or(0);
            }
            _ => {}
        }
    }
    RequestHead {
        bearer,
        content_length,
    }
}

/// Read the request head (until CRLFCRLF) then the Content-Length body. Returns
/// the parsed head and the body bytes, or an error on a malformed/oversized
/// request (fail-closed: the connection is then dropped).
async fn read_request(stream: &mut UnixStream) -> std::io::Result<(RequestHead, Vec<u8>)> {
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    // Read until the head terminator or the head cap.
    let head_end = loop {
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > MAX_HEAD {
            return Err(std::io::Error::other("request head too large"));
        }
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Err(std::io::Error::other("connection closed before request head"));
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head_str = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let head = parse_head(&head_str);
    if head.content_length > MAX_BODY {
        return Err(std::io::Error::other("request body too large"));
    }
    // Body starts after the CRLFCRLF; some of it may already be buffered.
    let body_start = head_end + 4;
    let mut body = buf[body_start..].to_vec();
    while body.len() < head.content_length {
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(head.content_length);
    Ok((head, body))
}

/// First index of `needle` in `haystack`, or `None`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

/// Build a minimal HTTP/1.1 response with a JSON body. `Connection: close` so the
/// client (and this handler) tear the connection down after one exchange.
fn http_response(status: u16, reason: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

/// Serve one completion request on an authenticated connection: read + parse,
/// verify the session token against the attested pid, forward the raw body
/// through ai-proxy, and write the upstream status + body back. Any auth or
/// transport failure is a fail-closed HTTP error with no upstream call.
async fn handle_connection(
    stream: &mut UnixStream,
    verifier: &Arc<dyn SessionVerifier>,
    proxy: &ProxyAIClient,
    audit_token: &str,
    pid: u32,
) {
    let (head, body) = match tokio::time::timeout(READ_TIMEOUT, read_request(stream)).await {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            debug!(error = %e, "completion request read failed");
            let _ = stream
                .write_all(&http_response(400, "Bad Request", b"{\"error\":\"malformed request\"}"))
                .await;
            return;
        }
        Err(_) => {
            debug!("completion request read timed out");
            let _ = stream
                .write_all(&http_response(408, "Request Timeout", b"{\"error\":\"request timeout\"}"))
                .await;
            return;
        }
    };

    // Authenticate: the bearer token pi presents (its provider API key) must be a
    // live session token bound to THIS attested pid (SO_PEERCRED), mirroring the
    // contract socket. No token, or a token not bound to this pid, is refused.
    let authed = head
        .bearer
        .as_deref()
        .map(|t| verifier.verify_session(&SessionToken::from_wire(t.to_string()), pid))
        .unwrap_or(false);
    if !authed {
        warn!(pid, "rejecting an unauthenticated pi completion request");
        let _ = stream
            .write_all(&http_response(401, "Unauthorized", b"{\"error\":\"unauthorized\"}"))
            .await;
        return;
    }

    // Forward the raw OpenAI body through the governed proxy (allowlist + audit +
    // SSRF-pinned dial live in ai-proxy). The daemon is the trusted egress caller.
    let body_str = String::from_utf8_lossy(&body);
    match proxy.forward(PROVIDER_NAME, &body_str, audit_token).await {
        Ok(resp) => {
            let _ = stream
                .write_all(&http_response(resp.upstream_status, "OK", resp.body.as_bytes()))
                .await;
        }
        Err(e) => {
            warn!(pid, error = %e, "completion forward failed");
            let _ = stream
                .write_all(&http_response(502, "Bad Gateway", b"{\"error\":\"upstream unavailable\"}"))
                .await;
        }
    }
}

/// Accept loop for the pi completion socket. Authenticates every peer from the
/// kernel (SO_PEERCRED, cross-uid rejected) before serving; the attested pid is
/// what the per-request session-token check binds to.
pub async fn serve_completion(
    listener: UnixListener,
    verifier: Arc<dyn SessionVerifier>,
    proxy: Arc<ProxyAIClient>,
    audit_token: Arc<str>,
    uid: u32,
) {
    // Bound the number of completion handlers running at once, so a same-uid client
    // that opens many slow connections cannot exhaust tasks and fds.
    let limiter = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        match listener.accept().await {
            Ok((mut stream, _)) => {
                // Read the kernel-attested (pid, uid) WITHOUT resolving the peer's
                // binary: pi is a generic `node` interpreter, which the binary-
                // resolving `ConnectionAuth::extract_from` rejects as UnknownBinary.
                // The authentication here is the session token bound to this pid
                // (checked per request); this only enforces same-uid + the pid.
                let (pid, peer_uid) = match peer_credentials(&stream) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(error = %e, "completion connection: could not read peer credentials");
                        continue;
                    }
                };
                if peer_uid != uid {
                    warn!(peer_uid, "rejecting cross-uid completion connection");
                    continue;
                }
                // Acquire a connection permit before serving; at capacity this waits,
                // bounding concurrent handlers. The permit is held for the handler's
                // lifetime and released when its task completes.
                let permit = match Arc::clone(&limiter).acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => {
                        warn!("completion connection limiter closed");
                        return;
                    }
                };
                let verifier = Arc::clone(&verifier);
                let proxy = Arc::clone(&proxy);
                let audit_token = Arc::clone(&audit_token);
                tokio::spawn(async move {
                    let _permit = permit;
                    handle_connection(&mut stream, &verifier, &proxy, &audit_token, pid).await;
                });
            }
            Err(e) => warn!(error = %e, "completion accept failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_head_extracts_bearer_and_length() {
        let head = "POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer tok-123\r\nContent-Length: 42\r\nContent-Type: application/json";
        let h = parse_head(head);
        assert_eq!(h.bearer.as_deref(), Some("tok-123"));
        assert_eq!(h.content_length, 42);
    }

    #[test]
    fn parse_head_is_case_insensitive_on_names() {
        let head = "POST / HTTP/1.1\r\nauthorization: bearer abc\r\ncontent-length: 7";
        let h = parse_head(head);
        assert_eq!(h.bearer.as_deref(), Some("abc"));
        assert_eq!(h.content_length, 7);
    }

    #[test]
    fn parse_head_missing_auth_yields_none() {
        let head = "POST / HTTP/1.1\r\nContent-Length: 0";
        let h = parse_head(head);
        assert!(h.bearer.is_none());
        assert_eq!(h.content_length, 0);
    }

    #[test]
    fn find_subslice_finds_head_terminator() {
        assert_eq!(find_subslice(b"ab\r\n\r\ncd", b"\r\n\r\n"), Some(2));
        assert_eq!(find_subslice(b"abcd", b"\r\n\r\n"), None);
    }

    #[test]
    fn http_response_frames_status_and_body() {
        let r = http_response(200, "OK", b"{}");
        let s = String::from_utf8(r).unwrap();
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(s.contains("Content-Length: 2\r\n"));
        assert!(s.ends_with("\r\n\r\n{}"));
    }
}
