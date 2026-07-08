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

/// Render the completed-actions JSON array from the compensation store, oldest
/// first. Pure and testable; the D-Bus method just locks the store and calls this.
fn completed_actions_json(store: &CompensationStore) -> String {
    let actions: Vec<CompletedAction> = store
        .entries()
        .into_iter()
        .map(|(id, r)| CompletedAction {
            id: id.to_string(),
            op_id: r.op_id.clone(),
            relation: r.relation_type.clone(),
            from: format!("{}/{}", r.from_type, r.from_id),
            to: format!("{}/{}", r.to_type, r.to_id),
        })
        .collect();
    serde_json::to_string(&actions).unwrap_or_else(|_| "[]".to_string())
}

/// The undo verdict, kept as a small helper so `compensate`'s flow is unit-tested
/// without a live bus. The wire is the string; this names the branches.
fn compensate_outcome_wire(outcome: RelationRetractOutcome) -> &'static str {
    match outcome {
        RelationRetractOutcome::Retracted => "retracted",
        RelationRetractOutcome::Absent => "nothing-to-undo",
    }
}

/// Run one `compensate`: undo the executed write recorded under `correlation_id`.
/// Fail-closed at every step, in order: refuse unless the executor is live; refuse
/// an unknown receipt; AUDIT BEFORE the retract (S13) and refuse if the audit
/// ledger will not record it (never an unaudited destructive act); then retract
/// exactly this write's own op-id-stamped edge. The receipt is cloned out and the
/// lock dropped before the awaits.
async fn run_compensate(
    executor_live: bool,
    correlation_id: &str,
    compensation: &Mutex<CompensationStore>,
    writer: &dyn RelationWriter,
    audit: &dyn AuditSink,
) -> String {
    if !executor_live {
        return "not-enabled".to_string();
    }
    let receipt = {
        let store = match compensation.lock() {
            Ok(s) => s,
            Err(_) => return "error: compensation store unavailable".to_string(),
        };
        match store.get(correlation_id) {
            Some(r) => r.clone(),
            None => return "no-such-receipt".to_string(),
        }
    };
    // Audit-before-act, fail-closed: an undo that cannot be recorded does not run.
    let event = behaviour_action_event("compensate", "retract-relation", correlation_id);
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
    async fn completed_actions(&self) -> String {
        self.compensation
            .lock()
            .map(|store| completed_actions_json(&store))
            .unwrap_or_else(|_| "[]".to_string())
    }

    /// Undo a completed action: retract the graph write recorded under
    /// `correlation_id`. Reversible curation is autonomous, so this is the user's
    /// after-the-fact undo. Re-reads `executor_live` live (a runtime flip to
    /// suggest-mode refuses the undo fail-safe); fail-closed on an unknown receipt,
    /// an unrecordable audit, or a retract error. Returns `retracted`,
    /// `nothing-to-undo` (the edge was already gone), `no-such-receipt`,
    /// `not-enabled` (suggest-mode) or `error: <reason>`.
    #[zbus(name = "compensate")]
    async fn compensate(&self, correlation_id: String) -> String {
        run_compensate(
            engine_config::executor_live(),
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
        let json = completed_actions_json(&store);
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
        assert_eq!(completed_actions_json(&CompensationStore::new(8)), "[]");
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

    #[tokio::test]
    async fn suggest_mode_refuses_the_undo_without_touching_the_store_or_writer() {
        let writer = RetractMock::new(Ok(RelationRetractOutcome::Retracted));
        let out =
            run_compensate(false, "corr-1", &store_with("corr-1", "op-1"), &writer, &MockAuditSink::accepting())
                .await;
        assert_eq!(out, "not-enabled");
        assert!(!writer.retract_called.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn an_unknown_receipt_is_refused() {
        let writer = RetractMock::new(Ok(RelationRetractOutcome::Retracted));
        let out = run_compensate(
            true,
            "missing",
            &Mutex::new(CompensationStore::new(8)),
            &writer,
            &MockAuditSink::accepting(),
        )
        .await;
        assert_eq!(out, "no-such-receipt");
        assert!(!writer.retract_called.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn an_unrecordable_audit_refuses_the_undo_and_never_retracts() {
        let writer = RetractMock::new(Ok(RelationRetractOutcome::Retracted));
        let out =
            run_compensate(true, "corr-1", &store_with("corr-1", "op-1"), &writer, &MockAuditSink::failing())
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
        let out = run_compensate(true, "corr-1", &store, &writer, &MockAuditSink::accepting()).await;
        assert_eq!(out, "retracted");
        assert!(writer.retract_called.load(Ordering::Relaxed));
        // The undone receipt is dropped so completed_actions won't re-offer it.
        assert!(store.lock().unwrap().get("corr-1").is_none());
    }

    #[tokio::test]
    async fn an_already_gone_edge_reports_nothing_to_undo() {
        let writer = RetractMock::new(Ok(RelationRetractOutcome::Absent));
        let out =
            run_compensate(true, "corr-1", &store_with("corr-1", "op-1"), &writer, &MockAuditSink::accepting())
                .await;
        assert_eq!(out, "nothing-to-undo");
    }
}
