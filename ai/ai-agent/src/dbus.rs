//! D-Bus interface object for `org.arlen.AIAgent1`.
//!
//! Exposes the agent's *live loop status* as a read-only property, so a caller
//! (the harness agent dashboard, the TopBar) can see what the running daemon is
//! doing right now: waiting for a trigger, or handling one. This is the
//! "telephone" surface from the design (`KG = blackboard (pull), notifications
//! = mailbox (push), D-Bus = telephone (live/blocking)`): D-Bus carries live
//! state, not pull data.
//!
//! It deliberately does NOT re-expose the static behaviour catalogue (name,
//! kind, enabled, provenance, reads). That set is pull data the harness already
//! reads in-process through `arlen_ai_agent::loader::load_configured`
//! (`ai_behaviours`), and it is identical whether read there or here because the
//! daemon tracks no extra per-behaviour state. Duplicating it over D-Bus would
//! add a second source of the same truth with no new information.
//!
//! The interface is served on the same connection that owns the well-known name
//! `org.arlen.AIAgent1` (see `main.rs`), so the existing `ai-proxy` peer-auth
//! contract is preserved: the connection that owns the name is also the one that
//! forwards LLM traffic through the proxy. Registering an interface object does
//! not add a second connection or change name ownership.
//!
//! Status changes are not announced with `PropertiesChanged`: the value flips on
//! every dispatched event, and the harness polls its read surfaces today (the
//! activity timeline refreshes rather than subscribes). The property is annotated
//! `emits_changed_signal = "false"` so the contract is honest about that. A live
//! subscribe path (idle/thinking/acting split + a change signal) is the follow-up
//! that lands when the harness moves from poll to subscribe.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};
use zbus::interface;

use crate::config::AgentConfig;
use crate::engine::PendingProposal;
use crate::executor::{CompensationOutcome, Compensator};
use crate::discovery::ai_config_path;
use crate::receipt_store::{completed_view, ReceiptStore, RetainedReceipt};
use crate::seams::GraphHandle;

/// The D-Bus object path the interface is registered under.
pub const AGENT_OBJECT_PATH: &str = "/org/arlen/AIAgent1";

/// The agent's live loop status: what the running daemon is doing right now.
///
/// This is the honest resolution the dispatch loop can report without reaching
/// into the engine: `Subscribing` before the Event Bus subscription is
/// established (the daemon is up but cannot yet receive triggers), `Idle` once
/// it is waiting for the next trigger, and `Busy` while a dispatched event is
/// being handled (for a `kind: agent` behaviour that covers the whole bounded
/// loop). The `Subscribing` state matters for honesty: without it a poller
/// would read `idle` during an Event Bus outage and mistake an unreachable
/// daemon for a healthy waiting one. A finer `thinking` (provider call) vs
/// `acting` (executor write) split needs engine-internal hooks and is a
/// follow-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopStatus {
    /// Up, but the Event Bus subscription is not yet established; no trigger
    /// can be received yet.
    Subscribing,
    /// Subscribed and waiting for the next trigger; no behaviour is running.
    Idle,
    /// Handling a dispatched event (matching, screening, or running a loop).
    Busy,
}

impl LoopStatus {
    /// The wire string a caller reads from the `Status` property.
    pub fn as_str(self) -> &'static str {
        match self {
            LoopStatus::Subscribing => "subscribing",
            LoopStatus::Idle => "idle",
            LoopStatus::Busy => "busy",
        }
    }

    /// Reconstruct from the atomic encoding, defaulting to `Subscribing` for any
    /// unexpected byte so a torn read fails toward "not yet ready" rather than a
    /// healthy-looking `idle`.
    fn from_u8(v: u8) -> Self {
        match v {
            1 => LoopStatus::Idle,
            2 => LoopStatus::Busy,
            _ => LoopStatus::Subscribing,
        }
    }

    /// The atomic encoding.
    fn to_u8(self) -> u8 {
        match self {
            LoopStatus::Subscribing => 0,
            LoopStatus::Idle => 1,
            LoopStatus::Busy => 2,
        }
    }
}

