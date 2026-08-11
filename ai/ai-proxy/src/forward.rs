//! Outbound forwarder abstraction.
//!
//! The proxy service depends on a [`Forwarder`] trait so the policy
//! layer can be unit-tested without TCP. The daemon binary plugs in
//! the real reqwest-backed implementation; tests substitute a stub.

use async_trait::async_trait;

/// Outcome of a successful forward call.
#[derive(Debug, Clone)]
pub struct ForwardResult {
    /// Upstream HTTP status code.
    pub status: u16,
    /// Upstream response body as a UTF-8 string. The proxy does not
    /// touch the body content; framing parsing happens at the
    /// AI daemon layer.
    pub body: String,
}

/// Default cap on an upstream response body. LLM completions are
/// large but bounded; 8 MiB leaves generous headroom. A wedged or
/// hostile provider (including the allowlisted localhost endpoint)
/// cannot push the proxy into memory pressure beyond this.
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// Errors that a [`Forwarder`] can return.
#[derive(Debug, thiserror::Error)]
pub enum ForwardError {
    /// Transport-level failure (connection refused, DNS error,
    /// TLS handshake, etc.).
    #[error("transport: {0}")]
    Transport(String),
    /// Upstream responded but the response body could not be read.
    #[error("body: {0}")]
    Body(String),
    /// Upstream response exceeded the size cap.
    #[error("upstream response exceeded the {limit}-byte cap")]
    ResponseTooLarge {
        /// The configured cap.
        limit: usize,
    },
}

/// An auth header to inject into the outbound request: `(name, value)`. The proxy
/// resolves this from the Connections daemon at egress time for a keyed provider;
/// `None` means no credential (a local, key-less provider).
pub type AuthHeader<'a> = Option<(&'a str, &'a str)>;

/// Async outbound HTTP forwarder.
#[async_trait]
pub trait Forwarder: Send + Sync {
    /// POST `body_json` to `endpoint_url`, injecting `auth` when present, and
    /// return the upstream response.
    async fn post(
        &self,
        endpoint_url: &str,
        body_json: &str,
        auth: AuthHeader<'_>,
    ) -> Result<ForwardResult, ForwardError>;

    /// GET `endpoint_url` (injecting `auth` when present) and return the upstream
    /// response. Used by the connection test (`test_provider`): a body-less probe of
    /// a catalogued provider's model-list endpoint. The same response cap and
    /// redirect-disable posture as `post` apply.
    async fn get(&self, endpoint_url: &str, auth: AuthHeader<'_>) -> Result<ForwardResult, ForwardError>;
}

/// Drop every resolved address that falls in a blocked range (loopback,
/// link-local metadata, RFC1918, ...) - the SSRF filter core, kept pure so the
/// guard is unit-tested without a live DNS lookup.
fn retain_safe(addrs: impl Iterator<Item = std::net::SocketAddr>) -> Vec<std::net::SocketAddr> {
    addrs
        .filter(|sa| !arlen_net_guard::is_blocked_destination(sa.ip()))
        .collect()
}

/// Whether `host` explicitly names the loopback interface: a loopback IP literal
/// (`127.0.0.1`, `::1`) or `localhost`.
///
/// This is what separates a LOCAL PROVIDER from a DNS-rebind attack. The SSRF
/// guard blocks loopback because a REMOTE hostname resolving INTO loopback is a
/// rebind - the host looks external but reaches an internal service. A host that
/// is ITSELF loopback is the opposite: the operator deliberately configured a
/// local model server (Ollama, llama.cpp - the default, key-less provider
/// category), which by definition lives on loopback. The allowlist already
/// permits `http://127.0.0.1:11434` / `http://localhost:11434` for exactly this,
/// so refusing it at the resolver was a self-contradiction that broke local-first
/// AI entirely. A remote name is never treated as local, so the rebind defense is
/// untouched.
fn host_is_explicit_loopback(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

/// A reqwest DNS resolver that refuses any REMOTE host resolving - or DNS-
/// rebinding - into a blocked range, so the forwarder can never dial an SSRF
/// target even when the ai-proxy runs unconfined in the host netns (review EG-1).
/// A host that explicitly names loopback ([`host_is_explicit_loopback`]) is a
/// local provider and keeps its loopback address; every other host is filtered,
/// so a remote name cannot rebind into loopback. Reqwest applies the request
/// URL's port to the addresses this returns, so the port-0 lookup here is only
/// used for its IP set.
struct GuardedResolver;

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let resolved = tokio::net::lookup_host((host.as_str(), 0)).await?;
            // A deliberately-local provider (loopback literal or `localhost`)
            // keeps its loopback address; the rebind defense only ever needs to
            // fire on a REMOTE name that resolves into a blocked range.
            let addrs: Vec<std::net::SocketAddr> = if host_is_explicit_loopback(&host) {
                resolved.collect()
            } else {
                retain_safe(resolved)
            };
            if addrs.is_empty() {
                return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                    "all resolved addresses are in a blocked range (SSRF guard)",
                ));
            }
            Ok(Box::new(addrs.into_iter()) as Box<dyn Iterator<Item = std::net::SocketAddr> + Send>)
        })
    }
}

