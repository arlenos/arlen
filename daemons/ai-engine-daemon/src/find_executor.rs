//! The `graph.find` proxy tool: keyword-to-ranked, no model in the loop.
//!
//! The read verb was one verb doing three jobs (`pi-gate-class-registry.md`).
//! `graph.ask` interprets a question, which costs a provider round trip and can
//! be wrong about what was asked; a great deal of what the engine actually wants
//! is "which of my things match this word", and the Knowledge Daemon already
//! answers exactly that on `0x03` - FTS5 keyword search fused with a bounded
//! graph expansion, ranked, and filtered to the caller's readable labels before
//! it leaves the daemon (RS-R1).
//!
//! So this verb is free and literal: no provider call, no generated Cypher, no
//! clause structure handed to the engine. It answers node ids, best match first.
//! The ids are the graph's own - a `File` id IS its path, which is the case the
//! verb exists for - and anything the engine wants to know beyond the id is a
//! separate, gated read.
//!
//! Its gate class is [`GateClass::Read`](crate::capability_map::GateClass), like
//! the ask verb: the daemon runs it in trusted Rust, and the scope it is bounded
//! by is the caller's own, enforced where the search happens rather than here.

use crate::dispatch::Executor;
use crate::session::SessionGrant;
use ai_engine_contract::{ContractError, Execute, ExecuteOutcome};
use async_trait::async_trait;
use std::sync::Arc;

/// The proxy-tool name for a deterministic keyword search.
const GRAPH_FIND_TOOL: &str = "graph.find";

/// The proxy-tool name for enumerating one label's handles.
const GRAPH_LIST_TOOL: &str = "graph.list";

/// The default number of results, when the call names none.
const DEFAULT_LIMIT: u32 = 10;

/// The most results one call may ask for. The daemon clamps too; this keeps an
/// absurd request from travelling at all.
const MAX_LIMIT: u32 = 100;

/// The same, for the enumeration verb; it answers handles, which are smaller.
const MAX_LIST_LIMIT: u32 = 200;

/// How many handles when the call names no limit.
const DEFAULT_LIST_LIMIT: u32 = 50;

/// Why a find did not answer. Deliberately coarse: the tool reports that the
/// search did not run, never what the store holds.
#[derive(Debug)]
pub enum FindFailure {
    /// The knowledge daemon could not be reached.
    Unreachable(String),
    /// The daemon refused the caller.
    Denied,
    /// The daemon answered, but not with a result.
    Rejected(String),
}

/// The keyword search the executor runs. A seam so the executor is testable
/// without a live Knowledge Daemon.
#[async_trait]
pub trait Finder: Send + Sync {
    /// Node ids matching `query`, best match first, at most `limit` of them.
    async fn find(&self, query: &str, limit: u32) -> Result<Vec<String>, FindFailure>;

    /// The handles of one enumerable label: an id and a display name per row.
    async fn list(
        &self,
        label: &str,
        limit: u32,
    ) -> Result<Vec<os_sdk::graph::Identity>, FindFailure>;
}

/// The production [`Finder`]: the Knowledge Daemon's `0x03` retrieve op through
/// the os-sdk client.
pub struct SocketFinder {
    client: os_sdk::graph::UnixGraphClient,
}

impl SocketFinder {
    /// Point a finder at the Knowledge Daemon socket. Construction is lazy (the
    /// client dials per call), so this never blocks on the daemon being up.
    pub fn new(socket_path: impl Into<String>) -> Self {
        Self { client: os_sdk::graph::UnixGraphClient::new(socket_path) }
    }
}

/// Map one os-sdk error onto the coarse failure this tool reports.
fn as_failure(e: os_sdk::graph::QueryError) -> FindFailure {
    match e {
        os_sdk::graph::QueryError::ConnectionFailed(m) => FindFailure::Unreachable(m),
        os_sdk::graph::QueryError::PermissionDenied => FindFailure::Denied,
        os_sdk::graph::QueryError::InvalidQuery(m) => FindFailure::Rejected(m),
    }
}

