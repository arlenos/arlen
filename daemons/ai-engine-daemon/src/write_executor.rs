//! Phase-1 graph-write executor: the [`Execute`] seam for the `graph.write`
//! proxy tool (`pi-agent-adoption.md` Phase 1, "atomic KG write socket").
//!
//! The daemon is the trusted runner for a graph mutation: the engine never holds
//! the KG socket, so it can only ASK to write via this seam, and the write runs
//! as a single atomic, op-id-keyed relation create against the Knowledge Daemon
//! (which independently authorises the caller's tier). The op id is returned in
//! the result so the [`Report`](ai_engine_contract::Report) verb can register the
//! compensation (an op-id-keyed retract) that undoes exactly this write - the
//! contract puts compensation registration on Report, not here, so this seam
//! stays the pure atomic write.
//!
//! The live writer is gated: a `graph.write` only reaches Execute after Authorize
//! returns Allow, which the `suggest_only` baseline never does (a mutation is a
//! proposal or a confirmation), and the executor-live flip is human-gated AND
//! must land together with the Report-side compensation registration. So Phase 1
//! wires the fail-closed [`DeniedWriter`]; [`UnixRelationWriter`] is the real
//! writer the executor-live cutover swaps in.

use crate::compensation::{CompensationStore, RetractReceipt};
use crate::dispatch::Executor;
use crate::session::SessionGrant;
use ai_engine_contract::{ContractError, Execute, ExecuteOutcome};
use arlen_ai_core::audit::behaviour_action_event;
use async_trait::async_trait;
use audit_proto::sink::AuditSink;
use os_sdk::graph::{RelationRetractOutcome, RelationWriteOutcome, UnixGraphClient};
use std::sync::{Arc, Mutex};

/// The proxy-tool name for an atomic graph relation write.
const GRAPH_WRITE_TOOL: &str = "graph.write";

/// Runs an atomic, op-id-keyed relation create in trusted Rust. The op id keys
/// the later compensating retract, so a writer impl must pass it through to the
/// Knowledge Daemon verbatim.
#[async_trait]
pub trait RelationWriter: Send + Sync {
    /// Create the relation `(from_type:from_id) -[relation_type]-> (to_type:to_id)`
    /// stamped with `op_id`. Returns whether the edge was created or already
    /// existed, or an error string.
    async fn create_relation(
        &self,
        from_type: &str,
        from_id: &str,
        to_type: &str,
        to_id: &str,
        relation_type: &str,
        op_id: &str,
    ) -> Result<RelationWriteOutcome, String>;

    /// Retract the op-id-stamped relation a `create_relation` made (the undo of a
    /// graph write). Op-id-keyed, so it removes exactly this write's own edge and
    /// nothing else; a retract of an edge that is already gone succeeds as
    /// [`RelationRetractOutcome::Absent`]. Returns the outcome or an error string.
    async fn retract_relation(
        &self,
        from_type: &str,
        from_id: &str,
        to_type: &str,
        to_id: &str,
        relation_type: &str,
        op_id: &str,
    ) -> Result<RelationRetractOutcome, String>;
}

/// A fail-closed writer for Phase 1: every write is refused. The real write +
/// the Report-side compensation registration land together at the executor-live
/// cutover; wiring the write executor over a runner now means that swap is a
/// one-line change (the writer), not a re-plumb of the Execute seam.
pub struct DeniedWriter;

#[async_trait]
impl RelationWriter for DeniedWriter {
    async fn create_relation(
        &self,
        _from_type: &str,
        _from_id: &str,
        _to_type: &str,
        _to_id: &str,
        _relation_type: &str,
        _op_id: &str,
    ) -> Result<RelationWriteOutcome, String> {
        Err("the engine daemon's graph write is wired at the executor-live cutover \
             (with its Report-side compensation)"
            .to_string())
    }

    async fn retract_relation(
        &self,
        _from_type: &str,
        _from_id: &str,
        _to_type: &str,
        _to_id: &str,
        _relation_type: &str,
        _op_id: &str,
    ) -> Result<RelationRetractOutcome, String> {
        Err("the engine daemon's graph retract is wired at the executor-live cutover".to_string())
    }
}

