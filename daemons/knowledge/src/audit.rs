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

use audit_proto::{AuditKind, CapabilityReach, IngestRequest, StructuralRecord};

use crate::revoke::RevokedReach;

/// Convert a daemon [`RevokedReach`] into the audit-proto wire reach. The two are
/// deliberately separate types (audit-proto stays dependency-light and cannot dep
/// the permissions crate), so a producer converts at the ingest boundary.
fn reach_to_audit(reach: &RevokedReach) -> CapabilityReach {
    match reach {
        RevokedReach::Read { entity_pattern } => CapabilityReach::Read {
            entity_pattern: entity_pattern.clone(),
        },
        RevokedReach::Write { entity_pattern } => CapabilityReach::Write {
            entity_pattern: entity_pattern.clone(),
        },
        RevokedReach::Relation {
            from,
            to,
            relation_type,
        } => CapabilityReach::Relation {
            from: from.clone(),
            to: to.clone(),
            relation_type: relation_type.clone(),
        },
        RevokedReach::InstanceAll => CapabilityReach::InstanceAll,
        RevokedReach::NetworkDomain { domain } => CapabilityReach::NetworkDomain {
            domain: domain.clone(),
        },
        RevokedReach::ClipboardCap { cap } => CapabilityReach::ClipboardCap { cap: cap.clone() },
        RevokedReach::NotificationsOff => CapabilityReach::NotificationsOff,
        RevokedReach::InputCap { cap } => CapabilityReach::InputCap { cap: cap.clone() },
        RevokedReach::SearchCap { cap } => CapabilityReach::SearchCap { cap: cap.clone() },
        RevokedReach::IntentsCap { cap } => CapabilityReach::IntentsCap { cap: cap.clone() },
        RevokedReach::FilesystemDir { dir } => CapabilityReach::FilesystemDir { dir: dir.clone() },
        RevokedReach::FilesystemPath { path } => CapabilityReach::FilesystemPath {
            path: path.clone(),
        },
        RevokedReach::EventBusSubscribe { pattern } => CapabilityReach::EventBusSubscribe {
            pattern: pattern.clone(),
        },
        RevokedReach::EventBusPublish { pattern } => CapabilityReach::EventBusPublish {
            pattern: pattern.clone(),
        },
        RevokedReach::SystemCap { cap } => CapabilityReach::SystemCap { cap: cap.clone() },
    }
}

/// The fixed `subject` of a capability-change audit record. Shared by the producer
/// ([`capability_change_event`]) and the reader's filter, so the two cannot drift.
pub const CAPABILITY_CHANGE_SUBJECT: &str = "capability.change";

/// The inverse of [`reach_to_audit`]: an audit-proto wire reach read back out of the
/// ledger, into the daemon [`RevokedReach`]. Used by the fold that reconstructs a
/// target app's removal ledger from its capability-change records.
pub fn audit_to_reach(reach: &CapabilityReach) -> RevokedReach {
    match reach {
        CapabilityReach::Read { entity_pattern } => RevokedReach::Read {
            entity_pattern: entity_pattern.clone(),
        },
        CapabilityReach::Write { entity_pattern } => RevokedReach::Write {
            entity_pattern: entity_pattern.clone(),
        },
        CapabilityReach::Relation {
            from,
            to,
            relation_type,
        } => RevokedReach::Relation {
            from: from.clone(),
            to: to.clone(),
            relation_type: relation_type.clone(),
        },
        CapabilityReach::InstanceAll => RevokedReach::InstanceAll,
        CapabilityReach::NetworkDomain { domain } => RevokedReach::NetworkDomain {
            domain: domain.clone(),
        },
        CapabilityReach::ClipboardCap { cap } => RevokedReach::ClipboardCap { cap: cap.clone() },
        CapabilityReach::NotificationsOff => RevokedReach::NotificationsOff,
        CapabilityReach::InputCap { cap } => RevokedReach::InputCap { cap: cap.clone() },
        CapabilityReach::SearchCap { cap } => RevokedReach::SearchCap { cap: cap.clone() },
        CapabilityReach::IntentsCap { cap } => RevokedReach::IntentsCap { cap: cap.clone() },
        CapabilityReach::FilesystemDir { dir } => RevokedReach::FilesystemDir { dir: dir.clone() },
        CapabilityReach::FilesystemPath { path } => RevokedReach::FilesystemPath {
            path: path.clone(),
        },
        CapabilityReach::EventBusSubscribe { pattern } => RevokedReach::EventBusSubscribe {
            pattern: pattern.clone(),
        },
        CapabilityReach::EventBusPublish { pattern } => RevokedReach::EventBusPublish {
            pattern: pattern.clone(),
        },
        CapabilityReach::SystemCap { cap } => RevokedReach::SystemCap { cap: cap.clone() },
    }
}

