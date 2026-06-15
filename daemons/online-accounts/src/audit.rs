//! Content-free audit of the credential handout (GAP-2 / GAP-15).
//!
//! `GetAccessToken` hands a stored OAuth token to a granted caller. That release
//! is a sensitive action and must land in the audit ledger, but, like every
//! structural record, it carries only coarse identifiers: WHO asked (the caller
//! app id), for WHICH service (the coarse `files`/`calendar`/... key), and the
//! outcome. The token itself, the account credential, and the account id are
//! NEVER recorded - the ledger says a credential was released, never its value.

use audit_proto::{AuditKind, IngestRequest, StructuralRecord};

/// Build the content-free `IngestRequest` for one credential handout.
///
/// A credential grant is a capability exercise, so it records as
/// [`AuditKind::Permission`]. `caller` is the granted app's id and `service` is
/// the coarse service key; neither the token nor the account id is carried.
pub fn credential_handout_event(caller: &str, service: &str, outcome: &str) -> IngestRequest {
    IngestRequest {
        kind: AuditKind::Permission,
        structural: StructuralRecord {
            subject: "credential.handout".to_string(),
            node_types: vec![caller.to_string(), service.to_string()],
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

    #[test]
    fn a_handout_records_caller_and_service_as_a_permission() {
        let req = credential_handout_event("com.example.editor", "files", "granted");
        assert_eq!(req.kind, AuditKind::Permission);
        assert_eq!(req.structural.subject, "credential.handout");
        assert_eq!(req.structural.node_types, vec!["com.example.editor", "files"]);
        assert_eq!(req.structural.outcome, "granted");
        req.validate().expect("within structural caps");
    }

    #[test]
    fn the_token_and_account_never_reach_the_ledger() {
        // Only the caller + the coarse service key are carried: no token bytes,
        // no account id, no forensic tier.
        let req = credential_handout_event("app", "calendar", "granted");
        assert!(req.forensic.is_none());
        let haystack = format!(
            "{} {}",
            req.structural.subject,
            req.structural.node_types.join(" ")
        );
        assert!(!haystack.contains("ya29")); // no OAuth access-token prefix
        assert!(!haystack.contains('@')); // no account/email-shaped id
    }
}
