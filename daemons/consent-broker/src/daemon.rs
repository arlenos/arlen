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
use crate::{assemble, classify, AttestedRequester, ConsentOutcome, ConsentRequest, SeverityTier};

/// The broker's in-memory store of remembered ("always allow") grants. A grant
/// covers a future request with the same (recipient, class, scope), so a
/// repeated Standard prompt can be silently granted. It is consulted ONLY to
/// downgrade a Standard request to Silent: a HighStakes or externally-triggered
/// request always re-prompts regardless of any grant (the non-configurable
/// confirms are never bypassed). Process-lived for now; the durable KG grant
/// node + the revoke surface are a later slice.
#[derive(Debug, Default)]
struct GrantStore {
    grants: Vec<ConsentGrant>,
}

impl GrantStore {
    /// Record a remembered grant, deduped on its (recipient, class, scope)
    /// identity (the revocation_handle is deterministic over that key, so
    /// re-consenting the same scope does not duplicate it).
    fn record(&mut self, grant: ConsentGrant) {
        if !self
            .grants
            .iter()
            .any(|g| g.revocation_handle == grant.revocation_handle)
        {
            self.grants.push(grant);
        }
    }

    /// Whether a live grant covers this request: same attested recipient, same
    /// class, same concrete scope.
    fn covers(&self, request: &ConsentRequest) -> bool {
        let recipient = request.requester.grant_recipient();
        self.grants.iter().any(|g| {
            g.recipient == recipient && g.class == request.class && g.scope == request.scope
        })
    }

    /// Every remembered grant, for the shell's "what you allowed" surface.
    fn list(&self) -> Vec<ConsentGrant> {
        self.grants.clone()
    }

