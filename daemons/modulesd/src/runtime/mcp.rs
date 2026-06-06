//! Tier 1 hosting for `mcp.server` modules.
//!
//! An `mcp.server` module is a WASM component exporting the
//! `mcp-server` world (`sdk/module-sdk/wit/mcp.wit`): `init`,
//! `list-tools`, `call-tool`, `shutdown`. [`McpModuleHost`] owns one
//! such instance and exposes async `list_tools` / `call_tool` calls
//! with the same fuel-refill and wall-clock discipline the
//! waypointer dispatch path uses.
//!
//! The rmcp JSON-RPC socket bridge that fronts a host with a Unix
//! socket under `$XDG_RUNTIME_DIR/arlen/mcp/modules/` is layered
//! on top of this module.

use std::borrow::Cow;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Mutex;

use os_sdk::mcp::rmcp;
use os_sdk::{UnixEventEmitter, UnixGraphClient};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, JsonObject, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};

use crate::error::Result;
use crate::host::CapabilityContext;
use crate::runtime::tier1::{McpInstance, Tier1Runtime, DEFAULT_FUEL_BUDGET};
use crate::runtime::wit;

/// Wall-clock budget for a single `list-tools` / `call-tool` guest
/// call. Fuel bounds CPU work; this bounds host-call hangs (a tool
/// that does a `network::fetch` can legitimately take seconds, so
/// the budget is generous and matches the network host's own 30 s
/// cap rather than the per-keystroke waypointer search budget).
const MCP_CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Module-id safety check and per-module MCP socket path, both
/// re-exported from `os-sdk` so modulesd (which binds the socket)
/// and the AI daemon (which connects to it) resolve the convention
/// identically. `start_mcp_server` rejects ids failing
/// `is_safe_module_id` before the path is ever formatted.
pub use os_sdk::mcp::{is_safe_module_id, mcp_module_socket_path};

/// One callable MCP tool, decoupled from the WIT-generated type so
/// the rest of modulesd does not depend on the bindgen output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolDef {
    /// Tool name, unique within the module.
    pub name: String,
    /// Human-readable description shown to the model.
    pub description: String,
    /// JSON Schema (as a JSON string) for the argument object. Empty
    /// means the tool takes no arguments.
    pub input_schema: String,
}

impl From<wit::mcp::guest_server::ToolDef> for McpToolDef {
    fn from(t: wit::mcp::guest_server::ToolDef) -> Self {
        Self {
            name: t.name,
            description: t.description,
            input_schema: t.input_schema,
        }
    }
}

/// A clean, module-reported failure of a `call-tool` invocation.
///
/// Distinct from [`McpHostError`]: a `McpToolError` means the module
/// ran correctly and decided the call could not succeed, so it does
/// **not** count toward crash recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpToolError {
    /// The arguments did not match the tool's input schema.
    InvalidInput(String),
    /// No tool with the given name is exported.
    NotFound(String),
    /// The tool ran but failed.
    ExecutionFailed(String),
}

impl From<wit::mcp::guest_server::ToolError> for McpToolError {
    fn from(e: wit::mcp::guest_server::ToolError) -> Self {
        use wit::mcp::guest_server::ToolError as W;
        match e {
            W::InvalidInput(m) => Self::InvalidInput(m),
            W::NotFound(m) => Self::NotFound(m),
            W::ExecutionFailed(m) => Self::ExecutionFailed(m),
        }
    }
}

impl std::fmt::Display for McpToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(m) => write!(f, "invalid input: {m}"),
            Self::NotFound(m) => write!(f, "tool not found: {m}"),
            Self::ExecutionFailed(m) => write!(f, "execution failed: {m}"),
        }
    }
}

/// A host-side failure of an MCP guest call.
#[derive(Debug, Clone)]
pub enum McpHostError {
    /// The guest trapped (panic, fuel exhaustion, ABI fault).
    /// Counts toward crash recovery.
    Trap(String),
    /// The call exceeded [`MCP_CALL_TIMEOUT`]. Counts toward crash
    /// recovery.
    Timeout,
    /// The host was revoked (module disabled, or torn down after a
    /// crash) before this call could reach the guest. **Not** a
    /// crash: a call that was queued on the instance mutex when the
    /// revoke landed fails closed with this rather than running.
    Revoked,
}

