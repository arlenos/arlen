//! The async assembly that wires the pure cores into a running broker
//! (system-dialog-plan.md, the daemon-main slice). It holds the shared
//! [`ConsentQueue`] + the system [`Capability`] + the deferred-reply correlation
//! map, and exposes three operations the two sockets drive:
//!
//! - `intake` (requester side): classify + enqueue a request under the
//!   kernel-attested peer id; a `Silent` request grants immediately, a
//!   dialog-requiring one parks a one-shot and the requester's connection awaits
//!   the user's decision.
//! - `front_view` (shell side): the front pending request as the trusted surface
//!   should render it.
//! - `resolve` (shell side): apply the user's decision, fire the waiting
//!   requester's one-shot with the outcome, and return what to persist.
//!
//! The correlation is the novel part this module owns (the cores are
//! synchronous and id-typed); it is unit-tested here without sockets. The socket
//! transport + the `ConnectionAuth` gate live in the binary (`main.rs`).

use std::collections::HashMap;
use std::sync::Mutex;

use arlen_ai_core::capability::Capability;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::control::{front_view, resolve_decision, PendingView, ResolvedDecision};
use crate::queue::{ConsentQueue, Enqueued, RequestId};
use crate::service::RequestBody;
use crate::{assemble, AttestedRequester, ConsentOutcome};

/// Shared broker state: the pending queue, the deferred-reply waiters, and the
/// immutable system capability used to classify each request. Wrapped in an
/// `Arc` by the daemon and shared between the two socket accept loops.
pub struct SharedState {
    /// The system capability (which apps are autonomous, the baseline mode) used
    /// to classify a request's severity. Immutable for the daemon's lifetime; a
    /// config-driven reload is a later slice.
    capability: Capability,
    inner: Mutex<Inner>,
}

struct Inner {
    queue: ConsentQueue,
    /// A parked requester per pending id: resolving the request fires the sender
    /// with the user's decision, unblocking the requester's intake connection.
    waiters: HashMap<RequestId, oneshot::Sender<ConsentOutcome>>,
}

/// What an intake resolved to: either a silent grant the caller may proceed on
/// at once, or a parked request whose decision arrives on the receiver once the
/// user resolves it on the trusted surface.
pub enum IntakeOutcome {
    /// Tier-1 silent grant: no dialog, the requester proceeds immediately (the
    /// grant is still recorded by the daemon).
    SilentGranted,
    /// Queued for a dialog: await `decision` for the user's outcome.
    Pending {
        /// The broker id this request was queued under (for logging).
        id: RequestId,
        /// Fires with the user's decision once the shell resolves the request.
        decision: oneshot::Receiver<ConsentOutcome>,
    },
}

/// The wire reply the requester reads back over the intake socket: a single
/// frame carrying the final disposition (silent grant, or the user's decision).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum IntakeResult {
    /// Granted without a dialog.
    SilentGranted,
    /// The user resolved the dialog with this outcome.
    Decided {
        /// The user's decision.
        outcome: ConsentOutcome,
    },
}

/// The wire request the trusted shell sends over the control socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ControlRequest {
    /// Fetch the front pending request to render.
    Fetch,
    /// Submit the user's decision for a pending request.
    Resolve {
        /// The id the shell received in the [`PendingView`].
        id: u64,
        /// The user's decision.
        outcome: ConsentOutcome,
    },
}

/// The wire reply to a [`ControlRequest`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum ControlReply {
    /// The front pending request, or `None` when the queue is empty.
    Pending {
        /// The dialog content, or `None`.
        view: Option<PendingView>,
    },
    /// The result of a resolve: `ok` is false for an unknown / already-resolved
    /// id (the decision changed nothing).
    Resolved {
        /// Whether a pending request was found and resolved.
        ok: bool,
    },
}

impl SharedState {
    /// A fresh broker over the given system capability.
    pub fn new(capability: Capability) -> Self {
        Self {
            capability,
            inner: Mutex::new(Inner {
                queue: ConsentQueue::new(),
                waiters: HashMap::new(),
            }),
        }
    }

    /// Intake a request from a peer whose `attested_app_id` was resolved from
    /// SO_PEERCRED (never the wire body). Classifies + enqueues; a silent request
    /// returns [`IntakeOutcome::SilentGranted`], a dialog one parks a one-shot and
    /// returns its receiver. The lock is not held across any await (the receiver
    /// is returned to the caller, which awaits it after this returns).
    pub fn intake(&self, body: RequestBody, attested_app_id: &str) -> IntakeOutcome {
        let request = assemble(body, AttestedRequester::new(attested_app_id));
        let mut inner = self.inner.lock().expect("consent state mutex poisoned");
        match inner.queue.enqueue(request, &self.capability) {
            Enqueued::SilentGrant => IntakeOutcome::SilentGranted,
            Enqueued::Queued(id) => {
                let (tx, rx) = oneshot::channel();
                inner.waiters.insert(id, tx);
                IntakeOutcome::Pending { id, decision: rx }
            }
        }
    }

    /// The front pending request as the shell should render it, or `None`.
    pub fn front_view(&self) -> Option<PendingView> {
        let inner = self.inner.lock().expect("consent state mutex poisoned");
        front_view(&inner.queue)
    }

