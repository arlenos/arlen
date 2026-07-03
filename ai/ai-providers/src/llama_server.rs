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
use std::path::Path;
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

/// Options for launching a `llama-server` instance to serve one GGUF. The daemon
/// spawns the bundled `llama-server` with [`launch_argv`] and points a
/// [`LlamaServerProvider`] at [`endpoint`].
#[derive(Debug, Clone)]
pub struct LaunchOptions {
    /// Host to bind. Always a loopback address: the model server is local and must
    /// never be network-exposed.
    pub host: String,
    /// TCP port to serve on.
    pub port: u16,
    /// Context window in tokens (`--ctx-size`); `None` leaves llama-server's default.
    pub ctx_size: Option<u32>,
    /// GPU layers to offload (`--n-gpu-layers`); `None` leaves the engine default
    /// (0 = CPU). On the APU target a high value offloads to the iGPU via Vulkan.
    pub n_gpu_layers: Option<u32>,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            ctx_size: None,
            n_gpu_layers: None,
        }
    }
}

/// The `llama-server` argv to serve `gguf_path` under `opts` (the GGUF's path is
/// its identity, decided 3 July). Pure, so the launch command is unit-tested
/// without the binary; the daemon owns the actual spawn + health-wait + lifecycle.
pub fn launch_argv(gguf_path: &Path, opts: &LaunchOptions) -> Vec<String> {
    let mut argv = vec![
        "--model".to_string(),
        gguf_path.display().to_string(),
        "--host".to_string(),
        opts.host.clone(),
        "--port".to_string(),
        opts.port.to_string(),
    ];
    if let Some(ctx) = opts.ctx_size {
        argv.push("--ctx-size".to_string());
        argv.push(ctx.to_string());
    }
    if let Some(ngl) = opts.n_gpu_layers {
        argv.push("--n-gpu-layers".to_string());
        argv.push(ngl.to_string());
    }
    argv
}

/// The base URL a `llama-server` launched with `opts` serves on, for a
/// [`LlamaServerConfig::endpoint`].
pub fn endpoint(opts: &LaunchOptions) -> String {
    format!("http://{}:{}", opts.host, opts.port)
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

    #[test]
    fn launch_argv_carries_model_path_host_and_port() {
        let opts = LaunchOptions {
            host: "127.0.0.1".to_string(),
            port: 8081,
            ctx_size: None,
            n_gpu_layers: None,
        };
        let argv = launch_argv(Path::new("/models/Qwen2.5-7B.gguf"), &opts);
        assert_eq!(
            argv,
            vec![
                "--model",
                "/models/Qwen2.5-7B.gguf",
                "--host",
                "127.0.0.1",
                "--port",
                "8081",
            ]
        );
        assert_eq!(endpoint(&opts), "http://127.0.0.1:8081");
    }

    #[test]
    fn launch_argv_appends_ctx_and_gpu_layers_when_set() {
        let opts = LaunchOptions {
            host: "127.0.0.1".to_string(),
            port: 8080,
            ctx_size: Some(4096),
            n_gpu_layers: Some(99),
        };
        let argv = launch_argv(Path::new("/m.gguf"), &opts);
        assert!(argv.windows(2).any(|w| w == ["--ctx-size", "4096"]));
        assert!(argv.windows(2).any(|w| w == ["--n-gpu-layers", "99"]));
    }

    #[test]
    fn default_launch_binds_loopback() {
        let opts = LaunchOptions::default();
        assert_eq!(opts.host, "127.0.0.1");
        assert!(endpoint(&opts).starts_with("http://127.0.0.1:"));
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
