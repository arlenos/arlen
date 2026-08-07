//! The `org.arlen.AIAgent1` pull-transparency + undo surface, re-homed from the
//! retired ai-agent onto the engine-daemon (pi-agent-adoption step 9 + the
//! planner's AIAgent1-fork ruling): DROP the approval queue (reversible writes
//! run autonomously under `executor_live`, irreversible ones Confirm via the
//! gate's consent path), KEEP the review-after-the-fact + undo methods. The engine
//! owns `org.arlen.AIAgent1` and serves exactly: status, completed_actions,
//! working_set, action_state, compensate, set_autonomous_app.
//!
//! This module grows one method at a time; each lands only when its engine-side
//! backing is real (no dormant stubs). It is registered on the engine's
//! `org.arlen.AI1` connection only once complete, because the ai-agent still owns
//! the name exclusively (`DoNotQueue`) until it is deleted.

use crate::compensation::CompensationStore;
use crate::engine_config;
use crate::write_executor::RelationWriter;
use arlen_ai_core::audit::behaviour_action_event;
use arlen_ai_skills::behaviour::{BehaviourKind, ReadScope};
use arlen_ai_skills::loader::LoadOutcome;
use audit_proto::sink::AuditSink;
use os_sdk::graph::RelationRetractOutcome;
use serde::Serialize;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

/// One enabled behaviour in the working-set shape: its name, kind and the KG read
/// scope it declares. Shape only, never any read CONTENT (the anti-Recall view is
/// "what the AI may read", not what it read).
#[derive(Debug, Serialize)]
struct BehaviourShape {
    /// The behaviour (skill) name.
    name: String,
    /// `workflow` (deterministic) or `agent` (bounded LLM loop).
    kind: &'static str,
    /// The declared minimum read scope (`minimal`/`session`/`project`/`time`/`full`).
    read_scope: &'static str,
}

/// The working-set introspection shape: the live loop status plus the enabled
/// behaviours and their declared read scopes.
#[derive(Debug, Serialize)]
struct WorkingSetShape {
    /// The live loop status (`subscribing`/`idle`/`busy`).
    status: String,
    /// The enabled behaviours' shape.
    behaviours: Vec<BehaviourShape>,
}

fn kind_str(k: BehaviourKind) -> &'static str {
    match k {
        BehaviourKind::Workflow => "workflow",
        BehaviourKind::Agent => "agent",
    }
}

fn read_scope_str(r: ReadScope) -> &'static str {
    match r {
        ReadScope::Minimal => "minimal",
        ReadScope::Session => "session",
        ReadScope::Project => "project",
        ReadScope::Time => "time",
        ReadScope::Full => "full",
    }
}

/// Render the working-set JSON from the live status and a behaviour-load outcome,
/// keeping only the ENABLED behaviours. Pure and testable.
fn working_set_json(status: &str, outcome: &LoadOutcome) -> String {
    let behaviours: Vec<BehaviourShape> = outcome
        .loaded
        .iter()
        .filter(|lb| lb.status.is_enabled())
        .map(|lb| {
            let m = &lb.behaviour.manifest;
            BehaviourShape {
                name: m.name.clone(),
                kind: kind_str(m.kind),
                read_scope: read_scope_str(m.reads),
            }
        })
        .collect();
    let shape = WorkingSetShape { status: status.to_string(), behaviours };
    serde_json::to_string(&shape).unwrap_or_else(|_| "{}".to_string())
}

/// The curator's live loop status, reported by `status`. `Subscribing` before the
/// event-bus subscription is established (up but no trigger can arrive yet - the
/// honest state during an outage, so a poller does not read a stalled daemon as a
/// healthy `idle`), `Idle` once waiting for the next trigger, `Busy` while a
/// dispatched event is being handled. A finer thinking/acting split needs engine-
/// internal hooks and is a follow-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopStatus {
    /// Up, but the event-bus subscription is not yet established.
    Subscribing,
    /// Subscribed and waiting for the next trigger.
    Idle,
    /// Handling a dispatched event.
    Busy,
}

impl LoopStatus {
    /// The wire string the `status` method returns.
    pub fn as_str(self) -> &'static str {
        match self {
            LoopStatus::Subscribing => "subscribing",
            LoopStatus::Idle => "idle",
            LoopStatus::Busy => "busy",
        }
    }

    /// Decode the atomic byte, any unexpected value fails toward `Subscribing`
    /// (not-yet-ready) rather than a healthy-looking `idle`.
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

/// A shared live-status cell, written by the curator loop and read by the `status`
/// method. A single atomic byte: point updates with no cross-field invariant, and
/// the reader only needs the latest value.
pub type StatusHandle = Arc<AtomicU8>;

/// A status handle initialised to `Subscribing` (up, not yet subscribed).
pub fn new_status_handle() -> StatusHandle {
    Arc::new(AtomicU8::new(LoopStatus::Subscribing.to_u8()))
}

/// Publish the current loop status.
pub fn set_status(handle: &StatusHandle, status: LoopStatus) {
    handle.store(status.to_u8(), Ordering::Relaxed);
}

