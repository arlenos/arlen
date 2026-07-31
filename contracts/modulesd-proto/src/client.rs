//! A single-shot client for callers that want one answer.
//!
//! The shell keeps a persistent connection because it consumes the event
//! stream; a caller that just wants to flip a switch does not, and should not
//! have to stand up an event pump to do it. Both speak the same framing, which
//! is why it lives here rather than being written twice.
//!
//! This exists so a caller reaches the module runtime instead of editing the
//! runtime's state behind its back. Settings used to write `modules.toml`
//! directly, which meant the toggle never passed the consent gate and the
//! runtime never learned the module had been switched off.

use std::path::PathBuf;

use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::{Event, Request, Response};

/// The largest frame accepted, matching the daemon's own bound.
const MAX_FRAME: usize = 8 * 1024 * 1024;

/// A failure talking to the module runtime.
#[derive(Debug, Error)]
pub enum ClientError {
    /// The daemon is not running or the socket is not reachable.
    #[error("modulesd is not reachable: {0}")]
    Unreachable(std::io::Error),
    /// The connection failed mid-exchange.
    #[error("modulesd connection failed: {0}")]
    Io(std::io::Error),
    /// The reply was not a frame this client understands.
    #[error("modulesd sent a malformed reply: {0}")]
    Malformed(String),
}

/// `$XDG_RUNTIME_DIR/arlen/modulesd.sock`, matching the daemon's bind.
///
/// Both the daemon and its clients call this, so they agree wherever it points.
/// It used to compute `/run/user/{uid}` directly, which is the same path in an
/// ordinary session and unreachable in any other: a nested session, a container,
/// or a test that wants its own runtime dir all set `XDG_RUNTIME_DIR`, and this
/// ignored them. It also meant a smoke run could not isolate modulesd, so it
/// bound over the real session's socket instead of a throwaway one.
///
/// Falls back to `/run/user/{uid}` when the variable is unset, which is what a
/// bare login shell without a session manager looks like.
pub fn socket_path() -> PathBuf {
    // The explicit override wins, so a test or a second instance can be pointed
    // somewhere without touching the session's runtime dir. It lived only on the
    // daemon side before, which meant setting it moved the bind and left every
    // client looking at the old path.
    if let Some(explicit) = std::env::var_os("ARLEN_MODULESD_SOCKET") {
        if !explicit.is_empty() {
            return PathBuf::from(explicit);
        }
    }
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| {
            // SAFETY: getuid is always safe; it reads the real uid and cannot fail.
            let uid = unsafe { libc::getuid() };
            PathBuf::from(format!("/run/user/{uid}"))
        });
    runtime_dir.join("arlen/modulesd.sock")
}

/// Send one request and return the daemon's reply to it.
///
/// The daemon starts pushing lifecycle [`Event`] frames as soon as a connection
/// opens, so the first frame back is frequently not the reply. Frames that are
/// not a [`Response`], or whose id is not the one asked for, are skipped rather
/// than treated as an answer or as a protocol error.
pub async fn request_once(socket: &std::path::Path, request: Request) -> Result<Response, ClientError> {
    let mut stream = UnixStream::connect(socket)
        .await
        .map_err(ClientError::Unreachable)?;
    exchange(&mut stream, request).await
}

/// The round trip over an already-connected stream, so the frame handling is
/// testable against a socket pair rather than needing a running daemon.
async fn exchange<S>(stream: &mut S, request: Request) -> Result<Response, ClientError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let want = request_id(&request).to_string();

    let payload = serde_json::to_vec(&request).map_err(|e| ClientError::Malformed(e.to_string()))?;
    let len = u32::try_from(payload.len())
        .map_err(|_| ClientError::Malformed("request too large".to_string()))?;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(ClientError::Io)?;
    stream.write_all(&payload).await.map_err(ClientError::Io)?;

    loop {
        let mut header = [0u8; 4];
        stream
            .read_exact(&mut header)
            .await
            .map_err(ClientError::Io)?;
        // Bounded before allocating, so a corrupt or hostile length cannot make
        // this reserve it.
        let len = u32::from_be_bytes(header) as usize;
        if len > MAX_FRAME {
            return Err(ClientError::Malformed(format!(
                "reply frame {len} exceeds {MAX_FRAME}"
            )));
        }
        let mut buf = vec![0u8; len];
        stream
            .read_exact(&mut buf)
            .await
            .map_err(ClientError::Io)?;
        // An `Event` shares this connection and does not parse as a `Response`,
        // so a parse failure means "not for us" far more often than a broken
        // peer. Skipped, but only after confirming it IS a well-formed event -
        // otherwise a genuinely corrupt stream would loop silently.
        let Ok(response) = serde_json::from_slice::<Response>(&buf) else {
            if serde_json::from_slice::<Event>(&buf).is_ok() {
                continue;
            }
            return Err(ClientError::Malformed(
                "frame is neither a response nor an event".to_string(),
            ));
        };
        if response_id(&response) == want {
            return Ok(response);
        }
    }
}

