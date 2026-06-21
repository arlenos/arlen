//! The intake-dispatch core: a client's wire request + the kernel-attested peer
//! identity become a queued (or silently-granted) consent request
//! (system-dialog-plan.md). The LOAD-BEARING rule lives here: the requester is
//! built from the SO_PEERCRED-attested app id, NEVER from the request payload -
//! the wire [`RequestBody`] structurally carries no requester field, so a
//! client cannot ask on another app's behalf (the macOS TCC CVE-2025-31250
//! spoof is unrepresentable). The socket accept loop that resolves the peer
//! (via `arlen_permissions` `ConnectionAuth` -> `path_to_app_id`) and frames
//! these is the daemon shell on top; this dispatch is pure and unit-tested.

use arlen_ai_core::capability::{ActionKind, Capability};
use serde::{Deserialize, Serialize};

use crate::queue::{ConsentQueue, Enqueued, RequestId};
use crate::{AttestedRequester, ConsentClass, ConsentRequest};

/// The wire request a client sends to the broker. It carries the action's
/// class, impact and scope - but NOT the requester: the broker fills that from
/// the attested peer, so identity cannot be spoofed over the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestBody {
    /// The request class (selects the rendered dialog).
    pub class: ConsentClass,
    /// The impact kind (drives the severity classification).
    pub kind: ActionKind,
    /// Whether this was triggered by external / untrusted content.
    #[serde(default)]
    pub triggered_by_external_content: bool,
    /// The plain-language risk/outcome summary.
    pub summary: String,
    /// The concrete scope / target, when there is one.
    #[serde(default)]
    pub scope: Option<String>,
}

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
    let request = assemble(body, AttestedRequester::new(attested_app_id));
    match queue.enqueue(request, capability) {
        Enqueued::Queued(id) => IntakeReply::Queued { id: RequestId::get(id) },
        Enqueued::SilentGrant => IntakeReply::SilentGranted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_ai_core::capability::{AccessTier, ActionPermissions, BaselineMode};

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