impl McpHostError {
    /// Whether this failure should count toward crash recovery. A
    /// revoked call is a clean refusal, not a module fault.
    pub fn is_fault(&self) -> bool {
        !matches!(self, Self::Revoked)
    }
}

impl std::fmt::Display for McpHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Trap(r) => write!(f, "guest trapped: {r}"),
            Self::Timeout => write!(
                f,
                "guest call exceeded {}s wall-clock budget",
                MCP_CALL_TIMEOUT.as_secs()
            ),
            Self::Revoked => write!(f, "module has been revoked"),
        }
    }
}

/// A live `mcp.server` module: one WASM instance, serialised behind a
/// `Mutex` because the wasmtime `Store` is `!Sync`. The rmcp socket
/// bridge can field concurrent JSON-RPC requests; they queue here.
pub struct McpModuleHost {
    module_id: String,
    instance: Mutex<McpInstance>,
    /// Cleared when the module is disabled or torn down after a
    /// crash. `serve_mcp_at` spawns a detached task per accepted
    /// connection, so a connection the AI daemon still holds open
    /// outlives the supervisor task. The bridge checks this before
    /// every list/call and fails closed, so a revoked module cannot
    /// keep serving tools over a stale connection.
    active: AtomicBool,
}

impl McpModuleHost {
    /// Compile and instantiate an `mcp.server` module, running its
    /// guest `init()`. A `WasmLoad` error is permanent (broken
    /// bytecode); a `WasmTrap` error counts toward crash recovery.
    pub async fn load(
        runtime: &Tier1Runtime,
        module_id: &str,
        wasm_path: &Path,
        ctx: CapabilityContext,
        graph_client: Arc<UnixGraphClient>,
        event_emitter: Arc<UnixEventEmitter>,
    ) -> Result<Self> {
        let component = runtime.compile(wasm_path).await?;
        let instance = runtime
            .instantiate_mcp(module_id, &component, ctx, graph_client, event_emitter)
            .await?;
        Ok(Self {
            module_id: module_id.to_string(),
            instance: Mutex::new(instance),
            active: AtomicBool::new(true),
        })
    }

    /// The module this host wraps.
    pub fn module_id(&self) -> &str {
        &self.module_id
    }

    /// Whether this host may still serve tool calls. Becomes `false`
    /// permanently once [`revoke`](Self::revoke) is called.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// Mark the host revoked: every subsequent bridge list/call fails
    /// closed. Called when the module is disabled or torn down after
    /// a crash. One-way — a restart builds a fresh host.
    pub fn revoke(&self) {
        self.active.store(false, Ordering::Release);
    }

    /// Call the guest `list-tools` export.
    pub async fn list_tools(&self) -> std::result::Result<Vec<McpToolDef>, McpHostError> {
        let mut guard = self.instance.lock().await;
        // Re-check revocation *after* acquiring the lock: a call can
        // sit queued behind another on this mutex while a disable or
        // crash teardown calls `revoke()`. Without the recheck the
        // queued call would still reach the guest.
        if !self.is_active() {
            return Err(McpHostError::Revoked);
        }
        // Refill fuel; a previous call may have left a partial budget.
        let _ = guard.store.set_fuel(DEFAULT_FUEL_BUDGET);
        let inst = &mut *guard;
        let outcome = tokio::time::timeout(
            MCP_CALL_TIMEOUT,
            inst.provider
                .arlen_waypointer_server()
                .call_list_tools(&mut inst.store),
        )
        .await;
        match outcome {
            Ok(Ok(wit_tools)) => {
                Ok(wit_tools.into_iter().map(McpToolDef::from).collect())
            }
            // A fault poisons the instance: revoke now so every call
            // already queued behind this one fails closed instead of
            // running against a trapped store.
            Ok(Err(trap)) => {
                self.revoke();
                Err(McpHostError::Trap(format!("list_tools: {trap}")))
            }
            Err(_elapsed) => {
                self.revoke();
                Err(McpHostError::Timeout)
            }
        }
    }

