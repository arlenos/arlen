//! The grant minted when a consent decision is "always allow"
//! (system-dialog-plan.md: every always-allow == a revocable grant node in the
//! KG + audit ledger). This is the pure record + builder: it turns a resolved
//! [`PendingRequest`] + its [`ConsentOutcome`] into a [`ConsentGrant`] the
//! daemon then persists into the shared LCG Grant node (Option A, in
//! `SharedState::resolve` via the `GrantPersister`) and the capability browser
//! later shows + revokes. This module builds the record and is unit-tested.

use serde::{Deserialize, Serialize};

use crate::queue::PendingRequest;
use crate::{ConsentClass, ConsentOutcome};

/// A revocable grant recorded from an "always allow" consent decision.
///
/// The `recipient` is the attested app id ([`crate::AttestedRequester::grant_recipient`]) -
/// the SAME value shown in the dialog, never a separate field. The
/// `revocation_handle` is deterministic over (recipient, class, scope) so
/// re-consenting the same scope STRENGTHENS the existing grant rather than
/// minting a duplicate (the Living-Capability-Graph idempotency rule).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentGrant {
    /// The attested recipient the grant authorises - shown == recorded.
    pub recipient: String,
    /// What was consented to.
    pub class: ConsentClass,
    /// The concrete scope, when the request carried one (a path, a host, ...).
    pub scope: Option<String>,
    /// The plain-language summary shown when consent was given (for the
    /// browser's "what you allowed" line).
    pub summary: String,
    /// The stable, idempotent revocation handle (recipient + class + scope).
    pub revocation_handle: String,
    /// When this grant stops authorising, in epoch microseconds, or `None` for
    /// one that lasts until it is revoked.
    ///
    /// A window is not a weaker kind of remembering, it is a different promise:
    /// the user allowed something for the interaction they were in, and the
    /// grant has to stop on its own because nothing will come back to close it.
    /// Readers must treat a passed expiry as not-live rather than relying on
    /// anyone to revoke it.
    #[serde(default)]
    pub expires_at_micros: Option<i64>,
}

/// How long a gesture-scoped elevation authorises for.
///
/// One number in one place rather than a duration each caller passes, because
/// this is policy and callers would each pick their own. Five minutes is chosen
/// to outlast the interaction that prompted it - a file dialog, a share sheet,
/// a settings toggle that takes a couple of tries - and to be short enough that
/// an elevation forgotten about is not authority anyone still holds. It is not
/// "until the interaction ends" because there is no signal for that; a fixed
/// window is the honest approximation and it errs short.
pub const GESTURE_WINDOW_MICROS: i64 = 5 * 60 * 1_000_000;

/// The deterministic revocation handle for a (recipient, class, scope) triple.
/// Stable so a re-grant of the same scope reuses it (idempotent); the scope is
/// length-prefixed so it cannot collide with the class segment.
fn revocation_handle(recipient: &str, class: ConsentClass, scope: Option<&str>) -> String {
    let scope = scope.unwrap_or("");
    // Length-prefix the scope so "a" + "b.c" cannot alias "a.b" + "c".
    format!("{recipient}|{}|{}:{scope}", class.as_key(), scope.len())
}

