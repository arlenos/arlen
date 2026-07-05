//! Content-free audit of a credential handout (connections-plan.md §2, the
//! GAP-15 fix: every handout is audited into the ledger).
//!
//! When the broker authorizes a request and the daemon releases a scoped token,
//! that release is a sensitive capability exercise and must land in the audit
//! ledger. Like every structural record it carries only coarse identifiers: WHO
//! asked (the attested app id), for WHICH connection (the connection id), and the
//! outcome. The credential, the released token, and the scope detail are NEVER
//! recorded; the ledger says a credential was released for a connection, never
//! its value.

use audit_proto::{AuditKind, IngestRequest, StructuralRecord};

/// Build the content-free `IngestRequest` for one credential handout decision. A
/// handout is a capability exercise, so it records as [`AuditKind::Permission`].
/// `caller` is the attested app id, `connection` the connection id, and `outcome`
/// the coarse result (`granted`, `denied`, ...). Neither the credential nor the
/// scope tokens are carried.
pub fn credential_handout_event(caller: &str, connection: &str, outcome: &str) -> IngestRequest {
    IngestRequest {
        kind: AuditKind::Permission,
        structural: StructuralRecord {
            subject: "connection.handout".to_string(),
            node_types: vec![caller.to_string(), connection.to_string()],
            relations: vec![],
            result_count: None,
            duration_ms: None,
            outcome: outcome.to_string(),
            depth: None,
            capability_change: None,
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
    fn a_handout_records_caller_and_connection_as_a_permission() {
        let req = credential_handout_event("com.example.editor", "github", "granted");
        assert_eq!(req.kind, AuditKind::Permission);
        assert_eq!(req.structural.subject, "connection.handout");
        assert_eq!(req.structural.node_types, vec!["com.example.editor", "github"]);
        assert_eq!(req.structural.outcome, "granted");
        req.validate().expect("within structural caps");
    }

    #[test]
    fn the_credential_and_scope_never_reach_the_ledger() {
        // Only the caller + the connection id are carried: no secret bytes, no
        // scope tokens, no forensic tier.
        let req = credential_handout_event("app", "github", "denied");
        assert!(req.forensic.is_none());
        let haystack = format!(
            "{} {} {}",
            req.structural.subject,
            req.structural.node_types.join(" "),
            req.structural.outcome
        );
        assert!(!haystack.contains("ghp_")); // no GitHub token prefix
        assert!(!haystack.contains("repo")); // no scope token
    }
}