#[async_trait]
impl Finder for SocketFinder {
    async fn find(&self, query: &str, limit: u32) -> Result<Vec<String>, FindFailure> {
        self.client.retrieve(query, i64::from(limit)).await.map_err(as_failure)
    }

    async fn list(
        &self,
        label: &str,
        limit: u32,
    ) -> Result<Vec<os_sdk::graph::Identity>, FindFailure> {
        self.client.list_identities(label, i64::from(limit)).await.map_err(as_failure)
    }
}

/// Runs the `graph.find` proxy tool.
pub struct GraphFindExecutor {
    finder: Arc<dyn Finder>,
}

impl GraphFindExecutor {
    /// Build the executor over a [`Finder`] (the [`SocketFinder`] in the daemon
    /// binary, a mock in tests).
    pub fn new(finder: Arc<dyn Finder>) -> Self {
        Self { finder }
    }
}

#[async_trait]
impl Executor for GraphFindExecutor {
    async fn execute(&self, req: &Execute, _grant: &SessionGrant) -> ExecuteOutcome {
        if req.tool_name == GRAPH_LIST_TOOL {
            return self.list(req).await;
        }
        if req.tool_name != GRAPH_FIND_TOOL {
            return ExecuteOutcome::Error {
                code: ContractError::UnknownTool,
                message: format!("{} is not the graph-find tool", req.tool_name),
            };
        }
        // A blank query is refused rather than run. The daemon answers an empty
        // search with an empty list, so running it would spend a round trip to
        // tell the engine nothing, and "nothing matched" reads as a fact about
        // the store rather than about the call.
        let query = req.tool_input.get("query").and_then(|v| v.as_str()).unwrap_or("").trim();
        if query.is_empty() {
            return ExecuteOutcome::Error {
                code: ContractError::InvalidArguments,
                message: "graph.find needs a non-empty 'query' string".to_string(),
            };
        }
        let limit = req
            .tool_input
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map(|n| n.min(u64::from(MAX_LIMIT)) as u32)
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_LIMIT);

        match self.finder.find(query, limit).await {
            Ok(ids) => ExecuteOutcome::Ok { result: serde_json::json!({ "ids": ids }) },
            Err(FindFailure::Denied) => ExecuteOutcome::Error {
                code: ContractError::PermissionDenied,
                message: "the session may not search the knowledge graph".to_string(),
            },
            Err(FindFailure::Unreachable(m)) => ExecuteOutcome::Error {
                code: ContractError::ExecutionFailed,
                message: format!("the knowledge daemon is unreachable: {m}"),
            },
            Err(FindFailure::Rejected(m)) => ExecuteOutcome::Error {
                code: ContractError::ExecutionFailed,
                message: format!("the search was refused: {m}"),
            },
        }
    }
}

impl GraphFindExecutor {
    /// The enumeration verb. Which labels answer is the daemon's list, not this
    /// executor's: it forwards the name and reports what comes back, so the
    /// judgement about what may be enumerated is made in one place.
    async fn list(&self, req: &Execute) -> ExecuteOutcome {
        let label = req.tool_input.get("label").and_then(|v| v.as_str()).unwrap_or("").trim();
        if label.is_empty() {
            return ExecuteOutcome::Error {
                code: ContractError::InvalidArguments,
                message: "graph.list needs a 'label' string".to_string(),
            };
        }
        let limit = req
            .tool_input
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map(|n| n.min(u64::from(MAX_LIST_LIMIT)) as u32)
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_LIST_LIMIT);

