//! Dual-ledger audit: every transfer is recorded in BOTH profiles' ledgers (profile-system-plan.md, Decided 5).
//!
//! The Transfer Daemon is the one service touching two profile uids, so a
//! transfer is audited to BOTH the source profile's and the destination
//! profile's `arlen-auditd` - neither side can later deny it happened nor
//! misattribute it. This is built on the audit protocol's actual shape:
//!
//! - `IngestRequest` carries NO actor. Each profile's `arlen-auditd` stamps the
//!   actor from the connection's kernel-attested `SO_PEERCRED`, which is the
//!   Transfer Daemon's own identity. That is exactly right: the broker is the
//!   actor that moved the bytes, recorded identically in both ledgers. A caller
//!   cannot suppress or misattribute the record.
//! - The two sinks are two [`audit_proto::LedgerAuditSink`], one per profile's
//!   per-uid ingest socket (`/run/user/{uid}/arlen/audit-ingest.sock`). The
//!   broker resolves the uid at the dual-uid boundary (deferred); the CORE takes
//!   the two sinks injected, so tests pass two mock sinks.
//! - The Structural tier stays content-free: a fixed coarse subject
//!   `transfer.<type>`, an outcome label, and the `(source, dest)` profile NAMES
//!   only. No file path, no clipboard content, no profile-internal id ever
//!   enters the always-recorded tier. A full file name, if ever recorded, is
//!   Forensic-tier only (not written here).
//!
//! Audit-before-act, both-must-succeed: the gate writes the decision to BOTH
//! sinks BEFORE the broker moves anything, and if EITHER sink fails the transfer
//! is refused (foundation §8.4.6: no un-audited flow). A denied attempt is also
//! audited to both.

use std::sync::Arc;

use audit_proto::{AuditKind, AuditSink, IngestRequest, StructuralRecord};

use crate::request::{ProfileId, TransferType};

/// The coarse outcome label written to the Structural tier. Never free-form.
pub mod outcome {
    /// The policy gate permitted the flow.
    pub const ALLOWED: &str = "allowed";
    /// The policy gate (or caller-auth, or validation) refused the flow.
    pub const DENIED: &str = "denied";
    /// The broker delivered the bytes to the destination profile.
    pub const DELIVERED: &str = "delivered";
    /// The transfer was undone within the cancellation window.
    pub const UNDONE: &str = "undone";
}

/// Why a dual-ledger write failed. Either sink failing refuses the transfer.
#[derive(Debug, thiserror::Error)]
pub enum DualLedgerError {
    /// The source profile's ledger did not record the event.
    #[error("source ledger: {0}")]
    Source(audit_proto::client::AuditClientError),
    /// The destination profile's ledger did not record the event.
    #[error("destination ledger: {0}")]
    Dest(audit_proto::client::AuditClientError),
}

/// The two per-profile audit sinks a transfer is recorded to.
///
/// Each sink targets one profile's `arlen-auditd`; that daemon stamps the actor
/// from the connection's `SO_PEERCRED` (the Transfer Daemon), so the actor is
/// never request-supplied. The daemon constructs each sink with the profile's
/// per-uid ingest socket path; tests inject two [`audit_proto::MockAuditSink`].
pub struct DualLedger {
    source_sink: Arc<dyn AuditSink>,
    dest_sink: Arc<dyn AuditSink>,
}

impl DualLedger {
    /// A dual ledger over the source and destination profiles' sinks.
    pub fn new(source_sink: Arc<dyn AuditSink>, dest_sink: Arc<dyn AuditSink>) -> Self {
        Self {
            source_sink,
            dest_sink,
        }
    }

