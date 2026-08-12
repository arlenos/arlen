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
    /// How long this grant lasts.
    ///
    /// One field rather than a second collection of session-scoped grants: a
    /// person asking what an app can reach has to get ONE answer, and two stores
    /// means a browser that looks complete while being partial. Revocation is the
    /// same operation however the grant arrived, for the same reason - two revoke
    /// paths is how one of them ends up not covering something.
    ///
    /// Absent means [`GrantLifetime::Session`], and so does a kind this build does
    /// not recognise. An omission must never confer the longer-lived thing.
    #[serde(default)]
    pub lifetime: GrantLifetime,
}

/// How long a grant authorises for.
///
/// The instant lives INSIDE the windowed variant rather than beside the enum, so
/// a window without an end and an end without a window are both unrepresentable.
/// The alternative - a kind field and a separate timestamp - is two places to
/// read one fact, and they drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GrantLifetime {
    /// Ends when the interaction's window closes, at this instant in epoch
    /// microseconds. The grant has to stop on its own: nothing comes back to
    /// close it, so a reader treats a passed instant as not-live rather than
    /// waiting for a revoke.
    Until { at_micros: i64 },
    /// Lasts until revoked, and is recorded beside the profile so it reads as
    /// something the user added rather than something the app shipped with.
    Persistent,
    /// Ends when this login does. The default, and where anything unrecognised
    /// lands: `#[serde(other)]` catches a kind written by a newer build, and
    /// `#[serde(default)]` on the field catches one that is missing entirely.
    ///
    /// A malformed `until` - the kind present, the instant not - fails the record
    /// rather than landing here, which drops the grant. That is stricter than
    /// this default and deliberately not softened: a window with no end is not a
    /// window, and conferring nothing is safer than conferring a session.
    #[serde(other)]
    #[default]
    Session,
}

impl GrantLifetime {
    /// The instant this stops authorising, for the one kind that has one.
    ///
    /// `None` is not "forever" - a session grant also ends - it is "no instant to
    /// compare against", and a caller that only checks this must apply the session
    /// boundary separately.
    pub fn expires_at_micros(self) -> Option<i64> {
        match self {
            GrantLifetime::Until { at_micros } => Some(at_micros),
            GrantLifetime::Persistent | GrantLifetime::Session => None,
        }
    }
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
    let lifetime = match outcome {
        ConsentOutcome::AllowedForWindow => GrantLifetime::Until {
            at_micros: now_micros.saturating_add(GESTURE_WINDOW_MICROS),
        },
        // Remembering is the choice to keep it past this login, which is the one
        // outcome that may outlive the session.
        _ => GrantLifetime::Persistent,
    };
    Some(ConsentGrant {
        recipient,
        class,
        scope,
        summary: pending.request.summary.clone(),
        revocation_handle,
        lifetime,
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
        assert_eq!(g.lifetime, GrantLifetime::Until { at_micros: now + GESTURE_WINDOW_MICROS });

        // Remembering carries no expiry, which is what makes it remembering.
        let r = mint_grant(&pending("app", Some("photos")), ConsentOutcome::AllowedRemembered, now)
            .unwrap();
        assert_eq!(r.lifetime, GrantLifetime::Persistent);

        // And the two are the same grant: renewing a window pushes the expiry out
        // rather than leaving a second row behind.
        assert_eq!(g.revocation_handle, r.revocation_handle);
    }

    /// The hazard that comes with one shared store: a lifetime bug that persists
    /// a session grant. So the fail-safe direction is asserted rather than
    /// assumed, on the two shapes a stored record can be wrong in.
    #[test]
    fn a_lifetime_that_cannot_be_read_is_a_session_not_a_promise() {
        // Written by an older build that had no lifetime at all.
        let absent: ConsentGrant = serde_json::from_str(
            r#"{"recipient":"app","class":"capability_grant","scope":"/p",
                "summary":"s","revocation_handle":"h"}"#,
        )
        .expect("a record without the field still parses");
        assert_eq!(absent.lifetime, GrantLifetime::Session);

        // Written by a NEWER build with a kind this one has never heard of. The
        // tempting failure is to treat unknown as unrestricted.
        let unknown: ConsentGrant = serde_json::from_str(
            r#"{"recipient":"app","class":"capability_grant","scope":"/p",
                "summary":"s","revocation_handle":"h",
                "lifetime":{"kind":"until_reboot"}}"#,
        )
        .expect("an unknown kind does not fail the record");
        assert_eq!(unknown.lifetime, GrantLifetime::Session);

        // And a window with no end is not a window: the record is refused, which
        // confers nothing at all - stricter than the default above, on purpose.
        assert!(serde_json::from_str::<ConsentGrant>(
            r#"{"recipient":"app","class":"capability_grant","scope":"/p",
                "summary":"s","revocation_handle":"h","lifetime":{"kind":"until"}}"#,
        )
        .is_err());
    }

    /// A session grant has no instant to compare against, and that must not read
    /// as "no expiry, therefore forever" at a call site that only asks for one.
    #[test]
    fn a_session_grant_reports_no_instant_without_reporting_permanence() {
        assert_eq!(GrantLifetime::Session.expires_at_micros(), None);
        assert_eq!(GrantLifetime::Persistent.expires_at_micros(), None);
        assert_eq!(GrantLifetime::Until { at_micros: 7 }.expires_at_micros(), Some(7));
        assert_ne!(
            GrantLifetime::Session,
            GrantLifetime::Persistent,
            "the two that share an absent instant are still different promises"
        );
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
