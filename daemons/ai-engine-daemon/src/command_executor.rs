//! The `run_command` executor: hands an approved command to the SEPARATE
//! terminal-run MCP server.
//!
//! `run_command` is the sharp edge of the ACT layer - opaque, unboundable,
//! un-undoable. Its safety story is **always-Confirm + confined + output-captured
//! + never-autonomous**, and the pieces are deliberately split across processes:
//! the gate classifies it `Confirm` (`capability_map`), the dispatch mints a
//! consent biscuit bound to the exact `(command, args)` only once a user Confirm
//! resolved (`dispatch::proof_for_allow`), the terminal-run MCP server verifies
//! that biscuit at its own boundary and runs the command under bwrap + a seccomp
//! allowlist. This executor is only the courier between the daemon and that
//! server; it decides nothing.
//!
//! It therefore forwards the tool input **verbatim**. Rewriting or re-deriving
//! `command`/`args` here would break the consent binding (the biscuit is bound to
//! the exact argv the user confirmed), so a mismatch must fail at the server, not
//! be papered over in transit.
//!
//! Fail-closed at every step: an unknown tool, a non-live executor, a missing
//! consent credential, an unreachable server, or a server-side refusal all return
//! an error and run nothing. The consent credential is checked for PRESENCE only -
//! its validity and its binding to the argv are the MCP server's call, never
//! re-implemented here.
//!
//! NB this executor is INERT until `run_command` is also advertised to the model:
//! the pi proxy plugin registers a fixed tool list and its `call_tool` refuses a
//! name outside it, so nothing can reach this path until that spec lands. Built
//! mechanism-first (like the compensation and canary cores) so the courier can be
//! reviewed before the tool is exposed.

use crate::dispatch::Executor;
use crate::session::SessionGrant;
use ai_engine_contract::{ContractError, Execute, ExecuteOutcome};
use arlen_ai_core::mcp::{CallChain, McpClient, ServerClass, ServerId};
use arlen_run_consent_token::RUN_COMMAND_TOOL;
use async_trait::async_trait;

/// The well-known app id (and MCP server id) of the terminal-run server. Matches
/// `arlen_terminal_run_mcp::server::SERVER_ID`; duplicated rather than depended on
/// so the daemon does not link the server crate (the two are separate processes on
/// purpose - the server is the namespace-creating half).
const TERMINAL_RUN_SERVER: &str = "system.terminal-run";

/// The tool the terminal-run server exposes. Matches
/// `arlen_terminal_run_mcp::server::RUN_TOOL`.
const TERMINAL_RUN_TOOL: &str = "run_command";

/// The key the gate shim writes the minted consent biscuit into on an `Allow`
/// (`pi-plugins/src/gate.ts`): it overwrites any model-supplied value and deletes
/// the key outright when no proof was minted, so a present value here always came
/// from the daemon's own minter.
const CONSENT_KEY: &str = "consent";

/// Routes an approved `run_command` to the terminal-run MCP server.
pub struct CommandExecutor {
    /// Re-read per call so a runtime `executor_live` flip takes effect
    /// immediately, matching the filesystem and settings executors.
    executor_live: Box<dyn Fn() -> bool + Send + Sync>,
    /// The terminal-run server's Unix socket. Injectable so the round trip is
    /// testable against a stub server without a live daemon.
    socket_path: String,
}

impl CommandExecutor {
    /// An executor dialing the well-known terminal-run socket under the per-user
    /// MCP runtime dir.
    pub fn new(executor_live: Box<dyn Fn() -> bool + Send + Sync>) -> Self {
        let socket_path = os_sdk::mcp::mcp_socket_path(TERMINAL_RUN_SERVER)
            .to_string_lossy()
            .into_owned();
        Self { executor_live, socket_path }
    }

    /// An executor dialing an explicit socket path (tests, and a future
    /// per-profile instance path).
    pub fn with_socket(
        executor_live: Box<dyn Fn() -> bool + Send + Sync>,
        socket_path: impl Into<String>,
    ) -> Self {
        Self { executor_live, socket_path: socket_path.into() }
    }
}

#[async_trait]
impl Executor for CommandExecutor {
    async fn execute(&self, req: &Execute, _grant: &SessionGrant) -> ExecuteOutcome {
        if req.tool_name != RUN_COMMAND_TOOL {
            return ExecuteOutcome::Error {
                code: ContractError::UnknownTool,
                message: format!("{} is not the run_command tool", req.tool_name),
            };
        }

        // Executor-live gate, re-read PER CALL (fail-closed): even a confirmed,
        // consent-bearing call runs nothing once executor_live is off.
        if !(self.executor_live)() {
            return ExecuteOutcome::Error {
                code: ContractError::ExecutionFailed,
                message: "run_command is not permitted: the executor is not live".to_string(),
            };
        }

        // The consent credential must be PRESENT before we even dial. Its validity
        // and its binding to (command, args) are verified by the MCP server, which
        // holds the consent root's public key; re-checking either here would
        // duplicate the boundary and risk diverging from it.
        let has_consent = req
            .tool_input
            .get(CONSENT_KEY)
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
        if !has_consent {
            return ExecuteOutcome::Error {
                code: ContractError::ExecutionFailed,
                message: "run_command carries no consent credential; it runs only after a user \
                          Confirm mints one"
                    .to_string(),
            };
        }

        // A command name is required, so a malformed call fails here rather than
        // producing an opaque server-side error. The value is NOT rewritten - it
        // and the args go to the server exactly as the user confirmed them.
        let named = req
            .tool_input
            .get("command")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.is_empty());
        if !named {
            return ExecuteOutcome::Error {
                code: ContractError::InvalidArguments,
                message: "run_command needs a non-empty string command in the tool input"
                    .to_string(),
            };
        }

