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

use crate::dispatch::Executor;
use crate::session::SessionGrant;
use ai_engine_contract::{ContractError, Execute, ExecuteOutcome};
use async_trait::async_trait;
use os_sdk::graph::{RelationWriteOutcome, UnixGraphClient};
use std::sync::Arc;

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
/// create through a [`RelationWriter`].
pub struct GraphWriteExecutor {
    writer: Arc<dyn RelationWriter>,
}

impl GraphWriteExecutor {
    /// Build the executor over a [`RelationWriter`] (the fail-closed
    /// [`DeniedWriter`] in Phase 1, [`UnixRelationWriter`] at the cutover).
    pub fn new(writer: Arc<dyn RelationWriter>) -> Self {
        Self { writer }
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
        match self
            .writer
            .create_relation(&from_type, &from_id, &to_type, &to_id, &relation_type, &op_id)
            .await
        {
            Ok(outcome) => {
                let created = matches!(outcome, RelationWriteOutcome::Created);
                // The result carries the op id + the written relation so the
                // Report verb can register the compensating retract for it.
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
    }

    #[tokio::test]
    async fn a_valid_write_runs_and_returns_an_op_id() {
        let exec = GraphWriteExecutor::new(Arc::new(MockWriter {
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
    async fn a_missing_field_is_invalid_arguments() {
        let exec = GraphWriteExecutor::new(Arc::new(MockWriter {
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
        let exec = GraphWriteExecutor::new(Arc::new(MockWriter {
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
        let exec = GraphWriteExecutor::new(Arc::new(DeniedWriter));
        assert!(matches!(
            exec.execute(&write(valid_input()), &grant()).await,
            ExecuteOutcome::Error { code: ContractError::ExecutionFailed, .. },
        ));
    }
}