/// Read the current loop status.
pub fn load_status(handle: &StatusHandle) -> LoopStatus {
    LoopStatus::from_u8(handle.load(Ordering::Relaxed))
}

/// The object path the interface is served at (unchanged from the ai-agent, so
/// existing callers reach the re-homed surface without a path change).
pub const AGENT_OBJECT_PATH: &str = "/org/arlen/AIAgent1";
/// The well-known name the engine owns for the agent surface.
pub const AGENT_BUS_NAME: &str = "org.arlen.AIAgent1";

/// One recently-executed, still-undoable write for the `completed_actions` feed.
/// Content-bounded: the edge written, never node content (the audit subject stays
/// content-free).
#[derive(Debug, Serialize)]
struct CompletedAction {
    /// The decision correlation id: the exact handle `compensate(id)` undoes by,
    /// so the harness's Undo button needs no extra lookup.
    id: String,
    /// The graph write's operation id (the durable retract key).
    op_id: String,
    /// The relation type written, for a quiet done-line.
    relation: String,
    /// The edge source node as `type/id`.
    from: String,
    /// The edge target node as `type/id`.
    to: String,
}

/// The graph completed-actions from the compensation store, oldest first.
fn graph_completed_actions(store: &CompensationStore) -> Vec<CompletedAction> {
    store
        .entries()
        .into_iter()
        .map(|(id, r)| CompletedAction {
            id: id.to_string(),
            op_id: r.op_id.clone(),
            relation: r.relation_type.clone(),
            from: format!("{}/{}", r.from_type, r.from_id),
            to: format!("{}/{}", r.to_type, r.to_id),
        })
        .collect()
}

/// One still-undoable NON-GRAPH action (a filesystem/settings inverse the FM, the
/// trash-first rm or the settings executor journaled to the signed undo-log), for the
/// `completed_actions` feed - so a filesystem undo is DISCOVERABLE there, not only a
/// graph-edge one. `kind` names what the undo does; `target` is the path/setting it
/// acts on (content-bounded, consistent with the graph entries' `from`/`to` node ids,
/// which are already file paths for File nodes).
#[derive(Debug, Serialize)]
struct CompletedNonGraphAction {
    /// The decision correlation id: the handle `compensate(id)` undoes by.
    id: String,
    /// The durable op id.
    op_id: String,
    /// What the undo does (`relocate` / `restore-from-trash` / `restore-setting` / ...).
    kind: &'static str,
    /// The path or setting the undo acts on.
    target: String,
}

/// Render the still-undoable NON-GRAPH actions from the signer's live entries. A
/// graph-edge inverse is skipped (the in-memory store surfaces it); a terminal entry
/// is already absent from the signer's live set. Pure and testable.
fn nongraph_completed_actions(
    entries: &[arlen_ai_undo_core::undo_log::UndoEntry],
) -> Vec<CompletedNonGraphAction> {
    use arlen_ai_undo_core::effect_model::InverseReceipt;
    entries
        .iter()
        .filter_map(|e| {
            let (kind, target) = match &e.inverse {
                InverseReceipt::RestorePath { prior, .. } => ("relocate", prior.as_str().to_string()),
                InverseReceipt::RestoreFromTrash { original, .. } => {
                    ("restore-from-trash", original.as_str().to_string())
                }
                InverseReceipt::RestoreValue { target, .. } => {
                    ("restore-setting", format!("{}:{}", target.file(), target.key()))
                }
                InverseReceipt::DeleteCreated { created } => {
                    ("delete-created", created.path().as_str().to_string())
                }
                InverseReceipt::TrashCreated { created } => {
                    ("trash-created", created.path().as_str().to_string())
                }
                InverseReceipt::RestoreSnapshot { snapshot, .. } => {
                    ("restore-snapshot", snapshot.as_str().to_string())
                }
                // A graph edge is the in-memory store's authoritative surface.
                InverseReceipt::RetractGraphEdge { .. } => return None,
            };
            Some(CompletedNonGraphAction {
                id: e.correlation_id.clone(),
                op_id: e.op_id.clone(),
                kind,
                target,
            })
        })
        .collect()
}

/// The app-ids allowed to invoke the destructive `compensate` verb: the harness
/// (the undo UI) and Settings. Every other caller is refused - `compensate`
/// retracts a graph write, so it is not an app-facing method. The ids resolve
/// through the F3 `path_to_app_id` chain (a root-owned `/usr/lib/arlen/apps/<id>`
/// path), the same identity model the knowledge daemon and installd key on; the
/// exact harness id is verified against its install at the name-transfer wiring.
const COMPENSATE_ADMITTED: &[&str] = &["harness", "settings"];

/// Whether `app_id` may invoke `compensate`.
fn compensate_caller_admitted(app_id: &str) -> bool {
    COMPENSATE_ADMITTED.contains(&app_id)
}

