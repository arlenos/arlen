//! Map an [`AuthEvent`] onto the audit ledger's content-free `IngestRequest`
//! (lockscreen-plan.md LS-R1, greeter-onboarding-plan.md: "a failed login is a
//! logged security event"). Every unlock attempt - greeter or lock screen,
//! success, denial or key release - becomes one ledger entry.
//!
//! The structural tier is always recorded and daemon-readable, so it stays
//! strictly content-free: the subject is `auth.<surface>`, the single
//! `node_types` label is the coarse factor kind, and the outcome is the coarse
//! label the composition produced. The PASSWORD never appears (it never leaves
//! [`crate::auth::Presentation`]), and neither does the ACCOUNT NAME: a username
//! in the daemon-readable tier is a privacy leak (it reveals who logged in or
//! failed), and the audit protocol's actor is the auth daemon's kernel-attested
//! identity, not the human. The security value the ledger carries - which
//! surface, which factor, success vs denial vs key-release - is preserved; the
//! per-account failed-attempt escalation lives in [`crate::tier::SessionState`],
//! the live auth flow's state, not the ledger.

use audit_proto::{AuditKind, IngestRequest, StructuralRecord};

use crate::auth::AuthEvent;

/// Build the content-free `IngestRequest` for one authentication attempt.
///
/// `kind` is [`AuditKind::Permission`]: an unlock is a capability decision, and
/// the audit protocol defines no dedicated authentication variant (the same
/// choice the transfer daemon makes for its capability gate). The account name
/// on the [`AuthEvent`] is deliberately NOT carried (see the module doc).
pub fn auth_audit_event(event: &AuthEvent) -> IngestRequest {
    IngestRequest {
        kind: AuditKind::Permission,
        structural: StructuralRecord {
            subject: format!("auth.{}", event.surface.as_key()),
            node_types: vec![event.factor.as_key().to_string()],
            relations: vec![],
            result_count: None,
            duration_ms: None,
            outcome: event.outcome.to_string(),
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
    use crate::auth::{FactorKind, Surface};

    fn event(surface: Surface, factor: FactorKind, outcome: &'static str, released: bool) -> AuthEvent {
        AuthEvent {
            account: "alice".into(),
            surface,
            factor,
            outcome,
            released_key: released,
        }
    }

    #[test]
    fn a_key_release_records_a_content_free_permission_entry() {
        let ev = event(Surface::Greeter, FactorKind::Password, "key-release", true);
        let req = auth_audit_event(&ev);
        assert_eq!(req.kind, AuditKind::Permission);
        assert_eq!(req.structural.subject, "auth.greeter");
        assert_eq!(req.structural.node_types, vec!["password"]);
        assert_eq!(req.structural.outcome, "key-release");
        // It passes the daemon-side size caps.
        req.validate().expect("within structural caps");
    }

    #[test]
    fn the_account_name_never_reaches_the_structural_tier() {
        // The account must not leak into any daemon-readable structural field,
        // even though the AuthEvent carries it for the live flow.
        let ev = event(Surface::LockScreen, FactorKind::Fingerprint, "denied:strong-auth-required", false);
        let req = auth_audit_event(&ev);
        let haystack = format!(
            "{}{}{}",
            req.structural.subject,
            req.structural.outcome,
            req.structural.node_types.join(",")
        );
        assert!(!haystack.contains("alice"), "account leaked into the structural tier: {haystack}");
        assert!(req.forensic.is_none(), "no forensic content carried");
    }

    #[test]
    fn each_surface_and_factor_maps_to_its_coarse_label() {
        let g = auth_audit_event(&event(Surface::Greeter, FactorKind::Fido2, "warm-unlock", false));
        assert_eq!(g.structural.subject, "auth.greeter");
        assert_eq!(g.structural.node_types, vec!["fido2"]);
        let l = auth_audit_event(&event(Surface::LockScreen, FactorKind::Proximity, "denied:bad-credential", false));
        assert_eq!(l.structural.subject, "auth.lock-screen");
        assert_eq!(l.structural.node_types, vec!["proximity"]);
    }
}
