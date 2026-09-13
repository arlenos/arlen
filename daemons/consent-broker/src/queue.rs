//! The broker's multi-request queue (system-dialog-plan.md, piece "multi-request
//! queueing + priority"). The trusted surface shows ONE dialog at a time, so
//! concurrent consent requests (today's Bluetooth-pairing collision is the
//! motivating case) must serialize. This is the pure ordering core: classify
//! each request, drop the [`SeverityTier::Silent`] ones (no dialog - the caller
//! silent-grants them), and present the rest highest-severity-first, FIFO within
//! a tier. The IPC socket that feeds it and the surface that renders the front
//! are later pieces; this orders requests deterministically and is unit-tested.

use arlen_ai_core::capability::Capability;

use crate::{classify, ConsentOutcome, ConsentRequest, SeverityTier};

/// A broker-assigned identifier for one pending request. Monotonic per queue,
/// so a resolved id is never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(u64);

impl RequestId {
    /// The raw counter value (for logging / wire correlation).
    pub fn get(self) -> u64 {
        self.0
    }

    /// Reconstruct an id from its raw wire value. The shell receives the raw id
    /// in a [`crate::control::PendingView`] and submits the decision against it;
    /// the daemon turns that u64 back into a `RequestId` to resolve. The id space
    /// is the broker's own, so a forged or stale value simply resolves to nothing
    /// (`resolve` returns `None`), never another request.
    pub fn from_raw(raw: u64) -> Self {
        RequestId(raw)
    }
}

/// A request awaiting a decision: the original request, its classified tier, and
/// the queue id + insertion order used to pick the one shown next.
#[derive(Debug, Clone)]
pub struct PendingRequest {
    /// The broker-assigned id (used to resolve this request later).
    pub id: RequestId,
    /// The request as raised by the daemon/app.
    pub request: ConsentRequest,
    /// The severity tier resolved at enqueue time (never `Silent` - those are
    /// not queued).
    pub tier: SeverityTier,
    /// Insertion order, the FIFO tiebreak within a tier.
    seq: u64,
}

/// Severity ordering for the queue: a high-stakes confirmation is shown before a
/// routine prompt (a security-critical decision must not sit behind an everyday
/// one), and `Silent` never reaches the queue.
fn tier_rank(tier: SeverityTier) -> u8 {
    match tier {
        SeverityTier::Silent => 0,
        SeverityTier::Standard => 1,
        SeverityTier::HighStakes => 2,
    }
}

/// The pending-consent queue. The front (highest tier, then earliest enqueued)
/// is the request the trusted surface renders; resolving it surfaces the next.
#[derive(Debug, Default)]
pub struct ConsentQueue {
    pending: Vec<PendingRequest>,
    next_id: u64,
    next_seq: u64,
}

/// The result of enqueuing a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enqueued {
    /// The request needs a dialog and was queued under this id.
    Queued(RequestId),
    /// The same question from the same requester is ALREADY waiting for an
    /// answer, so nothing was added: this caller joins the pending one under its
    /// id and gets the same decision when it resolves.
    ///
    /// Measured before it was built: three restarts of one daemon that asks at
    /// startup put three identical prompts in the queue, ids 0, 1 and 2. A person
    /// would have dismissed the same question three times, and a different,
    /// genuine prompt would have been buried behind them. A requester cannot know
    /// about other instances of itself, so only the queue can see this.
    ///
    /// It JOINS rather than replaces: the earlier caller may still be waiting,
    /// and dropping its waiter would leave it hanging on a question that was
    /// answered.
    AlreadyPending(RequestId),
    /// The request resolved to [`SeverityTier::Silent`]: no dialog is shown, the
    /// caller silently grants it (still recording the grant). Not queued.
    SilentGrant,
}

/// Whether two requests are the same question: same requester, same class, same
/// scope.
///
/// The same triple the grant store keys a revocation handle on, and for the same
/// reason - it is what makes one authorisation answer one question. Deliberately
/// NOT the summary: two callers wording the same request differently are still
/// asking one thing, and the wording is the dialog's to show rather than the
/// queue's to compare.
///
/// An externally-triggered request never collapses into another. Two of them may
/// look alike and come from different content, and the containment rule treats
/// each as its own decision.
fn same_question(a: &ConsentRequest, b: &ConsentRequest) -> bool {
    !a.triggered_by_external_content
        && !b.triggered_by_external_content
        && a.requester.display_id() == b.requester.display_id()
        && a.class == b.class
        && a.scope == b.scope
}