/// Build the grant to record for a resolved request, or `None` when nothing is
/// minted (a one-time allow records no grant; a denial records none).
///
/// The recipient is the attested identity, so the displayed requester and the
/// grant recipient are one value end to end.
///
/// `now_micros` is passed rather than read so the window is a pure function of
/// its inputs. A windowed outcome expires at `now + GESTURE_WINDOW_MICROS`; a
/// remembered one carries no expiry. Re-consenting the same scope reuses the
/// handle, so renewing a window pushes the same grant's expiry out rather than
/// leaving a second one behind.
pub fn mint_grant(
    pending: &PendingRequest,
    outcome: ConsentOutcome,
    now_micros: i64,
) -> Option<ConsentGrant> {
    if !outcome.mints_grant() {
        return None;
    }
    let recipient = pending.request.requester.grant_recipient().to_string();
    let class = pending.request.class;
    let scope = pending.request.scope.clone();
    let revocation_handle = revocation_handle(&recipient, class, scope.as_deref());
    let expires_at_micros = match outcome {
        ConsentOutcome::AllowedForWindow => Some(now_micros.saturating_add(GESTURE_WINDOW_MICROS)),
        _ => None,
    };
    Some(ConsentGrant {
        recipient,
        class,
        scope,
        summary: pending.request.summary.clone(),
        revocation_handle,
        expires_at_micros,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{ConsentQueue, Enqueued};
    use crate::{AttestedRequester, ConsentRequest};
    use arlen_ai_core::capability::{AccessTier, ActionKind, ActionPermissions, BaselineMode, Capability};

    fn cap_suggest() -> Capability {
        Capability::new(
            AccessTier::Minimal,
            ActionPermissions::new(BaselineMode::Suggest, Vec::<String>::new()),
        )
    }

    fn pending(app: &str, scope: Option<&str>) -> PendingRequest {
        let mut q = ConsentQueue::new();
        let req = ConsentRequest {
            requester: AttestedRequester::new(app),
            class: ConsentClass::CapabilityGrant,
            kind: ActionKind::PermanentDelete, // any dialog-requiring kind
            triggered_by_external_content: false,
            recipient: None,
            preview: None,
            targets: Vec::new(),
            total: None,
            summary: "delete stuff".to_string(),
            scope: scope.map(str::to_string),
        };
        match q.enqueue(req, &cap_suggest()) {
            Enqueued::Queued(id) => q.resolve(id, ConsentOutcome::Denied).unwrap().0,
            Enqueued::SilentGrant => panic!("expected a dialog-requiring request"),
        }
    }

    #[test]
    fn remembered_mints_a_grant_for_the_attested_recipient() {
        let p = pending("org.arlen.files", Some("/home/u/docs"));
        let grant = mint_grant(&p, ConsentOutcome::AllowedRemembered, 0).unwrap();
        assert_eq!(grant.recipient, "org.arlen.files", "recipient is the attested id");
        assert_eq!(grant.class, ConsentClass::CapabilityGrant);
        assert_eq!(grant.scope.as_deref(), Some("/home/u/docs"));
    }

    #[test]
    fn allowed_once_and_denied_mint_nothing() {
        let p = pending("org.arlen.files", None);
        assert!(mint_grant(&p, ConsentOutcome::AllowedOnce, 0).is_none());
        assert!(mint_grant(&p, ConsentOutcome::Denied, 0).is_none());
    }

    /// A windowed allow records a grant like a remembered one, and differs in
    /// exactly one thing: it stops.
    #[test]
    fn a_windowed_allow_records_a_grant_that_closes() {
        let now = 1_700_000_000_000_000;
        let g = mint_grant(&pending("app", Some("photos")), ConsentOutcome::AllowedForWindow, now)
            .expect("a window is real authority, so it is recorded");
        assert_eq!(g.expires_at_micros, Some(now + GESTURE_WINDOW_MICROS));

        // Remembering carries no expiry, which is what makes it remembering.
        let r = mint_grant(&pending("app", Some("photos")), ConsentOutcome::AllowedRemembered, now)
            .unwrap();
        assert_eq!(r.expires_at_micros, None);

        // And the two are the same grant: renewing a window pushes the expiry out
        // rather than leaving a second row behind.
        assert_eq!(g.revocation_handle, r.revocation_handle);
    }

    #[test]
    fn re_consenting_the_same_scope_is_idempotent() {
        let a = mint_grant(&pending("app", Some("photos")), ConsentOutcome::AllowedRemembered, 0).unwrap();
        let b = mint_grant(&pending("app", Some("photos")), ConsentOutcome::AllowedRemembered, 0).unwrap();
        assert_eq!(a.revocation_handle, b.revocation_handle, "same scope -> same handle");
    }

    #[test]
    fn different_scope_or_recipient_gets_a_distinct_handle() {
        let base = mint_grant(&pending("app", Some("photos")), ConsentOutcome::AllowedRemembered, 0).unwrap();
        let other_scope = mint_grant(&pending("app", Some("music")), ConsentOutcome::AllowedRemembered, 0).unwrap();
        let other_app = mint_grant(&pending("app2", Some("photos")), ConsentOutcome::AllowedRemembered, 0).unwrap();
        assert_ne!(base.revocation_handle, other_scope.revocation_handle);
        assert_ne!(base.revocation_handle, other_app.revocation_handle);
    }

    #[test]
    fn scope_length_prefix_prevents_segment_aliasing() {
        // "a"+"b|c" must not collide with "a|b"+"c" etc. The length prefix on
        // the scope segment guards the join.
        let g1 = mint_grant(&pending("a", Some("b")), ConsentOutcome::AllowedRemembered, 0).unwrap();
        let g2 = mint_grant(&pending("a", Some("bb")), ConsentOutcome::AllowedRemembered, 0).unwrap();
        assert_ne!(g1.revocation_handle, g2.revocation_handle);
    }
}
