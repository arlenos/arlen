//! Phase-1 graph-read executor: the [`Execute`] seam for the `graph.read` proxy
//! tool, re-pointed to the existing ai-core [`QueryRunner`]
//! (`pi-agent-adoption.md` Phase 1, "graph_query read-scope incl. GAP-21 is
//! re-pointed").
//!
//! The daemon runs the read in trusted Rust bounded by the session's scope
//! (resolved from its grant via [`grant_to_query_scope`], GAP-21-anchored); the
//! engine never touches the graph directly. Only `graph.read` is wired here: it
//! is a read, so it needs no gate-class classification, no compensation, and not
//! the human-gated executor-live flip. The write proxy tools land with the
//! executor + compensation + atomic-KG-write re-pointing.

use crate::capability_map::grant_to_query_scope;
use crate::dispatch::Executor;
use crate::session::SessionGrant;
use ai_engine_contract::{ContractError, Execute, ExecuteOutcome};
use arlen_ai_core::graph_query::QueryScope;
use arlen_ai_core::graph_schema::GraphSchema;
use arlen_ai_core::pipeline::{QueryRunner, RunFailure};
use async_trait::async_trait;
use std::sync::Arc;

/// The proxy-tool name for a bounded graph read.
const GRAPH_READ_TOOL: &str = "graph.read";

/// Runs the `graph.read` proxy tool through the existing scoped [`QueryRunner`].
pub struct GraphReadExecutor {
    runner: Arc<dyn QueryRunner>,
    schema: GraphSchema,
}

impl GraphReadExecutor {
    /// Build the executor over a [`QueryRunner`] (the production
    /// `CypherPipeline` in the daemon binary, a mock in tests).
    pub fn new(runner: Arc<dyn QueryRunner>) -> Self {
        Self { runner, schema: GraphSchema::knowledge_graph() }
    }
}

#[async_trait]
impl Executor for GraphReadExecutor {
    async fn execute(&self, req: &Execute, grant: &SessionGrant) -> ExecuteOutcome {
        if req.tool_name != GRAPH_READ_TOOL {
            return ExecuteOutcome::Error {
                code: ContractError::UnknownTool,
                message: format!("{} is not a graph-read tool this daemon runs", req.tool_name),
            };
        }
        let Some(prompt) = req.tool_input.get("query").and_then(|v| v.as_str()) else {
            return ExecuteOutcome::Error {
                code: ContractError::InvalidArguments,
                message: "graph.read needs a 'query' string in the tool input".to_string(),
            };
        };
        // Bound the read to the session's grant (GAP-21-anchored). An empty
        // scope means no graph-read access at all: refuse without running the
        // query, never burning a provider call on a query that cannot pass.
        let scope = grant_to_query_scope(grant, &self.schema);
        if scope.is_empty() {
            return ExecuteOutcome::Error {
                code: ContractError::PermissionDenied,
                message: "the session has no graph read scope".to_string(),
            };
        }
        match self.runner.run_query(prompt, &scope).await {
            Ok(answer) => ExecuteOutcome::Ok { result: serde_json::json!({ "answer": answer }) },
            Err(failure) => ExecuteOutcome::Error {
                code: ContractError::ExecutionFailed,
                message: format!("{}: {}", failure.code, failure.reason),
            },
        }
    }
}

/// A fail-closed [`QueryRunner`] for the side-by-side phase.
///
/// The live read runner is the proxied [`CypherPipeline`], which forwards LLM
/// traffic only over a connection that owns an ai-proxy-authorized bus name
/// (`org.arlen.AI1`/`AIAgent1`). The engine daemon cannot hold one of those
/// while the old ai-daemon owns it, so a `graph.read` is refused with a clear
/// reason until the Phase-2 cutover swaps the real pipeline in. Wiring the read
/// executor over a runner now means that swap is a one-line change (the runner),
/// not a re-plumb of the Execute seam.
pub struct DeniedRunner;

