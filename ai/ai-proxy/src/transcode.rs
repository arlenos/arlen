//! OpenAI <-> Anthropic wire transcoding for the multi-provider proxy path
//! (`ai-providers-plan.md`).
//!
//! The daemon and harness speak the OpenAI chat-completions shape; a provider
//! catalogued [`WireFormat::Anthropic`](crate::catalog::WireFormat::Anthropic)
//! needs the request reshaped to Anthropic's `/v1/messages` and the response
//! reshaped back. These are pure JSON transforms (no network, no key), so the
//! mapping is unit-tested directly and the proxy's forward path just calls them
//! around the POST.
//!
//! Scope: the **text** path (non-streaming, non-tool). The plan's gotcha list
//! names two follow-ups handled separately - `tool_use`/`tool_result` encoding
//! and the Anthropic typed-event SSE parser (no `[DONE]`). The mapping is
//! cross-checked against the Anthropic Messages API and the `graniet/llm` + `rig`
//! adapters (MIT, per `copy-policy.md`).
#![allow(dead_code)] // wired into the forward path by the dispatch slice that follows

use serde_json::{json, Map, Value};

/// Anthropic requires `max_tokens`; OpenAI makes it optional. When the caller
/// omits it, use this conservative default so the request is well-formed rather
/// than 400 at the backend.
const DEFAULT_MAX_TOKENS: u64 = 4096;

/// Reshape an OpenAI chat-completions request body into an Anthropic
/// `/v1/messages` request. The gotchas (plan §"gotcha list"): a `role:"system"`
/// message 400s on Anthropic, so system turns are lifted into the top-level
/// `system` field; `max_tokens` is mandatory; OpenAI `stop` becomes
/// `stop_sequences`. `temperature`/`top_p`/`stream` carry through.
pub fn openai_request_to_anthropic(req: &Value) -> Value {
    let mut out = Map::new();
    if let Some(model) = req.get("model") {
        out.insert("model".into(), model.clone());
    }

    // Split the system turns (lifted to top-level `system`) from user/assistant.
    let mut system = String::new();
    let mut messages = Vec::new();
    if let Some(arr) = req.get("messages").and_then(Value::as_array) {
        for m in arr {
            let role = m.get("role").and_then(Value::as_str).unwrap_or("user");
            let content = message_text(m.get("content"));
            if role == "system" {
                if !system.is_empty() {
                    system.push_str("\n\n");
                }
                system.push_str(&content);
            } else {
                // Anthropic roles are user/assistant only; map anything else to user.
                let role = if role == "assistant" { "assistant" } else { "user" };
                messages.push(json!({ "role": role, "content": content }));
            }
        }
    }
    if !system.is_empty() {
        out.insert("system".into(), Value::String(system));
    }
    out.insert("messages".into(), Value::Array(messages));

    let max_tokens = req
        .get("max_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_TOKENS);
    out.insert("max_tokens".into(), json!(max_tokens));
    for key in ["temperature", "top_p", "stream"] {
        if let Some(v) = req.get(key) {
            out.insert(key.into(), v.clone());
        }
    }
    // OpenAI `stop` (string | array) -> Anthropic `stop_sequences` (array).
    match req.get("stop") {
        Some(Value::String(s)) => {
            out.insert("stop_sequences".into(), json!([s]));
        }
        Some(arr @ Value::Array(_)) => {
            out.insert("stop_sequences".into(), arr.clone());
        }
        _ => {}
    }
    Value::Object(out)
}

/// Reshape an Anthropic `/v1/messages` response into an OpenAI chat-completions
/// response: concatenate the text content blocks, map `stop_reason` to
/// `finish_reason`, and map the usage counters (`input_tokens`/`output_tokens`
/// to `prompt_tokens`/`completion_tokens`/`total_tokens`).
pub fn anthropic_response_to_openai(resp: &Value) -> Value {
    let text = resp
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| {
                    if b.get("type").and_then(Value::as_str) == Some("text") {
                        b.get("text").and_then(Value::as_str)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();

    let finish_reason = match resp.get("stop_reason").and_then(Value::as_str) {
        Some("max_tokens") => "length",
        Some("tool_use") => "tool_calls",
        // "end_turn", "stop_sequence", and anything unknown map to the OpenAI
        // normal-completion reason.
        _ => "stop",
    };

    let (input, output) = resp
        .get("usage")
        .map(|u| {
            (
                u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0),
                u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
            )
        })
        .unwrap_or((0, 0));

    json!({
        "id": resp.get("id").and_then(Value::as_str).unwrap_or(""),
        "object": "chat.completion",
        "model": resp.get("model").and_then(Value::as_str).unwrap_or(""),
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": text },
            "finish_reason": finish_reason,
        }],
        "usage": {
            "prompt_tokens": input,
            "completion_tokens": output,
            "total_tokens": input + output,
        },
    })
}