/// The surfaces that speak for the user: the two first-party apps that render an
/// AI answer to them. Shared with `explain_system`, which is a full-scope KG read
/// (`reads: full` in the explain skill) handed straight back to whoever asked, so
/// it needs the same gate as the destructive verb even though it writes nothing.
///
/// Same list as [`COMPENSATE_ADMITTED`] today and deliberately a separate name: a
/// read surface and a destructive one may diverge, and a shared constant would
/// make that divergence look like a mistake.
/// Whether `app_id` is a user-facing surface allowed to ask the engine to read on
/// the user's behalf.
///
/// The list itself lives in `arlen_permissions::identity`, because the undo
/// service answers for the same actions and a surface admitted by one daemon and
/// not the other is a button that works on one page and fails on the next.
pub(crate) fn user_surface_admitted(app_id: &str) -> bool {
    arlen_permissions::identity::is_user_surface(app_id)
}

/// Resolve the calling app's Arlen identity from the D-Bus connection: the session
/// bus attests the sender PID (`GetConnectionUnixProcessID`, not a client value),
/// and `app_id_from_pid` resolves `/proc/<pid>/exe` through the F3 chain. Any
/// failure is an `Err`, treated as not-admitted (fail-closed). Documented residual
/// (the same one the whole F3 model carries): a sub-millisecond PID-reuse window
/// and a same-uid `exec` - closed only by the inode-attested identity registry.
pub(crate) async fn resolve_dbus_caller(
    header: &zbus::message::Header<'_>,
    connection: &zbus::Connection,
) -> Result<String, String> {
    let sender = header
        .sender()
        .ok_or_else(|| "no sender in message".to_string())?;
    let proxy = zbus::fdo::DBusProxy::new(connection)
        .await
        .map_err(|e| format!("DBusProxy: {e}"))?;
    let pid = proxy
        .get_connection_unix_process_id(sender.clone().into())
        .await
        .map_err(|e| format!("get caller pid: {e}"))?;
    arlen_permissions::identity::app_id_from_pid(pid).map_err(|e| format!("resolve app id: {e}"))
}

/// The undo verdict, kept as a small helper so `compensate`'s flow is unit-tested
/// without a live bus. The wire is the string; this names the branches.
fn compensate_outcome_wire(outcome: RelationRetractOutcome) -> &'static str {
    match outcome {
        RelationRetractOutcome::Retracted => "retracted",
        RelationRetractOutcome::Absent => "nothing-to-undo",
    }
}

/// The wire verdict for a non-graph (filesystem/settings) undo enactment.
fn enact_outcome_wire(outcome: crate::undo_enact::EnactOutcome) -> &'static str {
    use crate::undo_enact::EnactOutcome;
    match outcome {
        EnactOutcome::Restored => "restored",
        EnactOutcome::Deleted => "deleted",
        EnactOutcome::Trashed => "trashed",
        EnactOutcome::RefusedIdentityMismatch => "refused-identity-mismatch",
        EnactOutcome::RefusedPriorOccupied => "refused-prior-occupied",
        // A graph inverse reached the filesystem enact (it was in the durable log but
        // aged out of the in-memory graph store). The graph store is the authoritative
        // graph-undo path, so from here there is nothing this path can undo.
        EnactOutcome::NotFilesystem => "no-such-receipt",
    }
}

/// Enact one NON-GRAPH inverse (a filesystem/settings receipt the FM, the trash-first
/// rm or the settings executor journaled to the signed undo-log). Fail-closed:
/// AUDIT BEFORE the enact (S13, content-free, records WHO undid) and refuse if the
/// ledger will not record it; then run the identity-checked, fail-closed
/// `enact_inverse` off the async runtime (it does blocking filesystem I/O). The
/// executor_live gate + caller-auth are enforced upstream in `run_compensate` / the
/// D-Bus method, so this is reached only for an admitted, live, authorised undo.
async fn dispatch_nongraph_inverse(
    caller: &str,
    correlation_id: &str,
    inverse: arlen_ai_undo_core::effect_model::InverseReceipt,
    audit: &dyn AuditSink,
    signer_socket: &std::path::Path,
) -> String {
    let event = behaviour_action_event(
        "compensate",
        format!("enact-inverse:by={caller}"),
        correlation_id,
    );
    if audit.submit(event).await.is_err() {
        return "error: audit unavailable".to_string();
    }
    let outcome = match tokio::task::spawn_blocking(move || crate::undo_enact::enact_inverse(&inverse)).await {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(e)) => return format!("error: {e}"),
        Err(_) => return "error: enact task failed".to_string(),
    };
    // Mark the entry terminal after an undo that ACTUALLY mutated, so a second
    // `compensate(same-id)` is a no-op the state pre-gate catches rather than a
    // re-enact (which for `RestoreValue` would revert a user's later change). A
    // `Refused*`/`NotFilesystem` outcome did not act, so the entry stays live for a
    // legitimate retry. `Superseded` is the one terminal state reachable from an
    // `InFlight` entry (which is where the FM/rm producers leave their receipts).
    // Best-effort: the undo already happened; a failed transition only leaves a
    // re-enact possible, bounded + fail-safe for the destructive variants.
    if outcome_marks_terminal(outcome) {
        let _ = crate::undo_signer::transition(
            signer_socket,
            correlation_id,
            arlen_ai_undo_core::undo_log::UndoState::Superseded,
        )
        .await;
    }
    enact_outcome_wire(outcome).to_string()
}

