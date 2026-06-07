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
    // Allowed. Parse the arguments; a malformed argument string degrades to an
    // empty object rather than failing the dispatch outright.
    let args: serde_json::Value =
        serde_json::from_str(arguments).unwrap_or_else(|_| serde_json::json!({}));
    match client.call_tool(&id, tool, args, chain).await {
        Ok(result) => DispatchOutcome::Result(result),
        Err(e) => DispatchOutcome::Failed(e.to_string()),
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