impl ConsentQueue {
    /// A fresh, empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Classify and enqueue a request. A `Silent` request is NOT queued -
    /// [`Enqueued::SilentGrant`] tells the caller to grant it without a dialog.
    /// Any dialog-requiring request is queued and its id returned.
    pub fn enqueue(&mut self, request: ConsentRequest, capability: &Capability) -> Enqueued {
        let tier = classify(&request, capability);
        if tier == SeverityTier::Silent {
            return Enqueued::SilentGrant;
        }
        if let Some(pending) = self.pending.iter().find(|p| same_question(&p.request, &request)) {
            return Enqueued::AlreadyPending(pending.id);
        }
        let id = RequestId(self.next_id);
        self.next_id += 1;
        let seq = self.next_seq;
        self.next_seq += 1;
        self.pending.push(PendingRequest {
            id,
            request,
            tier,
            seq,
        });
        Enqueued::Queued(id)
    }

    /// The request to show now: highest tier, earliest-enqueued within a tier.
    /// `None` when nothing is pending.
    pub fn front(&self) -> Option<&PendingRequest> {
        self.pending.iter().max_by(|a, b| {
            tier_rank(a.tier)
                .cmp(&tier_rank(b.tier))
                .then(b.seq.cmp(&a.seq)) // earlier seq wins, so reverse for max
        })
    }

    /// Resolve a pending request by id with the user's decision, removing it
    /// from the queue. Returns the removed request paired with the outcome (the
    /// caller then mints/records the grant and replies to the requester), or
    /// `None` if the id is unknown (already resolved / never queued).
    pub fn resolve(
        &mut self,
        id: RequestId,
        outcome: ConsentOutcome,
    ) -> Option<(PendingRequest, ConsentOutcome)> {
        let pos = self.pending.iter().position(|p| p.id == id)?;
        let removed = self.pending.remove(pos);
        Some((removed, outcome))
    }

    /// How many requests are awaiting a decision.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether nothing is pending.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AttestedRequester, ConsentClass};
    use arlen_ai_core::capability::{AccessTier, ActionKind, ActionPermissions, BaselineMode};

    fn req(app: &str, kind: ActionKind, external: bool) -> ConsentRequest {
        ConsentRequest {
            requester: AttestedRequester::new(app),
            class: ConsentClass::CapabilityGrant,
            kind,
            triggered_by_external_content: external,
            recipient: None,
            preview: None,
            targets: Vec::new(),
            total: None,
            summary: "t".to_string(),
            scope: None,
        }
    }

    /// A dialog-requiring request with a scope, which is what the collapse keys on.
    fn scoped(app: &str, scope: &str) -> ConsentRequest {
        let mut r = req(app, ActionKind::Ordinary, false);
        r.scope = Some(scope.to_string());
        r
    }

    fn cap_autonomous(app: &str) -> Capability {
        Capability::new(
            AccessTier::Minimal,
            ActionPermissions::new(BaselineMode::Suggest, [app.to_string()]),
        )
    }

    fn cap_suggest() -> Capability {
        Capability::new(
            AccessTier::Minimal,
            ActionPermissions::new(BaselineMode::Suggest, Vec::<String>::new()),
        )
    }

    /// Measured before this existed: a daemon that asks at startup, restarted
    /// three times, left three identical prompts in the queue. One question, one
    /// dialog.
    #[test]
    fn the_same_question_asked_twice_is_one_dialog() {
        let mut q = ConsentQueue::new();
        let first = q.enqueue(scoped("sentineld", "location"), &cap_suggest());
        let second = q.enqueue(scoped("sentineld", "location"), &cap_suggest());
        let Enqueued::Queued(id) = first else { panic!("the first one queues") };
        assert!(matches!(second, Enqueued::AlreadyPending(joined) if joined == id));
        assert_eq!(q.len(), 1, "only one prompt is in front of anybody");
    }

