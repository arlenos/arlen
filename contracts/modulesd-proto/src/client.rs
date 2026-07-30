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

use crate::{Request, Response};

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

/// `/run/user/{uid}/arlen/modulesd.sock`, matching the daemon's bind.
pub fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{uid}/arlen/modulesd.sock"))
}

/// Send one request and return the daemon's reply to it.
///
/// The daemon pushes unsolicited events down the same connection, so replies
/// are matched by the request's id rather than by taking the first frame that
/// arrives - otherwise a module crashing at the wrong moment would be read as
/// the answer to whatever was just asked.
pub async fn request_once(socket: &std::path::Path, request: Request) -> Result<Response, ClientError> {
    let want = request_id(&request).to_string();
    let mut stream = UnixStream::connect(socket)
        .await
        .map_err(ClientError::Unreachable)?;

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
        let response: Response =
            serde_json::from_slice(&buf).map_err(|e| ClientError::Malformed(e.to_string()))?;
        if response_id(&response) == Some(want.as_str()) {
            return Ok(response);
        }
        // An event, or a reply to someone else's request on a shared
        // connection. Keep reading rather than reporting it as the answer.
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

/// The correlation id carried by a response, or `None` for an unsolicited event.
fn response_id(response: &Response) -> Option<&str> {
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
        | Response::Error { id, .. } => Some(id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(
            response_id(&Response::Acked { id: "r1".into() }),
            Some("r1")
        );
    }
}
