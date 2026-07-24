//! The intake-dispatch core: a client's wire request + the kernel-attested peer
//! identity become a queued (or silently-granted) consent request
//! (system-dialog-plan.md). The LOAD-BEARING rule lives here: the requester is
//! built from the SO_PEERCRED-attested app id, NEVER from the request payload -
//! the wire [`RequestBody`] structurally carries no requester field, so a
//! client cannot ask on another app's behalf (the macOS TCC CVE-2025-31250
//! spoof is unrepresentable). The socket accept loop that resolves the peer
//! (via `arlen_permissions` `ConnectionAuth` -> `path_to_app_id`) and frames
//! these is the daemon shell on top; this dispatch is pure and unit-tested.

use arlen_ai_core::capability::Capability;
use serde::{Deserialize, Serialize};

use crate::queue::{ConsentQueue, Enqueued, RequestId};
use crate::{AttestedRequester, ConsentRequest};

// The wire `RequestBody` lives in the shared `arlen-consent-contract` crate;
// re-exported here so `service::RequestBody` (and the lib's `pub use
// service::RequestBody`) and every internal reference are unchanged.
pub use arlen_consent_contract::RequestBody;

/// The broker's reply to an intake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum IntakeReply {
    /// The request needs a dialog and was queued under this id; the decision
    /// follows once the user resolves it on the trusted surface.
    Queued {
        /// The queue id (raw, for wire correlation).
        id: u64,
    },
    /// The request is Tier-1 (silent): granted without a dialog (still recorded).
    SilentGranted,
}

/// Assemble a full [`ConsentRequest`] from a wire body and the ATTESTED
/// requester. The requester is the only source of identity; the body never
/// contributes it.
pub fn assemble(body: RequestBody, requester: AttestedRequester) -> ConsentRequest {
    ConsentRequest {
        requester,
        class: body.class,
        kind: body.kind,
        triggered_by_external_content: body.triggered_by_external_content,
        summary: body.summary,
        scope: body.scope,
        recipient: body.recipient,
        preview: body.preview,
        targets: body.targets,
        total: body.total,
    }
}

/// The SO_PEERCRED-attested peers permitted to raise a request ON BEHALF OF an
/// app they have authenticated. The xdg portal mediates ScreenCast/Camera/Mic
/// for apps that reach the desktop-portal FRONTEND (which verifies the app via
/// `.flatpak-info`, xdg-portal §2 Option-A) rather than this broker directly, so
/// it must be able to attribute the capture grant to the app, not to itself. A
/// peer NOT in this set that sets `on_behalf_of` is ignored (attributed to
/// itself), so the field can never redirect a grant unless the peer is trusted.
const TRUSTED_INTERMEDIARIES: &[&str] = &["xdg-desktop-portal"];

/// Whether `peer` (an SO_PEERCRED-attested app id) may assert `on_behalf_of`.
fn is_trusted_intermediary(peer: &str) -> bool {
    TRUSTED_INTERMEDIARIES.contains(&peer)
}

