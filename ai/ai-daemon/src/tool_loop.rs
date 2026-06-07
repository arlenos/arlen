//! Interactive tool-use loop for the AI daemon (see
//! `docs/architecture/ai-tool-routing.md`).
//!
//! This is the prompt construction for the bounded tool-use loop: the daemon's
//! own prompt shape (a user question, the available MCP tool catalogue, and the
//! transcript of prior tool calls and results), distinct from the ai-agent's
//! behaviour-driven prompt. The orchestration (provider call, parse, gated
//! dispatch, budget) lands in later slices behind a default-off flag; this
//! slice is the pure, testable prompt builder.

use arlen_ai_core::audit::{self, AuditSink};
use arlen_ai_core::graph_query::QueryScope;
use arlen_ai_core::mcp::{
    AlwaysConfirmReason, CallChain, CallDecision, CatalogueTool, McpClient, ServerClass, ServerId,
};
use arlen_ai_core::pipeline::{extract_json, QueryRunner};
use arlen_ai_core::provider::{AIProvider, CompletionRequest};
use arlen_ai_core::tagging::{Block, Origin, TaggedPrompt};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Reserved server id of the internal, scope-enforcing graph tool. Graph
/// access in the interactive loop goes only through here, never through the
/// raw-Cypher knowledge MCP server: this tool routes through the validated
/// [`QueryRunner`], which checks every query against the caller's
/// [`QueryScope`] (the per-tier label scope derived from `access_level`). The
/// `system.` namespace is reserved from module ids, so no module can shadow it.
const GRAPH_TOOL_SERVER: &str = "system.graph";
/// The internal graph tool's only tool name: a natural-language question.
const GRAPH_TOOL_NAME: &str = "query";
/// The raw-Cypher knowledge MCP server. Its `query` tool accepts arbitrary
/// Cypher and cannot carry the per-tier label scope, so it is withheld from
/// the interactive catalogue AND refused if the model addresses it directly:
/// all graph access is routed through the scoped [`GRAPH_TOOL_SERVER`].
const RAW_KNOWLEDGE_SERVER: &str = "system.knowledge";

/// One completed step of the loop: a tool call and the result it returned.
/// The result is treated as data (an origin-tagged block), never instructions.
#[derive(Debug, Clone)]
pub struct ToolStep {
    /// The server the tool was called on.
    pub server: String,
    /// The tool name.
    pub tool: String,
    /// The call arguments, as a JSON string.
    pub arguments: String,
    /// The tool's result text.
    pub result: String,
}