    /// Call the guest `call-tool` export.
    ///
    /// The outer `Result` is the host-side outcome (a trap/timeout is
    /// crash-worthy); the inner `Result` is the module's own clean
    /// outcome (a JSON result string, or a typed [`McpToolError`]).
    pub async fn call_tool(
        &self,
        name: &str,
        arguments_json: &str,
    ) -> std::result::Result<std::result::Result<String, McpToolError>, McpHostError> {
        let mut guard = self.instance.lock().await;
        // See `list_tools`: the revocation recheck must happen after
        // the lock so a call queued before a disable/crash refuses.
        if !self.is_active() {
            return Err(McpHostError::Revoked);
        }
        let _ = guard.store.set_fuel(DEFAULT_FUEL_BUDGET);
        let inst = &mut *guard;
        let outcome = tokio::time::timeout(
            MCP_CALL_TIMEOUT,
            inst.provider
                .arlen_waypointer_server()
                .call_call_tool(&mut inst.store, name, arguments_json),
        )
        .await;
        match outcome {
            Ok(Ok(result)) => Ok(result.map_err(McpToolError::from)),
            Ok(Err(trap)) => {
                self.revoke();
                Err(McpHostError::Trap(format!("call_tool: {trap}")))
            }
            Err(_elapsed) => {
                self.revoke();
                Err(McpHostError::Timeout)
            }
        }
    }

    /// Best-effort guest `shutdown()`, used by the SIGTERM handler.
    pub async fn graceful_shutdown(&self) {
        self.instance
            .lock()
            .await
            .graceful_shutdown(&self.module_id)
            .await;
    }
}

/// Translate a host-side tool definition into the rmcp `Tool` shape.
///
/// `input_schema` arrives as a JSON string. An empty or unparseable
/// schema degrades to an empty object rather than failing the whole
/// `tools/list`: a missing schema only loses argument validation, it
/// must not make the tool undiscoverable.
fn tool_def_to_rmcp(def: McpToolDef) -> Tool {
    let schema: JsonObject = if def.input_schema.trim().is_empty() {
        JsonObject::new()
    } else {
        serde_json::from_str(&def.input_schema).unwrap_or_default()
    };
    Tool::new_with_raw(def.name, Some(Cow::Owned(def.description)), Arc::new(schema))
}

/// An rmcp `ServerHandler` that fronts one [`McpModuleHost`] with a
/// standard MCP server. modulesd binds one of these per `mcp.server`
/// module on a Unix socket under `$XDG_RUNTIME_DIR/arlen/mcp/modules/`.
///
/// `tools/list` and `tools/call` are forwarded to the WASM guest. A
/// guest trap or timeout is reported to the module's supervisor over
/// the `fault` channel so the crash ladder runs; the JSON-RPC client
/// sees an internal error for that one call. A clean module-level
/// tool failure is *not* a crash: it comes back as a `CallToolResult`
/// with `is_error: true`, which is the MCP-spec way to signal it.
#[derive(Clone)]
pub struct ModuleMcpBridge {
    host: Arc<McpModuleHost>,
    fault: UnboundedSender<String>,
}

impl ModuleMcpBridge {
    /// Wrap a host. `fault` carries the module id to the supervisor
    /// whenever a guest call traps or times out.
    pub fn new(host: Arc<McpModuleHost>, fault: UnboundedSender<String>) -> Self {
        Self { host, fault }
    }

    /// Signal the supervisor that the guest faulted on a call.
    fn report_fault(&self) {
        let _ = self.fault.send(self.host.module_id().to_string());
    }
}

