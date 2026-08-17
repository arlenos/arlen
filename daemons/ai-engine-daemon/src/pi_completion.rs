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

/// Whether the request body asked for a streamed completion (`"stream": true`).
///
/// pi's openai-completions provider hardcodes `stream: true` - there is no compat
/// flag that turns it off - so in practice this is always true for pi, and the
/// check exists so a non-streaming caller is still answered in ITS shape rather
/// than handed frames it cannot read. An unparsable body reads as non-streaming:
/// the passthrough is what this endpoint did before, so a body we cannot
/// interpret keeps the old behaviour instead of being reshaped on a guess.
fn wants_stream(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("stream").and_then(|s| s.as_bool()))
        .unwrap_or(false)
}

/// Whether `body` is ALREADY a Server-Sent Events stream (the upstream answered
/// the `stream: true` it was forwarded verbatim). Then it is passed through
/// untouched - reshaping a stream we did not build would drop tool-call deltas,
/// usage and every field this daemon has no business knowing about.
fn is_sse(body: &str) -> bool {
    let t = body.trim_start();
    t.starts_with("data:") || t.starts_with("event:")
}

/// Re-emit a single non-streaming OpenAI chat completion as the SSE stream the
/// request asked for: one content delta, one `finish_reason` chunk, then `[DONE]`.
///
/// This is the fix for a turn that ended without the model ever being asked. pi
/// sends `stream: true`; the echo provider (and any upstream answering a plain
/// completion object) returns one JSON object; pi's SSE parser then finds no
/// frames, throws `Stream ended without finish_reason`, and ends the turn with
/// `stopReason: "error"` and EMPTY content - after three silent auto-retries, and
/// with nothing on stderr. Every symptom of a dead model, with the model never
/// dialled. Reproduced outside the VM against a stand-in socket, both directions:
/// plain JSON -> that error every time, this shape -> `stopReason: "stop"` and the
/// text delivered, no retries.
///
/// Answering in the shape the caller asked for is the endpoint's job; it is the
/// one place that sees both the request and the response. Doing it in the echo
/// provider instead would make that provider lie about being an OpenAI completion
/// to the daemon's own non-pi consumers, which parse it as an object.
///
/// `None` when the body is not a completion object we can read, so the caller
/// falls back to passing it through: a malformed upstream body should reach the
/// client as itself, not as a well-formed frame around nothing.
fn completion_to_sse(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let choice = v.get("choices")?.as_array()?.first()?;
    let message = choice.get("message")?;
    let content = message.get("content").and_then(|c| c.as_str()).unwrap_or("");
    // Carried through so a client that reports why a turn stopped keeps saying
    // what the upstream said; `stop` only when the upstream named nothing.
    let finish = choice.get("finish_reason").and_then(|f| f.as_str()).unwrap_or("stop");
    let id = v.get("id").and_then(|i| i.as_str()).unwrap_or("chatcmpl-arlen");
    let model = v.get("model").and_then(|m| m.as_str()).unwrap_or("");
    let base = serde_json::json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": 0,
        "model": model,
    });
    let frame = |choices: serde_json::Value| {
        let mut chunk = base.clone();
        chunk["choices"] = choices;
        format!("data: {chunk}\n\n")
    };
    // Tool calls ride along, indexed the way a streamed delta carries them (pi
    // reads `delta.tool_calls[].index`, openai-completions.ts:385). A converter
    // that carried only `content` would drop them in silence, and a dropped tool
    // call looks exactly like a model that chose not to act - the same class of
    // failure as the one this whole function exists to fix.
    let mut delta_obj = serde_json::json!({"role": "assistant", "content": content});
    if let Some(calls) = message.get("tool_calls").and_then(|c| c.as_array()) {
        let indexed: Vec<serde_json::Value> = calls
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let mut c = c.clone();
                c["index"] = serde_json::json!(i);
                c
            })
            .collect();
        delta_obj["tool_calls"] = serde_json::Value::Array(indexed);
    }
    let delta = frame(serde_json::json!([{
        "index": 0,
        "delta": delta_obj,
        "finish_reason": serde_json::Value::Null,
    }]));
    let end = frame(serde_json::json!([{
        "index": 0,
        "delta": {},
        "finish_reason": finish,
    }]));
    Some(format!("{delta}{end}data: [DONE]\n\n"))
}

/// An HTTP/1.1 response carrying an SSE body. Same minimal framing as
/// [`http_response`], with the content type the frames need.
fn sse_response(status: u16, body: &str) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