/// Whether an enact outcome represents an undo that actually mutated the filesystem
/// (so the entry should be marked terminal), as opposed to a fail-safe no-op.
fn outcome_marks_terminal(outcome: crate::undo_enact::EnactOutcome) -> bool {
    use crate::undo_enact::EnactOutcome;
    matches!(
        outcome,
        EnactOutcome::Restored | EnactOutcome::Deleted | EnactOutcome::Trashed
    )
}

/// Run one `compensate`: undo the executed write recorded under `correlation_id`.
/// Fail-closed at every step, in order: refuse unless the executor is live; refuse
/// an unknown receipt; AUDIT BEFORE the retract (S13) and refuse if the audit
/// ledger will not record it (never an unaudited destructive act); then retract
/// exactly this write's own op-id-stamped edge. The receipt is cloned out and the
/// lock dropped before the awaits.
/// Deliberately gated on `executor_live` ALONE, not on `may_act` (the master
/// switch AND the executor gate) that every acting executor uses. Undo is the
/// opposite of acting: it removes something the assistant already did. Someone
/// who switches the AI off and then reaches for undo is cleaning up after it, and
/// refusing them because the thing they just turned off is off would be hostile.
/// The `executor_live` check stays because a receipt only exists when the
/// executor was live, so it costs nothing and keeps the surface fail-safe.
async fn run_compensate(
    executor_live: bool,
    caller: &str,
    correlation_id: &str,
    compensation: &Mutex<CompensationStore>,
    writer: &dyn RelationWriter,
    audit: &dyn AuditSink,
) -> String {
    if !executor_live {
        return "not-enabled".to_string();
    }
    // Sample the in-memory graph store (the authoritative graph-undo fast path),
    // dropping the lock before any await.
    let receipt = {
        let store = match compensation.lock() {
            Ok(s) => s,
            Err(_) => return "error: compensation store unavailable".to_string(),
        };
        store.get(correlation_id).cloned()
    };
    let receipt = match receipt {
        Some(r) => r,
        None => {
            // Fallthrough: the graph store is graph-only, but the durable signed
            // undo-log may hold a NON-GRAPH inverse (a filesystem/settings receipt).
            // The executor_live gate + caller-auth already passed; audit-before-act
            // runs inside `dispatch_nongraph_inverse`. Every lookup failure is
            // fail-closed.
            let socket = arlen_ai_undo_proto::socket_path();
            // Idempotency + corruption pre-gate: an already-terminal entry is a no-op
            // (do not re-enact), a corrupt chain is refused, an absent id is unknown.
            match crate::undo_signer::lookup_state(&socket, correlation_id).await {
                Ok(arlen_ai_undo_proto::StateReply::Absent) => {
                    return "no-such-receipt".to_string()
                }
                Ok(arlen_ai_undo_proto::StateReply::Present(state)) if state.is_terminal() => {
                    return "already-undone".to_string()
                }
                Ok(arlen_ai_undo_proto::StateReply::Present(_)) => {}
                Ok(arlen_ai_undo_proto::StateReply::Corrupt) => {
                    return "error: undo log corrupt".to_string()
                }
                Err(_) => return "error: undo log unavailable".to_string(),
            }
            return match crate::undo_signer::lookup_entry(&socket, correlation_id).await {
                Ok(Some(entry)) => {
                    dispatch_nongraph_inverse(caller, correlation_id, entry.inverse, audit, &socket)
                        .await
                }
                // Raced to terminal/removed between the two reads: nothing to undo.
                Ok(None) => "no-such-receipt".to_string(),
                Err(_) => "error: undo log unavailable".to_string(),
            };
        }
    };
    // Audit-before-act, fail-closed: an undo that cannot be recorded does not run.
    // The caller app-id (content-free) is recorded so the ledger shows WHO undid.
    let event =
        behaviour_action_event("compensate", format!("retract-relation:by={caller}"), correlation_id);
    if audit.submit(event).await.is_err() {
        return "error: audit unavailable".to_string();
    }
    match writer
        .retract_relation(
            &receipt.from_type,
            &receipt.from_id,
            &receipt.to_type,
            &receipt.to_id,
            &receipt.relation_type,
            &receipt.op_id,
        )
        .await
    {
        Ok(outcome) => {
            // The edge is now gone (retracted, or already absent on a retry), so
            // drop the receipt: completed_actions must not keep offering an undo
            // for an action that has already been undone.
            if let Ok(mut store) = compensation.lock() {
                store.remove(correlation_id);
            }
            compensate_outcome_wire(outcome).to_string()
        }
        Err(e) => format!("error: {e}"),
    }
}

