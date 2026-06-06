//! Knowledge Graph read-only MCP server.
//!
//! Exposes the Knowledge Graph's read interface as an MCP tool so the AI
//! daemon can query it through the same Model Context Protocol surface it
//! uses for every other server. This is a thin adapter: it speaks MCP on one
//! side and the daemon's existing read-query socket on the other, holding no
//! graph state of its own. Keeping it a separate process means the critical
//! knowledge daemon does not take on the `rmcp` dependency or the MCP serving
//! surface.
//!
//! It is a read-only server (`mcp-server-layer.md` §4.1, default-permit): the
//! one tool runs a read query and returns rows. Writes are rejected by the
//! daemon's read socket, and a rejected query comes back as a tool error, not
//! a transport error, so the caller can see what went wrong. The socket is
//! peer-authenticated by the `os-sdk` boilerplate so only the AI daemon is
//! served.

use std::borrow::Cow;
use std::sync::Arc;

use os_sdk::graph::UnixGraphClient;
use os_sdk::mcp::rmcp;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, JsonObject, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};

/// The well-known socket id for the system Knowledge Graph server. Resolves to
/// `$XDG_RUNTIME_DIR/arlen/mcp/system.knowledge.sock` via the `os-sdk` path
/// helper (`mcp-server-layer.md` §6.2).
pub const SERVER_ID: &str = "system.knowledge";

/// The single read-only tool this server exposes.
pub const QUERY_TOOL: &str = "query";

/// Default path of the knowledge daemon's read-query socket. Overridable with
/// `ARLEN_KNOWLEDGE_SOCKET` for tests and non-default layouts.
pub const DEFAULT_KNOWLEDGE_SOCKET: &str = "/run/arlen/knowledge.sock";

/// Resolve the knowledge daemon's read-query socket path, honouring
/// `ARLEN_KNOWLEDGE_SOCKET` and falling back to [`DEFAULT_KNOWLEDGE_SOCKET`].
pub fn knowledge_socket_path() -> String {
    std::env::var("ARLEN_KNOWLEDGE_SOCKET")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_KNOWLEDGE_SOCKET.to_string())
}

/// Read-only MCP server over the Knowledge Graph. Holds a lazy client to the
/// daemon's read socket; the `os-sdk` accept loop builds a fresh instance per
/// admitted connection.
#[derive(Clone)]
pub struct KnowledgeMcp {
    graph: Arc<UnixGraphClient>,
}

impl KnowledgeMcp {
    /// Build a server that queries the knowledge daemon at `knowledge_socket`.
    /// The client connects lazily on the first query.
    pub fn new(knowledge_socket: impl Into<String>) -> Self {
        Self {
            graph: Arc::new(UnixGraphClient::new(knowledge_socket)),
        }
    }

    /// The MCP tool definition for the read query.
    fn query_tool() -> Tool {
        let schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "cypher": {
                    "type": "string",
                    "description": "A read-only Cypher query against the Knowledge Graph."
                }
            },
            "required": ["cypher"]
        }))
        .expect("the static query-tool schema is valid JSON object");
        Tool::new_with_raw(
            QUERY_TOOL.to_owned(),
            Some(Cow::Borrowed(
                "Run a read-only Cypher query against the Knowledge Graph and return the matching rows as JSON.",
            )),
            Arc::new(schema),
        )
    }
}

/// Pull the required `cypher` string out of a `tools/call` argument object.
/// Kept separate from the async handler so the argument contract can be tested
/// without an MCP transport.
fn extract_cypher(arguments: Option<&JsonObject>) -> Result<&str, McpError> {
    arguments
        .and_then(|m| m.get("cypher"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            McpError::invalid_request(
                "tool 'query' requires a string argument 'cypher'",
                None,
            )
        })
}

impl ServerHandler for KnowledgeMcp {
    fn get_info(&self) -> ServerInfo {
        // `ServerInfo` (`InitializeResult`) is `#[non_exhaustive]`; build it
        // through the constructor rather than a struct literal.
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "Read-only access to the Arlen Knowledge Graph. The 'query' tool runs a read Cypher query and returns the matching rows.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(vec![Self::query_tool()]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        if request.name != QUERY_TOOL {
            return Err(McpError::invalid_request(
                format!("unknown tool: {}", request.name),
                None,
            ));
        }
        let cypher = extract_cypher(request.arguments.as_ref())?;
        match self.graph.query_rows(cypher).await {
            Ok(rows) => {
                // Rows are already JSON-typed cells; serialise the set.
                let json = serde_json::to_string(&rows)
                    .unwrap_or_else(|_| "[]".to_string());
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            // A rejected or failed query (including a write the read socket
            // refuses) is a clean tool error, not a transport error: the
            // caller sees the reason and the connection stays up.
            Err(err) => Ok(CallToolResult::error(vec![Content::text(format!(
                "query failed: {err}"
            ))])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knowledge_socket_path_defaults_when_env_unset() {
        // Not asserting on the env-set branch: the variable is process-global
        // and racing it against other tests is the bug we avoid elsewhere.
        if std::env::var("ARLEN_KNOWLEDGE_SOCKET").is_err() {
            assert_eq!(knowledge_socket_path(), DEFAULT_KNOWLEDGE_SOCKET);
        }
    }

    #[test]
    fn query_tool_is_named_and_requires_cypher() {
        let tool = KnowledgeMcp::query_tool();
        assert_eq!(tool.name, QUERY_TOOL);
        let schema = serde_json::to_value(&*tool.input_schema).unwrap();
        assert_eq!(schema["properties"]["cypher"]["type"], "string");
        assert_eq!(schema["required"][0], "cypher");
    }

    #[test]
    fn extract_cypher_reads_the_string_argument() {
        let args: JsonObject =
            serde_json::from_value(serde_json::json!({ "cypher": "MATCH (n) RETURN n LIMIT 1" }))
                .unwrap();
        assert_eq!(
            extract_cypher(Some(&args)).unwrap(),
            "MATCH (n) RETURN n LIMIT 1"
        );
    }

    #[test]
    fn extract_cypher_rejects_missing_or_non_string() {
        assert!(extract_cypher(None).is_err());
        let no_field: JsonObject =
            serde_json::from_value(serde_json::json!({ "other": 1 })).unwrap();
        assert!(extract_cypher(Some(&no_field)).is_err());
        let wrong_type: JsonObject =
            serde_json::from_value(serde_json::json!({ "cypher": 42 })).unwrap();
        assert!(extract_cypher(Some(&wrong_type)).is_err());
    }
}