/// reqwest-backed forwarder. Built once at daemon startup so
/// connections can be pooled across calls.
pub struct ReqwestForwarder {
    http: reqwest::Client,
    max_response_bytes: usize,
}

impl ReqwestForwarder {
    /// Build the forwarder with the default response cap. Returns an
    /// error if the underlying reqwest client cannot be constructed
    /// (TLS init failure, etc.).
    ///
    /// Redirects are disabled at the transport layer: an allowed
    /// upstream that returns 30x must not be silently followed to a
    /// different host because that would bypass the allowlist and
    /// mis-attribute the audit record. Foundation §8.4.6 lists
    /// redirect-following as a known SSRF pivot.
    pub fn new() -> Result<Self, ForwardError> {
        Self::with_max_response(DEFAULT_MAX_RESPONSE_BYTES)
    }

    /// Build with an explicit response cap. Tests use a small cap to
    /// exercise the oversized-response path cheaply.
    ///
    /// SSRF posture (review EG-1, now closed): the dial is defended at the
    /// host-STRING layer (the service's allowlist check on the catalogued URL), by
    /// the redirect-disable above, AND now by an `is_blocked_destination` IP floor
    /// on this client's own resolver ([`GuardedResolver`]) - so a user-configured
    /// provider host that resolves or DNS-rebinds into a blocked range (loopback,
    /// link-local metadata, RFC1918) is refused at resolution and never dialled,
    /// independently of the launch env (confined or unconfined in the host netns).
    pub fn with_max_response(max_response_bytes: usize) -> Result<Self, ForwardError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(std::sync::Arc::new(GuardedResolver))
            .build()
            .map_err(|err| ForwardError::Transport(err.to_string()))?;
        Ok(Self {
            http,
            max_response_bytes,
        })
    }
}

impl ReqwestForwarder {
    /// Read an upstream response under the configured cap. Streams the
    /// body so a missing or lying `Content-Length` cannot push
    /// unbounded data into memory. Shared by `post` and `get`.
    async fn read_capped(
        &self,
        mut resp: reqwest::Response,
    ) -> Result<ForwardResult, ForwardError> {
        let status = resp.status().as_u16();

        // Reject early on a declared length over the cap, so an
        // honest `Content-Length` saves the streaming read entirely.
        if let Some(len) = resp.content_length() {
            if len as usize > self.max_response_bytes {
                return Err(ForwardError::ResponseTooLarge {
                    limit: self.max_response_bytes,
                });
            }
        }

        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|err| ForwardError::Body(err.to_string()))?
        {
            if buf.len() + chunk.len() > self.max_response_bytes {
                return Err(ForwardError::ResponseTooLarge {
                    limit: self.max_response_bytes,
                });
            }
            buf.extend_from_slice(&chunk);
        }
        let body = String::from_utf8(buf)
            .map_err(|err| ForwardError::Body(format!("non-utf8 response: {err}")))?;
        Ok(ForwardResult { status, body })
    }
}

#[async_trait]
impl Forwarder for ReqwestForwarder {
    async fn post(
        &self,
        endpoint_url: &str,
        body_json: &str,
        auth: AuthHeader<'_>,
    ) -> Result<ForwardResult, ForwardError> {
        let mut builder = self
            .http
            .post(endpoint_url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body_json.to_string());
        if let Some((name, value)) = auth {
            builder = builder.header(name, value);
        }
        let resp = builder
            .send()
            .await
            .map_err(|err| ForwardError::Transport(err.to_string()))?;
        self.read_capped(resp).await
    }