/// Render the tool catalogue as a readable list for the prompt.
fn render_catalogue(catalogue: &[CatalogueTool]) -> String {
    if catalogue.is_empty() {
        return "(no tools available)".to_string();
    }
    catalogue
        .iter()
        .map(|c| {
            let desc = c.description.as_deref().unwrap_or("");
            format!("- {}/{}: {desc}", c.server.as_str(), c.name)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render the prior tool calls and results as the loop's working transcript.
fn render_transcript(transcript: &[ToolStep]) -> String {
    transcript
        .iter()
        .enumerate()
        .map(|(i, s)| {
            format!(
                "step {}: called {}/{} with {} -> {}",
                i + 1,
                s.server,
                s.tool,
                s.arguments,
                s.result
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Build the per-step prompt: static instructions plus the available tool
/// catalogue, then the user question and prior tool results as origin-tagged
/// data blocks (S18-A) so the model never treats them as instructions. The
/// model is told to reply with exactly one JSON object: a tool call or a final
/// answer.
pub fn build_tool_prompt(
    question: &str,
    catalogue: &[CatalogueTool],
    transcript: &[ToolStep],
) -> String {
    let tools = render_catalogue(catalogue);
    let instruction = format!(
        "You are the Arlen assistant. Answer the user's question, using the \
         available tools to gather information when you need it.\n\n\
         Available tools (server/tool: description):\n{tools}\n\n\
         Respond with EXACTLY one JSON object and nothing else, either calling \
         one tool or giving the final answer:\n\
         {{\"action\": \"call_tool\", \"server\": \"<server id>\", \"tool\": \"<tool name>\", \"arguments\": {{...}}}}\n\
         {{\"action\": \"answer\", \"text\": \"<the answer, in plain language>\"}}\n\
         Call a tool only when you need more information; otherwise answer."
    );
    // The question and prior tool results are data, tagged by origin. Tool
    // results come from the read-only servers (the knowledge graph), so they
    // are GRAPH-DATA; the question is USER-QUESTION.
    let transcript_text = render_transcript(transcript);
    let mut blocks = vec![Block {
        origin: Origin::UserInput,
        content: question,
    }];
    if !transcript_text.is_empty() {
        blocks.push(Block {
            origin: Origin::GraphData,
            content: &transcript_text,
        });
    }
    let tagged = TaggedPrompt::new(&blocks);
    format!("{instruction}\n\n{}\n{}", tagged.preamble(), tagged.rendered())
}

/// One parsed step the model produced in the loop: either call a tool or give
/// the final answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopStep {
    /// Call `tool` on `server` with JSON `arguments`.
    CallTool {
        /// The MCP server id to call.
        server: String,
        /// The tool name on that server.
        tool: String,
        /// The call arguments, as a JSON string (`{}` if the model omitted them).
        arguments: String,
    },
    /// The final answer in plain language.
    Answer {
        /// The answer text.
        text: String,
    },
}

/// The model's reply, before validation.
#[derive(Deserialize)]
struct RawStep {
    action: String,
    server: Option<String>,
    tool: Option<String>,
    arguments: Option<serde_json::Value>,
    text: Option<String>,
}

/// Parse the model's reply into a [`LoopStep`]. Fails closed (an error, not a
/// guess) on a missing JSON object, malformed JSON, an unknown action, or a
/// tool call missing its server or tool.
pub fn parse_loop_step(text: &str) -> Result<LoopStep, String> {
    let json = extract_json(text).ok_or("no JSON object in the response")?;
    let raw: RawStep =
        serde_json::from_str(json).map_err(|e| format!("invalid step JSON: {e}"))?;
    match raw.action.as_str() {
        "call_tool" => {
            let server = raw
                .server
                .filter(|s| !s.is_empty())
                .ok_or("a call_tool step must name a non-empty 'server'")?;
            let tool = raw
                .tool
                .filter(|t| !t.is_empty())
                .ok_or("a call_tool step must name a non-empty 'tool'")?;
            let arguments = raw
                .arguments
                .map(|v| v.to_string())
                .unwrap_or_else(|| "{}".to_string());
            Ok(LoopStep::CallTool {
                server,
                tool,
                arguments,
            })
        }
        "answer" => Ok(LoopStep::Answer {
            text: raw.text.unwrap_or_default(),
        }),
        other => Err(format!("unknown step action {other:?}")),
    }
}

/// The outcome of trying to dispatch one tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    /// The tool ran; here is its result text.
    Result(String),
    /// Blocked: the tool is in the hardcoded always-confirm set and needs
    /// explicit user confirmation (`mcp-server-layer.md` §4.3).
    NeedsConfirmation(AlwaysConfirmReason),
    /// Blocked: an action server with no per-session authorization grant.
    NeedsAuthorization,
    /// The call was attempted but failed (unknown server, depth exceeded,
    /// transport or server error).
    Failed(String),
}

/// Map a non-allowing gate decision to its blocked outcome. `Allow` returns
/// `None`: the caller dispatches. Pure, so the gate-to-outcome mapping is
/// tested without a live server.
fn outcome_for_blocked(decision: CallDecision) -> Option<DispatchOutcome> {
    match decision {
        CallDecision::Allow => None,
        CallDecision::NeedsConfirmation(reason) => {
            Some(DispatchOutcome::NeedsConfirmation(reason))
        }
        CallDecision::NeedsAuthorization => Some(DispatchOutcome::NeedsAuthorization),
    }
}

/// Gate one tool call, then dispatch it if allowed.
///
/// Applies `McpClient::decide` (the read-only-vs-action gate plus the §4.3
/// always-confirm list); a non-allowing decision returns the blocked outcome
/// without touching the server. An allowed call is dispatched via `call_tool`
/// (which audits content-free and enforces the call-chain depth). A blocked
/// call must not be silently dropped: the loop surfaces it for confirmation or
/// authorization.
pub async fn gated_dispatch(
    client: &McpClient,
    server: &str,
    tool: &str,
    arguments: &str,
    has_grant: bool,
    chain: &CallChain,
) -> DispatchOutcome {
    let id = ServerId(server.to_string());
    let decision = match client.decide(&id, tool, has_grant) {
        Ok(d) => d,
        Err(e) => return DispatchOutcome::Failed(e.to_string()),
    };
    if let Some(blocked) = outcome_for_blocked(decision) {
        return blocked;
    }
    // Allowed. Parse and validate the arguments. A malformed or non-object
    // argument string fails the call closed: it must NOT silently become an
    // empty object, which for a tool with optional or defaulted parameters
    // could dispatch a broader or different request than the model produced.
    let args = match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(v) if v.is_object() => v,
        Ok(_) => {
            return DispatchOutcome::Failed("tool arguments must be a JSON object".to_string())
        }
        Err(e) => return DispatchOutcome::Failed(format!("invalid tool arguments: {e}")),
    };
    match client.call_tool(&id, tool, args, chain).await {
        Ok(result) => DispatchOutcome::Result(result),
        Err(e) => DispatchOutcome::Failed(e.to_string()),
    }
}

/// The outcome of running the interactive tool-use loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopOutcome {
    /// The model produced a final answer.
    Answer(String),
    /// A tool call was blocked by the gate (needs confirmation or
    /// authorization); the loop stops and surfaces it rather than proceeding.
    Blocked(DispatchOutcome),
    /// A trust-boundary or policy denial: the model addressed a path it is
    /// not permitted to take (e.g. the raw-Cypher knowledge server directly,
    /// instead of the scoped graph tool). Terminal: the loop stops and
    /// surfaces the denial, so a hallucinated or injection-driven boundary
    /// hit cannot be masked behind a fabricated final answer. Distinct from an
    /// ordinary transient tool error, which is fed back for the model to
    /// adjust.
    Denied(String),
    /// The step budget ran out before a final answer.
    Exhausted,
    /// The loop could not proceed: a provider error or an unparseable reply.
    Failed(String),
}

/// Shape the catalogue the model sees in the interactive loop: prepend the
/// internal scope-enforcing graph tool and drop the raw-Cypher knowledge
/// server's tools. The model reaches the Knowledge Graph only through the
/// scoped tool; the raw server is never offered (and is refused at dispatch
/// even if the model addresses it directly), so a non-Minimal caller cannot
/// read labels outside their tier by writing raw Cypher.
fn interactive_catalogue(raw: Vec<CatalogueTool>) -> Vec<CatalogueTool> {
    let mut out = Vec::with_capacity(raw.len() + 1);
    out.push(CatalogueTool {
        server: ServerId(GRAPH_TOOL_SERVER.to_string()),
        class: ServerClass::ReadOnly,
        name: GRAPH_TOOL_NAME.to_string(),
        description: Some(
            "Ask a natural-language question of the Knowledge Graph. Returns an \
             answer scoped to your access tier. Argument: {\"question\": string}."
                .to_string(),
        ),
    });
    out.extend(
        raw.into_iter()
            .filter(|t| t.server.0 != RAW_KNOWLEDGE_SERVER),
    );
    out
}

/// Pull the natural-language `question` string out of the internal graph
/// tool's arguments. Fails closed: a missing, non-string, or malformed
/// argument is an error, never a silent empty question.
fn extract_graph_question(arguments: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|e| format!("invalid graph tool arguments: {e}"))?;
    match value.get("question").and_then(|q| q.as_str()) {
        Some(q) if !q.trim().is_empty() => Ok(q.to_string()),
        _ => Err("graph tool requires a non-empty string argument 'question'".to_string()),
    }
}

/// Run the bounded interactive tool-use loop: assemble the tool catalogue, then
/// repeatedly prompt the model, parse its step, and either answer or gate and
/// dispatch one tool call, feeding the result back, until a final answer or the
/// step budget. Read-only servers are default-permit; an action server with no
/// grant surfaces as `Blocked` instead of being called.
///
/// This is the orchestration of the loop's building blocks. It is additive and
/// wired into the daemon's query path behind a default-off flag
/// (`docs/architecture/ai-tool-routing.md`).
///
/// The shared [`McpClient`] is taken as a `&Mutex` and locked only around the
/// individual client operations (catalogue assembly and each tool call), never
/// across the provider completion. Holding the lock across the whole loop would
/// stall discovery's health/reconnect work and, worse, would block other
/// dispatch tasks on lock acquisition where they could not observe their own
/// cancellation. Locking per operation keeps the loop's `await` points free for
/// the caller's `select!` to fire a cancellation even while a tool call is
/// in flight.
pub async fn run_tool_loop(
    client: &Mutex<McpClient>,
    runner: &dyn QueryRunner,
    scope: &QueryScope,
    audit: Arc<dyn AuditSink>,
    query_id: &str,
    provider: &dyn AIProvider,
    question: &str,
    max_steps: u32,
) -> LoopOutcome {
    let raw_catalogue = client.lock().await.tool_catalogue().await;
    let catalogue = interactive_catalogue(raw_catalogue);
    // Seed the call chain from the query id so every tool entry in this loop
    // (MCP tools via call_tool, and the internal graph reads below) joins the
    // query's own dispatch/completion audit records under one correlation id.
    // The query id is a v4 UUID string; fall back to a fresh id if it is ever
    // not parseable rather than failing the loop.
    let chain = CallChain {
        id: uuid::Uuid::parse_str(query_id).unwrap_or_else(|_| uuid::Uuid::new_v4()),
        depth: 1,
    };
    let mut transcript: Vec<ToolStep> = Vec::new();

    for _ in 0..max_steps {
        let prompt = build_tool_prompt(question, &catalogue, &transcript);
        // Provider completion runs unlocked: it is the slow, network-bound
        // step, and the MCP client is not touched here.
        let reply = match provider
            .complete(CompletionRequest {
                prompt,
                extras: serde_json::json!({}),
            })
            .await
        {
            Ok(r) => r.text,
            Err(e) => return LoopOutcome::Failed(format!("provider error: {e}")),
        };
        match parse_loop_step(&reply) {
            Ok(LoopStep::Answer { text }) => return LoopOutcome::Answer(text),
            Ok(LoopStep::CallTool {
                server,
                tool,
                arguments,
            }) => {
                let outcome = if server == GRAPH_TOOL_SERVER {
                    // The scope-enforcing graph tool: route through the
                    // validated QueryRunner so every query is checked against
                    // the caller's per-tier QueryScope.
                    if tool != GRAPH_TOOL_NAME {
                        DispatchOutcome::Failed(format!(
                            "unknown tool '{tool}' on {GRAPH_TOOL_SERVER}"
                        ))
                    } else {
                        match extract_graph_question(&arguments) {
                            Ok(q) => {
                                // Audit-before-action (foundation §8.4.6),
                                // matching the MCP path: a fail-closed dispatch
                                // entry before the read, then a best-effort
                                // outcome entry. A read inside an already
                                // admitted query is still recorded per call, so
                                // a mid-loop ledger outage is caught and the
                                // activity surface sees every graph access.
                                if audit
                                    .submit(audit::mcp_event(
                                        GRAPH_TOOL_SERVER,
                                        "dispatched",
                                        chain.depth,
                                        &chain.id.to_string(),
                                        true,
                                    ))
                                    .await
                                    .is_err()
                                {
                                    return LoopOutcome::Failed(
                                        "graph query refused: audit log unavailable".to_string(),
                                    );
                                }
                                let outcome = match runner.run_query(&q, scope).await {
                                    Ok(answer) => DispatchOutcome::Result(answer),
                                    Err(f) => DispatchOutcome::Failed(f.reason),
                                };
                                let label = match &outcome {
                                    DispatchOutcome::Result(_) => "ok",
                                    _ => "failed",
                                };
                                let _ = audit
                                    .submit(audit::mcp_event(
                                        GRAPH_TOOL_SERVER,
                                        label,
                                        chain.depth,
                                        &chain.id.to_string(),
                                        true,
                                    ))
                                    .await;
                                outcome
                            }
                            Err(e) => DispatchOutcome::Failed(e),
                        }
                    }
                } else if server == RAW_KNOWLEDGE_SERVER {
                    // Refuse raw-Cypher graph access in the interactive loop:
                    // it cannot carry the per-tier label scope. This is a
                    // trust-boundary hit (likely hallucinated or
                    // injection-driven), so record it as a policy violation and
                    // end the loop denied, rather than feeding the error back
                    // where the model could continue and mask it behind a
                    // fabricated answer.
                    //
                    // The record is made DURABLE against a concurrent
                    // cancellation: the dispatch select races this loop future
                    // against the caller's cancel token, and a cancel can drop
                    // this future at the await below. Submitting on a spawned
                    // task means the policy-violation entry still lands even if
                    // this future is dropped (a tokio task is not cancelled
                    // when its JoinHandle is dropped), so a caller cannot hide a
                    // boundary probe by cancelling. Awaiting the handle keeps
                    // the normal (uncancelled) path ordered and deterministic.
                    let recorder = {
                        let audit = audit.clone();
                        let chain_id = chain.id.to_string();
                        let depth = chain.depth;
                        tokio::spawn(async move {
                            audit
                                .submit(audit::mcp_event(
                                    RAW_KNOWLEDGE_SERVER,
                                    "policy-denied",
                                    depth,
                                    &chain_id,
                                    true,
                                ))
                                .await
                        })
                    };
                    let _ = recorder.await;
                    return LoopOutcome::Denied(format!(
                        "{RAW_KNOWLEDGE_SERVER} is not callable directly; \
                         use {GRAPH_TOOL_SERVER}/{GRAPH_TOOL_NAME}"
                    ));
                } else {
                    // No per-session action grant in the interactive loop yet,
                    // so action servers surface as Blocked; other read-only
                    // servers are default-permit. Lock only for the call.
                    let guard = client.lock().await;
                    gated_dispatch(&guard, &server, &tool, &arguments, false, &chain).await
                };
                match outcome {
                    DispatchOutcome::Result(result) => transcript.push(ToolStep {
                        server,
                        tool,
                        arguments,
                        result,
                    }),
                    blocked @ (DispatchOutcome::NeedsConfirmation(_)
                    | DispatchOutcome::NeedsAuthorization) => {
                        return LoopOutcome::Blocked(blocked)
                    }
                    DispatchOutcome::Failed(e) => {
                        // Record the failure so the model can adjust on the next
                        // step rather than aborting the loop on one tool error.
                        transcript.push(ToolStep {
                            server,
                            tool,
                            arguments,
                            result: format!("error: {e}"),
                        });
                    }
                }
            }
            Err(e) => return LoopOutcome::Failed(format!("unparseable step: {e}")),
        }
    }
    LoopOutcome::Exhausted
}

/// Reduce a [`LoopOutcome`] to the single answer string the query path
/// returns, or an error for a non-success outcome. A blocked tool call (needs
/// confirmation or authorization) or an exhausted budget is not an error: it
/// becomes a plain-language answer explaining why the assistant stopped, so
/// the user gets a response. A `Denied` outcome is a trust-boundary policy
/// violation, not a normal stop: it maps to `Err` so the query is recorded as
/// **failed**, never `completed` — a denial must not be indistinguishable from
/// a successful query in the ledger or to the caller. `Failed` is also `Err`.
pub fn loop_outcome_to_answer(outcome: LoopOutcome) -> Result<String, String> {
    match outcome {
        LoopOutcome::Answer(text) => Ok(text),
        LoopOutcome::Exhausted => {
            Ok("I could not finish answering within the allowed number of steps.".to_string())
        }
        LoopOutcome::Blocked(_) => Ok("Answering that needs an action I cannot take from a \
             question alone (it requires confirmation or authorization)."
            .to_string()),
        LoopOutcome::Denied(reason) => Err(format!("policy denied: {reason}")),
        LoopOutcome::Failed(reason) => Err(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_ai_core::mcp::{ServerClass, ServerId};

    fn tool(server: &str, name: &str, desc: &str) -> CatalogueTool {
        CatalogueTool {
            server: ServerId(server.to_string()),
            class: ServerClass::ReadOnly,
            name: name.to_string(),
            description: Some(desc.to_string()),
        }
    }

    #[test]
    fn prompt_includes_question_tools_and_response_format() {
        let cat = vec![tool("system.knowledge", "query", "Run read Cypher")];
        let p = build_tool_prompt("what did I open today?", &cat, &[]);
        assert!(p.contains("what did I open today?"), "user question present");
        assert!(p.contains("system.knowledge/query"), "tool listed");
        assert!(p.contains("Run read Cypher"), "tool description listed");
        assert!(p.contains("\"action\": \"call_tool\""), "tool-call format shown");
        assert!(p.contains("\"action\": \"answer\""), "answer format shown");
        // The question is wrapped as a USER-QUESTION data block, not raw.
        assert!(p.contains("USER-QUESTION-"), "question is origin-tagged");
    }

    #[test]
    fn prompt_renders_prior_tool_results_as_data() {
        let cat = vec![tool("system.knowledge", "query", "Run read Cypher")];
        let steps = vec![ToolStep {
            server: "system.knowledge".to_string(),
            tool: "query".to_string(),
            arguments: "{\"cypher\":\"MATCH ...\"}".to_string(),
            result: "[{\"path\":\"/x\"}]".to_string(),
        }];
        let p = build_tool_prompt("q", &cat, &steps);
        assert!(p.contains("step 1: called system.knowledge/query"), "transcript present");
        assert!(p.contains("[{\"path\":\"/x\"}]"), "result present");
        assert!(p.contains("GRAPH-DATA-"), "results tagged as graph data");
    }

    #[test]
    fn empty_catalogue_is_stated() {
        let p = build_tool_prompt("q", &[], &[]);
        assert!(p.contains("(no tools available)"));
    }

    #[test]
    fn parse_call_tool_step() {
        let step = parse_loop_step(
            r#"{"action":"call_tool","server":"system.knowledge","tool":"query","arguments":{"cypher":"MATCH (n) RETURN n"}}"#,
        )
        .unwrap();
        assert_eq!(
            step,
            LoopStep::CallTool {
                server: "system.knowledge".to_string(),
                tool: "query".to_string(),
                arguments: r#"{"cypher":"MATCH (n) RETURN n"}"#.to_string(),
            }
        );
    }

    #[test]
    fn parse_answer_step_even_with_surrounding_text() {
        // extract_json tolerates chatter around the object.
        let step = parse_loop_step("Sure! {\"action\":\"answer\",\"text\":\"you opened 3 files\"}").unwrap();
        assert_eq!(
            step,
            LoopStep::Answer {
                text: "you opened 3 files".to_string()
            }
        );
    }

    #[test]
    fn call_tool_missing_server_or_tool_fails_closed() {
        assert!(parse_loop_step(r#"{"action":"call_tool","tool":"query"}"#).is_err());
        assert!(parse_loop_step(r#"{"action":"call_tool","server":"system.knowledge"}"#).is_err());
    }

    #[test]
    fn unknown_action_and_no_json_fail_closed() {
        assert!(parse_loop_step(r#"{"action":"delete_everything"}"#).is_err());
        assert!(parse_loop_step("no json here").is_err());
    }

    #[test]
    fn call_tool_defaults_missing_arguments_to_empty_object() {
        let step = parse_loop_step(r#"{"action":"call_tool","server":"s","tool":"t"}"#).unwrap();
        assert_eq!(
            step,
            LoopStep::CallTool {
                server: "s".to_string(),
                tool: "t".to_string(),
                arguments: "{}".to_string(),
            }
        );
    }

    #[test]
    fn interactive_catalogue_adds_scoped_graph_tool_and_drops_raw_knowledge() {
        let raw = vec![
            CatalogueTool {
                server: ServerId("system.knowledge".into()),
                class: ServerClass::ReadOnly,
                name: "query".into(),
                description: None,
            },
            CatalogueTool {
                server: ServerId("module.notes".into()),
                class: ServerClass::ReadOnly,
                name: "list".into(),
                description: None,
            },
        ];
        let shaped = interactive_catalogue(raw);
        // The raw knowledge server is gone; the scoped graph tool leads.
        assert_eq!(shaped[0].server.0, "system.graph");
        assert_eq!(shaped[0].name, "query");
        assert!(shaped.iter().all(|t| t.server.0 != "system.knowledge"));
        // Unrelated servers survive.
        assert!(shaped.iter().any(|t| t.server.0 == "module.notes"));
    }

    #[test]
    fn extract_graph_question_fails_closed() {
        assert_eq!(
            extract_graph_question(r#"{"question":"what did I open?"}"#).unwrap(),
            "what did I open?"
        );
        assert!(extract_graph_question(r#"{"question":""}"#).is_err());
        assert!(extract_graph_question(r#"{"question":42}"#).is_err());
        assert!(extract_graph_question(r#"{"q":"x"}"#).is_err());
        assert!(extract_graph_question("not json").is_err());
    }

    // Full-loop integration: a live read-only MCP server with a `query` tool,
    // a mock provider scripted to call it and then answer. Verifies the loop
    // composes catalogue + prompt + parse + gate + dispatch end to end.
    mod loop_integration {
        use super::super::*;
        use arlen_ai_core::mcp::{McpClient, ServerClass, ServerId};
        use audit_proto::MockAuditSink;
        use arlen_ai_core::provider::{
            AIProvider, CompletionRequest, CompletionResponse, ProviderAudit, ProviderError,
        };
        use async_trait::async_trait;
        use os_sdk::mcp::rmcp;
        use os_sdk::mcp::serve_mcp_at;
        use rmcp::handler::server::router::tool::ToolRouter;
        use rmcp::{tool, tool_handler, tool_router, ServerHandler};
        use std::path::PathBuf;
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        /// A provider that replays a fixed script of replies, one per call.
        struct ScriptedProvider {
            replies: Mutex<std::collections::VecDeque<String>>,
        }
        impl ScriptedProvider {
            fn new(replies: Vec<&str>) -> Self {
                Self {
                    replies: Mutex::new(replies.into_iter().map(String::from).collect()),
                }
            }
        }
        #[async_trait]
        impl AIProvider for ScriptedProvider {
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, ProviderError> {
                let text = self
                    .replies
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| r#"{"action":"answer","text":"(script exhausted)"}"#.into());
                Ok(CompletionResponse {
                    text,
                    audit: ProviderAudit {
                        provider_name: "scripted".into(),
                        model: "scripted".into(),
                        input_tokens: None,
                        output_tokens: None,
                    },
                })
            }
            async fn available(&self) -> bool {
                true
            }
            fn name(&self) -> &str {
                "scripted"
            }
        }

        use arlen_ai_core::graph_query::{AccessTier, QueryScope};
        use arlen_ai_core::graph_schema::GraphSchema;
        use arlen_ai_core::pipeline::{QueryRunner, RunFailure};

        /// A QueryRunner that records the prompt it was asked and replays a
        /// fixed answer, so a test can assert the scoped graph tool routed
        /// through it (rather than the raw MCP server).
        struct StubRunner {
            answer: Result<String, RunFailure>,
            seen: Mutex<Vec<String>>,
        }
        impl StubRunner {
            fn ok(answer: &str) -> Self {
                Self {
                    answer: Ok(answer.to_string()),
                    seen: Mutex::new(Vec::new()),
                }
            }
        }
        #[async_trait]
        impl QueryRunner for StubRunner {
            async fn run_query(
                &self,
                prompt: &str,
                _scope: &QueryScope,
            ) -> Result<String, RunFailure> {
                self.seen.lock().unwrap().push(prompt.to_string());
                self.answer.clone()
            }
        }

        fn test_scope() -> QueryScope {
            QueryScope::for_tier(AccessTier::Full, &GraphSchema::knowledge_graph())
        }

        /// A fixed v4 UUID standing in for the query id the daemon passes, so
        /// the loop's audit entries correlate to one query.
        const TEST_QUERY_ID: &str = "11111111-1111-4111-8111-111111111111";

        #[derive(Clone)]
        struct QueryServer {
            tool_router: ToolRouter<Self>,
        }
        #[tool_router(router = tool_router)]
        impl QueryServer {
            fn new() -> Self {
                Self {
                    tool_router: Self::tool_router(),
                }
            }
            #[tool(name = "query")]
            async fn query(&self) -> Result<String, String> {
                Ok(r#"[{"path":"/notes.txt"}]"#.to_string())
            }
        }
        #[tool_handler(router = self.tool_router)]
        impl ServerHandler for QueryServer {}

        fn temp_socket() -> PathBuf {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            std::env::temp_dir()
                .join(format!("arlen-toolloop-{}-{unique}", std::process::id()))
                .join("s.sock")
        }

        #[tokio::test]
        async fn loop_calls_a_tool_then_answers() {
            let socket = temp_socket();
            let socket_for_task = socket.clone();
            let server = tokio::spawn(async move {
                let _ = serve_mcp_at(&socket_for_task, QueryServer::new).await;
            });
            for _ in 0..200 {
                if socket.exists() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(socket.exists(), "test server did not bind");

            let mut client = McpClient::new();
            client
                .connect(
                    ServerId("test".into()),
                    socket.to_str().unwrap(),
                    ServerClass::ReadOnly,
                )
                .await
                .expect("connect to test server");
            // The loop takes the client behind a Mutex (shared with discovery).
            let client = tokio::sync::Mutex::new(client);

            // Step 1: call the tool. Step 2: answer.
            let provider = ScriptedProvider::new(vec![
                r#"{"action":"call_tool","server":"test","tool":"query","arguments":{}}"#,
                r#"{"action":"answer","text":"you have 1 note"}"#,
            ]);
            let runner = StubRunner::ok("(unused)");
            let audit = Arc::new(MockAuditSink::accepting());

            let outcome = run_tool_loop(
                &client,
                &runner,
                &test_scope(),
                audit.clone(),
                TEST_QUERY_ID,
                &provider,
                "what notes do I have?",
                5,
            )
            .await;
            assert_eq!(outcome, LoopOutcome::Answer("you have 1 note".to_string()));
            // The "test" MCP tool was used, not the graph runner.
            assert!(runner.seen.lock().unwrap().is_empty());

            server.abort();
        }

        #[tokio::test]
        async fn graph_tool_routes_through_the_scoped_runner() {
            // No MCP server connected: the only graph access is the internal
            // scope-enforcing tool, which must route through the QueryRunner.
            let client = tokio::sync::Mutex::new(McpClient::new());
            let runner = StubRunner::ok("you opened 3 files today");
            let audit = Arc::new(MockAuditSink::accepting());
            let provider = ScriptedProvider::new(vec![
                r#"{"action":"call_tool","server":"system.graph","tool":"query","arguments":{"question":"what did I open?"}}"#,
                r#"{"action":"answer","text":"3 files"}"#,
            ]);

            let outcome = run_tool_loop(
                &client,
                &runner,
                &test_scope(),
                audit.clone(),
                TEST_QUERY_ID,
                &provider,
                "what did I open today?",
                5,
            )
            .await;
            assert_eq!(outcome, LoopOutcome::Answer("3 files".to_string()));
            // The runner saw the model's question, under the passed scope.
            let seen = runner.seen.lock().unwrap();
            assert_eq!(seen.as_slice(), &["what did I open?".to_string()]);
            // The graph read was audited (dispatched + outcome), content-free,
            // correlated to the query id.
            let recorded = audit.recorded().await;
            let graph: Vec<_> = recorded
                .iter()
                .filter(|e| e.structural.subject == "system.graph")
                .collect();
            assert_eq!(graph.len(), 2, "dispatched + outcome");
            assert!(graph
                .iter()
                .all(|e| e.call_chain_id.as_deref() == Some(TEST_QUERY_ID)));
            assert!(graph.iter().any(|e| e.structural.outcome == "dispatched"));
            assert!(graph.iter().any(|e| e.structural.outcome == "ok"));
        }

        #[tokio::test]
        async fn graph_tool_fails_closed_when_the_audit_log_is_unavailable() {
            // The graph read must not run if its dispatch entry cannot be
            // recorded (foundation §8.4.6): no un-audited AI activity.
            let client = tokio::sync::Mutex::new(McpClient::new());
            let runner = StubRunner::ok("should never run");
            let audit = Arc::new(MockAuditSink::failing());
            let provider = ScriptedProvider::new(vec![
                r#"{"action":"call_tool","server":"system.graph","tool":"query","arguments":{"question":"what did I open?"}}"#,
            ]);

            let outcome = run_tool_loop(
                &client,
                &runner,
                &test_scope(),
                audit.clone(),
                TEST_QUERY_ID,
                &provider,
                "what did I open today?",
                5,
            )
            .await;
            assert!(matches!(outcome, LoopOutcome::Failed(_)), "got {outcome:?}");
            // The runner was never reached: the read did not happen unaudited.
            assert!(runner.seen.lock().unwrap().is_empty());
        }

        #[tokio::test]
        async fn raw_knowledge_server_is_refused_in_the_loop() {
            // The raw-Cypher knowledge server is connected, but the loop must
            // refuse a direct call to it: it cannot carry the per-tier scope.
            let socket = temp_socket();
            let socket_for_task = socket.clone();
            let server = tokio::spawn(async move {
                let _ = serve_mcp_at(&socket_for_task, QueryServer::new).await;
            });
            for _ in 0..200 {
                if socket.exists() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(socket.exists(), "test server did not bind");
            let mut raw = McpClient::new();
            raw.connect(
                ServerId("system.knowledge".into()),
                socket.to_str().unwrap(),
                ServerClass::ReadOnly,
            )
            .await
            .expect("connect knowledge server");
            let client = tokio::sync::Mutex::new(raw);
            let runner = StubRunner::ok("(unused)");
            let audit = Arc::new(MockAuditSink::accepting());
            // Step 1: try the raw server directly. The model scripts an answer
            // after, but the loop must never reach it: the denial is terminal.
            let provider = ScriptedProvider::new(vec![
                r#"{"action":"call_tool","server":"system.knowledge","tool":"query","arguments":{"cypher":"MATCH (n) RETURN n"}}"#,
                r#"{"action":"answer","text":"done"}"#,
            ]);

            let outcome = run_tool_loop(
                &client,
                &runner,
                &test_scope(),
                audit.clone(),
                TEST_QUERY_ID,
                &provider,
                "read everything",
                5,
            )
            .await;
            // The raw call is a trust-boundary hit: terminal Denied, not a
            // recoverable error masked by the model's fabricated "done". No
            // raw Cypher reached the server, and the runner was not used.
            assert!(matches!(outcome, LoopOutcome::Denied(_)), "got {outcome:?}");
            assert!(runner.seen.lock().unwrap().is_empty());
            // The denial was recorded as a policy violation, correlated to the
            // query id. And it maps to a non-success query outcome (Err), so it
            // is never marked completed.
            let recorded = audit.recorded().await;
            assert!(recorded.iter().any(|e| {
                e.kind == audit_proto::AuditKind::PolicyViolation
                    && e.structural.subject == "system.knowledge"
                    && e.call_chain_id.as_deref() == Some(TEST_QUERY_ID)
            }));
            assert!(loop_outcome_to_answer(outcome).is_err());

            server.abort();
        }

        /// A recording sink whose `submit` signals entry and then parks until
        /// released, so a test can drop the loop future while a submit is in
        /// flight and check the entry still lands.
        struct GateRecordSink {
            entered: tokio::sync::Notify,
            release: tokio::sync::Notify,
            recorded: tokio::sync::Mutex<Vec<audit_proto::IngestRequest>>,
        }
        #[async_trait]
        impl AuditSink for GateRecordSink {
            async fn submit(
                &self,
                event: audit_proto::IngestRequest,
            ) -> Result<u64, arlen_ai_core::audit::AuditClientError> {
                self.entered.notify_one();
                self.release.notified().await;
                self.recorded.lock().await.push(event);
                Ok(0)
            }
        }

        #[tokio::test]
        async fn raw_denial_audit_survives_the_loop_future_being_dropped() {
            // The dispatch select races this loop against the caller's cancel,
            // and a cancel drops the loop future. The policy-violation record
            // must still land: it is submitted on a spawned task that a dropped
            // JoinHandle does not cancel. Here we drop the loop future while its
            // denial submit is parked, then release and assert the entry lands.
            let client = tokio::sync::Mutex::new(McpClient::new());
            let runner = StubRunner::ok("(unused)");
            let audit = Arc::new(GateRecordSink {
                entered: tokio::sync::Notify::new(),
                release: tokio::sync::Notify::new(),
                recorded: tokio::sync::Mutex::new(Vec::new()),
            });
            let provider = ScriptedProvider::new(vec![
                r#"{"action":"call_tool","server":"system.knowledge","tool":"query","arguments":{"cypher":"MATCH (n) RETURN n"}}"#,
            ]);
            let scope = test_scope();

            let loop_fut = run_tool_loop(
                &client,
                &runner,
                &scope,
                audit.clone(),
                TEST_QUERY_ID,
                &provider,
                "read everything",
                5,
            );
            // Drop the loop future the moment its denial submit is in flight.
            tokio::select! {
                _ = loop_fut => panic!("loop should still be parked in the submit"),
                _ = audit.entered.notified() => {}
            }
            // The loop future is dropped; the spawned recorder task is not.
            audit.release.notify_waiters();
            // The policy-violation entry lands despite the dropped loop.
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            loop {
                if !audit.recorded.lock().await.is_empty() {
                    break;
                }
                if std::time::Instant::now() > deadline {
                    panic!("denial audit was lost when the loop future was dropped");
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let recorded = audit.recorded.lock().await;
            assert_eq!(recorded[0].kind, audit_proto::AuditKind::PolicyViolation);
            assert_eq!(recorded[0].structural.subject, "system.knowledge");
        }

        #[tokio::test]
        async fn malformed_arguments_fail_closed_without_calling_the_tool() {
            let socket = temp_socket();
            let socket_for_task = socket.clone();
            let server = tokio::spawn(async move {
                let _ = serve_mcp_at(&socket_for_task, QueryServer::new).await;
            });
            for _ in 0..200 {
                if socket.exists() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(socket.exists(), "test server did not bind");
            let mut client = McpClient::new();
            client
                .connect(
                    ServerId("test".into()),
                    socket.to_str().unwrap(),
                    ServerClass::ReadOnly,
                )
                .await
                .expect("connect to test server");

            // The gate allows (read-only), but the arguments are not valid
            // JSON, so the call fails closed; the tool is never invoked (a
            // Result would carry the server's query output).
            let bad = gated_dispatch(&client, "test", "query", "not json", false, &CallChain::root())
                .await;
            assert!(matches!(bad, DispatchOutcome::Failed(_)), "got {bad:?}");
            // A valid-JSON non-object is also rejected, not coerced.
            let arr = gated_dispatch(&client, "test", "query", "[1,2]", false, &CallChain::root())
                .await;
            assert!(matches!(arr, DispatchOutcome::Failed(_)), "got {arr:?}");

            server.abort();
        }
    }

    #[test]
    fn loop_outcome_maps_to_answer_or_error() {
        assert_eq!(
            loop_outcome_to_answer(LoopOutcome::Answer("hi".into())),
            Ok("hi".to_string())
        );
        // Exhausted and Blocked are answers, not errors: the user gets a reason.
        assert!(loop_outcome_to_answer(LoopOutcome::Exhausted).is_ok());
        assert!(loop_outcome_to_answer(LoopOutcome::Blocked(
            DispatchOutcome::NeedsAuthorization
        ))
        .is_ok());
        // A policy denial is a non-success outcome: it maps to Err so the
        // query is recorded as failed, never completed, and carries the reason.
        let denied = loop_outcome_to_answer(LoopOutcome::Denied("not allowed".into()));
        assert!(denied.is_err());
        assert!(denied.unwrap_err().contains("not allowed"));
        // A genuine failure is also an error.
        assert_eq!(
            loop_outcome_to_answer(LoopOutcome::Failed("provider down".into())),
            Err("provider down".to_string())
        );
    }

    #[test]
    fn allow_dispatches_blocked_decisions_surface() {
        // Allow means the caller proceeds to dispatch.
        assert_eq!(outcome_for_blocked(CallDecision::Allow), None);
        // A blocked decision is surfaced, never silently dropped.
        assert_eq!(
            outcome_for_blocked(CallDecision::NeedsAuthorization),
            Some(DispatchOutcome::NeedsAuthorization)
        );
        assert_eq!(
            outcome_for_blocked(CallDecision::NeedsConfirmation(
                AlwaysConfirmReason::FileDeletion
            )),
            Some(DispatchOutcome::NeedsConfirmation(
                AlwaysConfirmReason::FileDeletion
            ))
        );
    }
}