    /// Record one transfer event in BOTH ledgers, audit-before-act and
    /// both-must-succeed. The subject is content-free (`transfer.<ty>` plus the
    /// two profile names); `outcome` is one of [`outcome`]. If either sink
    /// returns `Err`, the whole record fails and the caller must refuse the
    /// transfer - no byte crosses the boundary without a record in both ledgers.
    ///
    /// Both writes are attempted so that a record exists in the side that
    /// succeeded even when the other is down; the caller still treats any error
    /// as fail-closed. The source ledger is written first, then the destination.
    pub async fn record(
        &self,
        source: &ProfileId,
        dest: &ProfileId,
        ty: TransferType,
        outcome: &str,
    ) -> Result<(), DualLedgerError> {
        let request = transfer_event(source, dest, ty, outcome);
        let source_result = self.source_sink.submit(request.clone()).await;
        let dest_result = self.dest_sink.submit(request).await;
        // Surface the source failure first, but only after attempting both so a
        // live ledger still gets the record.
        source_result.map_err(DualLedgerError::Source)?;
        dest_result.map_err(DualLedgerError::Dest)?;
        Ok(())
    }
}

/// Build the content-free `IngestRequest` for a transfer.
///
/// The subject is `transfer.<ty>`, and the `(source, dest)` profile NAMES are
/// carried as the two `node_types` labels - coarse identifiers, never content.
/// No file path, clipboard text, or profile-internal id is included. The
/// `AuditKind` is `Permission` (the closest existing kind for a capability
/// decision; the audit protocol defines no `Transfer` variant and adding one is
/// out of this slice's scope, see the module doc).
pub fn transfer_event(
    source: &ProfileId,
    dest: &ProfileId,
    ty: TransferType,
    outcome: &str,
) -> IngestRequest {
    IngestRequest {
        kind: AuditKind::Permission,
        structural: StructuralRecord {
            subject: format!("transfer.{}", ty.as_str()),
            node_types: vec![source.as_str().to_string(), dest.as_str().to_string()],
            relations: vec![],
            result_count: None,
            duration_ms: None,
            outcome: outcome.to_string(),
            depth: None,
        },
        forensic: None,
        call_chain_id: None,
        project_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use audit_proto::MockAuditSink;

    fn pid(name: &str) -> ProfileId {
        ProfileId::new(name).expect("valid test profile id")
    }

    #[test]
    fn the_event_is_content_free() {
        let ev = transfer_event(&pid("work"), &pid("personal"), TransferType::File, outcome::ALLOWED);
        assert_eq!(ev.structural.subject, "transfer.file");
        // Only the two profile names, no path or content.
        assert_eq!(ev.structural.node_types, vec!["work", "personal"]);
        assert!(ev.structural.relations.is_empty());
        assert!(ev.forensic.is_none());
        assert_eq!(ev.structural.outcome, "allowed");
    }

    #[tokio::test]
    async fn both_ledgers_record_before_the_act() {
        let source = Arc::new(MockAuditSink::accepting());
        let dest = Arc::new(MockAuditSink::accepting());
        let ledger = DualLedger::new(source.clone(), dest.clone());
        ledger
            .record(&pid("work"), &pid("personal"), TransferType::Clipboard, outcome::ALLOWED)
            .await
            .expect("both accepting sinks record");
        assert_eq!(source.count().await, 1, "the source ledger recorded");
        assert_eq!(dest.count().await, 1, "the destination ledger recorded");
    }

    #[tokio::test]
    async fn a_failing_sink_fails_the_whole_record() {
        // If either side cannot record, the caller must refuse the transfer.
        let source_down = Arc::new(MockAuditSink::failing());
        let dest = Arc::new(MockAuditSink::accepting());
        let ledger = DualLedger::new(source_down, dest.clone());
        let err = ledger
            .record(&pid("work"), &pid("personal"), TransferType::File, outcome::ALLOWED)
            .await
            .expect_err("a failing source ledger fails the record");
        assert!(matches!(err, DualLedgerError::Source(_)));

        let source = Arc::new(MockAuditSink::accepting());
        let dest_down = Arc::new(MockAuditSink::failing());
        let ledger = DualLedger::new(source, dest_down);
        let err = ledger
            .record(&pid("work"), &pid("personal"), TransferType::File, outcome::ALLOWED)
            .await
            .expect_err("a failing destination ledger fails the record");
        assert!(matches!(err, DualLedgerError::Dest(_)));
    }
}