    async fn get(&self, endpoint_url: &str, auth: AuthHeader<'_>) -> Result<ForwardResult, ForwardError> {
        let mut builder = self.http.get(endpoint_url);
        if let Some((name, value)) = auth {
            builder = builder.header(name, value);
        }
        let resp = builder
            .send()
            .await
            .map_err(|err| ForwardError::Transport(err.to_string()))?;
        self.read_capped(resp).await
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// Forwarder stub that records calls and returns a scripted
    /// response.
    #[derive(Clone)]
    pub struct StubForwarder {
        pub script: Arc<Mutex<Vec<Result<ForwardResult, ForwardError>>>>,
        pub calls: Arc<Mutex<Vec<(String, String)>>>,
        /// The auth header injected on each call, in call order, so an injection
        /// test can assert the credential reached (or did not reach) the wire.
        pub auth_headers: RecordedAuthHeaders,
    }

    /// The recorded auth headers, in call order (a factored alias to keep the stub
    /// field type readable).
    pub type RecordedAuthHeaders = Arc<Mutex<Vec<Option<(String, String)>>>>;

    impl StubForwarder {
        pub fn new(script: Vec<Result<ForwardResult, ForwardError>>) -> Self {
            Self {
                script: Arc::new(Mutex::new(script)),
                calls: Arc::new(Mutex::new(Vec::new())),
                auth_headers: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl Forwarder for StubForwarder {
        async fn post(
            &self,
            endpoint_url: &str,
            body_json: &str,
            auth: AuthHeader<'_>,
        ) -> Result<ForwardResult, ForwardError> {
            self.calls
                .lock()
                .await
                .push((endpoint_url.to_string(), body_json.to_string()));
            self.auth_headers
                .lock()
                .await
                .push(auth.map(|(n, v)| (n.to_string(), v.to_string())));
            let mut script = self.script.lock().await;
            if script.is_empty() {
                return Err(ForwardError::Transport("stub exhausted".to_string()));
            }
            script.remove(0)
        }

        async fn get(&self, endpoint_url: &str, auth: AuthHeader<'_>) -> Result<ForwardResult, ForwardError> {
            // A GET has no body; record an empty body so the call list
            // is a uniform `(url, body)` pair across post/get.
            self.calls
                .lock()
                .await
                .push((endpoint_url.to_string(), String::new()));
            self.auth_headers
                .lock()
                .await
                .push(auth.map(|(n, v)| (n.to_string(), v.to_string())));
            let mut script = self.script.lock().await;
            if script.is_empty() {
                return Err(ForwardError::Transport("stub exhausted".to_string()));
            }
            script.remove(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn only_an_explicit_loopback_host_is_treated_as_a_local_provider() {
        // A local model server (Ollama, the default) is configured as a loopback
        // literal or `localhost`; the SSRF guard must let it through, or the whole
        // local-first AI loop is dead - which is exactly the regression this
        // closed (every model call 502'd because the resolver dropped 127.0.0.1).
        assert!(host_is_explicit_loopback("127.0.0.1"));
        assert!(host_is_explicit_loopback("::1"));
        assert!(host_is_explicit_loopback("localhost"));
        assert!(host_is_explicit_loopback("LocalHost")); // case-insensitive
        // A REMOTE name is never local, so it stays subject to retain_safe - the
        // DNS-rebind defense (evil.com -> 127.0.0.1) is untouched.
        assert!(!host_is_explicit_loopback("api.openai.com"));
        assert!(!host_is_explicit_loopback("evil.example.com"));
        // A non-loopback IP literal (a public or metadata address) is not local
        // either; only loopback literals qualify.
        assert!(!host_is_explicit_loopback("169.254.169.254"));
        assert!(!host_is_explicit_loopback("1.1.1.1"));
        assert!(!host_is_explicit_loopback("192.168.1.5"));
    }

    #[test]
    fn retain_safe_drops_blocked_ranges_keeps_public() {
        let addrs: Vec<std::net::SocketAddr> = [
            "127.0.0.1:443",         // loopback
            "169.254.169.254:80",    // link-local cloud metadata
            "192.168.1.5:443",       // RFC1918 private
            "10.0.0.1:443",          // RFC1918 private
            "1.1.1.1:443",           // public
        ]
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
        let safe = retain_safe(addrs.into_iter());
        // Only the public address survives the SSRF filter.
        assert_eq!(safe, vec!["1.1.1.1:443".parse().unwrap()]);
    }

    #[tokio::test]
    async fn oversized_response_is_rejected() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(500)))
            .mount(&server)
            .await;
        let fwd = ReqwestForwarder::with_max_response(100).unwrap();
        let err = fwd
            .post(&format!("{}/x", server.uri()), "{}", None)
            .await
            .expect_err("must reject oversized body");
        assert!(matches!(
            err,
            ForwardError::ResponseTooLarge { limit: 100 }
        ));
    }

    #[tokio::test]
    async fn response_within_cap_passes_through() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;
        let fwd = ReqwestForwarder::with_max_response(1024).unwrap();
        let result = fwd
            .post(&format!("{}/x", server.uri()), "{}", None)
            .await
            .expect("within cap");
        assert_eq!(result.status, 200);
        assert_eq!(result.body, "ok");
    }

    #[tokio::test]
    async fn get_reads_the_models_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":[]}"#))
            .mount(&server)
            .await;
        let fwd = ReqwestForwarder::with_max_response(1024).unwrap();
        let result = fwd
            .get(&format!("{}/v1/models", server.uri()), None)
            .await
            .expect("get models");
        assert_eq!(result.status, 200);
        assert_eq!(result.body, r#"{"data":[]}"#);
    }
}

/// A forwarder that answers locally instead of dialling a model.
///
/// **Why this exists.** The AI path is the largest thing in this tree with no
/// positive control: the skill loads, the object is served, and between them sits
/// shell to daemon to skill to confined pi to answer, verified nowhere. A launcher
/// ask that comes back empty on a real install would look exactly like a green
/// build. Nothing in that chain needs a model that THINKS - it needs a provider
/// that RETURNS.
///
/// **What stays real.** Everything but the last hop. pi spawns confined, dials the
/// completion socket, the daemon authenticates the caller by SO_PEERCRED and
/// forwards through the governed proxy with its allowlist, its audit entry and its
/// credential resolution. Only the outbound HTTP request is answered here.
///
/// **It cannot be reached by accident.** `main` builds it only when
/// `ARLEN_AI_ECHO` is set, logs loudly when it does, and the answer says what it
/// is in its own text - so a screenshot of an echoed reply can never be mistaken
/// for a model working.
pub struct EchoForwarder {
    /// The sentence every completion returns.
    answer: String,
}

impl Default for EchoForwarder {
    fn default() -> Self {
        Self {
            answer: "This is the echo provider. No model was asked; the chain that \
                     carried this sentence is real."
                .to_string(),
        }
    }
}

impl EchoForwarder {
    /// A forwarder with the default answer.
    pub fn new() -> Self {
        Self::default()
    }

    /// The OpenAI-shaped chat completion the daemon and pi both already parse.
    /// Kept to the fields a consumer actually reads rather than a full fake of the
    /// upstream schema, which would be inventing a contract we do not own.
    fn completion(&self, echoed: &str) -> String {
        let content = if echoed.is_empty() {
            self.answer.clone()
        } else {
            format!("{} It was handed: {echoed}", self.answer)
        };
        serde_json::json!({
            "id": "echo",
            "object": "chat.completion",
            "model": "echo",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": content},
                "finish_reason": "stop",
            }],
        })
        .to_string()
    }

    /// The last user message in an OpenAI-shaped request, if the body carries one.
    /// Echoing what it was handed is what makes this a control rather than a fixed
    /// string: a run that never reached the provider cannot produce it.
    fn echoed_from(body_json: &str) -> String {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(body_json) else {
            return String::new();
        };
        v.get("messages")
            .and_then(|m| m.as_array())
            .and_then(|m| m.last())
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .chars()
            .take(200)
            .collect()
    }
}

#[async_trait]
impl Forwarder for EchoForwarder {
    async fn post(
        &self,
        _endpoint_url: &str,
        body_json: &str,
        _auth: AuthHeader<'_>,
    ) -> Result<ForwardResult, ForwardError> {
        Ok(ForwardResult {
            status: 200,
            body: self.completion(&Self::echoed_from(body_json)),
        })
    }

