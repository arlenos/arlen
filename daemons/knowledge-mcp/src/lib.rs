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

use arlen_ai_core::graph_schema::GraphSchema;

/// The well-known socket id for the system Knowledge Graph server. Resolves to
/// `$XDG_RUNTIME_DIR/arlen/mcp/system.knowledge.sock` via the `os-sdk` path
/// helper (`mcp-server-layer.md` §6.2).
pub const SERVER_ID: &str = "system.knowledge";

/// Read-only Cypher query tool.
pub const QUERY_TOOL: &str = "query";

/// Schema-description tool: lists the queryable node and edge types so a caller
/// can write valid Cypher without guessing labels.
pub const SCHEMA_TOOL: &str = "describe_schema";

/// System fallback path of the knowledge daemon's read-query socket, used
/// when no env override and no per-user runtime dir is available.
pub const DEFAULT_KNOWLEDGE_SOCKET: &str = "/run/arlen/knowledge.sock";

/// Resolve the knowledge daemon's read-query socket path. Mirrors the
/// resolution the AI daemon and the knowledge daemon itself use, so the
/// server always points at the same socket the daemon binds:
///
/// 1. `ARLEN_KNOWLEDGE_SOCKET`: this server's own explicit override.
/// 2. `ARLEN_DAEMON_SOCKET`: the knowledge daemon's socket env, so the two
///    stay in sync when the daemon's read socket is relocated.
/// 3. `$XDG_RUNTIME_DIR/arlen/knowledge.sock`: the normal per-user session.
/// 4. [`DEFAULT_KNOWLEDGE_SOCKET`]: the system fallback.
pub fn knowledge_socket_path() -> String {
    resolve_socket(
        std::env::var("ARLEN_KNOWLEDGE_SOCKET").ok(),
        std::env::var("ARLEN_DAEMON_SOCKET").ok(),
        std::env::var("XDG_RUNTIME_DIR").ok(),
    )
}

/// Pure socket resolution over the three env inputs. Separated from
/// [`knowledge_socket_path`] so the precedence can be tested without touching
/// process-global environment.
fn resolve_socket(
    explicit: Option<String>,
    daemon: Option<String>,
    xdg: Option<String>,
) -> String {
    let nonempty = |v: Option<String>| v.filter(|s| !s.is_empty());
    if let Some(p) = nonempty(explicit) {
        return p;
    }
    if let Some(p) = nonempty(daemon) {
        return p;
    }
    if let Some(dir) = nonempty(xdg) {
        return format!("{dir}/arlen/knowledge.sock");
    }
    DEFAULT_KNOWLEDGE_SOCKET.to_string()
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

    /// The MCP tool definition for the schema description. Takes no arguments.
    fn schema_tool() -> Tool {
        let schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {}
        }))
        .expect("the static schema-tool input is a valid JSON object");
        Tool::new_with_raw(
            SCHEMA_TOOL.to_owned(),
            Some(Cow::Borrowed(
                "Describe the Knowledge Graph schema: the queryable node labels with their fields and the edge types with their endpoints. Use this to write valid Cypher.",
            )),
            Arc::new(schema),
        )
    }
}

/// Render the canonical graph schema as JSON: node labels with their typed
/// fields, and edge labels with their from/to endpoints. Sourced from the
/// shared `arlen_ai_core::graph_schema` so it cannot drift from the daemon's
/// actual tables. Pure, so it is testable without an MCP transport.
fn schema_json() -> serde_json::Value {
    let schema = GraphSchema::knowledge_graph();
    let nodes: Vec<serde_json::Value> = schema
        .node_labels()
        .filter_map(|label| schema.node(label))
        .map(|n| {
            serde_json::json!({
                "label": n.label,
                "fields": n.fields.iter().map(|(name, ty)| serde_json::json!({
                    "name": name,
                    "type": format!("{ty:?}"),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let edges: Vec<serde_json::Value> = schema
        .edge_labels()
        .filter_map(|label| schema.edge(label))
        .map(|e| serde_json::json!({ "label": e.label, "from": e.from, "to": e.to }))
        .collect();
    serde_json::json!({ "nodes": nodes, "edges": edges })
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
                "Read-only access to the Arlen Knowledge Graph. 'describe_schema' lists the queryable node and edge types; 'query' runs a read Cypher query and returns the matching rows.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(vec![
            Self::query_tool(),
            Self::schema_tool(),
        ]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        match request.name.as_ref() {
            SCHEMA_TOOL => {
                let json = serde_json::to_string(&schema_json())
                    .unwrap_or_else(|_| "{}".to_string());
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            QUERY_TOOL => {
                let cypher = extract_cypher(request.arguments.as_ref())?;
                match self.graph.query_rows(cypher).await {
                    Ok(rows) => {
                        // Rows are already JSON-typed cells; serialise the set.
                        let json = serde_json::to_string(&rows)
                            .unwrap_or_else(|_| "[]".to_string());
                        Ok(CallToolResult::success(vec![Content::text(json)]))
                    }
                    // A rejected or failed query (including a write the read
                    // socket refuses) is a clean tool error, not a transport
                    // error: the caller sees the reason and the connection stays up.
                    Err(err) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "query failed: {err}"
                    ))])),
                }
            }
            other => Err(McpError::invalid_request(
                format!("unknown tool: {other}"),
                None,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_tool_is_named() {
        assert_eq!(KnowledgeMcp::schema_tool().name, SCHEMA_TOOL);
    }

    #[test]
    fn schema_json_lists_every_node_and_edge() {
        let v = schema_json();
        let nodes = v["nodes"].as_array().expect("nodes array");
        let edges = v["edges"].as_array().expect("edges array");
        // Mirrors the counts asserted in ai-core's graph_schema tests, so a
        // schema change there surfaces here too.
        assert_eq!(nodes.len(), 10, "all node labels rendered");
        assert_eq!(edges.len(), 7, "all edge labels rendered");
        assert!(nodes.iter().any(|n| n["label"] == "File"), "File node present");
        assert!(
            edges.iter().all(|e| e["from"].is_string() && e["to"].is_string()),
            "every edge has from/to endpoints"
        );
    }

    #[test]
    fn socket_resolution_precedence() {
        // Pure over its inputs, so no process-global env race.
        assert_eq!(
            resolve_socket(Some("/x.sock".into()), Some("/d.sock".into()), Some("/run/user/1000".into())),
            "/x.sock",
            "explicit override wins"
        );
        assert_eq!(
            resolve_socket(None, Some("/d.sock".into()), Some("/run/user/1000".into())),
            "/d.sock",
            "the daemon socket env is next"
        );
        assert_eq!(
            resolve_socket(None, None, Some("/run/user/1000".into())),
            "/run/user/1000/arlen/knowledge.sock",
            "per-user runtime path is the normal session case"
        );
        assert_eq!(
            resolve_socket(Some(String::new()), None, None),
            DEFAULT_KNOWLEDGE_SOCKET,
            "empty values are ignored and fall through to the system path"
        );
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
