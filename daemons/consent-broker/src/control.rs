//! The decision-return channel: the trusted shell fetches the front pending
//! request, renders it on the compositor-owned consent surface (the approved
//! `arlen-shell-overlay` `consent_*` class), and submits the user's decision;
//! the broker resolves the queue, mints/records the grant, and replies to the
//! waiting requester (system-dialog-plan.md piece 4, approved 21 June).
//!
//! This is the pure decision logic + the dialog-content projection: what the
//! shell may SEE (never internal queue bookkeeping) and how a submitted decision
//! becomes a grant + a requester reply. The async two-socket daemon (requester
//! intake + shell control, with the pending-reply correlation) wires these; the
//! correlation + the SO_PEERCRED/shell-trust gate are the daemon-main slice.

use serde::{Deserialize, Serialize};

use crate::grant::{mint_grant, ConsentGrant};
use crate::queue::{ConsentQueue, PendingRequest, RequestId};
use crate::{ConsentClass, ConsentOutcome, Reversibility, SeverityTier};

/// The dialog content the shell renders for one pending request - exactly what
/// the user must see to decide, and nothing internal (no seq / queue state). The
/// `requester` is the attested identity (shown == grant recipient).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingView {
    /// The id to submit the decision against.
    pub id: u64,
    /// The attested requester shown to the user (== the grant recipient).
    pub requester: String,
    /// The request class - selects the polymorphic dialog.
    pub class: ConsentClass,
    /// The severity tier - selects the visual treatment (Standard vs the
    /// high-stakes confirm-delay / re-type).
    pub tier: SeverityTier,
    /// The plain-language risk/outcome summary.
    pub summary: String,
    /// The concrete scope/target, when there is one.
    pub scope: Option<String>,
    /// Whether the action can be undone - drives the footer/tone; without it every
    /// request degrades to a single tone.
    pub reversibility: Reversibility,
}

impl PendingView {
    /// Project a queued request into the shell-visible dialog content.
    pub fn of(pending: &PendingRequest) -> Self {
        PendingView {
            id: pending.id.get(),
            requester: pending.request.requester.display_id().to_string(),
            class: pending.request.class,
            tier: pending.tier,
            summary: pending.request.summary.clone(),
            scope: pending.request.scope.clone(),
            reversibility: Reversibility::of(pending.request.kind),
        }
    }
}

/// The front pending request as the shell should render it, or `None` when
/// nothing is pending. The shell renders this on the trusted surface; resolving
/// it (via [`resolve_decision`]) surfaces the next.
pub fn front_view(queue: &ConsentQueue) -> Option<PendingView> {
    queue.front().map(PendingView::of)
}

/// The outcome of applying a user's decision to a pending request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDecision {
    /// The attested recipient the decision concerns (for the reply correlation +
    /// the audit subject).
    pub recipient: String,
    /// The reply to send the waiting requester.
    pub reply: ConsentOutcome,
    /// The grant to persist (KG + audit), `Some` only for an always-allow.
    pub grant: Option<ConsentGrant>,
}

