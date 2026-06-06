//! Audit ingest client.
//!
//! [`AuditClient`] submits one audit event to `arlen-auditd` and
//! waits for the acknowledgement. It opens a fresh connection per
//! submit: audit events are low-frequency, and a one-shot connection
//! needs no reconnect or stale-socket handling.
//!
//! Every failure mode — the daemon unreachable, a transport error, a
//! timeout, or the daemon itself reporting `Unavailable` — is an
//! `Err`. A caller that audits before acting must treat any `Err` as
//! "do not proceed": foundation §8.4.6 forbids un-audited AI
//! activity.
//!
//! Known limitation (same-uid, documented): the client trusts whatever
//! peer is bound at the socket path to be the real `arlen-auditd`;
//! it does not authenticate the server. Cross-uid impersonation is
//! already excluded — the socket lives under `$XDG_RUNTIME_DIR`
//! (mode 0700, per-user) and the daemon's bind is a singleton guard,
//! so another user cannot reach it and a peer cannot squat a path a
//! live daemon already holds. The residual is a *same-uid* attacker
//! who binds a fake socket while the daemon is down — the client-side
//! dual of the ingest admission gap (see
//! `docs/architecture/identity-spoof-mitigation.md`). The robust
//! closer is the same installer-attested identity registry / canonical
//! socket location as the server side; pulling identity resolution
//! into this deliberately thin client (so an audit client does not
//! drag in the permission crate) is not the right layer. This is the
//! same disclosed same-uid trust boundary as the on-disk HMAC key.

use std::path::PathBuf;
use std::time::Duration;

use tokio::net::UnixStream;

use crate::{
    decode_response, encode_request, read_frame, write_frame, IngestRequest,
    IngestResponse,
};

/// How long a single submit (connect + exchange) may take before it
/// is abandoned. A hung audit daemon must not wedge the caller.
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Why an audit submission did not succeed. Every variant means the
/// caller must fail closed.
#[derive(Debug, thiserror::Error)]
pub enum AuditClientError {
    /// The audit daemon could not be reached, or the exchange failed
    /// at the transport level.
    #[error("audit transport: {0}")]
    Transport(String),
    /// The audit daemon recorded nothing and reported why (a full
    /// disk, a storage fault, ...).
    #[error("audit unavailable: {0}")]
    Unavailable(String),
    /// The audit daemon did not answer within [`SUBMIT_TIMEOUT`].
    #[error("audit daemon timed out")]
    Timeout,
}

/// A client of the audit daemon's ingest socket.
#[derive(Debug, Clone)]
pub struct AuditClient {
    socket_path: PathBuf,
}

impl AuditClient {
    /// Build a client targeting `socket_path`. Use
    /// [`crate::ingest_socket_path`] for the canonical location.
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    /// Submit one audit event. Returns the assigned chain index on a
    /// committed append; any `Err` means the caller must not proceed.
    pub async fn submit(
        &self,
        request: &IngestRequest,
    ) -> std::result::Result<u64, AuditClientError> {
        match tokio::time::timeout(SUBMIT_TIMEOUT, self.exchange(request)).await {
            Ok(result) => result,
            Err(_elapsed) => Err(AuditClientError::Timeout),
        }
    }

    /// One connect-send-receive round trip, without the timeout.
    async fn exchange(
        &self,
        request: &IngestRequest,
    ) -> std::result::Result<u64, AuditClientError> {
        let transport = |e: String| AuditClientError::Transport(e);

        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| transport(format!("connect: {e}")))?;

        let body = encode_request(request).map_err(|e| transport(e.to_string()))?;
        write_frame(&mut stream, &body)
            .await
            .map_err(|e| transport(e.to_string()))?;

        let reply = read_frame(&mut stream)
            .await
            .map_err(|e| transport(e.to_string()))?;
        let response =
            decode_response(&reply).map_err(|e| transport(e.to_string()))?;

        match response {
            IngestResponse::Appended { index } => Ok(index),
            IngestResponse::Unavailable { reason } => {
                Err(AuditClientError::Unavailable(reason))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn submit_against_a_missing_socket_is_a_transport_error() {
        let client = AuditClient::new("/nonexistent/audit-ingest.sock");
        let req = IngestRequest {
            kind: crate::AuditKind::Query,
            structural: crate::StructuralRecord {
                subject: "graph".into(),
                node_types: vec![],
                relations: vec![],
                result_count: None,
                duration_ms: None,
                outcome: "ok".into(),
                depth: None,
            },
            forensic: None,
            call_chain_id: None,
            project_id: None,
        };
        match client.submit(&req).await {
            Err(AuditClientError::Transport(_)) => {}
            other => panic!("expected a transport error, got {other:?}"),
        }
    }
}