/// The `org.arlen.AIAgent1` interface object. Holds the shared compensation store
/// (for `completed_actions` + `compensate`), the graph writer the undo retracts
/// through, and the audit sink the undo records to before acting.
/// How many recent actions `undo_read` returns. A recent-actions panel shows
/// tens of rows, and each row costs one audit chain read; this is well past what
/// a person scrolls and still a bounded number of round trips.
const UNDO_READ_LIMIT: u32 = 50;

pub struct AgentAdminInterface {
    status: StatusHandle,
    compensation: Arc<Mutex<CompensationStore>>,
    writer: Arc<dyn RelationWriter>,
    audit: Arc<dyn AuditSink>,
}

impl AgentAdminInterface {
    /// Build the interface over the daemon's shared loop-status cell, compensation
    /// store, graph writer and audit sink.
    pub fn new(
        status: StatusHandle,
        compensation: Arc<Mutex<CompensationStore>>,
        writer: Arc<dyn RelationWriter>,
        audit: Arc<dyn AuditSink>,
    ) -> Self {
        Self { status, compensation, writer, audit }
    }
}

#[zbus::interface(name = "org.arlen.AIAgent1")]
impl AgentAdminInterface {
    /// The agent's recently-completed actions: the executed (silent-done) writes
    /// retained for the live-session undo path, oldest first, as a JSON array the
    /// harness renders as quiet done-lines each with an `[Undo]`. Each carries the
    /// decision correlation id that `compensate(id)` undoes by. Read-only,
    /// content-bounded, and bounded to the store's horizon (an aged-out action can
    /// neither be listed nor undone). Empty when nothing has executed.
    /// The curator's live loop status: `subscribing` (up, not yet subscribed to
    /// the event bus), `idle` (waiting for the next trigger) or `busy` (handling a
    /// dispatched event). Honest during an event-bus outage (stays `subscribing`
    /// rather than reading as a healthy `idle`).
    #[zbus(name = "status")]
    async fn status(&self) -> String {
        load_status(&self.status).as_str().to_string()
    }

    /// The agent's working set: the live status plus the enabled behaviours and
    /// their declared KG read scopes, as a JSON object the harness renders as the
    /// anti-Recall transparency view ("what the AI may read"). Shape only, never
    /// read content. Read live from the configured behaviour sources on each call.
    #[zbus(name = "working_set")]
    async fn working_set(&self) -> String {
        working_set_json(
            load_status(&self.status).as_str(),
            &crate::orchestrator::load_behaviours(),
        )
    }

    #[zbus(name = "completed_actions")]
    async fn completed_actions(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> String {
        // Same gate as `explain_system`, for the same reason: an entry's `from` and
        // `to` are `type/id`, and a File node's id is its path, so this list names
        // the user's files. A refused caller gets an empty array rather than an
        // error string, because the wire shape is JSON the caller parses; the
        // warning is there so a harness whose identity stopped resolving shows up
        // in the log instead of quietly seeing no actions.
        match resolve_dbus_caller(&header, connection).await {
            Ok(caller) if user_surface_admitted(&caller) => {}
            Ok(caller) => {
                tracing::warn!(%caller, "completed_actions refused: not a user surface");
                return "[]".to_string();
            }
            Err(e) => {
                tracing::warn!(error = %e, "completed_actions refused: caller unresolved");
                return "[]".to_string();
            }
        }
        // The graph actions from the in-memory store (fast, always present).
        let graph = self
            .compensation
            .lock()
            .map(|store| graph_completed_actions(&store))
            .unwrap_or_default();
        // The non-graph (filesystem/settings) actions from the signed undo-log, so a
        // filesystem undo is discoverable here too. Best-effort: an absent/unreachable
        // signer just yields the graph actions, never an error.
        let socket = arlen_ai_undo_proto::socket_path();
        let nongraph = match crate::undo_signer::fetch_live(&socket).await {
            Ok(entries) => nongraph_completed_actions(&entries),
            Err(_) => Vec::new(),
        };
        // Combine into one heterogeneous array; the graph entries keep their shape
        // (backward-compatible), the non-graph entries carry `kind`/`target`.
        let mut items: Vec<serde_json::Value> =
            graph.iter().filter_map(|a| serde_json::to_value(a).ok()).collect();
        items.extend(nongraph.iter().filter_map(|a| serde_json::to_value(a).ok()));
        serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string())
    }

