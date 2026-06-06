/// `shell.intents.dispatch` — first-party app surface for typed
/// cross-process action dispatch.
///
/// Apps connect to the shell-side IPC socket at
/// `$XDG_RUNTIME_DIR/arlen/intents.sock`, send a single
/// `DispatchRequest`, receive `DispatchResponse` (or
/// `IntentError`), and the connection drops. No persistent state,
/// no subscribe channel — the intent broker is intentionally
/// single-shot, mirroring `shell.search.open`.
///
/// Permission: profile must declare `[intents] dispatch = true`.
/// Default is deny (foundation §7.3 explicit-grant).
///
/// Long-lived "register as intent handler" is a separate surface
/// that ships through `arlen-modulesd` as a Tier-1 WASM module
/// (Phase 7). See `docs/architecture/module-system.md` for that
/// path. This SDK module covers only the Phase-6 dispatch case.
///
/// Foundation reference: §6.4 Listing 11 (`shell.intents`).
use std::path::PathBuf;

use prost::Message;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::proto_intents::{
    self, intent_envelope::Message as Envelope,
};

const MAX_FRAME_BYTES: usize = 1024 * 1024;
const SOCKET_NAME: &str = "intents.sock";

/// Built-in intent types accepted by the broker. The `data`
/// payload's expected shape is type-specific; see foundation §6.4
/// and `intent-system.md` §5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentType {
    /// URL string. Forwarded to xdg-portal-arlen OpenURI.
    Url,
    /// Absolute filesystem path or `file://` URI. Forwarded to
    /// the OS file dispatcher.
    File,
    /// Plain UTF-8 text up to 64 KB. Written to the clipboard.
    Text,
    /// `mailto:` URI. Forwarded as a URL.
    Email,
    /// Project id (resolved against the Knowledge Graph). Activates
    /// Focus Mode.
    Project,
}

impl IntentType {
    fn as_wire_str(self) -> &'static str {
        match self {
            IntentType::Url => "url",
            IntentType::File => "file",
            IntentType::Text => "text",
            IntentType::Email => "email",
            IntentType::Project => "project",
        }
    }
}

/// Errors from the `UnixIntentClient`.
#[derive(Debug, Error)]
pub enum IntentError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Caller's permission profile does not grant `intents.dispatch`.
    #[error("permission denied: missing scope intents.dispatch")]
    PermissionDenied,
    /// Built-in dispatcher missing or failed (e.g. xdg-open absent).
    #[error("no handler for intent")]
    NoHandler,
    /// Type-specific lookup failed (project_id not in graph, file
    /// path missing, etc.).
    #[error("not found: {0}")]
    NotFound(String),
    /// `data` is not valid UTF-8 or fails type-specific validation.
    #[error("invalid data: {0}")]
    InvalidData(String),
    /// `data` exceeds the per-type cap (text: 64 KB).
    #[error("data too large: {0}")]
    DataTooLarge(String),
    /// Broker returned a structured error not covered by the typed
    /// variants.
    #[error("broker error ({kind}): {detail}")]
    Broker { kind: i32, detail: String },
    /// Wire-format violation.
    #[error("protocol: {0}")]
    Protocol(String),
}

/// Successful dispatch result.
#[derive(Debug, Clone)]
pub struct DispatchResult {
    /// Identifier of the handler that ran (`builtin.url` etc).
    pub handler: String,
    /// Type-specific structured outcome (empty for url/file/email
    /// because xdg-open is fire-and-forget; project returns the
    /// resolved project name).
    pub outcome: String,
}

/// Single-shot client for `shell.intents.dispatch`.
///
/// Each call opens its own connection, sends one request, reads
/// one response, and drops the connection. The broker is
/// stateless from the client's perspective.
pub struct UnixIntentClient {
    socket_path: PathBuf,
}

