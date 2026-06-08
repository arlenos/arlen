//! System Monitor read-only MCP server (`mcp-server-layer.md` §4.1, §2 system
//! server, default-permit).
//!
//! Exposes read-only, argument-free tools to the AI daemon over MCP:
//! `list_processes` (the currently-active processes), `resource_usage`
//! (load average and memory), `disk_usage` (root filesystem), and `uptime`
//! (seconds since boot). It reads `/proc` directly and holds no state. Like the
//! Knowledge and File Manager servers it is a thin, separate process so the
//! `rmcp` surface stays out of the apps.
//!
//! There is no per-path scope to enforce: this is the same system-wide
//! information `ps`/`top`/`uptime` show any user. Read-only means there is no
//! write tool to authorize per session (§4.2); the AI permission level still
//! governs whether the daemon routes to this server at all.

pub mod sysinfo;

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use os_sdk::mcp::rmcp;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, JsonObject, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};

/// Well-known socket id for the system System-Monitor server. Resolves to
/// `$XDG_RUNTIME_DIR/arlen/mcp/system.monitor.sock` via the `os-sdk` path
/// helper (`mcp-server-layer.md` §6.2).
pub const SERVER_ID: &str = "system.monitor";

/// Active-process listing tool.
pub const LIST_TOOL: &str = "list_processes";

/// Resource-usage tool (load average, memory).
pub const RES_TOOL: &str = "resource_usage";

/// Disk-usage tool (root filesystem total/available).
pub const DISK_TOOL: &str = "disk_usage";

/// Uptime tool (seconds since boot plus a human-readable form).
pub const UPTIME_TOOL: &str = "uptime";

/// The OS-identity tool name (kernel, release, hostname).
pub const OS_TOOL: &str = "os_info";

/// Per-call wall budget. `/proc` reads are local and fast, so this is just a
/// backstop against a pathological stall; on timeout the call returns a tool
/// error rather than hanging the caller.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Read-only System Monitor MCP server. Stateless: each call reads `/proc`
/// fresh. The `os-sdk` accept loop builds a fresh handle per admitted
/// connection.
#[derive(Clone, Default)]
pub struct SystemMonitorMcp;

impl SystemMonitorMcp {
    /// Build a server handle.
    pub fn new() -> Self {
        Self
    }

    /// A tool that takes no arguments.
    fn no_arg_tool(name: &str, description: &str) -> Tool {
        let schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {}
        }))
        .expect("the static no-arg schema is a valid JSON object");
        Tool::new_with_raw(
            name.to_owned(),
            Some(Cow::Owned(description.to_owned())),
            Arc::new(schema),
        )
    }
}

impl ServerHandler for SystemMonitorMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Read-only system status. 'list_processes' returns the currently-active processes (name + state); 'resource_usage' returns the load average and memory; 'disk_usage' returns the root filesystem total/available; 'uptime' returns seconds since boot plus a human-readable form. The same information ps/top/df/uptime show; no arguments.",
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(vec![
            Self::no_arg_tool(
                LIST_TOOL,
                "List the currently-active processes (name and state). Read-only, no arguments.",
            ),
            Self::no_arg_tool(
                RES_TOOL,
                "Return load average (1/5/15 min) and memory (total/available kB). Read-only, no arguments.",
            ),
            Self::no_arg_tool(
                DISK_TOOL,
                "Return root filesystem disk usage (total/available bytes). Read-only, no arguments.",
            ),
            Self::no_arg_tool(
                UPTIME_TOOL,
                "Return system uptime (seconds since boot and a human-readable form). Read-only, no arguments.",
            ),
            Self::no_arg_tool(
                OS_TOOL,
                "Return OS identity: kernel name, kernel release and hostname. Read-only, no arguments.",
            ),
        ]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let tool = request.name.to_string();
        if tool != LIST_TOOL
            && tool != RES_TOOL
            && tool != DISK_TOOL
            && tool != UPTIME_TOOL
            && tool != OS_TOOL
        {
            return Err(McpError::invalid_request(format!("unknown tool: {tool}"), None));
        }

        // Read off the async runtime (sync `/proc` + `statvfs`), under a wall
        // budget.
        let work = tokio::task::spawn_blocking(move || {
            let reader = sysinfo::ProcReader::new();
            match tool.as_str() {
                LIST_TOOL => serde_json::to_value(reader.list_processes()),
                RES_TOOL => serde_json::to_value(reader.resource_usage()),
                UPTIME_TOOL => serde_json::to_value(reader.uptime()),
                OS_TOOL => serde_json::to_value(reader.os_info()),
                _ => serde_json::to_value(sysinfo::disk_usage("/")),
            }
        });

        match tokio::time::timeout(READ_TIMEOUT, work).await {
            Ok(Ok(Ok(value))) => {
                let json = serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string());
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            Ok(Ok(Err(e))) => Ok(CallToolResult::error(vec![Content::text(format!(
                "could not serialise system info: {e}"
            ))])),
            Ok(Err(join)) => Ok(CallToolResult::error(vec![Content::text(format!(
                "internal error: {join}"
            ))])),
            Err(_elapsed) => Ok(CallToolResult::error(vec![Content::text(
                "system read timed out".to_string(),
            )])),
        }
    }
}
