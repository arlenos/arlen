//! The Terminal-run MCP server (mcp-server-layer.md + ai-act-layer-plan.md): the
//! peer-authenticated surface exposing `run_command` over the AI daemon's MCP
//! transport. It is a thin, separate process so the confined-spawn machinery and a
//! runaway command's resource use stay out of the daemon (the plan reserves the
//! MCP-server pattern for exactly this - the confined sandbox boundary).
//!
//! ## Safety: fail-closed at the consent boundary
//!
//! `run_command` is the sharp edge - opaque, unboundable, un-undoable
//! (`OpaqueCommand`). Its whole safety story is **always-Confirm + confined +
//! output-captured + never-autonomous**. Two layers guard it here:
//!
//! 1. **Peer-auth** ([`os_sdk::mcp::serve_mcp`]): only the AI daemon may connect
//!    (`SO_PEERCRED` + the admitted-app allowlist), so no arbitrary process reaches
//!    this socket.
//! 2. **Per-action consent** ([`authorize_run`]): the pi gate classifies
//!    `run_command` `Confirm` and the consent broker surfaces it, but a compromised
//!    caller could otherwise invoke this server directly. So the server ITSELF
//!    verifies per-action consent at the boundary. That verification (the biscuit
//!    minted at Authorize, verified here - ai-act-layer-plan.md "biscuit per-action
//!    tie-in") is NOT wired yet, so [`authorize_run`] FAILS CLOSED: the full run
//!    pipeline is built and wired, but no command executes until the consent boundary
//!    lands (the same fail-closed-by-a-stub discipline the pi write path uses).

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use os_sdk::mcp::rmcp;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, JsonObject, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};

use crate::run::{run_confined, RunRequest, DEFAULT_TIMEOUT};
use arlen_confiner::NetworkPolicy;

/// Well-known socket id for the system Terminal-run server. Resolves to
/// `$XDG_RUNTIME_DIR/arlen/mcp/system.terminal-run.sock` via the os-sdk helper.
pub const SERVER_ID: &str = "system.terminal-run";

/// The one tool: run a single confirmed command in the sandbox.
pub const RUN_TOOL: &str = "run_command";

/// The hard ceiling on a command's wall-clock budget (a caller can request less).
const MAX_TIMEOUT: Duration = Duration::from_secs(120);

/// The Terminal-run MCP server. Stateless; a fresh handle is built per connection.
#[derive(Clone, Default)]
pub struct TerminalRunMcp;

impl TerminalRunMcp {
    /// Build a server handle.
    pub fn new() -> Self {
        Self
    }

    /// The `run_command` tool schema.
    fn run_tool() -> Tool {
        let schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The program to run (no shell; resolved on PATH)." },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Arguments, each a separate string (never a shell line)."
                },
                "timeout_ms": { "type": "integer", "description": "Wall-clock budget in ms (capped)." }
            },
            "required": ["command"]
        }))
        .expect("the static run_command schema is a valid JSON object");
        Tool::new_with_raw(
            RUN_TOOL.to_owned(),
            Some(Cow::Borrowed(
                "Run a single confirmed command in a confined sandbox (no host write, no network, no privilege) and return its captured output. Always requires prior user confirmation; never autonomous.",
            )),
            Arc::new(schema),
        )
    }
}

/// The parsed, validated `run_command` arguments.
struct RunArgs {
    command: String,
    args: Vec<String>,
    timeout: Duration,
}

/// Parse + validate the `run_command` arguments. A missing/empty command or a
/// non-string arg is a clean tool error, never a guess.
fn parse_run_args(arguments: Option<&JsonObject>) -> Result<RunArgs, McpError> {
    let map = arguments.ok_or_else(|| {
        McpError::invalid_request("run_command requires a 'command' argument", None)
    })?;
    let command = map
        .get("command")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| McpError::invalid_request("run_command needs a non-empty string 'command'", None))?
        .to_string();
    let args = match map.get("args") {
        None => Vec::new(),
        Some(v) => {
            let arr = v
                .as_array()
                .ok_or_else(|| McpError::invalid_request("'args' must be an array of strings", None))?;
            let mut out = Vec::with_capacity(arr.len());
            for a in arr {
                let s = a.as_str().ok_or_else(|| {
                    McpError::invalid_request("every 'args' entry must be a string", None)
                })?;
                out.push(s.to_string());
            }
            out
        }
    };
    let timeout = match map.get("timeout_ms").and_then(|v| v.as_u64()) {
        Some(ms) => Duration::from_millis(ms).min(MAX_TIMEOUT),
        None => DEFAULT_TIMEOUT,
    };
    Ok(RunArgs { command, args, timeout })
}

