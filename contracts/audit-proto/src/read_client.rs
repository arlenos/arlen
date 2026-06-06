//! Read-API client.
//!
//! [`ReadClient`] runs one range query against `lunaris-auditd`'s read
//! socket and returns a page of Structural-tier views. Like the
//! ingest [`AuditClient`](crate::client::AuditClient) it opens a fresh
//! connection per call — the Anomaly Detector polls at a low cadence,
//! so a one-shot connection needs no reconnect handling.
//!
//! Unlike the ingest client, a read failure is **not** safety-
//! critical: the detector is advisory, so a transport error is a
//! retriable condition the caller logs and backs off on, not a
//! fail-closed gate.

use std::path::PathBuf;
use std::time::Duration;

use tokio::net::UnixStream;

use crate::{read_frame, write_frame, ReadPage, ReadRequest, ReadResponse};

/// How long a single read (connect + exchange) may take.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Why a read did not succeed.
#[derive(Debug, thiserror::Error)]
pub enum ReadClientError {
    /// The daemon could not be reached or the exchange failed at the
    /// transport level.
    #[error("read transport: {0}")]
    Transport(String),
    /// The daemon answered with an error.
    #[error("read rejected: {0}")]
    Server(String),
    /// The daemon did not answer within [`READ_TIMEOUT`].
    #[error("read timed out")]
    Timeout,
}

/// A client of the audit daemon's read socket.
#[derive(Debug, Clone)]
pub struct ReadClient {
    socket_path: PathBuf,
}

impl ReadClient {
    /// Build a client targeting `socket_path`. Use
    /// [`crate::read_socket_path`] for the canonical location.
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    /// Read one page: entries with index in `[from, to)`, ascending,
    /// capped at `limit` (the daemon clamps to its own ceiling),
    /// optionally filtered to `project_id`. The returned [`ReadPage`]
    /// also carries the daemon's current tamper status.
    pub async fn read(
        &self,
        from: u64,
        to: u64,
        limit: u64,
        project_id: Option<&str>,
    ) -> Result<ReadPage, ReadClientError> {
        let req = ReadRequest {
            from,
            to,
            limit,
            project_id: project_id.map(|s| s.to_string()),
        };
        match tokio::time::timeout(READ_TIMEOUT, self.exchange(&req)).await {
            Ok(result) => result,
            Err(_elapsed) => Err(ReadClientError::Timeout),
        }
    }

    async fn exchange(&self, req: &ReadRequest) -> Result<ReadPage, ReadClientError> {
        let transport = ReadClientError::Transport;

        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| transport(format!("connect: {e}")))?;

        let body = serde_json::to_vec(req)
            .map_err(|e| transport(format!("encode request: {e}")))?;
        write_frame(&mut stream, &body)
            .await
            .map_err(|e| transport(e.to_string()))?;

        let reply = read_frame(&mut stream)
            .await
            .map_err(|e| transport(e.to_string()))?;
        let response: ReadResponse = serde_json::from_slice(&reply)
            .map_err(|e| transport(format!("decode response: {e}")))?;

        match response {
            ReadResponse::Page {
                entries,
                tampered,
                head,
            } => Ok(ReadPage {
                entries,
                tampered,
                head,
            }),
            ReadResponse::Error { reason } => Err(ReadClientError::Server(reason)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_against_a_missing_socket_is_a_transport_error() {
        let client = ReadClient::new("/nonexistent/audit-read.sock");
        match client.read(0, u64::MAX, 100, None).await {
            Err(ReadClientError::Transport(_)) => {}
            other => panic!("expected a transport error, got {other:?}"),
        }
    }
}