        match self.finder.list(label, limit).await {
            Ok(rows) => ExecuteOutcome::Ok {
                result: serde_json::json!({
                    "items": rows
                        .iter()
                        .map(|i| serde_json::json!({ "id": i.id, "name": i.name }))
                        .collect::<Vec<_>>()
                }),
            },
            Err(FindFailure::Denied) => ExecuteOutcome::Error {
                code: ContractError::PermissionDenied,
                message: "the session may not list that label".to_string(),
            },
            Err(FindFailure::Unreachable(m)) => ExecuteOutcome::Error {
                code: ContractError::ExecutionFailed,
                message: format!("the knowledge daemon is unreachable: {m}"),
            },
            Err(FindFailure::Rejected(m)) => ExecuteOutcome::Error {
                code: ContractError::ExecutionFailed,
                message: format!("the list was refused: {m}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_engine_contract::{CapabilityContext, ReadTier};
    use std::sync::Mutex;

    /// Records what the executor asked for, so a test can assert the limit it
    /// resolved rather than only the answer it passed through.
    struct MockFinder {
        seen: Mutex<Option<(String, u32)>>,
        result: Mutex<Option<Result<Vec<String>, FindFailure>>>,
    }

    impl MockFinder {
        fn ok(ids: &[&str]) -> Self {
            Self {
                seen: Mutex::new(None),
                result: Mutex::new(Some(Ok(ids.iter().map(|s| s.to_string()).collect()))),
            }
        }
        fn failing(f: FindFailure) -> Self {
            Self { seen: Mutex::new(None), result: Mutex::new(Some(Err(f))) }
        }
    }

    #[async_trait]
    impl Finder for MockFinder {
        async fn find(&self, query: &str, limit: u32) -> Result<Vec<String>, FindFailure> {
            *self.seen.lock().unwrap() = Some((query.to_string(), limit));
            self.result.lock().unwrap().take().expect("one call per mock")
        }

        async fn list(
            &self,
            label: &str,
            limit: u32,
        ) -> Result<Vec<os_sdk::graph::Identity>, FindFailure> {
            *self.seen.lock().unwrap() = Some((label.to_string(), limit));
            self.result
                .lock()
                .unwrap()
                .take()
                .expect("one call per mock")
                .map(|ids| {
                    ids.into_iter()
                        .map(|id| os_sdk::graph::Identity { name: id.clone(), id })
                        .collect()
                })
        }
    }

    fn grant() -> SessionGrant {
        SessionGrant {
            capability_context: CapabilityContext { generic_tools: vec![], proxy_tools: vec![] },
            project_anchor: None,
            read_tier: ReadTier::Full,
            externally_triggered: false,
            pid: 1,
        }
    }

    fn call(input: serde_json::Value) -> Execute {
        Execute { tool_name: "graph.find".into(), tool_input: input, proof: None }
    }

    #[tokio::test]
    async fn a_search_answers_the_ranked_ids() {
        let finder = Arc::new(MockFinder::ok(&["/w/notes.md", "/w/other.md"]));
        let out = GraphFindExecutor::new(finder.clone())
            .execute(&call(serde_json::json!({ "query": "notes" })), &grant())
            .await;
        match out {
            ExecuteOutcome::Ok { result } => {
                assert_eq!(result["ids"][0], "/w/notes.md");
                assert_eq!(result["ids"][1], "/w/other.md");
            }
            other => panic!("expected ids, got {other:?}"),
        }
        assert_eq!(finder.seen.lock().unwrap().clone(), Some(("notes".to_string(), DEFAULT_LIMIT)));
    }

    #[tokio::test]
    async fn an_absurd_limit_is_clamped_before_it_travels() {
        let finder = Arc::new(MockFinder::ok(&[]));
        let _ = GraphFindExecutor::new(finder.clone())
            .execute(&call(serde_json::json!({ "query": "x", "limit": 10_000 })), &grant())
            .await;
        assert_eq!(finder.seen.lock().unwrap().clone().unwrap().1, MAX_LIMIT);
    }

    /// A zero limit is a call that asks for nothing, so it takes the default
    /// rather than travelling as a search that cannot answer.
    #[tokio::test]
    async fn a_zero_limit_falls_back_to_the_default() {
        let finder = Arc::new(MockFinder::ok(&[]));
        let _ = GraphFindExecutor::new(finder.clone())
            .execute(&call(serde_json::json!({ "query": "x", "limit": 0 })), &grant())
            .await;
        assert_eq!(finder.seen.lock().unwrap().clone().unwrap().1, DEFAULT_LIMIT);
    }

    #[tokio::test]
    async fn a_blank_query_never_reaches_the_daemon() {
        let finder = Arc::new(MockFinder::ok(&[]));
        let out = GraphFindExecutor::new(finder.clone())
            .execute(&call(serde_json::json!({ "query": "   " })), &grant())
            .await;
        assert!(matches!(
            out,
            ExecuteOutcome::Error { code: ContractError::InvalidArguments, .. }
        ));
        assert!(finder.seen.lock().unwrap().is_none(), "nothing should have been searched");
    }

    #[tokio::test]
    async fn a_refused_caller_is_reported_as_a_permission_error() {
        let finder = Arc::new(MockFinder::failing(FindFailure::Denied));
        let out = GraphFindExecutor::new(finder)
            .execute(&call(serde_json::json!({ "query": "x" })), &grant())
            .await;
        assert!(matches!(
            out,
            ExecuteOutcome::Error { code: ContractError::PermissionDenied, .. }
        ));
    }

    #[tokio::test]
    async fn the_list_verb_answers_handles() {
        let finder = Arc::new(MockFinder::ok(&["p1", "p2"]));
        let out = GraphFindExecutor::new(finder.clone())
            .execute(
                &Execute {
                    tool_name: "graph.list".into(),
                    tool_input: serde_json::json!({ "label": "system.Project" }),
                    proof: None,
                },
                &grant(),
            )
            .await;
        match out {
            ExecuteOutcome::Ok { result } => {
                assert_eq!(result["items"][0]["id"], "p1");
                assert_eq!(result["items"][1]["id"], "p2");
            }
            other => panic!("expected handles, got {other:?}"),
        }
        assert_eq!(
            finder.seen.lock().unwrap().clone(),
            Some(("system.Project".to_string(), DEFAULT_LIST_LIMIT))
        );
    }

    /// A label the daemon will not enumerate answers the same as one that does
    /// not exist, so this cannot be used to ask what labels there are.
    #[tokio::test]
    async fn a_refused_label_is_a_permission_error_not_a_hint() {
        let finder = Arc::new(MockFinder::failing(FindFailure::Denied));
        let out = GraphFindExecutor::new(finder)
            .execute(
                &Execute {
                    tool_name: "graph.list".into(),
                    tool_input: serde_json::json!({ "label": "system.Message" }),
                    proof: None,
                },
                &grant(),
            )
            .await;
        assert!(matches!(
            out,
            ExecuteOutcome::Error { code: ContractError::PermissionDenied, .. }
        ));
    }

    #[tokio::test]
    async fn a_list_with_no_label_never_reaches_the_daemon() {
        let finder = Arc::new(MockFinder::ok(&[]));
        let out = GraphFindExecutor::new(finder.clone())
            .execute(
                &Execute {
                    tool_name: "graph.list".into(),
                    tool_input: serde_json::json!({}),
                    proof: None,
                },
                &grant(),
            )
            .await;
        assert!(matches!(
            out,
            ExecuteOutcome::Error { code: ContractError::InvalidArguments, .. }
        ));
        assert!(finder.seen.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn another_tool_is_not_this_executors_to_run() {
        let finder = Arc::new(MockFinder::ok(&[]));
        let out = GraphFindExecutor::new(finder)
            .execute(
                &Execute {
                    tool_name: "graph.ask".into(),
                    tool_input: serde_json::json!({ "query": "x" }),
                    proof: None,
                },
                &grant(),
            )
            .await;
        assert!(matches!(out, ExecuteOutcome::Error { code: ContractError::UnknownTool, .. }));
    }
}