    /// Forget the grant with this revocation handle, returning it (for the audit
    /// record). `None` = unknown / already revoked.
    fn revoke(&mut self, handle: &str) -> Option<ConsentGrant> {
        let pos = self
            .grants
            .iter()
            .position(|g| g.revocation_handle == handle)?;
        Some(self.grants.remove(pos))
    }
}

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
    /// Remembered ("always allow") grants, consulted to downgrade a repeated
    /// Standard request to Silent.
    grants: GrantStore,
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
    /// List the remembered grants (the "what you allowed" surface).
    ListGrants,
    /// Revoke a remembered grant by its revocation handle.
    RevokeGrant {
        /// The grant's revocation handle (from a [`ConsentGrant`]).
        handle: String,
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
    /// The remembered grants, for the "what you allowed" surface.
    Grants {
        /// Every live remembered grant.
        grants: Vec<ConsentGrant>,
    },
    /// The result of a revoke: `ok` is false for an unknown / already-revoked
    /// handle.
    Revoked {
        /// Whether a grant was found and revoked.
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

/// The content-free audit entry for a revoked grant: the recipient whose
/// standing permission was withdrawn + the fixed `revoked` disposition.
fn consent_revoke_entry(recipient: &str) -> IngestRequest {
    IngestRequest {
        kind: AuditKind::Permission,
        structural: StructuralRecord {
            subject: recipient.to_string(),
            node_types: Vec::new(),
            relations: Vec::new(),
            result_count: None,
            duration_ms: None,
            outcome: "revoked".to_string(),
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
                grants: GrantStore::default(),
            }),
        }
    }

    /// Intake a request from a peer whose `attested_app_id` was resolved from
    /// SO_PEERCRED (never the wire body). Classifies, consults remembered grants,
    /// and enqueues; a silent request returns [`IntakeOutcome::SilentGranted`], a
    /// dialog one parks a one-shot and returns its receiver. The lock is not held
    /// across any await (the receiver is returned to the caller, which awaits it
    /// after this returns).
    ///
    /// Grant consultation may ONLY downgrade a Standard request to Silent on an
    /// exact (recipient, class, scope) match: a HighStakes or externally-triggered
    /// request always re-prompts regardless of any remembered grant, so a grant
    /// can never silently skip a confirmation the high-impact / injection-
    /// containment rules require.
    pub fn intake(&self, body: RequestBody, attested_app_id: &str) -> IntakeOutcome {
        let request = assemble(body, AttestedRequester::new(attested_app_id));
        let mut inner = self.inner.lock().expect("consent state mutex poisoned");

        // A remembered grant only covers a Standard, non-externally-triggered
        // request. HighStakes classifies away from Standard (external trigger maps
        // to HighStakes too), so those never reach this downgrade; the explicit
        // external guard is belt-and-suspenders.
        let tier = classify(&request, &self.capability);
        if matches!(tier, SeverityTier::Standard)
            && !request.triggered_by_external_content
            && inner.grants.covers(&request)
        {
            return IntakeOutcome::SilentGranted;
        }

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
        // An audited always-allow is remembered so a future matching Standard
        // request is silently granted (consultation). A failed audit records
        // nothing (the request was failed closed). Re-lock briefly after the
        // await; the grant-record holds no lock across an await.
        let grant = if audited { decision.grant } else { None };
        if let Some(g) = &grant {
            self.inner
                .lock()
                .expect("consent state mutex poisoned")
                .grants
                .record(g.clone());
        }
        if let Some(tx) = tx {
            let _ = tx.send(reply);
        }
        ResolveResult::Resolved {
            audited,
            reply,
            grant,
        }
    }

    /// Every remembered grant, for the shell's "what you allowed" surface.
    pub fn list_grants(&self) -> Vec<ConsentGrant> {
        self.inner
            .lock()
            .expect("consent state mutex poisoned")
            .grants
            .list()
    }

    /// Revoke a remembered grant by its handle, returning whether one was
    /// removed. The removal is unconditional (revoking is always the safe
    /// direction); the audit record is best-effort (unlike a grant release, a
    /// revoke that cannot be audited still proceeds, since failing to forget a
    /// grant would be the unsafe outcome). Audits only an actual removal.
    pub async fn revoke_grant(&self, handle: &str) -> bool {
        let removed = self
            .inner
            .lock()
            .expect("consent state mutex poisoned")
            .grants
            .revoke(handle);
        match removed {
            Some(grant) => {
                let _ = self.audit.submit(consent_revoke_entry(&grant.recipient)).await;
                true
            }
            None => false,
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

    /// An Ordinary request (under the Suggest baseline this classifies Standard).
    fn standard_body(scope: Option<&str>) -> RequestBody {
        RequestBody {
            class: ConsentClass::CapabilityGrant,
            kind: ActionKind::Ordinary,
            triggered_by_external_content: false,
            summary: "use a capability".to_string(),
            scope: scope.map(str::to_string),
        }
    }

    /// Drive a request to an AllowedRemembered resolution, recording its grant.
    async fn remember(state: &SharedState, b: RequestBody, app: &str) {
        let id = match state.intake(b, app) {
            IntakeOutcome::Pending { id, .. } => id,
            IntakeOutcome::SilentGranted => panic!("the request should have prompted"),
        };
        let r = state.resolve(id, ConsentOutcome::AllowedRemembered).await;
        assert!(
            matches!(r, ResolveResult::Resolved { audited: true, .. }),
            "the always-allow must be audited+recorded"
        );
    }

    #[tokio::test]
    async fn a_remembered_standard_grant_makes_a_matching_request_silent() {
        let state = state_default();
        remember(&state, standard_body(Some("/x")), "app.s").await;
        // A second identical Standard request is now covered -> no dialog.
        assert!(
            matches!(
                state.intake(standard_body(Some("/x")), "app.s"),
                IntakeOutcome::SilentGranted
            ),
            "a matching Standard request is covered by the remembered grant"
        );
    }

    #[tokio::test]
    async fn a_high_stakes_request_always_prompts_even_with_a_matching_grant() {
        // The security invariant: a remembered grant NEVER downgrades a HighStakes
        // confirm. Record a grant for (app, Destructive, /x) via a delete, then a
        // second identical delete must still prompt.
        let state = state_default();
        remember(&state, body(ActionKind::PermanentDelete), "app.h").await;
        assert!(
            matches!(
                state.intake(body(ActionKind::PermanentDelete), "app.h"),
                IntakeOutcome::Pending { .. }
            ),
            "a HighStakes request always re-prompts regardless of a remembered grant"
        );
    }

    #[tokio::test]
    async fn an_external_request_always_prompts_even_with_a_matching_grant() {
        // A remembered Standard grant must not cover the same request once it is
        // externally triggered (injection containment): external maps to HighStakes.
        let state = state_default();
        remember(&state, standard_body(Some("/x")), "app.e").await;
        let mut ext = standard_body(Some("/x"));
        ext.triggered_by_external_content = true;
        assert!(
            matches!(state.intake(ext, "app.e"), IntakeOutcome::Pending { .. }),
            "an externally-triggered request always re-prompts even with a matching grant"
        );
    }

    #[tokio::test]
    async fn a_grant_is_scoped_a_different_scope_still_prompts() {
        let state = state_default();
        remember(&state, standard_body(Some("/x")), "app.d").await;
        assert!(
            matches!(
                state.intake(standard_body(Some("/y")), "app.d"),
                IntakeOutcome::Pending { .. }
            ),
            "a grant covers only its own scope; a different scope still prompts"
        );
    }

    #[tokio::test]
    async fn allow_once_remembers_nothing() {
        // Only AllowedRemembered records a grant; AllowedOnce leaves the store
        // empty, so the next identical request still prompts.
        let state = state_default();
        let id = match state.intake(standard_body(Some("/x")), "app.o") {
            IntakeOutcome::Pending { id, .. } => id,
            IntakeOutcome::SilentGranted => panic!("expected a prompt"),
        };
        let _ = state.resolve(id, ConsentOutcome::AllowedOnce).await;
        assert!(
            matches!(
                state.intake(standard_body(Some("/x")), "app.o"),
                IntakeOutcome::Pending { .. }
            ),
            "allow-once remembers nothing"
        );
    }

    #[tokio::test]
    async fn list_grants_surfaces_each_remembered_grant() {
        let state = state_default();
        remember(&state, standard_body(Some("/x")), "app.a").await;
        remember(&state, standard_body(Some("/y")), "app.a").await;
        let grants = state.list_grants();
        assert_eq!(grants.len(), 2, "both remembered grants are listed");
        assert!(grants.iter().all(|g| g.recipient == "app.a"));
    }

    #[tokio::test]
    async fn revoking_a_grant_makes_the_request_prompt_again() {
        let state = state_default();
        remember(&state, standard_body(Some("/x")), "app.r").await;
        // Covered now.
        assert!(matches!(
            state.intake(standard_body(Some("/x")), "app.r"),
            IntakeOutcome::SilentGranted
        ));
        let handle = state.list_grants()[0].revocation_handle.clone();
        assert!(state.revoke_grant(&handle).await, "the grant is revoked");
        assert!(state.list_grants().is_empty(), "the store no longer holds it");
        // No longer covered -> prompts again.
        assert!(
            matches!(
                state.intake(standard_body(Some("/x")), "app.r"),
                IntakeOutcome::Pending { .. }
            ),
            "a revoked grant no longer silences the prompt"
        );
    }

    #[tokio::test]
    async fn revoking_an_unknown_handle_is_false() {
        let state = state_default();
        assert!(
            !state.revoke_grant("no-such-handle").await,
            "an unknown handle revokes nothing"
        );
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