/// Verify per-action consent for a `run_command` at the MCP boundary. FAIL-CLOSED:
/// the biscuit-minted-at-Authorize, verified-here tie-in is not wired yet, so this
/// always refuses. run_command is the sharp edge; it must never run without a
/// verified, per-action user confirmation, so the server executes nothing until the
/// consent boundary lands. Replacing this stub with the biscuit verification is the
/// go-live step (paired with the pi executor-live cutover).
fn authorize_run(_args: &RunArgs) -> Result<(), String> {
    Err("per-action consent verification is not wired yet (the biscuit-at-the-MCP-boundary \
         follow-up); run_command executes nothing until it lands"
        .to_string())
}

/// A per-call writable scratch dir, UNIQUE per call and removed on drop. Two
/// concurrent runs must not share a workdir (one command's scratch must never be
/// visible to another), and each command gets a fresh empty dir; `Drop` cleans it
/// up so a command's scratch never leaks into the next.
struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    /// Create a unique empty scratch dir under `$XDG_RUNTIME_DIR/arlen/terminal-run`.
    fn create() -> std::io::Result<ScratchDir> {
        static N: AtomicU64 = AtomicU64::new(0);
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let path =
            base.join("arlen").join("terminal-run").join(format!("{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path)?;
        Ok(ScratchDir { path })
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

impl ServerHandler for TerminalRunMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Run a single, previously-confirmed command in a confined sandbox and return its captured output. The command runs with no host write access, no network, and no privilege. Every run requires prior per-action user confirmation; nothing runs autonomously.",
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(vec![Self::run_tool()]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let tool = request.name.to_string();
        if tool != RUN_TOOL {
            return Err(McpError::invalid_request(format!("unknown tool: {tool}"), None));
        }
        let args = parse_run_args(request.arguments.as_ref())?;

        // FAIL-CLOSED consent boundary FIRST: nothing runs without verified consent.
        if let Err(why) = authorize_run(&args) {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "run_command refused: {why}"
            ))]));
        }

        // Reached only once consent is wired: run the confined command in a fresh,
        // isolated, per-call scratch dir (cleaned up when `scratch` drops) + return
        // the captured output.
        let scratch = match ScratchDir::create() {
            Ok(s) => s,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "run_command: could not prepare the scratch dir: {e}"
                ))]))
            }
        };
        let req = RunRequest {
            command: args.command,
            args: args.args,
            read_only_roots: vec![PathBuf::from("/")],
            workdir: scratch.path.clone(),
            network: NetworkPolicy::None,
            timeout: args.timeout,
        };
        let result = run_confined(&req).await;
        drop(scratch); // clean the per-call scratch before returning.
        match result {
            Ok(outcome) => {
                let json = serde_json::to_string(&outcome).unwrap_or_else(|_| "{}".to_string());
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "run_command failed: {e}"
            ))])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(command: &str) -> JsonObject {
        serde_json::from_value(serde_json::json!({ "command": command, "args": ["-la"] })).unwrap()
    }

    #[test]
    fn parse_requires_a_non_empty_command() {
        assert!(parse_run_args(Some(&args("ls"))).is_ok());
        let empty: JsonObject = serde_json::from_value(serde_json::json!({ "command": "" })).unwrap();
        assert!(parse_run_args(Some(&empty)).is_err());
        let missing: JsonObject = serde_json::from_value(serde_json::json!({ "args": [] })).unwrap();
        assert!(parse_run_args(Some(&missing)).is_err());
        assert!(parse_run_args(None).is_err());
    }

    #[test]
    fn parse_rejects_non_string_args() {
        let bad: JsonObject =
            serde_json::from_value(serde_json::json!({ "command": "ls", "args": [1, 2] })).unwrap();
        assert!(parse_run_args(Some(&bad)).is_err());
    }

    #[test]
    fn parse_clamps_the_timeout() {
        let long: JsonObject = serde_json::from_value(
            serde_json::json!({ "command": "ls", "timeout_ms": 9_999_999 }),
        )
        .unwrap();
        assert_eq!(parse_run_args(Some(&long)).unwrap().timeout, MAX_TIMEOUT);
    }

    #[test]
    fn consent_is_fail_closed_until_wired() {
        // The sharp edge refuses to run until the per-action consent boundary lands.
        let a = parse_run_args(Some(&args("ls"))).unwrap();
        assert!(authorize_run(&a).is_err(), "run_command must fail closed on consent");
    }

    #[test]
    fn scratch_dirs_are_unique_and_cleaned_on_drop() {
        let a = ScratchDir::create().unwrap();
        let b = ScratchDir::create().unwrap();
        assert_ne!(a.path, b.path, "each call gets its own scratch dir");
        assert!(a.path.is_dir() && b.path.is_dir(), "both created");
        let a_path = a.path.clone();
        drop(a);
        assert!(!a_path.exists(), "the scratch is removed on drop, no leak into the next run");
        assert!(b.path.is_dir(), "the other run's scratch is untouched");
    }
}
