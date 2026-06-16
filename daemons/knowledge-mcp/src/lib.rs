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

use arlen_graph_schema::GraphSchema;

/// The well-known socket id for the system Knowledge Graph server. Resolves to
/// `$XDG_RUNTIME_DIR/arlen/mcp/system.knowledge.sock` via the `os-sdk` path
/// helper (`mcp-server-layer.md` §6.2).
pub const SERVER_ID: &str = "system.knowledge";

/// Read-only Cypher query tool.
pub const QUERY_TOOL: &str = "query";

/// Schema-description tool: lists the queryable node and edge types so a caller
/// can write valid Cypher without guessing labels.
pub const SCHEMA_TOOL: &str = "describe_schema";

/// Code-graph analysis tool (CG-R5): the token-free god-symbols + surprises over
/// the whole `CodeSymbol` call graph. The daemon gates it to system-anchored
/// callers; over MCP the agent reaches it as `knowledge-mcp` (a FirstParty
/// principal), so a third-party app cannot use the agent as a proxy for it.
pub const CODE_ANALYSIS_TOOL: &str = "code_analysis";

/// Code-symbol-context tool (CG-R6): a symbol's defining file, the project that
/// file belongs to (bitemporally, optionally as-of a timestamp), and the apps
/// that accessed it. Daemon-gated to system-anchored callers, same as
/// [`CODE_ANALYSIS_TOOL`]; the agent reaches it as a FirstParty principal.
pub const CODE_SYMBOL_TOOL: &str = "code_symbol_context";

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

    /// The MCP tool definition for the code-graph analysis (CG-R5). Takes no
    /// arguments.
    fn code_analysis_tool() -> Tool {
        let schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {}
        }))
        .expect("the static code-analysis-tool input is a valid JSON object");
        Tool::new_with_raw(
            CODE_ANALYSIS_TOOL.to_owned(),
            Some(Cow::Borrowed(
                "Analyse the code graph: the god-symbols (the most-coupled functions/types by call degree) and surprises (the lone calls bridging two modules). Token-free graph metrics, returned as JSON {god_symbols, surprises} for explaining the codebase's structure.",
            )),
            Arc::new(schema),
        )
    }

    /// The MCP tool definition for the code-symbol-context fusion (CG-R6).
    fn code_symbol_context_tool() -> Tool {
        let schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "symbol_id": {
                    "type": "string",
                    "description": "The CodeSymbol node id (e.g. \"/p/lib.rs#function:helper@1\")."
                },
                "as_of_micros": {
                    "type": "integer",
                    "description": "Optional bitemporal as-of (microseconds since epoch). Omit for the current (live) project membership."
                }
            },
            "required": ["symbol_id"]
        }))
        .expect("the static code-symbol-tool schema is a valid JSON object");
        Tool::new_with_raw(
            CODE_SYMBOL_TOOL.to_owned(),
            Some(Cow::Borrowed(
                "Resolve a code symbol's activity context: its defining file, the project that file belongs to (optionally as of a past time), and the apps that have accessed it. Returns JSON {symbol_id, file_path, project, accessed_by}.",
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

/// Pull the `symbol_id` (required) and `as_of_micros` (optional) out of a
/// `code_symbol_context` call's arguments. Separate from the async handler so
/// the argument contract is testable without an MCP transport.
fn extract_code_symbol_args(
    arguments: Option<&JsonObject>,
) -> Result<(String, Option<i64>), McpError> {
    let symbol_id = arguments
        .and_then(|m| m.get("symbol_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            McpError::invalid_request(
                "tool 'code_symbol_context' requires a string argument 'symbol_id'",
                None,
            )
        })?
        .to_owned();
    let as_of = arguments
        .and_then(|m| m.get("as_of_micros"))
        .and_then(serde_json::Value::as_i64);
    Ok((symbol_id, as_of))
}

impl ServerHandler for KnowledgeMcp {
    fn get_info(&self) -> ServerInfo {
        // `ServerInfo` (`InitializeResult`) is `#[non_exhaustive]`; build it
        // through the constructor rather than a struct literal.
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "Read-only access to the Arlen Knowledge Graph. 'describe_schema' lists the queryable node and edge types; 'query' runs a read Cypher query and returns the matching rows; 'code_analysis' returns the code graph's god-symbols and surprises; 'code_symbol_context' resolves a code symbol's file, project and access provenance.",
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
            Self::code_analysis_tool(),
            Self::code_symbol_context_tool(),
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
            CODE_ANALYSIS_TOOL => match self.graph.code_analysis().await {
                Ok(analysis) => {
                    let json =
                        serde_json::to_string(&analysis).unwrap_or_else(|_| "{}".to_string());
                    Ok(CallToolResult::success(vec![Content::text(json)]))
                }
                // A denied (not system-anchored) or failed analysis is a clean
                // tool error, not a transport error: the caller sees the reason
                // and the connection stays up.
                Err(err) => Ok(CallToolResult::error(vec![Content::text(format!(
                    "code analysis failed: {err}"
                ))])),
            },
            CODE_SYMBOL_TOOL => {
                let (symbol_id, as_of) = extract_code_symbol_args(request.arguments.as_ref())?;
                match self.graph.code_symbol_context(&symbol_id, as_of).await {
                    Ok(ctx) => {
                        let json =
                            serde_json::to_string(&ctx).unwrap_or_else(|_| "{}".to_string());
                        Ok(CallToolResult::success(vec![Content::text(json)]))
                    }
                    // A denied (not system-anchored) or failed read is a clean
                    // tool error, not a transport error.
                    Err(err) => Ok(CallToolResult::error(vec![Content::text(format!(
                        "code symbol context failed: {err}"
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
    fn code_symbol_context_tool_requires_symbol_id() {
        let tool = KnowledgeMcp::code_symbol_context_tool();
        assert_eq!(tool.name, CODE_SYMBOL_TOOL);
        let schema = serde_json::to_value(&*tool.input_schema).unwrap();
        assert_eq!(schema["required"], serde_json::json!(["symbol_id"]));
        assert!(schema["properties"]["as_of_micros"].is_object());
    }

    #[test]
    fn extract_code_symbol_args_parses_id_and_optional_as_of() {
        let with_as_of: JsonObject = serde_json::from_value(serde_json::json!({
            "symbol_id": "/p/lib.rs#fn:helper@1",
            "as_of_micros": 150
        }))
        .unwrap();
        assert_eq!(
            extract_code_symbol_args(Some(&with_as_of)).unwrap(),
            ("/p/lib.rs#fn:helper@1".to_string(), Some(150))
        );

        let just_id: JsonObject =
            serde_json::from_value(serde_json::json!({"symbol_id": "x"})).unwrap();
        assert_eq!(
            extract_code_symbol_args(Some(&just_id)).unwrap(),
            ("x".to_string(), None)
        );

        // Missing symbol_id is a clean request error.
        let empty: JsonObject = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(extract_code_symbol_args(Some(&empty)).is_err());
        assert!(extract_code_symbol_args(None).is_err());
    }

    #[test]
    fn code_analysis_tool_is_named_and_takes_no_args() {
        let tool = KnowledgeMcp::code_analysis_tool();
        assert_eq!(tool.name, CODE_ANALYSIS_TOOL);
        let schema = serde_json::to_value(&*tool.input_schema).unwrap();
        // No required arguments: the op is selected by name alone.
        assert!(schema.get("required").is_none());
        assert_eq!(schema["properties"], serde_json::json!({}));
    }

    #[test]
    fn schema_json_lists_every_node_and_edge() {
        let v = schema_json();
        let nodes = v["nodes"].as_array().expect("nodes array");
        let edges = v["edges"].as_array().expect("edges array");
        // Render EVERY label the canonical schema declares, dropping none.
        // Counted against the schema itself rather than a hardcoded number, so
        // adding a node/edge type (e.g. the CodeSymbol/CALLS code graph) never
        // silently desyncs this test from the schema it mirrors.
        let schema = GraphSchema::knowledge_graph();
        assert_eq!(nodes.len(), schema.node_labels().count(), "all node labels rendered");
        assert_eq!(edges.len(), schema.edge_labels().count(), "all edge labels rendered");
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