/// Extract plain text from an OpenAI message `content`: a string, or the
/// multimodal array of `{type:"text", text}` parts (non-text parts dropped - the
/// text path; image parts are a follow-up).
fn message_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_lifts_system_and_requires_max_tokens() {
        let openai = json!({
            "model": "claude-x",
            "messages": [
                { "role": "system", "content": "be terse" },
                { "role": "user", "content": "hi" },
                { "role": "assistant", "content": "hello" },
            ],
            "stop": "STOP",
        });
        let a = openai_request_to_anthropic(&openai);
        // System lifted to the top level (a role:"system" message would 400).
        assert_eq!(a["system"], json!("be terse"));
        let msgs = a["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0], json!({ "role": "user", "content": "hi" }));
        assert_eq!(msgs[1], json!({ "role": "assistant", "content": "hello" }));
        // max_tokens is mandatory; absent -> the default.
        assert_eq!(a["max_tokens"], json!(DEFAULT_MAX_TOKENS));
        // stop -> stop_sequences (array).
        assert_eq!(a["stop_sequences"], json!(["STOP"]));
        assert_eq!(a["model"], json!("claude-x"));
    }

    #[test]
    fn request_keeps_an_explicit_max_tokens_and_multimodal_text() {
        let openai = json!({
            "model": "claude-x",
            "max_tokens": 256,
            "messages": [
                { "role": "user", "content": [
                    { "type": "text", "text": "part one " },
                    { "type": "text", "text": "part two" },
                ] },
            ],
        });
        let a = openai_request_to_anthropic(&openai);
        assert_eq!(a["max_tokens"], json!(256));
        assert!(a.get("system").is_none());
        assert_eq!(a["messages"][0]["content"], json!("part one part two"));
    }

    #[test]
    fn response_concatenates_text_and_maps_reason_and_usage() {
        let anthropic = json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": "claude-x",
            "content": [
                { "type": "text", "text": "hello " },
                { "type": "text", "text": "world" },
            ],
            "stop_reason": "max_tokens",
            "usage": { "input_tokens": 10, "output_tokens": 5 },
        });
        let o = anthropic_response_to_openai(&anthropic);
        assert_eq!(o["object"], json!("chat.completion"));
        assert_eq!(o["choices"][0]["message"]["content"], json!("hello world"));
        assert_eq!(o["choices"][0]["message"]["role"], json!("assistant"));
        // max_tokens -> length.
        assert_eq!(o["choices"][0]["finish_reason"], json!("length"));
        assert_eq!(o["usage"]["prompt_tokens"], json!(10));
        assert_eq!(o["usage"]["completion_tokens"], json!(5));
        assert_eq!(o["usage"]["total_tokens"], json!(15));
        assert_eq!(o["id"], json!("msg_1"));
    }

    #[test]
    fn response_end_turn_maps_to_stop() {
        let anthropic = json!({
            "content": [{ "type": "text", "text": "ok" }],
            "stop_reason": "end_turn",
        });
        let o = anthropic_response_to_openai(&anthropic);
        assert_eq!(o["choices"][0]["finish_reason"], json!("stop"));
        // Absent usage -> zeros, not a panic.
        assert_eq!(o["usage"]["total_tokens"], json!(0));
    }
}