/// The correlation id carried by a request.
fn request_id(request: &Request) -> &str {
    match request {
        Request::Hello { id, .. }
        | Request::ListModules { id }
        | Request::WaypointerSearch { id, .. }
        | Request::WaypointerSearchAll { id, .. }
        | Request::WaypointerExecute { id, .. }
        | Request::IframeMint { id, .. }
        | Request::IframeLookup { id, .. }
        | Request::HostCall { id, .. }
        | Request::Subscribe { id, .. }
        | Request::SetEnabled { id, .. }
        | Request::Retry { id, .. } => id,
    }
}

/// The correlation id carried by a response. Every variant has one, and the
/// match is exhaustive so a new variant has to say what its id is rather than
/// falling into a catch-all that would silently never match.
fn response_id(response: &Response) -> &str {
    match response {
        Response::Hello { id, .. }
        | Response::ModuleList { id, .. }
        | Response::WaypointerResults { id, .. }
        | Response::Executed { id, .. }
        | Response::IframeIssued { id, .. }
        | Response::WaypointerAggregate { id, .. }
        | Response::HostReply { id, .. }
        | Response::Subscribed { id, .. }
        | Response::IframeMeta { id, .. }
        | Response::Acked { id }
        | Response::Error { id, .. } => id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn write_frame<S: AsyncWriteExt + Unpin>(s: &mut S, bytes: &[u8]) {
        s.write_all(&(bytes.len() as u32).to_be_bytes()).await.unwrap();
        s.write_all(bytes).await.unwrap();
    }

    async fn serve_frames(mut daemon: tokio::net::UnixStream, frames: Vec<Vec<u8>>) {
        let mut header = [0u8; 4];
        daemon.read_exact(&mut header).await.unwrap();
        let mut body = vec![0u8; u32::from_be_bytes(header) as usize];
        daemon.read_exact(&mut body).await.unwrap();
        for f in frames {
            write_frame(&mut daemon, &f).await;
        }
    }

    /// modulesd starts pushing lifecycle events the moment a connection opens,
    /// so the first frame back is frequently not the reply. Reading it as one
    /// would fail every toggle that raced a module starting up.
    #[tokio::test]
    async fn an_event_arriving_first_does_not_become_the_answer() {
        let (mut client, daemon) = tokio::net::UnixStream::pair().unwrap();
        let event = serde_json::to_vec(&Event::ModuleLoaded {
            module_id: "com.example.other".into(),
        })
        .unwrap();
        let reply = serde_json::to_vec(&Response::Acked { id: "r1".into() }).unwrap();
        tokio::spawn(serve_frames(daemon, vec![event, reply]));

        let got = exchange(
            &mut client,
            Request::SetEnabled {
                id: "r1".into(),
                module_id: "com.example.a".into(),
                enabled: false,
            },
        )
        .await
        .expect("the reply is found past the event");
        assert!(matches!(got, Response::Acked { .. }));
    }

    /// A frame that is neither must not loop silently.
    #[tokio::test]
    async fn a_frame_that_is_neither_is_an_error() {
        let (mut client, daemon) = tokio::net::UnixStream::pair().unwrap();
        tokio::spawn(serve_frames(daemon, vec![b"{\"not\":\"ours\"}".to_vec()]));
        let err = exchange(&mut client, Request::ListModules { id: "r1".into() })
            .await
            .expect_err("a foreign frame is refused");
        assert!(matches!(err, ClientError::Malformed(_)), "{err:?}");
    }

    #[test]
    fn the_socket_path_matches_the_daemons_bind() {
        let p = socket_path();
        assert!(p.to_string_lossy().ends_with("/arlen/modulesd.sock"), "{p:?}");
    }

    /// Replies are matched by id, so a module crashing mid-call cannot be read
    /// as the answer to whatever was just asked.
    #[test]
    fn a_request_and_its_reply_carry_the_same_id() {
        let req = Request::SetEnabled {
            id: "r1".into(),
            module_id: "com.example.a".into(),
            enabled: true,
        };
        assert_eq!(request_id(&req), "r1");
        assert_eq!(response_id(&Response::Acked { id: "r1".into() }), "r1");
    }
}