/// Shared live-status cell, written by the dispatch loop and read by the D-Bus
/// property getter. A single atomic byte: the only states are `Idle`/`Busy`, the
/// writes are point updates with no cross-field invariant, and the reader only
/// needs the latest value, so no lock is warranted.
pub type StatusHandle = Arc<AtomicU8>;

/// Create a status handle initialised to `Subscribing` (the daemon is up but
/// has not yet established its Event Bus subscription, so it cannot receive
/// triggers; reporting `idle` here would hide that).
pub fn new_status_handle() -> StatusHandle {
    Arc::new(AtomicU8::new(LoopStatus::Subscribing.to_u8()))
}

/// Publish the current loop status for the D-Bus property to read.
pub fn set_status(handle: &StatusHandle, status: LoopStatus) {
    handle.store(status.to_u8(), Ordering::Relaxed);
}

/// Read the current loop status.
pub fn load_status(handle: &StatusHandle) -> LoopStatus {
    LoopStatus::from_u8(handle.load(Ordering::Relaxed))
}

/// The `org.arlen.AIAgent1` D-Bus interface object.
///
/// Holds the shared status cell so the property always reflects the live loop
/// state, not a startup snapshot.
pub struct AgentInterface {
    /// Live loop status, updated by the dispatch loop.
    pub status: StatusHandle,
    /// The owned compensator for the undo path. Always built (its writer/audit
    /// are startup-stable); whether an undo is *permitted* is gated at call time
    /// on `executor_live`, not by its presence.
    pub compensator: Compensator,
    /// The graph handle the compensation reads/retracts through.
    pub graph: Arc<dyn GraphHandle>,
    /// The execution receipts the dispatch loop retains, shared so a `compensate`
    /// call can look up the write to undo by its decision's correlation id.
    pub receipts: Arc<Mutex<ReceiptStore<RetainedReceipt>>>,
    /// Confirmation-needing gate decisions the dispatch loop retains, shared so the
    /// harness can read the pending proposals it renders as inline gate cards
    /// (harness-redesign emit seam 2), keyed by audit-ledger index.
    pub pending: Arc<Mutex<ReceiptStore<PendingProposal>>>,
    /// Carries a user-invoke (`run_skill`) request to the per-epoch dispatch
    /// loop, which runs the named skill on the LIVE dispatcher. A bounded
    /// channel: a backlog of invokes applies backpressure rather than growing
    /// unbounded.
    pub manual_tx: mpsc::Sender<ManualInvoke>,
}

/// A user-invoke request: run the named skill, reply with a status summary.
///
/// Carried from the D-Bus [`AgentInterface::run_skill`] method to the per-epoch
/// dispatch loop (`main.rs`), which owns the live `Dispatcher` and runs the
/// named behaviour through it (current provider/config), replying over
/// `respond`. Lives here, not in the binary, so the lib-side interface and the
/// binary's loop share one type.
pub struct ManualInvoke {
    /// The skill (behaviour) name to run.
    pub name: String,
    /// One-shot reply channel for the run's status summary.
    pub respond: oneshot::Sender<String>,
}

#[interface(name = "org.arlen.AIAgent1")]
impl AgentInterface {
    /// The agent's live loop status: `"subscribing"` before it can receive
    /// triggers, `"idle"` when waiting for one, `"busy"` when handling one.
    /// Read-only.
    ///
    /// `emits_changed_signal = "false"`: the value flips on every dispatched
    /// event and callers poll it; no `PropertiesChanged` is sent.
    #[zbus(property(emits_changed_signal = "false"))]
    async fn status(&self) -> String {
        load_status(&self.status).as_str().to_string()
    }

