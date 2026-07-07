//! The Execute-seam router.
//!
//! The daemon is the trusted runner for several proxy tools (`graph.read` now;
//! `graph.write`, OS proxy tools, and our own knowledge-mcp/file-manager-mcp
//! surfaced as gated custom tools later - `pi-agent-adoption.md` Phase 1). The
//! contract has one [`Executor`] seam, so this routes an [`Execute`] to the
//! sub-executor registered for its tool name; an unregistered tool is refused
//! with [`ContractError::UnknownTool`] (fail-closed: a tool the daemon does not
//! run never executes). Each sub-executor enforces its own scope (the read
//! executor bounds by the grant's read scope; the write executor will register
//! compensation), so the router only dispatches.

use crate::dispatch::Executor;
use crate::session::SessionGrant;
use ai_engine_contract::{ContractError, Execute, ExecuteOutcome};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// Routes an [`Execute`] to the registered sub-executor for its tool name.
#[derive(Default)]
pub struct ProxyExecutor {
    tools: HashMap<String, Arc<dyn Executor>>,
}

impl ProxyExecutor {
    /// An empty router (every tool is unknown until registered).
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    /// Register `exec` as the runner for `tool`, returning self so registrations
    /// chain. A later registration for the same tool replaces the earlier one.
    pub fn register(mut self, tool: impl Into<String>, exec: Arc<dyn Executor>) -> Self {
        self.tools.insert(tool.into(), exec);
        self
    }
}

#[async_trait]
impl Executor for ProxyExecutor {
    async fn execute(&self, req: &Execute, grant: &SessionGrant) -> ExecuteOutcome {
        match self.tools.get(&req.tool_name) {
            Some(exec) => exec.execute(req, grant).await,
            None => ExecuteOutcome::Error {
                code: ContractError::UnknownTool,
                message: format!("{} is not a tool this daemon runs", req.tool_name),
            },
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

    fn exec(tool: &str) -> Execute {
        Execute { tool_name: tool.into(), tool_input: serde_json::json!({}), proof: None }
    }

    /// A sub-executor that records it ran and returns a marker, so a test can see
    /// the router delegated to the right one.
    struct MarkerExecutor {
        marker: &'static str,
    }
    #[async_trait]
    impl Executor for MarkerExecutor {
        async fn execute(&self, _req: &Execute, _grant: &SessionGrant) -> ExecuteOutcome {
            ExecuteOutcome::Ok { result: serde_json::json!({ "ran": self.marker }) }
        }
    }

    #[tokio::test]
    async fn it_routes_to_the_registered_sub_executor() {
        let router = ProxyExecutor::new()
            .register("graph.read", Arc::new(MarkerExecutor { marker: "read" }))
            .register("graph.write", Arc::new(MarkerExecutor { marker: "write" }));

        match router.execute(&exec("graph.read"), &grant()).await {
            ExecuteOutcome::Ok { result } => assert_eq!(result["ran"], "read"),
            other => panic!("expected the read executor, got {other:?}"),
        }
        match router.execute(&exec("graph.write"), &grant()).await {
            ExecuteOutcome::Ok { result } => assert_eq!(result["ran"], "write"),
            other => panic!("expected the write executor, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unregistered_tool_is_unknown() {
        let router = ProxyExecutor::new()
            .register("graph.read", Arc::new(MarkerExecutor { marker: "read" }));
        assert!(matches!(
            router.execute(&exec("rm.rf"), &grant()).await,
            ExecuteOutcome::Error { code: ContractError::UnknownTool, .. },
        ));
    }

    #[tokio::test]
    async fn an_empty_router_refuses_every_tool() {
        let router = ProxyExecutor::new();
        assert!(matches!(
            router.execute(&exec("graph.read"), &grant()).await,
            ExecuteOutcome::Error { code: ContractError::UnknownTool, .. },
        ));
    }
}
