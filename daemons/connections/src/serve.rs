//! The request-handling composition (connections-plan.md §2).
//!
//! This ties the pieces together, transport-free: resolve the requested
//! connection, authorize it against the config grants + load the sealed
//! credential ([`authorize_and_load`]), and AUDIT the outcome. The security
//! ordering is S13 audit-before-act: a credential RELEASE (a grant) is audited
//! BEFORE it is returned and fails closed if the ledger is down, so a credential
//! is never released without a ledger record. A denial is not a release, so it is
//! audited best-effort and a down ledger never turns a legitimate denial into an
//! error. The D-Bus/socket transport calls this with the kernel-attested app id;
//! being transport-free it is tested with a mock sink.

use audit_proto::AuditSink;

use crate::audit::credential_handout_event;
use crate::broker::{ConnectionGrant, ConnectionId, CredentialRequest, DenyReason};
use crate::store::{authorize_and_load, AuthorizedHandout, CredentialStore, HandoutError};

/// Why a request failed to produce a handout.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The requested connection id is malformed (not a valid connection).
    #[error("unknown connection")]
    UnknownConnection,
    /// The broker refused the request (no grant or scope beyond the ceiling).
    #[error("denied: {0:?}")]
    Denied(DenyReason),
    /// Authorized, but no credential is stored for the connection.
    #[error("no credential stored for the connection")]
    NoCredential,
    /// The credential store failed.
    #[error("store: {0}")]
    Store(String),
    /// The audit ledger was unreachable, so a grant was refused rather than
    /// released un-audited (fail closed).
    #[error("audit unavailable")]
    AuditUnavailable,
}

/// Handle one credential request end to end: authorize against `grants`, and on a
/// grant audit-before-return (fail closed) and hand back the authorized handout.
/// `attested_app_id` MUST be the kernel/bus-attested caller identity the transport
/// resolved, never a client-supplied value.
pub async fn serve_request(
    attested_app_id: &str,
    connection: &str,
    requested_scope: Vec<String>,
    grants: &[ConnectionGrant],
    store: &CredentialStore,
    audit: &dyn AuditSink,
) -> Result<AuthorizedHandout, ServeError> {
    // Resolve the connection id (fail closed). A malformed id is a denied handout.
    let Some(connection_id) = ConnectionId::new(connection) else {
        best_effort_audit(audit, attested_app_id, connection, "denied").await;
        return Err(ServeError::UnknownConnection);
    };
    let request = CredentialRequest {
        app_id: attested_app_id.to_string(),
        connection_id,
        requested_scope,
    };

    match authorize_and_load(&request, grants, store) {
        Ok(handout) => {
            // A release: audit BEFORE returning. If the ledger is down, refuse -
            // no credential leaves the daemon un-audited.
            audit
                .submit(credential_handout_event(attested_app_id, connection, "granted"))
                .await
                .map_err(|_| ServeError::AuditUnavailable)?;
            Ok(handout)
        }
        Err(HandoutError::Denied(reason)) => {
            best_effort_audit(audit, attested_app_id, connection, "denied").await;
            Err(ServeError::Denied(reason))
        }
        Err(HandoutError::NoCredential) => {
            best_effort_audit(audit, attested_app_id, connection, "no-credential").await;
            Err(ServeError::NoCredential)
        }
        Err(HandoutError::Store(e)) => {
            best_effort_audit(audit, attested_app_id, connection, "error").await;
            Err(ServeError::Store(e.to_string()))
        }
    }
}

/// Audit a non-release outcome best-effort: a denial is not a credential release,
/// so a down ledger must not turn it into an error.
async fn best_effort_audit(audit: &dyn AuditSink, caller: &str, connection: &str, outcome: &str) {
    let _ = audit
        .submit(credential_handout_event(caller, connection, outcome))
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use audit_proto::sink::MockAuditSink;

    use crate::broker::CredentialKind;
    use crate::store::Credential;

    fn conn(s: &str) -> ConnectionId {
        ConnectionId::new(s).unwrap()
    }

    fn store_with(secret: &[u8]) -> (tempfile::TempDir, CredentialStore) {
        let tmp = tempfile::TempDir::new().unwrap();
        let s = CredentialStore::new([3u8; 32], tmp.path().join("connections"));
        s.put(
            &conn("github"),
            &Credential {
                kind: CredentialKind::OAuthRefreshToken,
                secret: secret.to_vec(),
            },
        )
        .unwrap();
        (tmp, s)
    }

    fn grants() -> Vec<ConnectionGrant> {
        vec![ConnectionGrant {
            app_id: "com.example.app".to_string(),
            connection_id: conn("github"),
            max_scope: vec!["repo".to_string(), "read:user".to_string()],
        }]
    }

    #[tokio::test]
    async fn a_grant_audits_then_releases() {
        let (_tmp, store) = store_with(b"refresh-xyz");
        let audit = MockAuditSink::accepting();
        let out = serve_request(
            "com.example.app",
            "github",
            vec!["read:user".to_string()],
            &grants(),
            &store,
            &audit,
        )
        .await
        .unwrap();
        assert_eq!(out.scope, vec!["read:user".to_string()]);
        assert_eq!(out.credential.secret, b"refresh-xyz");
        // The handout was recorded as granted.
        let recorded = audit.recorded().await;
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].structural.outcome, "granted");
    }

    #[tokio::test]
    async fn a_grant_with_the_ledger_down_is_refused_not_released() {
        let (_tmp, store) = store_with(b"refresh-xyz");
        let audit = MockAuditSink::failing();
        let err = serve_request(
            "com.example.app",
            "github",
            vec!["read:user".to_string()],
            &grants(),
            &store,
            &audit,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ServeError::AuditUnavailable));
    }

    #[tokio::test]
    async fn a_denial_returns_and_is_audited_best_effort() {
        let (_tmp, store) = store_with(b"refresh-xyz");
        let audit = MockAuditSink::accepting();
        // A different app: no grant.
        let err = serve_request(
            "com.other.app",
            "github",
            vec!["repo".to_string()],
            &grants(),
            &store,
            &audit,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ServeError::Denied(DenyReason::NoGrant)));
        let recorded = audit.recorded().await;
        assert_eq!(recorded[0].structural.outcome, "denied");
    }

    #[tokio::test]
    async fn a_denial_survives_a_down_ledger() {
        // A denial is not a release, so a failing ledger does not turn it into an
        // error - the deny still returns.
        let (_tmp, store) = store_with(b"refresh-xyz");
        let audit = MockAuditSink::failing();
        let err = serve_request(
            "com.other.app",
            "github",
            vec!["repo".to_string()],
            &grants(),
            &store,
            &audit,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ServeError::Denied(DenyReason::NoGrant)));
    }

    #[tokio::test]
    async fn a_malformed_connection_is_unknown() {
        let (_tmp, store) = store_with(b"x");
        let audit = MockAuditSink::accepting();
        let err = serve_request(
            "com.example.app",
            "Bad/Conn",
            vec![],
            &grants(),
            &store,
            &audit,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ServeError::UnknownConnection));
    }
}