    /// Undo a previously-executed write, identified by its decision's
    /// correlation id (the activity entry the harness shows carries it). Returns
    /// a short status string: `not-enabled` when the executor is in suggest mode,
    /// `no-such-receipt` when no retained write matches, `retracted` /
    /// `nothing-to-undo` on a compensation, or `error: …` on a failed undo.
    ///
    /// The compensation is the same op-id-keyed retract `LiveExecutor` performs
    /// (fail-closed audit before the retract, keyed to the receipt's own op id,
    /// only a real `Created` write is undone). The call-time `executor_live` gate
    /// is the conservative posture "the executor is off, so no executor operation
    /// runs, undo included": re-reading the config means a flip is honoured
    /// without a restart. It is deliberately fail-safe over fully-undoable: a
    /// runtime live to suggest flip leaves real receipts in the store, and undo
    /// then refuses (`not-enabled`) until the executor is re-enabled, rather than
    /// retracting under a config that says the executor is off. Refusing an undo
    /// is always safe (the write already happened), so erring toward refusal here
    /// costs only convenience.
    ///
    /// Authorisation today is the session bus's same-user boundary: the KG is the
    /// user's own and the undo is reversible curation the agent re-derives on the
    /// next promotion pass, so a hostile same-user peer can at most force a
    /// transient un-tag, not an escalation across a trust boundary. That is weaker
    /// than the read-only `status` property warrants for a destructive verb,
    /// though: the defense-in-depth closure is a caller allowlist (the harness /
    /// settings, the `audit-daemon` `ADMITTED` pattern) plus recording the D-Bus
    /// caller in the retract audit (today the retract is attributed to the
    /// originating behaviour, not the invoker). That is deferred until a canonical
    /// harness app id exists to name in the allowlist (the same precedent as the
    /// `settings` app id added for revoke); enforcing an allowlist against a
    /// not-yet-canonical caller would be a dead gate.
    #[zbus(name = "compensate")]
    async fn compensate(&self, correlation_id: String) -> String {
        if !current_executor_live() {
            return "not-enabled: the executor is in suggest mode".to_string();
        }
        // Clone the receipt out under the lock; never hold the std Mutex across
        // the async compensation.
        let retained = match self.receipts.lock() {
            Ok(store) => store.get(&correlation_id),
            Err(_) => return "error: receipt store unavailable".to_string(),
        };
        let Some(retained) = retained else {
            return "no-such-receipt".to_string();
        };
        match self
            .compensator
            .compensate(&retained.receipt, &*self.graph, &retained.behaviour)
            .await
        {
            Ok(CompensationOutcome::Retracted) => "retracted".to_string(),
            Ok(CompensationOutcome::NothingToUndo) => "nothing-to-undo".to_string(),
            Err(e) => format!("error: {e}"),
        }
    }

    /// The agent's working-set **shape** as a JSON object (AIT-R1): the live
    /// loop status and, per enabled behaviour, its name, kind and declared read
    /// scope. Shape only, never content - it answers "what can the AI hold" (the
    /// configured reach) and "what is it doing now" (the status), never a node
    /// or a field of the user's data. When the master `[ai] enabled` switch is
    /// off nothing is enabled, so `behaviours` is empty.
    ///
    /// The behaviour set is read live from the configured sources on each call
    /// (mirroring `compensate`'s live config read), so a Settings change is
    /// reflected without a daemon restart. A live held-slice node-count is a
    /// follow-up needing an engine ingestion hook (see `working_set`'s module
    /// doc); today's slices are ephemeral per gate decision.
    #[zbus(name = "working_set")]
    async fn working_set(&self) -> String {
        let status = load_status(&self.status).as_str();
        let outcome = crate::discovery::load_configured();
        let shape = crate::working_set::working_set_shape(status, &outcome);
        serde_json::to_string(&shape).unwrap_or_else(|_| "{}".to_string())
    }