/// The real writer: an atomic relation create over the os-sdk graph client. Used
/// at the executor-live cutover; construction is lazy (the client dials per
/// write), so it never blocks on the Knowledge Daemon being up.
pub struct UnixRelationWriter {
    client: UnixGraphClient,
}

impl UnixRelationWriter {
    /// Build a writer pointed at the Knowledge Daemon socket.
    pub fn new(socket_path: impl Into<String>) -> Self {
        Self { client: UnixGraphClient::new(socket_path) }
    }
}

#[async_trait]
impl RelationWriter for UnixRelationWriter {
    async fn create_relation(
        &self,
        from_type: &str,
        from_id: &str,
        to_type: &str,
        to_id: &str,
        relation_type: &str,
        op_id: &str,
    ) -> Result<RelationWriteOutcome, String> {
        self.client
            .create_relation(from_type, from_id, to_type, to_id, relation_type, op_id)
            .await
            .map_err(|e| e.to_string())
    }

    async fn retract_relation(
        &self,
        from_type: &str,
        from_id: &str,
        to_type: &str,
        to_id: &str,
        relation_type: &str,
        op_id: &str,
    ) -> Result<RelationRetractOutcome, String> {
        self.client
            .retract_relation(from_type, from_id, to_type, to_id, relation_type, op_id)
            .await
            .map_err(|e| e.to_string())
    }
}

/// Mint a 128-bit op id (hex), the key that ties this write to its compensating
/// retract. A retry deriving a fresh id would create a second edge; tying the id
/// to the originating request is the executor-live refinement (the Knowledge
/// Daemon's create is idempotent per op id, so a same-id retry is `exists`).
fn mint_op_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Runs the `graph.write` proxy tool: a single atomic, op-id-keyed relation
/// create through a [`RelationWriter`], AUDITED before it applies and its
/// compensating retract registered at apply time.
///
/// The daemon does not defer the audit or the undo receipt to the engine's
/// optional `Report` verb: a non-cooperative engine that skips Report would
/// otherwise leave a committed write unaudited and un-undoable. So this executor
/// audits before the write (fail-closed: no ledger entry, no write) and registers
/// the op-id-keyed compensation from its OWN minted op id and validated args the
/// moment a write is created.
pub struct GraphWriteExecutor {
    writer: Arc<dyn RelationWriter>,
    /// The audit sink for the S13 audit-before-act entry; when absent (tests that
    /// only exercise the write mechanics) the write proceeds unaudited. The daemon
    /// always wires it.
    audit: Option<Arc<dyn AuditSink>>,
    /// The compensation store the op-id-keyed retract is registered into at apply
    /// time, so a created write is undoable regardless of the engine.
    compensation: Option<Arc<Mutex<CompensationStore>>>,
    /// The separate-uid undo signer's socket, when set: a created write's
    /// compensation is also submitted here (best-effort) so it survives a restart
    /// in the signed, HMAC-chained log. A signer that is absent or failing never
    /// fails the write - the in-memory `compensation` store covers the session.
    undo_signer: Option<std::path::PathBuf>,
    /// The executor-live gate, re-read PER CALL (fail-closed). A write applies only
    /// when it returns true, so a runtime `[agent] executor_live` change - either
    /// direction - takes effect on the next write with no restart, and a mid-flight
    /// disable is honoured at Execute even for an already-authorized proof. The gate
    /// reads the same switch at Authorize; this is the Execute-side companion.
    executor_live: fn() -> bool,
}

impl GraphWriteExecutor {
    /// Build the executor over a [`RelationWriter`] (the fail-closed
    /// [`DeniedWriter`] when AI is off, [`UnixRelationWriter`] when on), gated by
    /// the on-disk `[agent] executor_live` per call, with no audit sink or
    /// compensation store yet.
    pub fn new(writer: Arc<dyn RelationWriter>) -> Self {
        Self {
            writer,
            audit: None,
            compensation: None,
            undo_signer: None,
            executor_live: crate::engine_config::executor_live,
        }
    }

    /// Override the executor-live gate with a fixed source (tests, so a write does
    /// not depend on the developer's `ai.toml`). Production uses [`Self::new`],
    /// which reads `[agent] executor_live` per call.
    pub fn with_executor_live_gate(mut self, executor_live: fn() -> bool) -> Self {
        self.executor_live = executor_live;
        self
    }