    /// Apply the shell-submitted decision for `id`: remove it from the queue,
    /// fire the parked requester's one-shot with the outcome (a dropped requester
    /// is tolerated, the decision is still recorded), and return what to persist
    /// + reply. `None` for an unknown / already-resolved id.
    pub fn resolve(&self, id: RequestId, outcome: ConsentOutcome) -> Option<ResolvedDecision> {
        let mut inner = self.inner.lock().expect("consent state mutex poisoned");
        let decision = resolve_decision(&mut inner.queue, id, outcome)?;
        if let Some(tx) = inner.waiters.remove(&id) {
            // The requester may have disconnected; the decision still stands.
            let _ = tx.send(decision.reply);
        }
        Some(decision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConsentClass;
    use arlen_ai_core::capability::{
        AccessTier, ActionKind, ActionPermissions, BaselineMode,
    };

    fn state_default() -> SharedState {
        // Suggest baseline, no autonomous apps: nothing resolves to Silent, so
        // every request needs a dialog (the conservative default; config-driven
        // autonomy is a later slice).
        SharedState::new(Capability::new(
            AccessTier::Minimal,
            ActionPermissions::new(BaselineMode::Suggest, Vec::<String>::new()),
        ))
    }

    fn state_autonomous(app: &str) -> SharedState {
        SharedState::new(Capability::new(
            AccessTier::Minimal,
            ActionPermissions::new(BaselineMode::Suggest, [app.to_string()]),
        ))
    }

    fn body(kind: ActionKind) -> RequestBody {
        RequestBody {
            class: ConsentClass::Destructive,
            kind,
            triggered_by_external_content: false,
            summary: "permanently delete 3 files".to_string(),
            scope: Some("/x".to_string()),
        }
    }

    #[tokio::test]
    async fn a_silent_request_grants_without_parking_a_waiter() {
        let state = state_autonomous("org.arlen.files");
        // Ordinary + the caller's own app autonomous -> Silent.
        let out = state.intake(
            RequestBody {
                class: ConsentClass::CapabilityGrant,
                kind: ActionKind::Ordinary,
                triggered_by_external_content: false,
                summary: "routine".to_string(),
                scope: None,
            },
            "org.arlen.files",
        );
        assert!(matches!(out, IntakeOutcome::SilentGranted));
        assert!(state.front_view().is_none(), "a silent grant is never queued");
    }

    #[tokio::test]
    async fn a_dialog_request_pends_then_resolves_to_the_users_decision() {
        let state = state_default();
        let out = state.intake(body(ActionKind::PermanentDelete), "org.arlen.files");
        let (id, decision) = match out {
            IntakeOutcome::Pending { id, decision } => (id, decision),
            IntakeOutcome::SilentGranted => panic!("a delete must require a dialog"),
        };
        // The shell sees exactly this request, attested-id == recipient.
        let view = state.front_view().expect("the request is pending");
        assert_eq!(view.id, id.get());
        assert_eq!(view.requester, "org.arlen.files");
        // The shell resolves it; the parked requester unblocks with the outcome.
        let resolved = state
            .resolve(id, ConsentOutcome::AllowedRemembered)
            .expect("the pending request resolves");
        assert_eq!(resolved.recipient, "org.arlen.files");
        assert_eq!(resolved.reply, ConsentOutcome::AllowedRemembered);
        assert!(resolved.grant.is_some(), "always-allow mints a grant");
        assert_eq!(
            decision.await.unwrap(),
            ConsentOutcome::AllowedRemembered,
            "the waiting requester receives the decision"
        );
        assert!(state.front_view().is_none(), "the resolved request leaves the queue");
    }

    #[tokio::test]
    async fn resolving_an_unknown_id_changes_nothing() {
        let state = state_default();
        assert!(
            state
                .resolve(RequestId::from_raw(9999), ConsentOutcome::Denied)
                .is_none(),
            "an id never queued resolves to nothing"
        );
    }

    #[tokio::test]
    async fn a_disconnected_requester_does_not_break_resolution() {
        let state = state_default();
        let id = match state.intake(body(ActionKind::PermanentDelete), "app.gone") {
            IntakeOutcome::Pending { id, decision } => {
                drop(decision); // the requester disconnected before the decision
                id
            }
            IntakeOutcome::SilentGranted => panic!("expected a dialog request"),
        };
        let resolved = state.resolve(id, ConsentOutcome::Denied);
        assert!(
            resolved.is_some(),
            "the decision is still recorded even though the requester is gone"
        );
    }

    #[tokio::test]
    async fn high_stakes_is_shown_before_an_earlier_standard() {
        let state = state_default();
        // A Standard request first (Ordinary + suggest baseline), then a
        // HighStakes (delete): the delete is front despite arriving second.
        let standard = match state.intake(
            RequestBody {
                class: ConsentClass::CapabilityGrant,
                kind: ActionKind::Ordinary,
                triggered_by_external_content: false,
                summary: "routine".to_string(),
                scope: None,
            },
            "app.a",
        ) {
            IntakeOutcome::Pending { id, .. } => id,
            IntakeOutcome::SilentGranted => panic!("suggest baseline must prompt"),
        };
        let high = match state.intake(body(ActionKind::PermanentDelete), "app.b") {
            IntakeOutcome::Pending { id, .. } => id,
            IntakeOutcome::SilentGranted => panic!("delete must prompt"),
        };
        assert_eq!(state.front_view().unwrap().id, high.get());
        state.resolve(high, ConsentOutcome::Denied).unwrap();
        assert_eq!(state.front_view().unwrap().id, standard.get());
    }
}
