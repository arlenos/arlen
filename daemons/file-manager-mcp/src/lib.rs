//! File Manager read-only MCP server (`mcp-server-layer.md` §4.1, default-permit).
//!
//! Exposes two read-only tools to the AI daemon over MCP: `list_directory`
//! (entries under a directory) and `file_metadata` (one path's attributes).
//! It reads the filesystem directly and holds no state beyond its access
//! scope. Like the Knowledge server it is a thin, separate process so the
//! `rmcp` surface stays out of the apps.
//!
//! Access is **fail-closed** and capability-based (see [`scope`]): the server
//! reads only paths under a configured allowlist of canonical roots
//! (`~/.config/arlen/file-manager-mcp.toml`, `[scope] roots = [...]`), each held
//! as an open directory capability, and that allowlist is **empty by default**,
//! so a fresh server exposes no filesystem at all. Reads go through the
//! capability, so symlink and `..` escapes are refused at access time. *Which*
//! roots to grant is the user's deliberate privacy decision; this server only
//! provides the fail-closed mechanism. Read-only means there is no write tool to
//! authorize per session (§4.2); the AI permission level still governs whether
//! the daemon routes to this server at all.
//!
//! The scope is read fresh from config on every tool call, not cached at
//! startup, so narrowing or removing roots takes effect on the next call (live
//! revocation), and a transiently-malformed config simply denies until fixed.

pub mod scope;

use std::borrow::Cow;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::sync::Semaphore;

use os_sdk::mcp::rmcp;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, JsonObject, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};

use scope::ScopeError;

/// Well-known socket id for the system File Manager server. Resolves to
/// `$XDG_RUNTIME_DIR/arlen/mcp/system.file-manager.sock` via the `os-sdk` path
/// helper (`mcp-server-layer.md` §6.2).
pub const SERVER_ID: &str = "system.file-manager";

/// Directory-listing tool.
pub const LIST_TOOL: &str = "list_directory";

/// Single-path metadata tool.
pub const META_TOOL: &str = "file_metadata";

/// Maximum directory entries returned in one listing. A larger directory is
/// truncated (with a marker) rather than returned whole, so one listing cannot
/// drive unbounded memory or response size through the MCP path.
const MAX_LIST_ENTRIES: usize = 1000;

/// Per-call wall budget for the filesystem read. A slow or wedged mount cannot
/// hang the caller: the call returns a timeout tool error. The blocking thread
/// is then abandoned to finish on its own (a `spawn_blocking` cannot be
/// cancelled); a small concurrency cap to bound abandoned threads is a
/// follow-up if a wedged mount is ever seen in practice.
const FS_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum concurrent filesystem jobs across all connections. A timed-out job
/// on a wedged mount keeps running (a `spawn_blocking` cannot be cancelled) and
/// holds its permit until it finishes, so this caps how many stuck workers can
/// accumulate: once saturated, new calls fail fast with "busy" instead of
/// parking more blocking-pool threads.
const MAX_CONCURRENT_FS: usize = 8;

/// Process-global permit pool bounding concurrent filesystem jobs.
fn fs_permits() -> &'static Arc<Semaphore> {
    static PERMITS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    PERMITS.get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_FS)))
}

/// Read-only File Manager MCP server. Stateless: it loads the access scope
/// fresh from config on each call, so the active scope always reflects the
/// current config (live revocation). The `os-sdk` accept loop builds a fresh
/// handle per admitted connection.
#[derive(Clone, Default)]
pub struct FileManagerMcp;

impl FileManagerMcp {
    /// Build a server handle.
    pub fn new() -> Self {
        Self
    }

    /// A tool taking a single required `path` string.
    fn path_tool(name: &str, description: &str) -> Tool {
        let schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "An absolute filesystem path." }
            },
            "required": ["path"]
        }))
        .expect("the static path-tool schema is a valid JSON object");
        Tool::new_with_raw(
            name.to_owned(),
            Some(Cow::Owned(description.to_owned())),
            Arc::new(schema),
        )
    }
}