#[async_trait]
impl QueryRunner for DeniedRunner {
    async fn run_query(&self, _prompt: &str, _scope: &QueryScope) -> Result<String, RunFailure> {
        Err(RunFailure {
            code: "provider-unavailable".to_string(),
            reason: "the engine daemon's read provider is wired at the Phase-2 cutover".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_engine_contract::{CapabilityContext, ReadTier};
    use arlen_ai_core::graph_query::QueryScope;
    use arlen_ai_core::pipeline::RunFailure;
    use std::sync::Mutex;

    /// A `QueryRunner` that records the (prompt, had-anchor) of its last call
    /// and returns a canned result, so a test can assert both the scope the
    /// executor built and that an empty scope short-circuits before any call.
    struct MockRunner {
        seen: Mutex<Option<(String, bool)>>,
        result: Result<String, RunFailure>,
    }
    impl MockRunner {
        fn ok(answer: &str) -> Self {
            Self { seen: Mutex::new(None), result: Ok(answer.to_string()) }
        }
        fn failing() -> Self {
            Self {
                seen: Mutex::new(None),
                result: Err(RunFailure { code: "graph-error".into(), reason: "boom".into() }),
            }
        }
        fn was_called(&self) -> bool {
            self.seen.lock().unwrap().is_some()
        }
        fn last_had_anchor(&self) -> Option<bool> {
            self.seen.lock().unwrap().as_ref().map(|(_, a)| *a)
        }
    }
    #[async_trait]
    impl QueryRunner for MockRunner {
        async fn run_query(&self, prompt: &str, scope: &QueryScope) -> Result<String, RunFailure> {
            *self.seen.lock().unwrap() = Some((prompt.to_string(), scope.project_anchor().is_some()));
            self.result.clone()
        }
    }

    fn grant(read_tier: ReadTier, anchor: Option<&str>) -> SessionGrant {
        SessionGrant {
            capability_context: CapabilityContext { generic_tools: vec![], proxy_tools: vec![] },
            project_anchor: anchor.map(str::to_string),
            read_tier,
            pid: 1,
        }
    }

    fn read(query: serde_json::Value) -> Execute {
        Execute { tool_name: GRAPH_READ_TOOL.to_string(), tool_input: query }
    }

    #[tokio::test]
    async fn a_scoped_read_runs_through_the_runner() {
        let runner = Arc::new(MockRunner::ok("3 files"));
        let exec = GraphReadExecutor::new(runner.clone());
        let outcome = exec
            .execute(&read(serde_json::json!({ "query": "how many files" })), &grant(ReadTier::Full, None))
            .await;
        match outcome {
            ExecuteOutcome::Ok { result } => assert_eq!(result["answer"], "3 files"),
            other => panic!("expected Ok, got {other:?}"),
        }
        assert!(runner.was_called());
    }

    #[tokio::test]
    async fn the_denied_runner_refuses_a_scoped_read() {
        // The side-by-side fallback: a fully-scoped read still reaches the runner
        // but is refused (the live provider lands at the Phase-2 cutover), so the
        // read executor maps it to ExecutionFailed rather than the blanket
        // Phase-0 Unavailable placeholder.
        let exec = GraphReadExecutor::new(Arc::new(DeniedRunner));
        let outcome = exec
            .execute(&read(serde_json::json!({ "query": "how many files" })), &grant(ReadTier::Full, None))
            .await;
        assert!(matches!(
            outcome,
            ExecuteOutcome::Error { code: ContractError::ExecutionFailed, .. },
        ));
    }

    #[tokio::test]
    async fn an_empty_scope_is_refused_without_running() {
        let runner = Arc::new(MockRunner::ok("never"));
        let exec = GraphReadExecutor::new(runner.clone());
        let outcome = exec
            .execute(&read(serde_json::json!({ "query": "x" })), &grant(ReadTier::None, None))
            .await;
        assert!(matches!(
            outcome,
            ExecuteOutcome::Error { code: ContractError::PermissionDenied, .. },
        ));
        assert!(!runner.was_called(), "an empty scope must not reach the runner");
    }

    #[tokio::test]
    async fn a_project_scoped_read_is_anchored() {
        let runner = Arc::new(MockRunner::ok("ok"));
        let exec = GraphReadExecutor::new(runner.clone());
        // ReadTier::Standard -> ProjectScoped; with an anchor the scope carries it.
        let _ = exec
            .execute(&read(serde_json::json!({ "query": "x" })), &grant(ReadTier::Standard, Some("p1")))
            .await;
        assert_eq!(runner.last_had_anchor(), Some(true), "GAP-21: the read is anchored");
    }

    #[tokio::test]
    async fn a_non_graph_read_tool_is_unknown() {
        let runner = Arc::new(MockRunner::ok("x"));
        let exec = GraphReadExecutor::new(runner.clone());
        let outcome = exec
            .execute(
                &Execute { tool_name: "graph.write".into(), tool_input: serde_json::json!({}) },
                &grant(ReadTier::Full, None),
            )
            .await;
        assert!(matches!(
            outcome,
            ExecuteOutcome::Error { code: ContractError::UnknownTool, .. },
        ));
        assert!(!runner.was_called());
    }

    #[tokio::test]
    async fn a_missing_query_is_invalid_arguments() {
        let runner = Arc::new(MockRunner::ok("x"));
        let exec = GraphReadExecutor::new(runner.clone());
        let outcome = exec
            .execute(&read(serde_json::json!({})), &grant(ReadTier::Full, None))
            .await;
        assert!(matches!(
            outcome,
            ExecuteOutcome::Error { code: ContractError::InvalidArguments, .. },
        ));
        assert!(!runner.was_called());
    }

    #[tokio::test]
    async fn a_runner_failure_maps_to_execution_failed() {
        let runner = Arc::new(MockRunner::failing());
        let exec = GraphReadExecutor::new(runner);
        let outcome = exec
            .execute(&read(serde_json::json!({ "query": "x" })), &grant(ReadTier::Full, None))
            .await;
        assert!(matches!(
            outcome,
            ExecuteOutcome::Error { code: ContractError::ExecutionFailed, .. },
        ));
    }
}
