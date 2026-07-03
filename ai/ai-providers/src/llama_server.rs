//! `llama-server` (llama.cpp) provider adapter.
//!
//! The Arlen-owned local provider (local-model-bundle-plan.md Decision 1, LOCKED):
//! Arlen bundles `llama-server` (llama.cpp, MIT, in Debian apt) as the engine and
//! owns the thin management layer, graduating off Ollama. `llama-server` speaks the
//! same OpenAI-compatible `POST {endpoint}/v1/chat/completions` as Ollama, so this
//! adapter is nearly identical to [`crate::ollama`]; the differences are the
//! model-by-PATH identity (a GGUF's identity IS its `--model` path, decided 3 July)
//! and the real `/health` liveness endpoint llama-server exposes.
//!
//! One `llama-server` process serves one loaded GGUF, so the `model` here is the
//! model the running server was launched with; switching models is the daemon's
//! process-management concern (spawn `llama-server --model <path>`), not this
//! single-shot completion layer.

use arlen_ai_core::provider::{
    AIProvider, CompletionRequest, CompletionResponse, ProviderAudit, ProviderError,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Default request timeout: 60 s comfortably fits an 8B model on modest hardware
/// while still failing closed on a stalled backend (mirrors the Ollama adapter).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Configuration for a `llama-server` adapter instance.
#[derive(Debug, Clone)]
pub struct LlamaServerConfig {
    /// Logical provider name used in routing rules and the audit log
    /// (e.g. `"llama-server"`).
    pub name: String,
    /// Endpoint base URL (no trailing slash), e.g. `http://127.0.0.1:8080`.
    pub endpoint: String,
    /// The identity of the loaded model - its GGUF path (or a stable id derived
    /// from it). Reported in the audit; `llama-server` serves whatever it was
    /// launched with, so this is descriptive, not a per-request selector.
    pub model: String,
    /// Per-call timeout. Defaults to [`DEFAULT_TIMEOUT`] if `None`.
    pub timeout: Option<Duration>,
}

/// `llama-server` adapter implementing [`AIProvider`].
pub struct LlamaServerProvider {
    config: LlamaServerConfig,
    http: reqwest::Client,
}

impl LlamaServerProvider {
    /// Build a new adapter with a pooled HTTP client.
    pub fn new(config: LlamaServerConfig) -> Result<Self, ProviderError> {
        let timeout = config.timeout.unwrap_or(DEFAULT_TIMEOUT);
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|err| ProviderError::Internal(err.to_string()))?;
        Ok(Self { config, http })
    }

    fn chat_completions_url(&self) -> String {
        format!("{}/v1/chat/completions", self.config.endpoint.trim_end_matches('/'))
    }

    fn health_url(&self) -> String {
        format!("{}/health", self.config.endpoint.trim_end_matches('/'))
    }
}

#[async_trait]
impl AIProvider for LlamaServerProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let body = ChatRequest {
            model: &self.config.model,
            messages: vec![ChatMessage {
                role: "user",
                content: &req.prompt,
            }],
            stream: false,
        };

        let response = self
            .http
            .post(self.chat_completions_url())
            .json(&body)
            .send()
            .await
            .map_err(|err| {
                if err.is_timeout() {
                    ProviderError::Timeout
                } else if err.is_connect() || err.is_request() {
                    ProviderError::Unavailable(err.to_string())
                } else {
                    ProviderError::Internal(err.to_string())
                }
            })?;

        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ProviderError::RateLimited);
        }
        if status.is_server_error() {
            return Err(ProviderError::Unavailable(format!(
                "llama-server returned HTTP {}",
                status.as_u16()
            )));
        }
        if !status.is_success() {
            return Err(ProviderError::Internal(format!(
                "llama-server returned HTTP {}",
                status.as_u16()
            )));
        }

        let parsed: ChatResponse = response
            .json()
            .await
            .map_err(|err| ProviderError::Internal(format!("invalid response body: {err}")))?;

        let text = parsed
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message.content)
            .unwrap_or_default();

        Ok(CompletionResponse {
            text,
            audit: ProviderAudit {
                provider_name: self.config.name.clone(),
                model: self.config.model.clone(),
                input_tokens: parsed.usage.as_ref().map(|u| u.prompt_tokens),
                output_tokens: parsed.usage.map(|u| u.completion_tokens),
            },
        })
    }

    async fn available(&self) -> bool {
        // `GET /health` is llama-server's authoritative liveness probe (a real
        // no-op health endpoint, unlike Ollama's `/api/tags`).
        match self.http.get(self.health_url()).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    fn name(&self) -> &str {
        &self.config.name
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    content: String,
}

#[derive(Deserialize)]
struct ChatUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(server: &MockServer) -> LlamaServerProvider {
        LlamaServerProvider::new(LlamaServerConfig {
            name: "llama-server".to_string(),
            endpoint: server.uri(),
            model: "/usr/share/arlen/models/Llama-3.2-1B-Instruct-Q4_K_M.gguf".to_string(),
            timeout: Some(Duration::from_secs(5)),
        })
        .expect("provider builds")
    }

    #[tokio::test]
    async fn complete_returns_first_choice_text_and_usage() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"role": "assistant", "content": "hi from llama.cpp"}}],
                "usage": {"prompt_tokens": 5, "completion_tokens": 4}
            })))
            .mount(&server)
            .await;

        let resp = provider(&server)
            .complete(CompletionRequest {
                prompt: "hello".to_string(),
                extras: serde_json::json!({}),
            })
            .await
            .expect("complete ok");
        assert_eq!(resp.text, "hi from llama.cpp");
        assert_eq!(resp.audit.provider_name, "llama-server");
        assert_eq!(resp.audit.input_tokens, Some(5));
        assert_eq!(resp.audit.output_tokens, Some(4));
    }

    #[tokio::test]
    async fn available_probes_health() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        assert!(provider(&server).available().await);
    }

    #[tokio::test]
    async fn available_is_false_when_health_is_down() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        assert!(!provider(&server).available().await);
    }

    #[tokio::test]
    async fn http_429_maps_to_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;
        let err = provider(&server)
            .complete(CompletionRequest {
                prompt: "x".to_string(),
                extras: serde_json::json!({}),
            })
            .await
            .expect_err("429 is an error");
        assert!(matches!(err, ProviderError::RateLimited));
    }
}