    async fn get(
        &self,
        _endpoint_url: &str,
        _auth: AuthHeader<'_>,
    ) -> Result<ForwardResult, ForwardError> {
        // The connection test asks a provider for its model list. Answering with
        // one model named `echo` keeps `test_provider` truthful about what this
        // provider can actually do.
        Ok(ForwardResult {
            status: 200,
            body: serde_json::json!({"data": [{"id": "echo", "object": "model"}]}).to_string(),
        })
    }
}

#[cfg(test)]
mod echo_tests {
    use super::*;

    #[tokio::test]
    async fn it_answers_without_dialling_anything() {
        let r = EchoForwarder::new()
            .post("http://127.0.0.1:1/never-dialled", "{}", None)
            .await
            .expect("the echo provider always returns");
        assert_eq!(r.status, 200);
        assert!(r.body.contains("echo provider"), "{}", r.body);
    }

    /// The property that makes this a control: the answer carries what the caller
    /// sent, so a reply proves the request travelled the whole chain rather than
    /// being produced by something short of it.
    #[tokio::test]
    async fn the_answer_carries_what_it_was_handed() {
        let body = serde_json::json!({
            "messages": [
                {"role": "system", "content": "ignored"},
                {"role": "user", "content": "what am I working on"},
            ]
        })
        .to_string();
        let r = EchoForwarder::new().post("http://x", &body, None).await.unwrap();
        assert!(r.body.contains("what am I working on"), "{}", r.body);
    }

    /// A body it cannot parse still gets an answer: this must never be the thing
    /// that fails a run, or a green sweep would depend on the shape of a request
    /// rather than on the chain being whole.
    #[tokio::test]
    async fn an_unparsable_body_still_answers() {
        let r = EchoForwarder::new().post("http://x", "not json", None).await.unwrap();
        assert!(r.body.contains("echo provider"), "{}", r.body);
    }
}
