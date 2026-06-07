//! Interactive tool-use loop for the AI daemon (see
//! `docs/architecture/ai-tool-routing.md`).
//!
//! This is the prompt construction for the bounded tool-use loop: the daemon's
//! own prompt shape (a user question, the available MCP tool catalogue, and the
//! transcript of prior tool calls and results), distinct from the ai-agent's
//! behaviour-driven prompt. The orchestration (provider call, parse, gated
//! dispatch, budget) lands in later slices behind a default-off flag; this
//! slice is the pure, testable prompt builder.

use arlen_ai_core::mcp::{
    AlwaysConfirmReason, CallChain, CallDecision, CatalogueTool, McpClient, ServerId,
};
use arlen_ai_core::pipeline::extract_json;
use arlen_ai_core::provider::{AIProvider, CompletionRequest};
use arlen_ai_core::tagging::{Block, Origin, TaggedPrompt};
use serde::Deserialize;

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
    /// The step budget ran out before a final answer.
    Exhausted,
    /// The loop could not proceed: a provider error or an unparseable reply.
    Failed(String),
}

/// Run the bounded interactive tool-use loop: assemble the tool catalogue, then
/// repeatedly prompt the model, parse its step, and either answer or gate and
/// dispatch one tool call, feeding the result back, until a final answer or the
/// step budget. Read-only servers are default-permit; an action server with no
/// grant surfaces as `Blocked` instead of being called.
///
/// This is the orchestration of the loop's building blocks. It is additive and
/// not yet wired into the daemon's query path; that wiring lands behind a
/// default-off flag with an integration test and a Codex pass
/// (`docs/architecture/ai-tool-routing.md`).
pub async fn run_tool_loop(
    client: &McpClient,
    provider: &dyn AIProvider,
    question: &str,
    max_steps: u32,
) -> LoopOutcome {
    let catalogue = client.tool_catalogue().await;
    let chain = CallChain::root();
    let mut transcript: Vec<ToolStep> = Vec::new();

    for _ in 0..max_steps {
        let prompt = build_tool_prompt(question, &catalogue, &transcript);
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
                // No per-session action grant in the interactive loop yet, so
                // action servers surface as Blocked; read-only servers (the
                // knowledge graph) are default-permit.
                match gated_dispatch(client, &server, &tool, &arguments, false, &chain).await {
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
/// returns, or an error for a genuine failure. A blocked tool call or an
/// exhausted budget is not an error: it becomes a plain-language answer
/// explaining why the assistant stopped, so the user gets a response rather
/// than an opaque failure. Only `Failed` maps to `Err`.
pub fn loop_outcome_to_answer(outcome: LoopOutcome) -> Result<String, String> {
    match outcome {
        LoopOutcome::Answer(text) => Ok(text),
        LoopOutcome::Exhausted => {
            Ok("I could not finish answering within the allowed number of steps.".to_string())
        }
        LoopOutcome::Blocked(_) => Ok("Answering that needs an action I cannot take from a \
             question alone (it requires confirmation or authorization)."
            .to_string()),
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

    // Full-loop integration: a live read-only MCP server with a `query` tool,
    // a mock provider scripted to call it and then answer. Verifies the loop
    // composes catalogue + prompt + parse + gate + dispatch end to end.
    mod loop_integration {
        use super::super::*;
        use arlen_ai_core::mcp::{McpClient, ServerClass, ServerId};
        use arlen_ai_core::provider::{
            AIProvider, CompletionRequest, CompletionResponse, ProviderAudit, ProviderError,
        };
        use async_trait::async_trait;
        use os_sdk::mcp::rmcp;
        use os_sdk::mcp::serve_mcp_at;
        use rmcp::handler::server::router::tool::ToolRouter;
        use rmcp::{tool, tool_handler, tool_router, ServerHandler};
        use std::path::PathBuf;
        use std::sync::Mutex;
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

            // Step 1: call the tool. Step 2: answer.
            let provider = ScriptedProvider::new(vec![
                r#"{"action":"call_tool","server":"test","tool":"query","arguments":{}}"#,
                r#"{"action":"answer","text":"you have 1 note"}"#,
            ]);

            let outcome = run_tool_loop(&client, &provider, "what notes do I have?", 5).await;
            assert_eq!(outcome, LoopOutcome::Answer("you have 1 note".to_string()));

            server.abort();
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
        // Only a genuine failure is an error.
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
