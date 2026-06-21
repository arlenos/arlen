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
use std::sync::{Arc, Mutex};

use arlen_ai_core::capability::Capability;
use audit_proto::{AuditKind, AuditSink, IngestRequest, StructuralRecord};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::control::{front_view, resolve_decision, PendingView, ResolvedDecision};
use crate::grant::ConsentGrant;
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
    /// The audit ledger sink: every resolved decision is recorded before the
    /// requester may act on it (S13 audit-before-act, fail-closed).
    audit: Arc<dyn AuditSink>,
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

/// The disposition of a [`SharedState::resolve`] call.
#[derive(Debug)]
pub enum ResolveResult {
    /// The id was not pending (unknown / already resolved); nothing changed.
    Unknown,
    /// The request resolved. `audited` is false when the audit write failed, in
    /// which case the requester was failed closed to [`ConsentOutcome::Denied`]
    /// regardless of the user's choice (S13: no audit, no action) and no grant is
    /// returned.
    Resolved {
        /// Whether the decision was successfully recorded in the audit ledger.
        audited: bool,
        /// The outcome actually delivered to the requester.
        reply: ConsentOutcome,
        /// The grant to persist into the KG (the audit half is already done);
        /// `Some` only for an audited always-allow.
        grant: Option<ConsentGrant>,
    },
}

/// Build the content-free audit entry for a resolved decision: the acting
/// principal (the attested recipient) + the coarse disposition only, never the
/// action's summary or scope (S13 keeps the always-recorded tier content-free).
fn consent_decision_entry(decision: &ResolvedDecision) -> IngestRequest {
    let outcome = match decision.reply {
        ConsentOutcome::AllowedOnce => "granted-once",
        ConsentOutcome::AllowedRemembered => "granted-remembered",
        ConsentOutcome::Denied => "denied",
    };
    IngestRequest {
        kind: AuditKind::Permission,
        structural: StructuralRecord {
            subject: decision.recipient.clone(),
            node_types: Vec::new(),
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

impl SharedState {
    /// A fresh broker over the given system capability + audit sink.
    pub fn new(capability: Capability, audit: Arc<dyn AuditSink>) -> Self {
        Self {
            capability,
            audit,
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
    /// audit the decision (S13 audit-before-act), then fire the parked
    /// requester's one-shot with the outcome. The audit happens BEFORE the
    /// requester is told the result, and a failed audit fails the request closed
    /// to [`ConsentOutcome::Denied`] (no audit, no action) regardless of the
    /// user's choice. A disconnected requester is tolerated (the decision is
    /// still recorded). The queue lock is taken only for the synchronous removal
    /// and is released before the audit await.
    pub async fn resolve(&self, id: RequestId, outcome: ConsentOutcome) -> ResolveResult {
        // Step 1: remove from the queue + take the parked sender under the lock,
        // with no await held (the guard is dropped at the end of this block).
        let taken = {
            let mut inner = self.inner.lock().expect("consent state mutex poisoned");
            match resolve_decision(&mut inner.queue, id, outcome) {
                Some(decision) => Some((decision, inner.waiters.remove(&id))),
                None => None,
            }
        };
        let (decision, tx) = match taken {
            Some(t) => t,
            None => return ResolveResult::Unknown,
        };

        // Step 2: audit-before-act. Record the decision before the requester is
        // told; a failed audit fails closed to a denial.
        let audited = self
            .audit
            .submit(consent_decision_entry(&decision))
            .await
            .is_ok();
        let reply = if audited {
            decision.reply
        } else {
            ConsentOutcome::Denied
        };
        if let Some(tx) = tx {
            let _ = tx.send(reply);
        }
        ResolveResult::Resolved {
            audited,
            reply,
            grant: if audited { decision.grant } else { None },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConsentClass;
    use arlen_ai_core::capability::{
        AccessTier, ActionKind, ActionPermissions, BaselineMode,
    };
    use audit_proto::sink::MockAuditSink;

    fn cap_default() -> Capability {
        // Suggest baseline, no autonomous apps: nothing resolves to Silent, so
        // every request needs a dialog (the conservative default; config-driven
        // autonomy is a later slice).
        Capability::new(
            AccessTier::Minimal,
            ActionPermissions::new(BaselineMode::Suggest, Vec::<String>::new()),
        )
    }

    fn state_default() -> SharedState {
        SharedState::new(cap_default(), Arc::new(MockAuditSink::accepting()))
    }

    fn state_autonomous(app: &str) -> SharedState {
        SharedState::new(
            Capability::new(
                AccessTier::Minimal,
                ActionPermissions::new(BaselineMode::Suggest, [app.to_string()]),
            ),
            Arc::new(MockAuditSink::accepting()),
        )
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
        // The shell resolves it; audited, and the parked requester unblocks.
        let resolved = state.resolve(id, ConsentOutcome::AllowedRemembered).await;
        match resolved {
            ResolveResult::Resolved { audited, reply, grant } => {
                assert!(audited, "the accepting sink records the decision");
                assert_eq!(reply, ConsentOutcome::AllowedRemembered);
                assert!(grant.is_some(), "an audited always-allow yields a grant");
            }
            ResolveResult::Unknown => panic!("the pending request resolves"),
        }
        assert_eq!(
            decision.await.unwrap(),
            ConsentOutcome::AllowedRemembered,
            "the waiting requester receives the decision"
        );
        assert!(state.front_view().is_none(), "the resolved request leaves the queue");
    }

    #[tokio::test]
    async fn a_failed_audit_fails_the_decision_closed() {
        // S13: a decision that cannot be audited must not release the grant. The
        // user's always-allow is downgraded to a denial and no grant is returned.
        let state = SharedState::new(cap_default(), Arc::new(MockAuditSink::failing()));
        let (id, decision) = match state.intake(body(ActionKind::PermanentDelete), "app.x") {
            IntakeOutcome::Pending { id, decision } => (id, decision),
            IntakeOutcome::SilentGranted => panic!("expected a dialog request"),
        };
        match state.resolve(id, ConsentOutcome::AllowedRemembered).await {
            ResolveResult::Resolved { audited, reply, grant } => {
                assert!(!audited, "the failing sink reports the audit did not land");
                assert_eq!(reply, ConsentOutcome::Denied, "fail-closed to a denial");
                assert!(grant.is_none(), "no grant is minted when the audit fails");
            }
            ResolveResult::Unknown => panic!("the request was pending"),
        }
        assert_eq!(
            decision.await.unwrap(),
            ConsentOutcome::Denied,
            "the requester is told the fail-closed denial, not its requested allow"
        );
    }

    #[tokio::test]
    async fn resolving_an_unknown_id_changes_nothing() {
        let state = state_default();
        assert!(
            matches!(
                state
                    .resolve(RequestId::from_raw(9999), ConsentOutcome::Denied)
                    .await,
                ResolveResult::Unknown
            ),
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
        assert!(
            matches!(
                state.resolve(id, ConsentOutcome::Denied).await,
                ResolveResult::Resolved { .. }
            ),
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
        let _ = state.resolve(high, ConsentOutcome::Denied).await;
        assert_eq!(state.front_view().unwrap().id, standard.get());
    }
}