    /// The agent's currently-pending action proposals: the confirmation-needing
    /// gate decisions awaiting the user, oldest first, as a JSON array the harness
    /// renders as inline gate cards (harness-redesign emit seam 2). Read-only and
    /// content-bounded (each carries the display summary + the faithful reason +
    /// the registered effects; the audit subject stays content-free). Empty when
    /// nothing is pending. A later `approve`/`deny` keys off each entry's `id`.
    #[zbus(name = "pending_proposals")]
    async fn pending_proposals(&self) -> String {
        let proposals = self
            .pending
            .lock()
            .map(|store| store.values())
            .unwrap_or_default();
        serde_json::to_string(&proposals).unwrap_or_else(|_| "[]".to_string())
    }

    /// Dismiss a pending gate proposal: the user declined the confirmation, so
    /// the proposal is dropped from the pending set and the action is not taken
    /// (harness-redesign emit seam 2). `id` is the proposal's audit-ledger index
    /// (its `pending_proposals` handle). Returns `denied` when an entry was
    /// dropped, `no-such-proposal` when nothing matched (already acted on, aged
    /// out of the bounded store, or never pending), `error: ...` on a poisoned
    /// store lock.
    ///
    /// Deny is purely local and always available: it forgoes an action, which is
    /// safe in any mode, so it is **not** gated on `executor_live` (unlike
    /// `approve`, which would perform the write). The decline is not separately
    /// audited here; the gate already recorded the `RequireConfirmation` decision
    /// (the `id` is that audit index), and not acting leaves no further effect to
    /// attribute.
    #[zbus(name = "deny")]
    async fn deny(&self, id: u64) -> String {
        match self.pending.lock() {
            Ok(mut store) => match store.remove(&id.to_string()) {
                Some(_) => "denied".to_string(),
                None => "no-such-proposal".to_string(),
            },
            Err(_) => "error: pending store unavailable".to_string(),
        }
    }

    /// The agent's recently-completed actions: the executed (silent-done) writes
    /// retained for the live-session undo path, oldest first, as a JSON array the
    /// harness renders as quiet done-lines each with an `[Undo]`
    /// (harness-redesign emit seam 3). Each entry carries the decision's
    /// correlation id — the exact handle the built `compensate(id)` method undoes
    /// by — so the Undo button needs no extra lookup. Read-only and
    /// content-bounded (a description of the edge written; the audit subject stays
    /// content-free). Empty when nothing has executed (suggest-mode retains no
    /// receipt). The store is bounded, so an action that aged out can no longer be
    /// listed or undone — the same horizon as `compensate`.
    #[zbus(name = "completed_actions")]
    async fn completed_actions(&self) -> String {
        let actions: Vec<_> = self
            .receipts
            .lock()
            .map(|store| store.values().iter().map(completed_view).collect())
            .unwrap_or_default();
        serde_json::to_string(&actions).unwrap_or_else(|_| "[]".to_string())
    }

    /// The current autonomy-dial state, as a JSON object the harness renders as
    /// the dial (harness-redesign §3): `{ action_mode, autonomous_apps,
    /// executor_live }`. Read fresh from `ai.toml` on each call (mirroring
    /// `compensate`'s live `executor_live` read) so a runtime change is reflected
    /// without a daemon restart; the harness must NOT read `ai.toml` directly
    /// (Settings owns the file). `action_mode` is `suggest`|`supervised` - never
    /// `autonomous`, since the baseline can never be autonomous (autonomy is the
    /// per-app `autonomous_apps` grant list). `executor_live` is the orthogonal
    /// Tim-gated master, surfaced so the dial shows the honest inert state when it
    /// is off (a supervised baseline still does nothing while the executor is not
    /// live). Read-only and content-free. Fail-closed to the safe shape (`suggest`
    /// / `[]` / `false`) on any read/parse failure.
    #[zbus(name = "action_state")]
    async fn action_state(&self) -> String {
        let cfg = std::fs::read_to_string(ai_config_path())
            .ok()
            .map(|t| AgentConfig::parse(&t))
            .unwrap_or_else(AgentConfig::fail_closed);
        let autonomous_apps: Vec<&str> = cfg.actions.autonomous_apps().collect();
        serde_json::json!({
            "action_mode": cfg.actions.default_mode().as_str(),
            "autonomous_apps": autonomous_apps,
            "executor_live": cfg.executor_live,
        })
        .to_string()
    }