impl ServerHandler for ModuleMcpBridge {
    fn get_info(&self) -> ServerInfo {
        // `ServerInfo` (`InitializeResult`) is `#[non_exhaustive]`;
        // build it through the constructor rather than a literal.
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(format!(
                "MCP tools exported by the Arlen module {}.",
                self.host.module_id()
            ))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, McpError> {
        match self.host.list_tools().await {
            Ok(defs) => Ok(ListToolsResult::with_all_items(
                defs.into_iter().map(tool_def_to_rmcp).collect(),
            )),
            Err(host_err) => Err(self.host_error_to_mcp(host_err)),
        }
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResult, McpError> {
        // The WIT boundary takes the argument object as a JSON string.
        let arguments_json = match &request.arguments {
            Some(map) => serde_json::to_string(map)
                .unwrap_or_else(|_| "{}".to_string()),
            None => "{}".to_string(),
        };
        match self.host.call_tool(&request.name, &arguments_json).await {
            Ok(Ok(result_json)) => {
                Ok(CallToolResult::success(vec![Content::text(result_json)]))
            }
            Ok(Err(tool_err)) => {
                // A clean module-level failure: surfaced as an MCP
                // tool error, not a transport error, and not a crash.
                Ok(CallToolResult::error(vec![Content::text(
                    tool_err.to_string(),
                )]))
            }
            Err(host_err) => Err(self.host_error_to_mcp(host_err)),
        }
    }
}

impl ModuleMcpBridge {
    /// Map a host-side failure to a JSON-RPC error, reporting a fault
    /// to the supervisor only for real crashes. A revoked call is a
    /// clean refusal: the client sees an error, the crash ladder is
    /// left alone.
    fn host_error_to_mcp(&self, host_err: McpHostError) -> McpError {
        if host_err.is_fault() {
            self.report_fault();
            McpError::internal_error(
                format!("module mcp server failed: {host_err}"),
                None,
            )
        } else {
            McpError::invalid_request("module is disabled", None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_error_fault_classification() {
        // Traps and timeouts feed the crash ladder.
        assert!(McpHostError::Trap("boom".into()).is_fault());
        assert!(McpHostError::Timeout.is_fault());
        // A revoked call is a clean refusal, not a crash.
        assert!(!McpHostError::Revoked.is_fault());
    }

    #[test]
    fn tool_error_maps_every_wit_variant() {
        use wit::mcp::guest_server::ToolError as W;
        assert_eq!(
            McpToolError::from(W::InvalidInput("bad".into())),
            McpToolError::InvalidInput("bad".into())
        );
        assert_eq!(
            McpToolError::from(W::NotFound("gone".into())),
            McpToolError::NotFound("gone".into())
        );
        assert_eq!(
            McpToolError::from(W::ExecutionFailed("boom".into())),
            McpToolError::ExecutionFailed("boom".into())
        );
    }

    #[test]
    fn tool_def_conversion_preserves_fields() {
        let wit_def = wit::mcp::guest_server::ToolDef {
            name: "echo".into(),
            description: "echo the input".into(),
            input_schema: r#"{"type":"object"}"#.into(),
        };
        let def = McpToolDef::from(wit_def);
        assert_eq!(def.name, "echo");
        assert_eq!(def.description, "echo the input");
        assert_eq!(def.input_schema, r#"{"type":"object"}"#);
    }

    #[test]
    fn tool_def_to_rmcp_parses_schema() {
        let def = McpToolDef {
            name: "convert".into(),
            description: "convert units".into(),
            input_schema: r#"{"type":"object","properties":{"q":{"type":"string"}}}"#
                .into(),
        };
        let tool = tool_def_to_rmcp(def);
        assert_eq!(tool.name, "convert");
        assert!(tool.input_schema.contains_key("properties"));
    }

    #[test]
    fn tool_def_to_rmcp_tolerates_missing_or_broken_schema() {
        // Empty schema: an empty object, tool still discoverable.
        let empty = tool_def_to_rmcp(McpToolDef {
            name: "noargs".into(),
            description: "no arguments".into(),
            input_schema: String::new(),
        });
        assert_eq!(empty.name, "noargs");
        assert!(empty.input_schema.is_empty());
        // Garbage schema: degrades to empty, does not panic.
        let broken = tool_def_to_rmcp(McpToolDef {
            name: "weird".into(),
            description: "broken schema".into(),
            input_schema: "this is not json".into(),
        });
        assert_eq!(broken.name, "weird");
        assert!(broken.input_schema.is_empty());
    }
}