        let id = ServerId(TERMINAL_RUN_SERVER.to_string());
        let mut client = McpClient::new();
        // The terminal-run server mutates the world, so it is an Action server: the
        // registry holds it to the action-server rules, never the read-only default.
        if let Err(err) = client
            .connect(id.clone(), &self.socket_path, ServerClass::Action)
            .await
        {
            return ExecuteOutcome::Error {
                code: ContractError::ExecutionFailed,
                message: format!("the terminal-run server is unavailable: {err}"),
            };
        }

        // Forward the input verbatim (command, args, consent, timeout_ms).
        match client
            .call_tool(&id, TERMINAL_RUN_TOOL, req.tool_input.clone(), &CallChain::root())
            .await
        {
            Ok(text) => ExecuteOutcome::Ok {
                result: serde_json::Value::String(text),
            },
            Err(err) => ExecuteOutcome::Error {
                code: ContractError::ExecutionFailed,
                message: format!("run_command failed at the terminal-run server: {err}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ai_engine_contract::{CapabilityContext, ReadTier};

    /// The courier never reads the grant (the consent biscuit is the authority), so
    /// the tests pass a minimal one.
    fn grant() -> SessionGrant {
        SessionGrant {
            capability_context: CapabilityContext { generic_tools: vec![], proxy_tools: vec![] },
            project_anchor: None,
            read_tier: ReadTier::None,
            externally_triggered: false,
            pid: 1,
        }
    }

    fn live() -> Box<dyn Fn() -> bool + Send + Sync> {
        Box::new(|| true)
    }
    fn not_live() -> Box<dyn Fn() -> bool + Send + Sync> {
        Box::new(|| false)
    }

    fn req(input: serde_json::Value) -> Execute {
        Execute {
            tool_name: RUN_COMMAND_TOOL.to_string(),
            tool_input: input,
            proof: None,
        }
    }

    /// The socket is never dialed on a refusal, so an unroutable path is safe in
    /// the tests that assert a pre-dial refusal.
    fn exec(live: Box<dyn Fn() -> bool + Send + Sync>) -> CommandExecutor {
        CommandExecutor::with_socket(live, "/nonexistent/terminal-run.sock")
    }

    #[tokio::test]
    async fn a_foreign_tool_is_refused() {
        let e = exec(live());
        let mut r = req(serde_json::json!({"command": "ls", "consent": "tok"}));
        r.tool_name = "fs.move".into();
        let out = e.execute(&r, &grant()).await;
        assert!(
            matches!(out, ExecuteOutcome::Error { code: ContractError::UnknownTool, .. }),
            "{out:?}"
        );
    }

    #[tokio::test]
    async fn a_non_live_executor_runs_nothing() {
        let e = exec(not_live());
        let out = e
            .execute(
                &req(serde_json::json!({"command": "ls", "consent": "tok"})),
                &grant(),
            )
            .await;
        match out {
            ExecuteOutcome::Error { code: ContractError::ExecutionFailed, message } => {
                assert!(message.contains("not live"), "{message}");
            }
            other => panic!("expected a not-live refusal, got {other:?}"),
        }
    }

    /// The consent biscuit is the whole boundary: without one the courier refuses
    /// before dialing, so a call that never went through a user Confirm cannot even
    /// reach the server.
    #[tokio::test]
    async fn a_call_without_consent_is_refused_before_dialing() {
        let e = exec(live());
        for input in [
            serde_json::json!({"command": "ls"}),
            serde_json::json!({"command": "ls", "consent": ""}),
            serde_json::json!({"command": "ls", "consent": 7}),
        ] {
            let out = e.execute(&req(input.clone()), &grant()).await;
            match out {
                ExecuteOutcome::Error { code: ContractError::ExecutionFailed, message } => {
                    assert!(message.contains("consent"), "{message} for {input}");
                }
                other => panic!("expected a consent refusal for {input}, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn a_call_without_a_command_is_refused() {
        let e = exec(live());
        let out = e
            .execute(&req(serde_json::json!({"consent": "tok"})), &grant())
            .await;
        assert!(
            matches!(out, ExecuteOutcome::Error { code: ContractError::InvalidArguments, .. }),
            "{out:?}"
        );
    }

    /// An unreachable server fails closed rather than reporting success.
    #[tokio::test]
    async fn an_unreachable_server_fails_closed() {
        let e = exec(live());
        let out = e
            .execute(
                &req(serde_json::json!({"command": "ls", "consent": "tok"})),
                &grant(),
            )
            .await;
        match out {
            ExecuteOutcome::Error { code: ContractError::ExecutionFailed, message } => {
                assert!(message.contains("unavailable"), "{message}");
            }
            other => panic!("expected an unavailable-server refusal, got {other:?}"),
        }
    }
}
