//! The audit sink: the producer-side abstraction both AI-layer
//! daemons depend on.
//!
//! A sink takes an [`IngestRequest`] and commits it to the audit
//! ledger, returning the assigned chain index. Every `Err` means the
//! event was **not** recorded: a caller that audits before acting
//! must fail closed, because foundation §8.4.6 admits no un-audited
//! AI activity.
//!
//! The trait lives here, in the shared wire crate, so the AI daemon
//! (`ai-core`) and the network proxy (`ai-proxy`) use one definition
//! rather than each declaring its own — the two consumers cannot
//! drift into different trust levels.

use async_trait::async_trait;

use crate::client::{AuditClient, AuditClientError};
use crate::IngestRequest;

// Used only by the `test-util`-gated MockAuditSink.
#[cfg(any(test, feature = "test-util"))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(any(test, feature = "test-util"))]
use tokio::sync::Mutex;

/// A producer-side sink for audit events.
///
/// `submit` returns the committed hash-chain index on success. Any
/// `Err` means the event was not recorded; a caller on the
/// fail-closed path must not proceed.
#[async_trait]
pub trait AuditSink: Send + Sync {
    /// Submit one audit event and wait for the daemon's
    /// acknowledgement.
    async fn submit(&self, event: IngestRequest) -> Result<u64, AuditClientError>;
}

/// Production sink: submits to `arlen-auditd` over its ingest
/// socket.
pub struct LedgerAuditSink {
    client: AuditClient,
}

impl LedgerAuditSink {
    /// Build a sink over an explicit ingest client.
    pub fn new(client: AuditClient) -> Self {
        Self { client }
    }

    /// Build a sink targeting the canonical ingest socket
    /// ([`crate::ingest_socket_path`]).
    pub fn at_default_socket() -> Self {
        Self::new(AuditClient::new(crate::ingest_socket_path()))
    }
}

#[async_trait]
impl AuditSink for LedgerAuditSink {
    async fn submit(&self, event: IngestRequest) -> Result<u64, AuditClientError> {
        self.client.submit(&event).await
    }
}

/// In-memory sink for tests, in this workspace and downstream.
///
/// [`accepting`](Self::accepting) records every event and hands back
/// ascending indices; [`failing`](Self::failing) rejects every event
/// so a caller's fail-closed path can be exercised.
///
/// Gated behind the `test-util` feature (and this crate's own tests):
/// an accepting mock returns success without a ledger entry, so it is
/// a fail-open sink and must never be reachable in a production build.
/// Downstream test suites enable `test-util` as a dev-dependency
/// feature.
#[cfg(any(test, feature = "test-util"))]
pub struct MockAuditSink {
    accepting: bool,
    recorded: Mutex<Vec<IngestRequest>>,
    next_index: AtomicU64,
}

#[cfg(any(test, feature = "test-util"))]
impl MockAuditSink {
    /// A sink that accepts and records every submitted event.
    pub fn accepting() -> Self {
        Self {
            accepting: true,
            recorded: Mutex::new(Vec::new()),
            next_index: AtomicU64::new(0),
        }
    }

    /// A sink that rejects every event with an `Unavailable` error.
    pub fn failing() -> Self {
        Self {
            accepting: false,
            recorded: Mutex::new(Vec::new()),
            next_index: AtomicU64::new(0),
        }
    }

    /// Every event recorded so far, in submission order.
    pub async fn recorded(&self) -> Vec<IngestRequest> {
        self.recorded.lock().await.clone()
    }

    /// Number of events recorded so far.
    pub async fn count(&self) -> usize {
        self.recorded.lock().await.len()
    }
}

#[cfg(any(test, feature = "test-util"))]
#[async_trait]
impl AuditSink for MockAuditSink {
    async fn submit(&self, event: IngestRequest) -> Result<u64, AuditClientError> {
        if !self.accepting {
            return Err(AuditClientError::Unavailable(
                "mock audit sink: failing".to_string(),
            ));
        }
        let index = self.next_index.fetch_add(1, Ordering::SeqCst);
        self.recorded.lock().await.push(event);
        Ok(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuditKind, StructuralRecord};

    fn event() -> IngestRequest {
        IngestRequest {
            kind: AuditKind::Query,
            structural: StructuralRecord {
                subject: "ai.query".into(),
                node_types: vec![],
                relations: vec![],
                result_count: None,
                duration_ms: None,
                outcome: "dispatched".into(),
                depth: None,
            },
            forensic: None,
            call_chain_id: None,
            project_id: None,
        }
    }

    #[tokio::test]
    async fn accepting_mock_records_events_with_ascending_indices() {
        let sink = MockAuditSink::accepting();
        assert_eq!(sink.submit(event()).await.unwrap(), 0);
        assert_eq!(sink.submit(event()).await.unwrap(), 1);
        assert_eq!(sink.count().await, 2);
    }

    #[tokio::test]
    async fn failing_mock_rejects_every_event() {
        let sink = MockAuditSink::failing();
        let err = sink.submit(event()).await.expect_err("failing sink rejects");
        assert!(matches!(err, AuditClientError::Unavailable(_)));
        assert_eq!(sink.count().await, 0);
    }
}