/// A plausible reverse-DNS app id: the broker attributes a grant + keys a
/// revocation handle on this, so a mediator-supplied subject is charset-bounded
/// (defense-in-depth on top of the trusted mediator having resolved it itself).
fn is_plausible_app_id(app: &str) -> bool {
    !app.is_empty()
        && app.len() <= 255
        && app
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// Resolve the attested requester. `on_behalf_of` is HONORED only when the
/// attested peer is a trusted intermediary AND the named subject is a plausible
/// app id distinct from the peer; in every other case the field is ignored and
/// the requester is the attested peer itself (the load-bearing fail-safe). Used
/// by BOTH intake paths (this dispatch and the daemon's stateful `intake`).
pub(crate) fn resolve_requester(attested_peer: &str, on_behalf_of: Option<&str>) -> AttestedRequester {
    match on_behalf_of {
        Some(subject)
            if is_trusted_intermediary(attested_peer)
                && is_plausible_app_id(subject)
                && subject != attested_peer =>
        {
            AttestedRequester::on_behalf_of(subject, attested_peer)
        }
        _ => AttestedRequester::new(attested_peer),
    }
}

/// Handle one inbound request: build the request from the body + the attested
/// peer app id, classify and enqueue it, and return the reply. `attested_app_id`
/// MUST be the value the socket resolved from SO_PEERCRED (`path_to_app_id`),
/// never anything the client supplied.
pub fn handle_intake(
    body: RequestBody,
    attested_app_id: &str,
    capability: &Capability,
    queue: &mut ConsentQueue,
) -> IntakeReply {
    let requester = resolve_requester(attested_app_id, body.on_behalf_of.as_deref());
    let request = assemble(body, requester);
    match queue.enqueue(request, capability) {
        Enqueued::Queued(id) => IntakeReply::Queued { id: RequestId::get(id) },
        Enqueued::SilentGrant => IntakeReply::SilentGranted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_ai_core::capability::{AccessTier, ActionKind, ActionPermissions, BaselineMode};
    use crate::ConsentClass;

    fn cap_suggest() -> Capability {
        Capability::new(
            AccessTier::Minimal,
            ActionPermissions::new(BaselineMode::Suggest, Vec::<String>::new()),
        )
    }

    fn cap_autonomous(app: &str) -> Capability {
        Capability::new(
            AccessTier::Minimal,
            ActionPermissions::new(BaselineMode::Suggest, [app.to_string()]),
        )
    }

    fn body(kind: ActionKind) -> RequestBody {
        RequestBody {
            class: ConsentClass::CapabilityGrant,
            kind,
            triggered_by_external_content: false,
            recipient: None,
            preview: None,
            targets: Vec::new(),
            total: None,
            on_behalf_of: None,
            summary: "do a thing".to_string(),
            scope: Some("/x".to_string()),
        }
    }

    #[test]
    fn the_requester_is_the_attested_peer_not_the_body() {
        // The body has no requester field; the attested id is the only source.
        let mut q = ConsentQueue::new();
        let reply = handle_intake(body(ActionKind::PermanentDelete), "org.arlen.files", &cap_suggest(), &mut q);
        assert!(matches!(reply, IntakeReply::Queued { .. }));
        let front = q.front().unwrap();
        assert_eq!(front.request.requester.grant_recipient(), "org.arlen.files");
        assert_eq!(front.request.requester.display_id(), "org.arlen.files");
    }

    #[test]
    fn a_trusted_intermediary_grants_on_behalf_of_the_named_app() {
        // The portal, an allowlisted intermediary, requests for an app it
        // authenticated: the grant + the shown identity are the APP; the portal
        // is recorded only as the mediator.
        let r = resolve_requester("xdg-desktop-portal", Some("org.example.recorder"));
        assert_eq!(r.grant_recipient(), "org.example.recorder");
        assert_eq!(r.display_id(), "org.example.recorder");
        assert_eq!(r.mediator(), Some("xdg-desktop-portal"));
    }

    #[test]
    fn a_non_intermediary_cannot_redirect_a_grant() {
        // The load-bearing fail-safe: a peer NOT on the intermediary allowlist
        // that sets on_behalf_of is attributed to ITSELF, never the claimed app.
        let r = resolve_requester("com.evil.app", Some("org.arlen.files"));
        assert_eq!(r.grant_recipient(), "com.evil.app");
        assert_eq!(r.mediator(), None);
    }

    #[test]
    fn every_trusted_intermediary_is_a_reserved_identity() {
        // The trust anchor: an intermediary id must be one only a root-owned path
        // (path_to_app_id rule 1) can mint, so a same-uid process cannot squat the
        // name via a user-app directory (rule 4) and impersonate the intermediary
        // to redirect a grant. Without this the allowlist is bypassable.
        for id in TRUSTED_INTERMEDIARIES {
            assert!(
                arlen_permissions::identity::is_reserved_app_id(id),
                "{id} must be a reserved id or the intermediary allowlist is bypassable"
            );
        }
    }

    #[test]
    fn a_trusted_intermediary_without_on_behalf_is_a_direct_request() {
        let r = resolve_requester("xdg-desktop-portal", None);
        assert_eq!(r.grant_recipient(), "xdg-desktop-portal");
        assert_eq!(r.mediator(), None);
    }

    #[test]
    fn an_implausible_or_self_on_behalf_is_ignored() {
        // Empty, bad-charset (path/space injection into the grant subject), or a
        // subject equal to the peer all fall back to the attested peer.
        for bad in ["", "has space", "../etc/x", "xdg-desktop-portal"] {
            let r = resolve_requester("xdg-desktop-portal", Some(bad));
            assert_eq!(r.grant_recipient(), "xdg-desktop-portal", "subject {bad:?} must be ignored");
            assert_eq!(r.mediator(), None);
        }
    }

    #[test]
    fn a_silent_request_replies_silent_granted_and_is_not_queued() {
        // Ordinary + the caller's own app autonomous -> Silent.
        let mut q = ConsentQueue::new();
        let reply = handle_intake(body(ActionKind::Ordinary), "org.arlen.files", &cap_autonomous("org.arlen.files"), &mut q);
        assert_eq!(reply, IntakeReply::SilentGranted);
        assert!(q.is_empty());
    }

    #[test]
    fn body_round_trips_over_json_without_a_requester() {
        let b = body(ActionKind::SendExternalMessage);
        let json = serde_json::to_string(&b).unwrap();
        assert!(!json.contains("requester"), "the wire body must not carry a requester");
        let back: RequestBody = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind, ActionKind::SendExternalMessage);
        assert_eq!(back.scope.as_deref(), Some("/x"));
    }

    #[test]
    fn two_intakes_from_different_peers_keep_their_own_identities() {
        use crate::ConsentOutcome;
        let mut q = ConsentQueue::new();
        handle_intake(body(ActionKind::PermanentDelete), "app.a", &cap_suggest(), &mut q);
        handle_intake(body(ActionKind::PermanentDelete), "app.b", &cap_suggest(), &mut q);
        assert_eq!(q.len(), 2);
        // Same tier, so FIFO: app.a is front, then app.b after it resolves.
        let first = q.front().unwrap().id;
        assert_eq!(q.front().unwrap().request.requester.grant_recipient(), "app.a");
        q.resolve(first, ConsentOutcome::Denied).unwrap();
        assert_eq!(q.front().unwrap().request.requester.grant_recipient(), "app.b");
    }
}