impl UnixIntentClient {
    /// Connect to the default socket at
    /// `$XDG_RUNTIME_DIR/arlen/intents.sock`.
    pub fn new() -> Result<Self, IntentError> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
            IntentError::Io(std::io::Error::new(
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

    /// Dispatch a typed intent.
    ///
    /// `action` is an app-defined verb ("view" / "edit" / …);
    /// Phase-6 built-ins ignore it but Phase-7 register'd handlers
    /// can pattern-match on it. `intent_type` selects the
    /// dispatcher. `data` is the type-specific payload (see
    /// [`IntentType`] doc strings). `fallback` is reserved for
    /// Phase 7 and silently ignored today.
    pub async fn dispatch(
        &self,
        action: &str,
        intent_type: IntentType,
        data: &[u8],
        fallback: Option<&str>,
    ) -> Result<DispatchResult, IntentError> {
        let stream = UnixStream::connect(&self.socket_path).await?;
        let (mut reader, mut writer) = stream.into_split();

        let envelope = proto_intents::IntentEnvelope {
            message: Some(Envelope::DispatchRequest(proto_intents::DispatchRequest {
                action: action.to_string(),
                r#type: intent_type.as_wire_str().to_string(),
                data: data.to_vec(),
                fallback: fallback.unwrap_or("").to_string(),
            })),
        };
        let body = envelope.encode_to_vec();
        let len = (body.len() as u32).to_be_bytes();
        writer.write_all(&len).await?;
        writer.write_all(&body).await?;

        let mut len_buf = [0u8; 4];
        reader.read_exact(&mut len_buf).await?;
        let resp_len = u32::from_be_bytes(len_buf) as usize;
        if resp_len == 0 || resp_len > MAX_FRAME_BYTES {
            return Err(IntentError::Protocol(format!(
                "response length {resp_len} out of range"
            )));
        }
        let mut body = vec![0u8; resp_len];
        reader.read_exact(&mut body).await?;
        let resp = proto_intents::IntentEnvelope::decode(body.as_slice())
            .map_err(|e| IntentError::Protocol(format!("decode: {e}")))?;

        match resp.message {
            Some(Envelope::DispatchResponse(r)) => Ok(DispatchResult {
                handler: r.handler,
                outcome: r.outcome,
            }),
            Some(Envelope::Error(err)) => Err(map_error(err)),
            _ => Err(IntentError::Protocol(
                "unexpected envelope variant from broker".into(),
            )),
        }
    }
}

fn map_error(err: proto_intents::IntentError) -> IntentError {
    match proto_intents::ErrorKind::try_from(err.kind)
        .unwrap_or(proto_intents::ErrorKind::ErrorUnknown)
    {
        proto_intents::ErrorKind::ErrorPermissionDenied => IntentError::PermissionDenied,
        proto_intents::ErrorKind::ErrorNoHandler => IntentError::NoHandler,
        proto_intents::ErrorKind::ErrorNotFound => IntentError::NotFound(err.detail),
        proto_intents::ErrorKind::ErrorInvalidData
        | proto_intents::ErrorKind::ErrorUnknownType => IntentError::InvalidData(err.detail),
        proto_intents::ErrorKind::ErrorDataTooLarge => IntentError::DataTooLarge(err.detail),
        _ => IntentError::Broker {
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
    fn intent_type_wire_strings() {
        assert_eq!(IntentType::Url.as_wire_str(), "url");
        assert_eq!(IntentType::File.as_wire_str(), "file");
        assert_eq!(IntentType::Text.as_wire_str(), "text");
        assert_eq!(IntentType::Email.as_wire_str(), "email");
        assert_eq!(IntentType::Project.as_wire_str(), "project");
    }

    #[test]
    fn unix_client_constructs_with_xdg() {
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
        let c = UnixIntentClient::new().expect("client");
        assert_eq!(
            c.socket_path,
            PathBuf::from("/run/user/1000/arlen/intents.sock")
        );
    }

    #[test]
    fn unix_client_at_path() {
        let p = PathBuf::from("/tmp/test.sock");
        let c = UnixIntentClient::at_path(p.clone());
        assert_eq!(c.socket_path, p);
    }
}