    /// The loaded skills as a JSON array, for the user-invoke discovery surface
    /// (PR-5 part 3a / the deferred S-U3b "behaviours list"). Each entry carries
    /// the skill's name, description, agent-match `whenToUse` hint, kind, and
    /// enabled state — identity and routing hints only, never the body or any
    /// user data. Read live from the configured sources on each call (mirroring
    /// `working_set`), so a Settings change is reflected without a restart. The
    /// harness renders this list; running a picked skill is `run_skill`.
    #[zbus(name = "list_skills")]
    async fn list_skills(&self) -> String {
        let outcome = crate::discovery::load_configured();
        let summaries = crate::skills::skill_summaries(&outcome.loaded);
        serde_json::to_string(&summaries).unwrap_or_else(|_| "[]".to_string())
    }

    /// Run a loaded skill by name (PR-5 part 3b user-invoke), returning a short
    /// status summary. The named behaviour runs through the bounded loop on the
    /// LIVE dispatcher (current provider, config and grants) via a manual invoke
    /// — bypassing event-routing, enablement-gated, and (for an agent skill)
    /// still provider/budget/tier-gated. The request is handed to the dispatch
    /// loop over a channel and this awaits its reply; a dropped or full channel
    /// (the loop is mid-rebuild) returns an error string rather than blocking.
    /// The harness renders the result; running is the agent's, not the caller's.
    #[zbus(name = "run_skill")]
    async fn run_skill(&self, name: String) -> String {
        let (respond, rx) = oneshot::channel();
        if self
            .manual_tx
            .send(ManualInvoke { name, respond })
            .await
            .is_err()
        {
            return "error: the agent run loop is unavailable".to_string();
        }
        match rx.await {
            Ok(summary) => summary,
            Err(_) => "error: the agent run loop dropped the request".to_string(),
        }
    }
}

/// Whether the executor is currently live, re-read from `ai.toml` so a runtime
/// flip is honoured without a daemon restart. Fail-closed to `false` (suggest
/// mode, undo refused) on any read/parse failure.
fn current_executor_live() -> bool {
    std::fs::read_to_string(ai_config_path())
        .ok()
        .map(|t| AgentConfig::parse(&t).executor_live)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_handle_reads_subscribing_not_idle() {
        // The honesty contract: before the daemon subscribes, a poller must not
        // see a healthy-looking "idle".
        let handle = new_status_handle();
        assert_eq!(load_status(&handle), LoopStatus::Subscribing);
        assert_eq!(load_status(&handle).as_str(), "subscribing");
    }

    #[test]
    fn set_then_load_round_trips() {
        let handle = new_status_handle();
        set_status(&handle, LoopStatus::Busy);
        assert_eq!(load_status(&handle), LoopStatus::Busy);
        assert_eq!(load_status(&handle).as_str(), "busy");
        set_status(&handle, LoopStatus::Idle);
        assert_eq!(load_status(&handle), LoopStatus::Idle);
        assert_eq!(load_status(&handle).as_str(), "idle");
        set_status(&handle, LoopStatus::Subscribing);
        assert_eq!(load_status(&handle), LoopStatus::Subscribing);
    }

    #[test]
    fn an_unknown_byte_decodes_to_subscribing() {
        // Fail toward "not yet ready", never a healthy-looking idle.
        let handle: StatusHandle = Arc::new(AtomicU8::new(200));
        assert_eq!(load_status(&handle), LoopStatus::Subscribing);
    }

    #[test]
    fn the_handle_is_shared_not_copied() {
        let handle = new_status_handle();
        let clone = Arc::clone(&handle);
        set_status(&clone, LoopStatus::Busy);
        // A write through one Arc is visible through the other: it is one cell.
        assert_eq!(load_status(&handle), LoopStatus::Busy);
    }
}