    /// The recent-actions history: every producer's journalled reversible action,
    /// newest first, as a JSON array. This is the read behind the desktop-wide undo
    /// surface, and it is a COMPENSABLE-ACTION history rather than a global Ctrl-Z -
    /// each row carries its own inverse, and one that this daemon cannot carry out
    /// says so in `enactable` instead of offering a button that fails.
    ///
    /// Same user-surface gate as `completed_actions` and for the same reason: a row
    /// names the object it acted on, and a file's object is its path. A refused
    /// caller gets an empty array rather than an error, because the wire shape is
    /// JSON the caller parses.
    ///
    /// Degrades in two directions, deliberately differently. An unreachable signer
    /// yields no rows - there is nothing to offer an undo of. An unreachable audit
    /// daemon yields rows with no `description`, because the action is still
    /// undoable and saying so is the point.
    #[zbus(name = "undo_read")]
    async fn undo_read(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> String {
        match resolve_dbus_caller(&header, connection).await {
            Ok(caller) if user_surface_admitted(&caller) => {}
            Ok(caller) => {
                tracing::warn!(%caller, "undo_read refused: not a user surface");
                return "[]".to_string();
            }
            Err(e) => {
                tracing::warn!(error = %e, "undo_read refused: caller unresolved");
                return "[]".to_string();
            }
        }
        let rows = crate::undo_history::recent_rows(
            &arlen_ai_undo_proto::socket_path(),
            &crate::undo_history::LedgerChains::at_default_socket(),
            UNDO_READ_LIMIT,
        )
        .await;
        serde_json::to_string(&rows).unwrap_or_else(|_| "[]".to_string())
    }

    /// Undo a completed action: retract the graph write recorded under
    /// `correlation_id`. Reversible curation is autonomous, so this is the user's
    /// after-the-fact undo. Re-reads `executor_live` live (a runtime flip to
    /// suggest-mode refuses the undo fail-safe); fail-closed on an unknown receipt,
    /// an unrecordable audit, or a retract error. Returns `retracted`,
    /// `nothing-to-undo` (the edge was already gone), `no-such-receipt`,
    /// `not-enabled` (suggest-mode) or `error: <reason>`.
    #[zbus(name = "compensate")]
    async fn compensate(
        &self,
        correlation_id: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> String {
        // Caller-auth: only the harness / Settings may invoke this destructive
        // verb (it retracts a graph write). Any other or unresolvable caller is
        // refused before any store read, audit or write - fail-closed.
        let caller = match resolve_dbus_caller(&header, connection).await {
            Ok(c) if compensate_caller_admitted(&c) => c,
            _ => return "not-permitted".to_string(),
        };
        run_compensate(
            engine_config::executor_live(),
            &caller,
            &correlation_id,
            &self.compensation,
            &*self.writer,
            &*self.audit,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compensation::RetractReceipt;
    use serde_json::Value;

    fn receipt(op: &str) -> RetractReceipt {
        RetractReceipt::for_write(op, "File", "f-1", "Project", "proj-1", "FILE_PART_OF")
    }

    #[test]
    fn completed_actions_render_oldest_first_with_the_undo_handle() {
        let mut store = CompensationStore::new(8);
        store.register("corr-1", receipt("op-1"));
        store.register("corr-2", receipt("op-2"));
        let json = serde_json::to_string(&graph_completed_actions(&store)).unwrap();
        let v: Value = serde_json::from_str(&json).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], "corr-1");
        assert_eq!(arr[0]["op_id"], "op-1");
        assert_eq!(arr[0]["relation"], "FILE_PART_OF");
        assert_eq!(arr[0]["from"], "File/f-1");
        assert_eq!(arr[0]["to"], "Project/proj-1");
        assert_eq!(arr[1]["id"], "corr-2");
    }

    #[test]
    fn an_empty_store_renders_an_empty_array() {
        let empty = serde_json::to_string(&graph_completed_actions(&CompensationStore::new(8))).unwrap();
        assert_eq!(empty, "[]");
    }

    #[test]
    fn working_set_reflects_status_with_no_behaviours() {
        let outcome = LoadOutcome { loaded: vec![], errors: vec![] };
        let json = working_set_json("idle", &outcome);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["status"], "idle");
        assert_eq!(v["behaviours"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn kind_and_scope_map_to_the_manifest_vocabulary() {
        assert_eq!(kind_str(BehaviourKind::Agent), "agent");
        assert_eq!(kind_str(BehaviourKind::Workflow), "workflow");
        assert_eq!(read_scope_str(ReadScope::Project), "project");
        assert_eq!(read_scope_str(ReadScope::Full), "full");
        assert_eq!(read_scope_str(ReadScope::Minimal), "minimal");
    }

    #[test]
    fn a_status_handle_defaults_to_subscribing_and_round_trips() {
        let h = new_status_handle();
        assert_eq!(load_status(&h), LoopStatus::Subscribing);
        set_status(&h, LoopStatus::Busy);
        assert_eq!(load_status(&h), LoopStatus::Busy);
        assert_eq!(LoopStatus::Busy.as_str(), "busy");
        assert_eq!(LoopStatus::Idle.as_str(), "idle");
        assert_eq!(LoopStatus::Subscribing.as_str(), "subscribing");
    }

    use audit_proto::sink::MockAuditSink;
    use os_sdk::graph::RelationWriteOutcome;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A writer that records whether its retract was called and returns a canned
    /// retract outcome, so a test can assert the fail-closed audit gate really
    /// prevents the retract.
    struct RetractMock {
        outcome: Result<RelationRetractOutcome, String>,
        retract_called: AtomicBool,
    }

    impl RetractMock {
        fn new(outcome: Result<RelationRetractOutcome, String>) -> Self {
            Self { outcome, retract_called: AtomicBool::new(false) }
        }
    }

    #[async_trait::async_trait]
    impl RelationWriter for RetractMock {
        async fn create_relation(
            &self,
            _ft: &str,
            _fi: &str,
            _tt: &str,
            _ti: &str,
            _rt: &str,
            _op: &str,
        ) -> Result<RelationWriteOutcome, String> {
            Err("create not used in the compensate tests".to_string())
        }
        async fn retract_relation(
            &self,
            _ft: &str,
            _fi: &str,
            _tt: &str,
            _ti: &str,
            _rt: &str,
            _op: &str,
        ) -> Result<RelationRetractOutcome, String> {
            self.retract_called.store(true, Ordering::Relaxed);
            self.outcome.clone()
        }
    }

    fn store_with(id: &str, op: &str) -> Mutex<CompensationStore> {
        let mut s = CompensationStore::new(8);
        s.register(id, receipt(op));
        Mutex::new(s)
    }

    use arlen_ai_undo_core::effect_model::{CanonicalPath, InverseReceipt};

    fn tmp() -> std::path::PathBuf {
        use std::sync::atomic::AtomicU64;
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("compensate-nongraph-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    fn restore_path_inverse(now: &std::path::Path, prior: &std::path::Path) -> InverseReceipt {
        InverseReceipt::RestorePath {
            now: CanonicalPath::new(now.to_str().unwrap()).unwrap(),
            prior: CanonicalPath::new(prior.to_str().unwrap()).unwrap(),
        }
    }

    /// A path with no signer listening, so the best-effort terminal-marking
    /// transition fails silently in a unit test (the enact + wire still hold).
    fn no_signer() -> &'static std::path::Path {
        std::path::Path::new("/nonexistent/store-no-signer.sock")
    }

    #[test]
    fn enact_wire_maps_the_variants() {
        use crate::undo_enact::EnactOutcome::*;
        assert_eq!(enact_outcome_wire(Restored), "restored");
        assert_eq!(enact_outcome_wire(Deleted), "deleted");
        assert_eq!(enact_outcome_wire(Trashed), "trashed");
        assert_eq!(enact_outcome_wire(RefusedIdentityMismatch), "refused-identity-mismatch");
        assert_eq!(enact_outcome_wire(RefusedPriorOccupied), "refused-prior-occupied");
        // A graph inverse reaching the fs enact is nothing this path can undo.
        assert_eq!(enact_outcome_wire(NotFilesystem), "no-such-receipt");
    }

    #[test]
    fn nongraph_completed_actions_render_fs_inverses_and_skip_graph() {
        use arlen_ai_undo_core::undo_log::UndoEntry;
        let entry = |cid: &str, inverse| UndoEntry {
            op_id: cid.to_string(),
            correlation_id: cid.to_string(),
            inverse,
        };
        let entries = vec![
            entry(
                "c1",
                restore_path_inverse(
                    std::path::Path::new("/a/now.txt"),
                    std::path::Path::new("/a/prior.txt"),
                ),
            ),
            entry(
                "c2",
                InverseReceipt::RetractGraphEdge {
                    op_id: "c2".into(),
                    from_type: "system.File".into(),
                    from_id: "/x".into(),
                    to_type: "system.Project".into(),
                    to_id: "p".into(),
                    relation_type: "FILE_PART_OF".into(),
                },
            ),
        ];
        let rendered = nongraph_completed_actions(&entries);
        // The graph inverse is skipped (the in-memory store surfaces it); the
        // filesystem one renders with its kind + restore target.
        assert_eq!(rendered.len(), 1);
        assert_eq!(rendered[0].id, "c1");
        assert_eq!(rendered[0].kind, "relocate");
        assert_eq!(rendered[0].target, "/a/prior.txt");
    }

    #[test]
    fn only_mutating_outcomes_mark_the_entry_terminal() {
        use crate::undo_enact::EnactOutcome::*;
        // A completed undo marks the entry terminal (no re-enact).
        assert!(outcome_marks_terminal(Restored));
        assert!(outcome_marks_terminal(Deleted));
        assert!(outcome_marks_terminal(Trashed));
        // A fail-safe no-op leaves the entry live for a legitimate retry.
        assert!(!outcome_marks_terminal(RefusedIdentityMismatch));
        assert!(!outcome_marks_terminal(RefusedPriorOccupied));
        assert!(!outcome_marks_terminal(NotFilesystem));
    }

    #[tokio::test]
    async fn a_nongraph_inverse_is_enacted_when_admitted_and_audited() {
        let dir = tmp();
        let now = dir.join("moved.txt");
        let prior = dir.join("orig.txt");
        std::fs::write(&now, b"x").unwrap();
        let out = dispatch_nongraph_inverse(
            "settings",
            "corr-1",
            restore_path_inverse(&now, &prior),
            &MockAuditSink::accepting(),
            no_signer(),
        )
        .await;
        assert_eq!(out, "restored");
        assert!(prior.exists() && !now.exists(), "the entity moved back to its prior path");
    }

    #[tokio::test]
    async fn a_nongraph_undo_refuses_and_does_not_act_when_the_audit_fails() {
        let dir = tmp();
        let now = dir.join("moved.txt");
        let prior = dir.join("orig.txt");
        std::fs::write(&now, b"x").unwrap();
        let out = dispatch_nongraph_inverse(
            "settings",
            "corr-1",
            restore_path_inverse(&now, &prior),
            &MockAuditSink::failing(),
            no_signer(),
        )
        .await;
        assert_eq!(out, "error: audit unavailable");
        assert!(now.exists() && !prior.exists(), "no enact runs when the audit cannot record");
    }

    #[tokio::test]
    async fn a_graph_inverse_on_the_nongraph_path_is_nothing_to_undo() {
        let inverse = InverseReceipt::RetractGraphEdge {
            op_id: "op".into(),
            from_type: "system.File".into(),
            from_id: "/x".into(),
            to_type: "system.Project".into(),
            to_id: "p".into(),
            relation_type: "FILE_PART_OF".into(),
        };
        let out =
            dispatch_nongraph_inverse("settings", "corr-1", inverse, &MockAuditSink::accepting(), no_signer())
                .await;
        assert_eq!(out, "no-such-receipt");
    }

    #[test]
    fn only_the_harness_and_settings_may_compensate() {
        assert!(compensate_caller_admitted("harness"));
        assert!(compensate_caller_admitted("settings"));
        assert!(!compensate_caller_admitted("com.example.app"));
        assert!(!compensate_caller_admitted("ai-agent"));
        assert!(!compensate_caller_admitted(""));
    }

    /// The same gate on the read side. A confined app reaches the session bus, so
    /// an ungated `explain_system` would hand it a full-scope summary of the user's
    /// work; only the two surfaces that show the answer to the user may ask.
    #[test]
    fn only_the_harness_and_settings_may_ask_the_engine_to_read() {
        assert!(user_surface_admitted("harness"));
        assert!(user_surface_admitted("settings"));
        assert!(!user_surface_admitted("com.example.app"));
        assert!(!user_surface_admitted("ai-agent"));
        assert!(!user_surface_admitted(""));
        // The dev ids are exact, so a neighbouring cargo binary is still refused
        // even in a debug build, and the prefix alone never admits.
        assert!(!user_surface_admitted("dev.arlen-run"));
        assert!(!user_surface_admitted("dev."));
    }

    #[tokio::test]
    async fn suggest_mode_refuses_the_undo_without_touching_the_store_or_writer() {
        let writer = RetractMock::new(Ok(RelationRetractOutcome::Retracted));
        let out =
            run_compensate(false, "settings", "corr-1", &store_with("corr-1", "op-1"), &writer, &MockAuditSink::accepting())
                .await;
        assert_eq!(out, "not-enabled");
        assert!(!writer.retract_called.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn an_unknown_receipt_falls_through_to_the_log_and_fails_closed() {
        let writer = RetractMock::new(Ok(RelationRetractOutcome::Retracted));
        let out = run_compensate(
            true,
            "settings",
            "missing",
            &Mutex::new(CompensationStore::new(8)),
            &writer,
            &MockAuditSink::accepting(),
        )
        .await;
        // Not in the in-memory graph store, so it consults the durable undo-log for a
        // non-graph inverse. With no signer reachable in the unit env, that is
        // fail-closed: the undo is refused ("undo log unavailable") rather than a
        // false "no-such-receipt" that could hide a real filesystem receipt. The
        // graph retract is never touched on this path.
        assert_eq!(out, "error: undo log unavailable");
        assert!(!writer.retract_called.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn an_unrecordable_audit_refuses_the_undo_and_never_retracts() {
        let writer = RetractMock::new(Ok(RelationRetractOutcome::Retracted));
        let out =
            run_compensate(true, "settings", "corr-1", &store_with("corr-1", "op-1"), &writer, &MockAuditSink::failing())
                .await;
        assert_eq!(out, "error: audit unavailable");
        assert!(
            !writer.retract_called.load(Ordering::Relaxed),
            "audit-before-act must gate the destructive retract"
        );
    }

    #[tokio::test]
    async fn a_live_undo_retracts_its_own_edge_and_drops_the_receipt() {
        let writer = RetractMock::new(Ok(RelationRetractOutcome::Retracted));
        let store = store_with("corr-1", "op-1");
        let out = run_compensate(true, "settings", "corr-1", &store, &writer, &MockAuditSink::accepting()).await;
        assert_eq!(out, "retracted");
        assert!(writer.retract_called.load(Ordering::Relaxed));
        // The undone receipt is dropped so completed_actions won't re-offer it.
        assert!(store.lock().unwrap().get("corr-1").is_none());
    }

    #[tokio::test]
    async fn an_already_gone_edge_reports_nothing_to_undo() {
        let writer = RetractMock::new(Ok(RelationRetractOutcome::Absent));
        let out =
            run_compensate(true, "settings", "corr-1", &store_with("corr-1", "op-1"), &writer, &MockAuditSink::accepting())
                .await;
        assert_eq!(out, "nothing-to-undo");
    }
}