    /// Attach the audit sink so a write is recorded content-free BEFORE it applies
    /// (S13 audit-before-act); a ledger that cannot record the intent refuses the
    /// write.
    pub fn with_audit(mut self, audit: Arc<dyn AuditSink>) -> Self {
        self.audit = Some(audit);
        self
    }

    /// Attach the compensation store so a created write's op-id-keyed retract is
    /// registered at apply time, from the daemon's own op id (not an engine report).
    pub fn with_compensation(mut self, store: Arc<Mutex<CompensationStore>>) -> Self {
        self.compensation = Some(store);
        self
    }

    /// Attach the undo-signer socket so a created write's compensation is also
    /// persisted to the signed log (best-effort, in addition to the in-memory
    /// store), surviving a restart.
    pub fn with_undo_signer(mut self, socket: std::path::PathBuf) -> Self {
        self.undo_signer = Some(socket);
        self
    }
}

#[async_trait]
impl Executor for GraphWriteExecutor {
    async fn execute(&self, req: &Execute, _grant: &SessionGrant) -> ExecuteOutcome {
        if req.tool_name != GRAPH_WRITE_TOOL {
            return ExecuteOutcome::Error {
                code: ContractError::UnknownTool,
                message: format!("{} is not a graph-write tool this daemon runs", req.tool_name),
            };
        }
        // Executor-live gate, re-read PER CALL (fail-closed). Even a valid,
        // already-authorized HIGH-1 proof cannot apply a write once executor_live
        // is off: a mid-flight disable is honoured here at Execute, closing the
        // window between Authorize (which minted the proof while it was on) and
        // this Execute. Nothing is audited or written when it is off.
        if !(self.executor_live)() {
            return ExecuteOutcome::Error {
                code: ContractError::ExecutionFailed,
                message: "graph.write is not permitted: the executor is not live".to_string(),
            };
        }
        // The five relation fields are all required; a missing one is a malformed
        // request, never guessed.
        let field = |k: &str| req.tool_input.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let (Some(from_type), Some(from_id), Some(to_type), Some(to_id), Some(relation_type)) = (
            field("from_type"),
            field("from_id"),
            field("to_type"),
            field("to_id"),
            field("relation_type"),
        ) else {
            return ExecuteOutcome::Error {
                code: ContractError::InvalidArguments,
                message: "graph.write needs string from_type, from_id, to_type, to_id, \
                          relation_type in the tool input"
                    .to_string(),
            };
        };
        let op_id = match mint_op_id() {
            Ok(id) => id,
            Err(e) => {
                return ExecuteOutcome::Error {
                    code: ContractError::ExecutionFailed,
                    message: format!("could not mint an op id: {e}"),
                }
            }
        };
        // S13 audit-before-act: record the write intent content-free BEFORE it
        // applies, correlated by the daemon's own op id. Fail closed - a ledger
        // that cannot record the intent refuses the write, so an autonomous write
        // is never invisible to the audit trail (the review's HIGH-2: the daemon
        // must not defer this to the engine's optional Report).
        if let Some(audit) = &self.audit {
            let event = behaviour_action_event(GRAPH_WRITE_TOOL, "graph-write", &op_id);
            if audit.submit(event).await.is_err() {
                return ExecuteOutcome::Error {
                    code: ContractError::ExecutionFailed,
                    message: "audit ledger unavailable; graph.write refused".to_string(),
                };
            }
        }
        match self
            .writer
            .create_relation(&from_type, &from_id, &to_type, &to_id, &relation_type, &op_id)
            .await
        {
            Ok(outcome) => {
                let created = matches!(outcome, RelationWriteOutcome::Created);
                // Register the op-id-keyed compensation AUTHORITATIVELY at apply
                // time, from the daemon's own minted op id + the args it validated
                // - never from an engine-supplied Report (which could omit it,
                // leaving the write un-undoable, or forge a wrong receipt). Only a
                // genuine create needs an undo; an already-existing edge created
                // nothing.
                if created {
                    let receipt = RetractReceipt::for_write(
                        &op_id,
                        &from_type,
                        &from_id,
                        &to_type,
                        &to_id,
                        &relation_type,
                    );
                    if let Some(store) = &self.compensation {
                        if let Ok(mut s) = store.lock() {
                            s.register(op_id.clone(), receipt.clone());
                        }
                    }
                    // Best-effort: also persist the compensation to the signed undo
                    // log so it survives a restart. A signer that is absent or
                    // failing must never fail the write - the in-memory store above
                    // already covers the session. No gate correlation id reaches the
                    // executor, so the durable entry keys the retract on the write's
                    // own op id.
                    if let Some(signer) = &self.undo_signer {
                        if signer.exists() {
                            let entry = receipt.to_undo_entry(op_id.as_str());
                            if let Err(e) =
                                crate::undo_signer::submit_created(signer, &entry).await
                            {
                                tracing::debug!(
                                    "undo signer submit failed (in-memory store still covers it): {e}"
                                );
                            }
                        }
                    }
                }
                // The result still carries the op id + relation for the Report path
                // (content screening); the audit + compensation no longer depend on
                // it.
                ExecuteOutcome::Ok {
                    result: serde_json::json!({
                        "op_id": op_id,
                        "created": created,
                        "from_type": from_type,
                        "from_id": from_id,
                        "to_type": to_type,
                        "to_id": to_id,
                        "relation_type": relation_type,
                    }),
                }
            }
            Err(message) => ExecuteOutcome::Error { code: ContractError::ExecutionFailed, message },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_engine_contract::{CapabilityContext, ReadTier};

    fn grant() -> SessionGrant {
        SessionGrant {
            capability_context: CapabilityContext { generic_tools: vec![], proxy_tools: vec![] },
            project_anchor: None,
            read_tier: ReadTier::None,
            externally_triggered: false,
            pid: 1,
        }
    }

    fn write(input: serde_json::Value) -> Execute {
        Execute { tool_name: GRAPH_WRITE_TOOL.to_string(), tool_input: input, proof: None }
    }

    fn valid_input() -> serde_json::Value {
        serde_json::json!({
            "from_type": "File", "from_id": "/a.rs",
            "to_type": "Project", "to_id": "proj-1",
            "relation_type": "FILE_PART_OF",
        })
    }

    /// A writer that records the op id it was passed and returns a canned result.
    struct MockWriter {
        result: Result<RelationWriteOutcome, String>,
    }
    #[async_trait]
    impl RelationWriter for MockWriter {
        async fn create_relation(
            &self,
            _ft: &str,
            _fi: &str,
            _tt: &str,
            _ti: &str,
            _rt: &str,
            _op: &str,
        ) -> Result<RelationWriteOutcome, String> {
            self.result.clone()
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
            Ok(RelationRetractOutcome::Retracted)
        }
    }

    /// A write executor over `writer` with the executor-live gate forced ON, so a
    /// test's write does not depend on the developer's ai.toml.
    fn live_exec(writer: Arc<dyn RelationWriter>) -> GraphWriteExecutor {
        GraphWriteExecutor::new(writer).with_executor_live_gate(|| true)
    }

    #[tokio::test]
    async fn a_valid_write_runs_and_returns_an_op_id() {
        let exec = live_exec(Arc::new(MockWriter {
            result: Ok(RelationWriteOutcome::Created),
        }));
        match exec.execute(&write(valid_input()), &grant()).await {
            ExecuteOutcome::Ok { result } => {
                assert_eq!(result["created"], true);
                assert_eq!(result["relation_type"], "FILE_PART_OF");
                let op = result["op_id"].as_str().unwrap();
                assert_eq!(op.len(), 32, "a 128-bit op id is 32 hex chars");
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_created_write_audits_before_it_applies_and_registers_its_compensation() {
        // The daemon audits the write BEFORE it applies and registers the op-id-
        // keyed retract at apply time, from its OWN op id - not the engine's Report.
        use audit_proto::sink::MockAuditSink;
        let audit = Arc::new(MockAuditSink::accepting());
        let store = Arc::new(Mutex::new(CompensationStore::new(8)));
        let exec = live_exec(Arc::new(MockWriter {
            result: Ok(RelationWriteOutcome::Created),
        }))
        .with_audit(audit.clone())
        .with_compensation(store.clone());
        let op = match exec.execute(&write(valid_input()), &grant()).await {
            ExecuteOutcome::Ok { result } => result["op_id"].as_str().unwrap().to_string(),
            other => panic!("expected Ok, got {other:?}"),
        };
        // One audit entry recorded, and the compensation is keyed by the daemon's
        // op id and targets exactly the written edge.
        assert_eq!(audit.recorded().await.len(), 1);
        let s = store.lock().unwrap();
        let receipt = s.get(&op).expect("compensation registered under the op id");
        assert_eq!(receipt.op_id, op);
        assert_eq!(receipt.from_id, "/a.rs");
        assert_eq!(receipt.relation_type, "FILE_PART_OF");
    }

    #[tokio::test]
    async fn an_audit_down_refuses_the_write_fail_closed() {
        // S13: if the ledger cannot record the intent, the write is refused and
        // nothing applies - so an autonomous write is never invisible to the audit.
        use audit_proto::sink::MockAuditSink;
        let store = Arc::new(Mutex::new(CompensationStore::new(8)));
        let exec = live_exec(Arc::new(MockWriter {
            result: Ok(RelationWriteOutcome::Created),
        }))
        .with_audit(Arc::new(MockAuditSink::failing()))
        .with_compensation(store.clone());
        match exec.execute(&write(valid_input()), &grant()).await {
            ExecuteOutcome::Error { code, .. } => assert_eq!(code, ContractError::ExecutionFailed),
            other => panic!("expected fail-closed Error, got {other:?}"),
        }
        // The write never ran, so no compensation was registered.
        assert!(store.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_already_existing_write_registers_no_compensation() {
        // Only a genuine create needs an undo; an already-present edge created
        // nothing, so there is nothing to retract.
        let store = Arc::new(Mutex::new(CompensationStore::new(8)));
        let exec = live_exec(Arc::new(MockWriter {
            result: Ok(RelationWriteOutcome::AlreadyExists),
        }))
        .with_compensation(store.clone());
        let out = exec.execute(&write(valid_input()), &grant()).await;
        assert!(matches!(out, ExecuteOutcome::Ok { .. }));
        assert!(store.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_missing_field_is_invalid_arguments() {
        let exec = live_exec(Arc::new(MockWriter {
            result: Ok(RelationWriteOutcome::Created),
        }));
        let mut input = valid_input();
        input.as_object_mut().unwrap().remove("to_id");
        assert!(matches!(
            exec.execute(&write(input), &grant()).await,
            ExecuteOutcome::Error { code: ContractError::InvalidArguments, .. },
        ));
    }

    #[tokio::test]
    async fn a_non_write_tool_is_unknown() {
        let exec = live_exec(Arc::new(MockWriter {
            result: Ok(RelationWriteOutcome::Created),
        }));
        let req = Execute { tool_name: "graph.read".into(), tool_input: valid_input(), proof: None };
        assert!(matches!(
            exec.execute(&req, &grant()).await,
            ExecuteOutcome::Error { code: ContractError::UnknownTool, .. },
        ));
    }

    #[tokio::test]
    async fn the_denied_writer_refuses_a_valid_write() {
        // Phase-1 fail-closed: a fully-formed write is refused (the live write +
        // compensation land at the executor-live cutover).
        let exec = live_exec(Arc::new(DeniedWriter));
        assert!(matches!(
            exec.execute(&write(valid_input()), &grant()).await,
            ExecuteOutcome::Error { code: ContractError::ExecutionFailed, .. },
        ));
    }

    #[tokio::test]
    async fn a_write_is_refused_when_executor_live_is_off() {
        // The per-call executor-live gate: with executor_live off, even a valid
        // write is refused fail-closed - nothing is audited or written.
        use audit_proto::sink::MockAuditSink;
        let audit = Arc::new(MockAuditSink::accepting());
        let store = Arc::new(Mutex::new(CompensationStore::new(8)));
        let exec = GraphWriteExecutor::new(Arc::new(MockWriter {
            result: Ok(RelationWriteOutcome::Created),
        }))
        .with_executor_live_gate(|| false)
        .with_audit(audit.clone())
        .with_compensation(store.clone());
        assert!(matches!(
            exec.execute(&write(valid_input()), &grant()).await,
            ExecuteOutcome::Error { code: ContractError::ExecutionFailed, .. },
        ));
        // The gate short-circuits before the audit + the write.
        assert_eq!(audit.recorded().await.len(), 0);
        assert!(store.lock().unwrap().is_empty());
    }
}
