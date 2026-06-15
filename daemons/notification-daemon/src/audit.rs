//! Content-free audit of notification display (GAP-2).
//!
//! A notification the daemon handles is a system action worth recording: it is
//! the channel an app uses to reach the user's attention, and one that can
//! pierce Do-Not-Disturb. Like every structural record it carries only coarse
//! identifiers: WHICH app posted (its self-reported name, not an attested id)
//! and the disposition (shown / suppressed / queued / dropped). The summary,
//! the body, the icon and the actions are NEVER recorded — the ledger says an
//! app posted a notification and what the daemon did with it, never the message
//! text (foundation §8.4 + GAP-4: no notification content in the ledger).
//!
//! The audited ACTOR is the notification daemon itself (kernel-attested at the
//! ingest socket via SO_PEERCRED → `notifyd`); the posting app travels as a
//! coarse `node_type`, carrying no attestation.

use audit_proto::{AuditKind, IngestRequest, StructuralRecord};

/// Build the content-free `IngestRequest` for one notification disposition.
///
/// A notification is observed, non-AI system activity, so the record is
/// [`AuditKind::AppAction`]. `app_name` is the posting app's self-reported name
/// and `outcome` is the disposition (`shown`/`suppressed`/`queued`/`dropped`);
/// no message text is carried.
pub fn notification_event(app_name: &str, outcome: &str) -> IngestRequest {
    IngestRequest {
        kind: AuditKind::AppAction,
        structural: StructuralRecord {
            subject: "notification.shown".to_string(),
            node_types: vec![app_name.to_string()],
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
    fn a_shown_notification_records_app_and_disposition() {
        let req = notification_event("Element", "shown");
        assert_eq!(req.kind, AuditKind::AppAction);
        assert_eq!(req.structural.subject, "notification.shown");
        assert_eq!(req.structural.node_types, vec!["Element"]);
        assert_eq!(req.structural.outcome, "shown");
        req.validate().expect("within structural caps");
    }

    #[test]
    fn the_message_never_reaches_the_ledger() {
        // Only the posting app + the disposition are carried: no summary,
        // no body, no icon, no actions, no forensic tier.
        let req = notification_event("Signal", "suppressed");
        assert!(req.forensic.is_none());
        let haystack = format!(
            "{} {} {}",
            req.structural.subject,
            req.structural.node_types.join(" "),
            req.structural.outcome
        );
        // The record is structurally incapable of carrying message content;
        // it only ever holds the app name and a fixed disposition token.
        assert!(haystack.contains("Signal"));
        assert!(haystack.contains("suppressed"));
    }
}
