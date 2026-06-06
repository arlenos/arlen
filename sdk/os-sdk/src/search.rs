/// `shell.search.open` — first-party app surface for opening
/// the Waypointer launcher with a prefilled query and optional
/// routing hint.
///
/// Apps connect to the shell-side IPC socket at
/// `$XDG_RUNTIME_DIR/arlen/search.sock`, send a single
/// `OpenRequest`, receive `OpenResponse` (or `SearchError`), and
/// the connection drops. No persistent state, no subscribe
/// channel — the search broker is intentionally single-shot.
///
/// Permission: profile must declare `[search] open = true`. Default
/// is deny (foundation §7.3 explicit-grant).
///
/// Long-lived "register as a search-result provider" is a separate
/// surface that ships through `arlen-modulesd` as a Tier-1 WASM
/// module (Phase 7). See `docs/architecture/module-system.md` for
/// that path. This SDK module covers only the open-and-prefill
/// case.
///
/// Foundation reference: §6.4 Listing 9 (`shell.search.open`).
use std::path::PathBuf;

use prost::Message;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::proto_search::{
    self, search_envelope::Message as Envelope,
};

const MAX_FRAME_BYTES: usize = 1024 * 1024;
const SOCKET_NAME: &str = "search.sock";

/// Routing hint passed alongside the query. The broker forwards
/// matched modes as a `"<mode>: "` query prefix so plugins that
/// opt in can filter; unknown values are silently ignored
/// (forward-compat).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// AI-mode hint (Phase 9 — currently no plugin reads it).
    Ai,
    /// Files-mode hint (consumed by app-files in Phase 8).
    Files,
    /// Apps-mode hint (limits results to application launches).
    Apps,
}

impl SearchMode {
    fn as_wire_str(self) -> &'static str {
        match self {
            SearchMode::Ai => "ai",
            SearchMode::Files => "files",
            SearchMode::Apps => "apps",
        }
    }
}

/// Errors from the `UnixSearchClient`.
#[derive(Debug, Error)]
pub enum SearchError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Caller's permission profile does not grant the required scope.
    #[error("permission denied: missing scope {scope}")]
    PermissionDenied { scope: String },
    /// Query exceeds the broker's per-message byte cap.
    #[error("query too large: {actual} > {max}")]
    QueryTooLarge { actual: usize, max: usize },
    /// Waypointer launcher window has not been created yet (shell
    /// is still starting). Caller may retry shortly.
    #[error("waypointer window not ready")]
    WindowNotReady,
    /// Broker returned a structured error not covered by the typed
    /// variants (e.g. a future ErrorKind value).
    #[error("broker error ({kind}): {detail}")]
    Broker { kind: i32, detail: String },
    /// Wire-format violation.
    #[error("protocol: {0}")]
    Protocol(String),
}

/// Single-shot client for the `shell.search.open` IPC.
///
/// Each call to `open` opens its own connection, sends one request,
/// reads one response, and drops the connection. The broker is
/// not stateful from the client's side, so connection pooling
/// would only add cost.
pub struct UnixSearchClient {
    socket_path: PathBuf,
}

impl UnixSearchClient {
    /// Connect to the default socket at
    /// `$XDG_RUNTIME_DIR/arlen/search.sock`.
    pub fn new() -> Result<Self, SearchError> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
            SearchError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "XDG_RUNTIME_DIR not set",
            ))
        })?;
        let mut p = PathBuf::from(runtime);
        p.push("arlen");
        p.push(SOCKET_NAME);
        Ok(Self { socket_path: p })
    }

    /// Connect to a specific socket path. Used by tests + dev
    /// sandboxes where the shell runs under a non-default
    /// `XDG_RUNTIME_DIR`.
    pub fn at_path(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    /// Open the Waypointer launcher with a prefilled `query`. Pass
    /// `mode = None` for no routing hint, or one of the
    /// [`SearchMode`] variants. The broker silent-drops unknown
    /// modes for forward-compat.
    pub async fn open(
        &self,
        query: &str,
        mode: Option<SearchMode>,
    ) -> Result<(), SearchError> {
        let stream = UnixStream::connect(&self.socket_path).await?;
        let (mut reader, mut writer) = stream.into_split();

        let envelope = proto_search::SearchEnvelope {
            message: Some(Envelope::OpenRequest(proto_search::OpenRequest {
                query: query.to_string(),
                mode: mode.map(|m| m.as_wire_str().to_string()).unwrap_or_default(),
            })),
        };
        let body = envelope.encode_to_vec();
        let len = (body.len() as u32).to_be_bytes();
        writer.write_all(&len).await?;
        writer.write_all(&body).await?;

        // Read the response: 4-byte BE length + protobuf body.
        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf).await?;
        let resp_len = u32::from_be_bytes(len_buf) as usize;
        if resp_len == 0 || resp_len > MAX_FRAME_BYTES {
            return Err(SearchError::Protocol(format!(
                "response length {resp_len} out of range"
            )));
        }
        let mut body = vec![0u8; resp_len];
        reader.read_exact(&mut body).await?;
        let resp = proto_search::SearchEnvelope::decode(body.as_slice())
            .map_err(|e| SearchError::Protocol(format!("decode: {e}")))?;

        match resp.message {
            Some(Envelope::OpenResponse(_)) => Ok(()),
            Some(Envelope::Error(err)) => Err(map_error(err)),
            _ => Err(SearchError::Protocol(
                "unexpected envelope variant from broker".into(),
            )),
        }
    }
}

fn map_error(err: proto_search::SearchError) -> SearchError {
    match proto_search::ErrorKind::try_from(err.kind)
        .unwrap_or(proto_search::ErrorKind::ErrorUnknown)
    {
        proto_search::ErrorKind::ErrorPermissionDenied => SearchError::PermissionDenied {
            scope: "search.open".into(),
        },
        proto_search::ErrorKind::ErrorQueryTooLarge => SearchError::QueryTooLarge {
            actual: 0, // detail string carries the precise size
            max: 4096,
        },
        proto_search::ErrorKind::ErrorWindowNotReady => SearchError::WindowNotReady,
        _ => SearchError::Broker {
            kind: err.kind,
            detail: err.detail,
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_mode_wire_strings() {
        assert_eq!(SearchMode::Ai.as_wire_str(), "ai");
        assert_eq!(SearchMode::Files.as_wire_str(), "files");
        assert_eq!(SearchMode::Apps.as_wire_str(), "apps");
    }

    #[test]
    fn unix_client_constructs_with_xdg() {
        // SAFETY: env mutation in single-threaded test scope.
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
        let c = UnixSearchClient::new().expect("client");
        assert_eq!(
            c.socket_path,
            PathBuf::from("/run/user/1000/arlen/search.sock")
        );
    }

    #[test]
    fn unix_client_at_path() {
        let p = PathBuf::from("/tmp/test.sock");
        let c = UnixSearchClient::at_path(p.clone());
        assert_eq!(c.socket_path, p);
    }
}