    /// A different scope, or a different requester, is a different question.
    #[test]
    fn a_different_question_is_its_own_dialog() {
        let mut q = ConsentQueue::new();
        q.enqueue(scoped("sentineld", "location"), &cap_suggest());
        assert!(matches!(
            q.enqueue(scoped("sentineld", "microphone"), &cap_suggest()),
            Enqueued::Queued(_)
        ));
        assert!(matches!(
            q.enqueue(scoped("other", "location"), &cap_suggest()),
            Enqueued::Queued(_)
        ));
        assert_eq!(q.len(), 3);
    }

    /// Two externally-triggered requests never collapse: they may look alike and
    /// come from different content, and each is its own decision.
    #[test]
    fn an_externally_triggered_request_never_joins_another() {
        let mut q = ConsentQueue::new();
        let mut a = scoped("sentineld", "location");
        a.triggered_by_external_content = true;
        let mut b = scoped("sentineld", "location");
        b.triggered_by_external_content = true;
        q.enqueue(a, &cap_suggest());
        assert!(matches!(q.enqueue(b, &cap_suggest()), Enqueued::Queued(_)));
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn a_silent_request_is_not_queued() {
        let mut q = ConsentQueue::new();
        // Ordinary + autonomous -> Silent.
        let e = q.enqueue(req("org.arlen.files", ActionKind::Ordinary, false), &cap_autonomous("org.arlen.files"));
        assert_eq!(e, Enqueued::SilentGrant);
        assert!(q.is_empty());
        assert!(q.front().is_none());
    }

    #[test]
    fn high_stakes_is_shown_before_standard() {
        let mut q = ConsentQueue::new();
        // First a Standard (Ordinary + suggest), then a HighStakes (delete).
        let standard = match q.enqueue(req("a", ActionKind::Ordinary, false), &cap_suggest()) {
            Enqueued::Queued(id) | Enqueued::AlreadyPending(id) => id,
            _ => panic!("expected queued"),
        };
        let high = match q.enqueue(req("b", ActionKind::PermanentDelete, false), &cap_suggest()) {
            Enqueued::Queued(id) | Enqueued::AlreadyPending(id) => id,
            _ => panic!("expected queued"),
        };
        assert_eq!(q.len(), 2);
        // Despite being enqueued second, the high-stakes request is front.
        assert_eq!(q.front().unwrap().id, high);
        // Resolving it surfaces the standard one.
        let (removed, _) = q.resolve(high, ConsentOutcome::Denied).unwrap();
        assert_eq!(removed.id, high);
        assert_eq!(q.front().unwrap().id, standard);
    }

    #[test]
    fn same_tier_is_fifo() {
        let mut q = ConsentQueue::new();
        let first = match q.enqueue(req("a", ActionKind::Ordinary, false), &cap_suggest()) {
            Enqueued::Queued(id) | Enqueued::AlreadyPending(id) => id,
            _ => panic!(),
        };
        let _second = q.enqueue(req("b", ActionKind::Ordinary, false), &cap_suggest());
        // Two Standard requests: the earlier one is shown first.
        assert_eq!(q.front().unwrap().id, first);
    }

    #[test]
    fn resolve_unknown_id_is_none() {
        let mut q = ConsentQueue::new();
        let id = match q.enqueue(req("a", ActionKind::PermanentDelete, false), &cap_suggest()) {
            Enqueued::Queued(id) | Enqueued::AlreadyPending(id) => id,
            _ => panic!(),
        };
        q.resolve(id, ConsentOutcome::AllowedOnce).unwrap();
        // Already resolved: a second resolve finds nothing.
        assert!(q.resolve(id, ConsentOutcome::AllowedOnce).is_none());
        assert!(q.is_empty());
    }

    #[test]
    fn ids_are_not_reused_after_resolve() {
        let mut q = ConsentQueue::new();
        let id0 = match q.enqueue(req("a", ActionKind::PermanentDelete, false), &cap_suggest()) {
            Enqueued::Queued(id) | Enqueued::AlreadyPending(id) => id,
            _ => panic!(),
        };
        q.resolve(id0, ConsentOutcome::Denied);
        let id1 = match q.enqueue(req("b", ActionKind::PermanentDelete, false), &cap_suggest()) {
            Enqueued::Queued(id) | Enqueued::AlreadyPending(id) => id,
            _ => panic!(),
        };
        assert_ne!(id0, id1, "a resolved id is never reused");
    }
}