/// Apply the shell-submitted decision for request `id`: remove it from the
/// queue, mint a grant for an always-allow, and return what to reply to the
/// requester + what to persist. `None` when the id is unknown (already resolved,
/// or never queued). The daemon then sends `reply` to the waiting requester,
/// persists `grant` (KG + audit), and audits the decision.
pub fn resolve_decision(
    queue: &mut ConsentQueue,
    id: RequestId,
    outcome: ConsentOutcome,
) -> Option<ResolvedDecision> {
    let (pending, outcome) = queue.resolve(id, outcome)?;
    let grant = mint_grant(&pending, outcome);
    Some(ResolvedDecision {
        recipient: pending.request.requester.grant_recipient().to_string(),
        reply: outcome,
        grant,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::Enqueued;
    use crate::{AttestedRequester, ConsentRequest};
    use arlen_ai_core::capability::{AccessTier, ActionKind, ActionPermissions, BaselineMode, Capability};

    fn cap_suggest() -> Capability {
        Capability::new(
            AccessTier::Minimal,
            ActionPermissions::new(BaselineMode::Suggest, Vec::<String>::new()),
        )
    }

    fn enqueue(q: &mut ConsentQueue, app: &str, kind: ActionKind, scope: Option<&str>) -> RequestId {
        let req = ConsentRequest {
            requester: AttestedRequester::new(app),
            class: ConsentClass::Destructive,
            kind,
            triggered_by_external_content: false,
            summary: "permanently delete 3 files".to_string(),
            scope: scope.map(str::to_string),
        };
        match q.enqueue(req, &cap_suggest()) {
            Enqueued::Queued(id) => id,
            Enqueued::SilentGrant => panic!("expected a dialog-requiring request"),
        }
    }

    #[test]
    fn front_view_shows_the_attested_requester_and_tier_not_internals() {
        let mut q = ConsentQueue::new();
        enqueue(&mut q, "org.arlen.files", ActionKind::PermanentDelete, Some("/x"));
        let v = front_view(&q).unwrap();
        assert_eq!(v.requester, "org.arlen.files");
        assert_eq!(v.class, ConsentClass::Destructive);
        assert_eq!(v.tier, SeverityTier::HighStakes); // PermanentDelete
        assert_eq!(v.summary, "permanently delete 3 files");
        assert_eq!(v.scope.as_deref(), Some("/x"));
        assert_eq!(v.reversibility, Reversibility::Irreversible); // PermanentDelete has no undo
    }

    #[test]
    fn front_view_reversibility_is_derived_from_the_kind() {
        let mut q = ConsentQueue::new();
        enqueue(&mut q, "org.arlen.mail", ActionKind::SendExternalMessage, None);
        assert_eq!(front_view(&q).unwrap().reversibility, Reversibility::Irreversible);
        let mut q2 = ConsentQueue::new();
        enqueue(&mut q2, "org.arlen.installd", ActionKind::ElevatedPrivilege, None);
        assert_eq!(front_view(&q2).unwrap().reversibility, Reversibility::ReversibleWithCost);
    }

    #[test]
    fn front_view_is_none_when_empty() {
        assert!(front_view(&ConsentQueue::new()).is_none());
    }

    #[test]
    fn always_allow_resolves_to_a_grant_and_removes_from_queue() {
        let mut q = ConsentQueue::new();
        let id = enqueue(&mut q, "org.arlen.files", ActionKind::PermanentDelete, Some("/x"));
        let d = resolve_decision(&mut q, id, ConsentOutcome::AllowedRemembered).unwrap();
        assert_eq!(d.recipient, "org.arlen.files");
        assert_eq!(d.reply, ConsentOutcome::AllowedRemembered);
        let grant = d.grant.expect("always-allow mints a grant");
        assert_eq!(grant.recipient, "org.arlen.files");
        assert!(q.is_empty(), "the resolved request leaves the queue");
    }

    #[test]
    fn deny_resolves_without_a_grant() {
        let mut q = ConsentQueue::new();
        let id = enqueue(&mut q, "app", ActionKind::PermanentDelete, None);
        let d = resolve_decision(&mut q, id, ConsentOutcome::Denied).unwrap();
        assert_eq!(d.reply, ConsentOutcome::Denied);
        assert!(d.grant.is_none(), "a denial mints nothing");
    }

    #[test]
    fn allow_once_resolves_without_a_grant() {
        let mut q = ConsentQueue::new();
        let id = enqueue(&mut q, "app", ActionKind::PermanentDelete, None);
        let d = resolve_decision(&mut q, id, ConsentOutcome::AllowedOnce).unwrap();
        assert_eq!(d.reply, ConsentOutcome::AllowedOnce);
        assert!(d.grant.is_none(), "allow-once records no grant");
    }

    #[test]
    fn an_unknown_id_resolves_to_none() {
        let mut q = ConsentQueue::new();
        let id = enqueue(&mut q, "app", ActionKind::PermanentDelete, None);
        resolve_decision(&mut q, id, ConsentOutcome::Denied).unwrap();
        // Already resolved.
        assert!(resolve_decision(&mut q, id, ConsentOutcome::Denied).is_none());
    }
}