/// Pull the required `path` string out of a `tools/call` argument object.
/// Separated so the argument contract is testable without an MCP transport.
fn extract_path(arguments: Option<&JsonObject>) -> Result<&str, McpError> {
    arguments
        .and_then(|m| m.get("path"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::invalid_request("the tool requires a string argument 'path'", None))
}

/// Map a [`ScopeError`] to the text shown in a (clean) tool error, so a refused
/// path comes back as a tool result the caller can read, not a transport error.
fn scope_error_text(path: &str, err: &ScopeError) -> String {
    match err {
        ScopeError::NotPermitted => {
            format!("access to '{path}' is not permitted by the configured scope")
        }
        ScopeError::Unresolvable => format!("path '{path}' does not exist or cannot be read"),
    }
}

impl ServerHandler for FileManagerMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Read-only filesystem access within a configured scope. 'list_directory' returns the entries of a directory; 'file_metadata' returns one path's attributes. Paths outside the configured scope, and symlinks leaving it, are refused.",
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(vec![
            Self::path_tool(
                LIST_TOOL,
                "List the entries of a directory (name, kind, size, modified time). Read-only.",
            ),
            Self::path_tool(
                META_TOOL,
                "Return one path's metadata (kind, size, modified time, read-only flag). Read-only.",
            ),
        ]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let tool = request.name.to_string();
        if tool != LIST_TOOL && tool != META_TOOL {
            return Err(McpError::invalid_request(format!("unknown tool: {tool}"), None));
        }
        let path = extract_path(request.arguments.as_ref())?.to_string();

        // Bound concurrent filesystem jobs: fail fast when saturated so stuck
        // workers on a wedged mount cannot exhaust the blocking pool. The permit
        // is held by the job until it finishes (even past the timeout below).
        let Ok(permit) = Arc::clone(fs_permits()).try_acquire_owned() else {
            return Ok(CallToolResult::error(vec![Content::text(
                "file manager is busy; try again".to_string(),
            )]));
        };

        // Scope check + capability read off the async runtime (sync fs), under
        // a wall budget so a wedged mount cannot hang the caller. The scope is
        // loaded fresh here so a config change is reflected at once.
        let work = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let scope = load_scope();
            let value = if tool == LIST_TOOL {
                scope
                    .list(Path::new(&path), MAX_LIST_ENTRIES)
                    .map(|l| serde_json::to_value(l).unwrap_or_default())
            } else {
                scope
                    .stat(Path::new(&path))
                    .map(|m| serde_json::to_value(m).unwrap_or_default())
            };
            value.map_err(|e| scope_error_text(&path, &e))
        });

        match tokio::time::timeout(FS_TIMEOUT, work).await {
            // A scope refusal or read error is a clean tool error (the caller
            // sees why, the connection stays up), not a transport error.
            Ok(Ok(Ok(value))) => {
                let json = serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string());
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            Ok(Ok(Err(msg))) => Ok(CallToolResult::error(vec![Content::text(msg)])),
            Ok(Err(join)) => Ok(CallToolResult::error(vec![Content::text(format!(
                "internal error: {join}"
            ))])),
            Err(_elapsed) => Ok(CallToolResult::error(vec![Content::text(
                "filesystem read timed out".to_string(),
            )])),
        }
    }
}

/// Parse the `[scope] roots = [...]` allowlist from the config text. Missing
/// file, missing section, or a malformed document all yield an empty list (the
/// fail-closed default). Pure, so the contract is testable without a config file.
pub fn parse_roots(toml_text: &str) -> Vec<String> {
    let Ok(doc) = toml_text.parse::<toml::Table>() else {
        return Vec::new();
    };
    doc.get("scope")
        .and_then(toml::Value::as_table)
        .and_then(|t| t.get("roots"))
        .and_then(toml::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Path to the scope config: `$XDG_CONFIG_HOME/arlen/file-manager-mcp.toml`.
pub fn config_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_default()
        .join("arlen")
        .join("file-manager-mcp.toml")
}

/// Load the access scope from a specific config file. A missing or unreadable
/// config yields an empty (deny-all) scope, so the server is fail-closed.
pub fn load_scope_from(path: &Path) -> scope::Scope {
    let roots = std::fs::read_to_string(path)
        .map(|t| parse_roots(&t))
        .unwrap_or_default();
    scope::Scope::new(roots)
}

/// Load the access scope from the default config path. Called fresh on each
/// tool call, so a config change takes effect on the next call.
pub fn load_scope() -> scope::Scope {
    load_scope_from(&config_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_tools_are_named() {
        assert_eq!(FileManagerMcp::path_tool(LIST_TOOL, "d").name, LIST_TOOL);
        assert_eq!(FileManagerMcp::path_tool(META_TOOL, "d").name, META_TOOL);
    }

    #[test]
    fn extract_path_requires_a_string() {
        let args: JsonObject = serde_json::from_value(serde_json::json!({ "path": "/x" })).unwrap();
        assert_eq!(extract_path(Some(&args)).unwrap(), "/x");
        let empty: JsonObject = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(extract_path(Some(&empty)).is_err());
        assert!(extract_path(None).is_err());
    }

    #[test]
    fn parse_roots_reads_the_allowlist() {
        let text = "[scope]\nroots = [\"/home/tim/notes\", \"/srv/shared\"]\n";
        assert_eq!(parse_roots(text), vec!["/home/tim/notes", "/srv/shared"]);
    }

    #[test]
    fn load_scope_reflects_config_changes_live() {
        // A fresh load mirrors the current config: configuring a root grants
        // it, narrowing to none revokes it. Since the server loads per call,
        // this is the live-revocation path.
        let root = tempfile::tempdir().unwrap();
        let cfg = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            cfg.path(),
            format!("[scope]\nroots = [{:?}]\n", root.path().to_str().unwrap()),
        )
        .unwrap();
        assert!(!load_scope_from(cfg.path()).is_empty(), "root configured");
        std::fs::write(cfg.path(), "[scope]\nroots = []\n").unwrap();
        assert!(load_scope_from(cfg.path()).is_empty(), "narrowed to none revokes");
    }

    #[test]
    fn parse_roots_missing_or_malformed_is_empty_fail_closed() {
        assert!(parse_roots("").is_empty());
        assert!(parse_roots("not toml {{{").is_empty());
        assert!(parse_roots("[other]\nkey = 1\n").is_empty());
        assert!(parse_roots("[scope]\nroots = \"not an array\"\n").is_empty());
    }
}
