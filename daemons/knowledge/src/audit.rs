//! Audit event builders for the graph daemon (S13 audit-before-act).
//!
//! Only the app-tier entity-write path (foreign-app-bridges) audits today:
//! a third-party app upserting a node of its own declared namespace is a
//! durable, content-free record of who wrote what type, fail-closed before
//! the write persists. The audited ACTOR is set by the audit daemon from the
//! submitter's kernel-attested `SO_PEERCRED` (the graph daemon, `knowledge`),
//! never from the request; the calling app is recorded as a coarse identifier
//! in `node_types`. No field bodies, no instance key — the structural tier
//! stays content-free.

use audit_proto::{AuditKind, IngestRequest, StructuralRecord};

/// Build the content-free audit record for one app-tier entity upsert. The
/// bridge app and the qualified entity type are coarse identifiers
/// (`node_types`); `outcome` is `ok` (authorised, about to persist) or an
/// error label. The per-instance external key and the field values are
/// deliberately omitted.
pub fn entity_upsert_event(app_id: &str, qualified_type: &str, outcome: &str) -> IngestRequest {
    IngestRequest {
        kind: AuditKind::AppAction,
        structural: StructuralRecord {
            subject: "entity.upsert".to_string(),
            node_types: vec![app_id.to_string(), qualified_type.to_string()],
            relations: Vec::new(),
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

/// Build the content-free audit record for one app-tier entity link. The bridge
/// app and the two endpoint types are coarse identifiers (`node_types`), the
/// edge type a coarse label (`relations`); `outcome` is `ok` or an error label.
/// The per-instance external keys are deliberately omitted.
pub fn entity_link_event(
    app_id: &str,
    edge_type: &str,
    from_type: &str,
    to_type: &str,
    outcome: &str,
) -> IngestRequest {
    IngestRequest {
        kind: AuditKind::AppAction,
        structural: StructuralRecord {
            subject: "entity.link".to_string(),
            node_types: vec![
                app_id.to_string(),
                from_type.to_string(),
                to_type.to_string(),
            ],
            relations: vec![edge_type.to_string()],
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
    fn entity_upsert_event_is_content_free() {
        let e = entity_upsert_event("com.example.app", "com.example.app.Note", "ok");
        assert_eq!(e.structural.subject, "entity.upsert");
        assert_eq!(
            e.structural.node_types,
            vec!["com.example.app".to_string(), "com.example.app.Note".to_string()]
        );
        assert_eq!(e.structural.outcome, "ok");
        // No instance key, no field bodies, no forensic content.
        assert!(e.forensic.is_none());
        assert!(matches!(e.kind, AuditKind::AppAction));
    }
}