/// Build the content-free audit record for a capability change: the user (via the
/// Settings app) narrowed (`revoked`) or later re-widened (`restored`) an app's
/// reach. The target app is a coarse identifier (`node_types`); the specific reach
/// rides the typed `capability_change` field, which is authority-metadata and NOT
/// user content, so recording it does not breach the S13 content-free boundary
/// (which governs data-access records). This is the durable record the
/// profile-first restore reads back as its ceiling (living-capability-graph.md §6,
/// the 1-July audit-ledger decision).
pub fn capability_change_event(
    target_app_id: &str,
    reach: &RevokedReach,
    outcome: &str,
) -> IngestRequest {
    IngestRequest {
        kind: AuditKind::CapabilityChange,
        structural: StructuralRecord {
            subject: CAPABILITY_CHANGE_SUBJECT.to_string(),
            node_types: vec![target_app_id.to_string()],
            relations: Vec::new(),
            result_count: None,
            duration_ms: None,
            outcome: outcome.to_string(),
            depth: None,
            capability_change: Some(reach_to_audit(reach)),
        },
        forensic: None,
        call_chain_id: None,
        project_id: None,
    }
}

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
            capability_change: None,
        },
        forensic: None,
        call_chain_id: None,
        project_id: None,
    }
}

/// Build the content-free audit record for a structural-canary trip
/// (canary-honeytools.md §3): an attempt to create a node whose id bears the
/// reserved canary namespace. No honest producer mints a canary id (promotion ids
/// are derived paths, entity ids are server-minted), so such an id can only reach
/// the caller-supplied-id write path if external content injected it, which is the
/// genuine agent-hijack condition. The record names only the acting app
/// (attribution) and the fixed `canary.trip` subject; the injected id itself is
/// deliberately omitted so the ledger never carries attacker-supplied content. The
/// write is refused by the ingestion reservation regardless; this is the SIGNAL
/// the anomaly detector surfaces, not the containment.
pub fn canary_trip_event(actor_app_id: &str) -> IngestRequest {
    IngestRequest {
        kind: AuditKind::PolicyViolation,
        structural: StructuralRecord {
            subject: "canary.trip".to_string(),
            node_types: vec![actor_app_id.to_string()],
            relations: Vec::new(),
            result_count: None,
            duration_ms: None,
            // The anomaly detector keys its alert cooldown on this outcome as the
            // content-free CAUSE class (detect.rs: e.g. `canary-tripped:structural`,
            // `honeytool-tripped`), so it must be a distinct cause label, not a bare
            // `refused` that would share a bucket with every other PolicyViolation.
            outcome: "canary-tripped:ingestion".to_string(),
            depth: None,
            capability_change: None,
        },
        forensic: None,
        call_chain_id: None,
        project_id: None,
    }
}

/// Build the content-free audit record for persisting one consent grant into the
/// KG (system-dialog-plan.md Option A). The acting broker, the grant recipient
/// and the consent class are coarse identifiers; the concrete scope is omitted.
pub fn consent_grant_event(broker: &str, recipient: &str, consent_class: &str) -> IngestRequest {
    IngestRequest {
        kind: AuditKind::Permission,
        structural: StructuralRecord {
            subject: "consent.grant.persist".to_string(),
            node_types: vec![broker.to_string(), recipient.to_string()],
            relations: vec![consent_class.to_string()],
            result_count: None,
            duration_ms: None,
            outcome: "ok".to_string(),
            depth: None,
            capability_change: None,
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

    #[test]
    fn canary_trip_event_is_content_free_and_a_policy_violation() {
        let e = canary_trip_event("dev.arlen-ai-engine-daemon");
        assert!(matches!(e.kind, AuditKind::PolicyViolation));
        assert_eq!(e.structural.subject, "canary.trip");
        // Only the acting app (attribution); the injected id is never recorded.
        assert_eq!(
            e.structural.node_types,
            vec!["dev.arlen-ai-engine-daemon".to_string()]
        );
        // The outcome is the anomaly detector's content-free cause class.
        assert_eq!(e.structural.outcome, "canary-tripped:ingestion");
        assert!(e.forensic.is_none());
    }

    #[test]
    fn capability_change_event_carries_the_reach() {
        let reach = RevokedReach::Read {
            entity_pattern: "system.File".to_string(),
        };
        let e = capability_change_event("com.example.app", &reach, "revoked");
        assert!(matches!(e.kind, AuditKind::CapabilityChange));
        assert_eq!(e.structural.subject, "capability.change");
        assert_eq!(e.structural.node_types, vec!["com.example.app".to_string()]);
        assert_eq!(e.structural.outcome, "revoked");
        // The typed reach rides the class-scoped field, not the coarse subject.
        assert_eq!(
            e.structural.capability_change,
            Some(CapabilityReach::Read {
                entity_pattern: "system.File".to_string()
            })
        );
        assert!(e.forensic.is_none());
    }

    #[test]
    fn reach_converts_round_trip() {
        for reach in [
            RevokedReach::Read { entity_pattern: "system.File".into() },
            RevokedReach::Write { entity_pattern: "com.x.Note".into() },
            RevokedReach::Relation {
                from: "system.File".into(),
                to: "system.Project".into(),
                relation_type: "FILE_PART_OF".into(),
            },
            RevokedReach::InstanceAll,
        ] {
            assert_eq!(audit_to_reach(&reach_to_audit(&reach)), reach);
        }
    }
}