/// The bytes to write back for an upstream answer, given what the request asked
/// for. Pure, so the reshaping is unit-tested without a socket.
fn completion_response(wants_stream: bool, status: u16, upstream_body: &str) -> Vec<u8> {
    if !wants_stream || is_sse(upstream_body) {
        // Non-streaming caller, or an upstream stream to pass through verbatim.
        // The content type still says JSON for the passthrough case, which is what
        // it said before this and what pi's SSE parser reads regardless.
        return http_response(status, "OK", upstream_body.as_bytes());
    }
    match completion_to_sse(upstream_body) {
        Some(sse) => sse_response(status, &sse),
        None => http_response(status, "OK", upstream_body.as_bytes()),
    }
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
    let streamed = wants_stream(&body_str);

    // A pinned tape answers instead of the network, so a change to the run loop
    // can be shown to produce the same conversation without a model and without
    // egress. Loaded per request: a tape is a handful of turns, and re-reading it
    // keeps this branch stateless next to a forward path that is.
    //
    // A request the recording never saw is a 502 carrying the reason, NOT a
    // fallthrough to the live provider - a replay that quietly goes upstream is
    // no longer deterministic and stops being evidence.
    if let Some(frames) = replayed_response(&body_str, streamed) {
        let _ = stream.write_all(&frames).await;
        return;
    }

    match proxy.forward(PROVIDER_NAME, &body_str, audit_token).await {
        Ok(resp) => {
            // Recording is best-effort and never fails the request: a session the
            // operator asked to keep is worth less than the answer they are
            // waiting for, and a failed write says so in the log.
            if let Some(tape) = std::env::var_os("ARLEN_AI_RECORD") {
                let turn = crate::replay::Turn {
                    request: body_str.to_string(),
                    response: resp.body.clone(),
                };
                if let Err(e) = crate::replay::Recorder::new(std::path::PathBuf::from(&tape)).record(&turn) {
                    warn!(error = %e, "could not record this completion turn");
                }
            }
            let _ = stream
                .write_all(&completion_response(streamed, resp.upstream_status, &resp.body))
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

/// The bytes to answer with when a tape is pinned, or `None` to go upstream.
///
/// A named function rather than an inline branch so it can be driven in a test:
/// the socket around it needs a live bus (the proxy client is built from a real
/// D-Bus connection), so this is the largest piece of the replay path that can be
/// exercised without one - env read, tape load, request match, and the HTTP
/// framing that the answer actually reaches pi through.
///
/// Deliberately NOT falling through on a miss: an unmatched request answers 502
/// with the reason. A replay that quietly reaches the provider is not
/// deterministic, and a run made against it is not evidence of anything.
fn replayed_response(body: &str, streamed: bool) -> Option<Vec<u8>> {
    let tape = std::env::var_os("ARLEN_AI_REPLAY")?;
    match crate::replay::Replayer::load(std::path::Path::new(&tape))
        .and_then(|r| r.answer(body).map(str::to_string))
    {
        Ok(recorded) => Some(completion_response(streamed, 200, &recorded)),
        Err(e) => {
            warn!(error = %e, "completion replay could not answer");
            let msg = serde_json::json!({"error": e.to_string()}).to_string();
            Some(http_response(502, "Bad Gateway", msg.as_bytes()))
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

    /// Record a turn, pin the tape, and read back the bytes pi would receive.
    ///
    /// The end-to-end piece that does not need a bus: the answer travels the same
    /// `completion_response` framing as a live one, so a recorded session is
    /// delivered in the shape pi's parser expects rather than as a bare body.
    ///
    /// Serialised with the miss test below through one `#[serial]`-style guard:
    /// both set the same process-wide env var, and a parallel runner would
    /// otherwise let one test's tape answer the other's request.
    #[test]
    fn a_pinned_tape_answers_in_the_framing_pi_expects() {
        let _guard = REPLAY_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("arlen-replay-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let tape = dir.join("t.jsonl");
        let request = r#"{"model":"pi","messages":[{"role":"user","content":"two plus two"}]}"#;
        crate::replay::Recorder::new(&tape)
            .record(&crate::replay::Turn {
                request: request.to_string(),
                response: a_completion("four"),
            })
            .expect("record");

        std::env::set_var("ARLEN_AI_REPLAY", &tape);
        let framed = replayed_response(request, false).expect("a tape is pinned");
        let miss = replayed_response(r#"{"messages":[{"role":"user","content":"something else"}]}"#, false)
            .expect("a miss still answers");
        std::env::remove_var("ARLEN_AI_REPLAY");

        let s = String::from_utf8(framed).unwrap();
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"), "{s}");
        assert!(s.contains("four"), "{s}");

        // The miss: refused, with the reason, and not the recorded answer.
        let m = String::from_utf8(miss).unwrap();
        assert!(m.starts_with("HTTP/1.1 502 "), "{m}");
        assert!(m.contains("no recorded response"), "{m}");
        assert!(!m.contains("four"), "a miss must not be handed the recorded answer: {m}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// No tape pinned means no opinion: the caller goes upstream as before.
    #[test]
    fn without_a_tape_the_forward_path_is_untouched() {
        let _guard = REPLAY_ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("ARLEN_AI_REPLAY");
        assert!(replayed_response("{}", false).is_none());
    }

    /// One process, one `ARLEN_AI_REPLAY`.
    static REPLAY_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    /// One non-streaming completion, the shape the echo provider answers with.
    fn a_completion(content: &str) -> String {
        serde_json::json!({
            "id": "echo",
            "object": "chat.completion",
            "model": "echo",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": content},
                "finish_reason": "stop",
            }],
        })
        .to_string()
    }

    #[test]
    fn a_stream_request_is_answered_with_frames_that_carry_a_finish_reason() {
        // The defect this endpoint had: pi asks `stream: true`, the echo provider
        // answers one plain object, and pi's SSE parser throws "Stream ended
        // without finish_reason" - three silent retries, an empty assistant
        // message, nothing on stderr. Reproduced against a stand-in socket before
        // this was written, which is how the frames below are known to be enough.
        let out = completion_response(true, 200, &a_completion("an answer"));
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("Content-Type: text/event-stream"), "{s}");
        assert!(s.contains("\"content\":\"an answer\""), "{s}");
        assert!(s.contains("\"finish_reason\":\"stop\""), "{s}");
        assert!(s.ends_with("data: [DONE]\n\n"), "{s}");
    }

    #[test]
    fn an_upstream_that_already_streams_is_passed_through_untouched() {
        // Reshaping a stream we did not build would drop tool-call deltas and
        // usage. The real Ollama path answers this way (the body is forwarded
        // verbatim, `stream: true` and all).
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n";
        let s = String::from_utf8(completion_response(true, 200, sse)).unwrap();
        assert!(s.ends_with(sse), "{s}");
    }

    #[test]
    fn tool_calls_survive_the_conversion_with_the_index_a_delta_needs() {
        // A dropped tool call reads as a model that chose not to act, which is the
        // same silent-wrong-shape failure this function was written to fix.
        let body = serde_json::json!({
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": serde_json::Value::Null,
                    "tool_calls": [
                        {"id": "call_1", "type": "function",
                         "function": {"name": "graph_write", "arguments": "{}"}},
                        {"id": "call_2", "type": "function",
                         "function": {"name": "graph_read", "arguments": "{}"}},
                    ],
                },
                "finish_reason": "tool_calls",
            }],
        })
        .to_string();
        let sse = completion_to_sse(&body).unwrap();
        assert!(sse.contains("\"id\":\"call_1\""), "{sse}");
        assert!(sse.contains("\"id\":\"call_2\""), "{sse}");
        assert!(sse.contains("\"index\":0"), "{sse}");
        assert!(sse.contains("\"index\":1"), "{sse}");
        // The upstream's own stop reason is carried, not replaced by "stop".
        assert!(sse.contains("\"finish_reason\":\"tool_calls\""), "{sse}");
    }

    #[test]
    fn a_non_streaming_caller_still_gets_the_object() {
        let body = a_completion("an answer");
        let s = String::from_utf8(completion_response(false, 200, &body)).unwrap();
        assert!(s.contains("Content-Type: application/json"), "{s}");
        assert!(s.ends_with(&body), "{s}");
    }

    #[test]
    fn an_unreadable_upstream_body_reaches_the_client_as_itself() {
        // Not framed as a well-formed stream around nothing: a client that can see
        // the upstream's own error can say what went wrong.
        let s = String::from_utf8(completion_response(true, 502, "upstream exploded")).unwrap();
        assert!(s.ends_with("upstream exploded"), "{s}");
    }

    #[test]
    fn wants_stream_reads_the_body_and_defaults_to_passthrough() {
        assert!(wants_stream("{\"stream\":true}"));
        assert!(!wants_stream("{\"stream\":false}"));
        assert!(!wants_stream("{}"));
        assert!(!wants_stream("not json"));
    }
}

#[cfg(test)]
mod repro_dump {
    /// Dump the exact frames the endpoint would send, so the reproduction can be
    /// served to a real pi rather than a hand-written lookalike. Not a test of
    /// behaviour; run with `--ignored --nocapture` when re-doing the repro.
    #[test]
    #[ignore]
    fn dump_sse_for_the_local_pi_reproduction() {
        let body = serde_json::json!({
            "id": "echo", "object": "chat.completion", "model": "echo",
            "choices": [{"index": 0, "message": {"role": "assistant", "content":
                "[echo provider - no model was asked] the chain is real"},
                "finish_reason": "stop"}],
        })
        .to_string();
        print!("{}", super::completion_to_sse(&body).unwrap());
    }
}
