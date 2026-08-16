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

/// The opening every echoed answer carries. Deliberately a plain sentence rather
/// than a code or a tag: it has to mean something to a person who meets it in a
/// launcher pane with no idea what a forwarder is.
pub const ECHO_MARKER: &str = "[echo provider - no model was asked]";

/// The OpenAI-shaped completion body both local forwarders answer with.
///
/// One function rather than one per forwarder. A wire shape written out twice is
/// two shapes, and they drift - which is the whole reason the settings index has
/// a shared key file and the rename preview has shared vectors. `tag` names the
/// pretend model, and it is the only thing that differs between them.
fn completion_json(tag: &str, content: &str) -> String {
    serde_json::json!({
        "id": tag,
        "object": "chat.completion",
        "model": tag,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop",
        }],
    })
    .to_string()
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
/// **It cannot be reached by accident, and it cannot pass for a model.** Those are
/// two different guarantees and it needs both. `main` builds it only when
/// `ARLEN_AI_ECHO` is set and logs at WARN when it does - there is no config path
/// that selects it, so accidental selection means someone typed the variable. And
/// every answer OPENS with [`ECHO_MARKER`], before any content, on every path
/// including the ones that could not parse the request: an echoed reply is
/// self-identifying wherever it is rendered, quoted or pasted.
///
/// Of the two shapes offered - carry the origin in the answer, or refuse to load
/// outside test configuration - this takes the first, because the second needs a
/// reliable notion of "test configuration" and a wrong one either blocks the
/// offline demo case or fails open on the install it was meant to protect. An
/// origin that travels with the text needs no such judgement.
pub struct EchoForwarder {
    /// The sentence every completion returns after the marker.
    answer: String,
}

impl Default for EchoForwarder {
    fn default() -> Self {
        Self {
            answer: "The chain that carried this sentence is real; the answer is not."
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
        // The marker leads, always. A consumer that shows only the first line, a
        // notification that truncates, a screenshot of the top of a pane: each of
        // those still says where the answer came from.
        let content = if echoed.is_empty() {
            format!("{ECHO_MARKER} {}", self.answer)
        } else {
            format!("{ECHO_MARKER} {} It was handed: {echoed}", self.answer)
        };
        completion_json("echo", &content)
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

/// A forwarder that answers from a script, one answer per call, in order.
///
/// **What the echo provider cannot do.** It returns the same sentence every time,
/// so it proves the chain carries A answer and nothing about a chain that has to
/// carry SEVERAL. An agent loop that proposes, reads a result and proposes again
/// walks through states, and against a fixed string every state looks alike. This
/// is that one step further: a scripted session, so a run loop has a deterministic
/// oracle and a change to it either reproduces the recorded behaviour or does not.
///
/// **Scripts are written, not captured.** The obvious other half - record a real
/// session and replay it - is deliberately absent, and its absence is a decision
/// rather than an omission. A recorder in this position writes user prompts and
/// whatever graph context was assembled for them into a file, which is a new store
/// of exactly the material the rest of this system spends its effort bounding. That
/// is a judgement about user data, not a coding convenience, so it waits for one.
/// A hand-written script costs a little more to author and can be read in review.
///
/// **It cannot pass for a model either.** Every answer carries [`ECHO_MARKER`] for
/// the same reason the echo provider does: wherever a replayed answer is rendered,
/// quoted or pasted, it says what produced it. Running out of script is answered,
/// loudly and in the content, rather than by falling back to the last answer - a
/// loop that asked more times than the script expected is exactly the defect this
/// exists to catch, and repeating the final answer would hide it.
pub struct ScriptedForwarder {
    /// The answers, in the order they will be handed out.
    answers: Vec<String>,
    /// How many have been handed out. Shared behind `&self` like any forwarder, so
    /// it is atomic rather than a lock.
    next: std::sync::atomic::AtomicUsize,
}

impl ScriptedForwarder {
    /// Parse a script: `{"answers": ["first", "second"]}`.
    ///
    /// An empty list is refused. A script that answers nothing is indistinguishable
    /// at the call site from one that was never loaded, and the failure would show
    /// up as a strange answer several components away.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let v: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("replay script is not JSON: {e}"))?;
        let answers: Vec<String> = v
            .get("answers")
            .and_then(|a| a.as_array())
            .ok_or_else(|| "replay script has no `answers` array".to_string())?
            .iter()
            .map(|a| a.as_str().unwrap_or_default().to_string())
            .collect();
        if answers.is_empty() {
            return Err("replay script has no answers in it".to_string());
        }
        Ok(Self {
            answers,
            next: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// Load a script from disk.
    pub fn from_path(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("replay script {}: {e}", path.display()))?;
        Self::from_json(&text)
    }

    /// How many answers the script holds.
    pub fn len(&self) -> usize {
        self.answers.len()
    }

    /// Whether the script holds no answers. Never true for a loaded one, since
    /// [`Self::from_json`] refuses an empty script; present because clippy asks for
    /// it beside `len`.
    pub fn is_empty(&self) -> bool {
        self.answers.is_empty()
    }

    /// The next answer, or the exhausted notice once the script has run out.
    fn take_next(&self) -> String {
        let i = self
            .next
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        match self.answers.get(i) {
            Some(a) => format!("{ECHO_MARKER} {a}"),
            None => format!(
                "{ECHO_MARKER} the replay script ran out: {} answer(s) were scripted and this is \
                 request {}",
                self.answers.len(),
                i + 1
            ),
        }
    }
}

#[async_trait]
impl Forwarder for ScriptedForwarder {
    async fn post(
        &self,
        _endpoint_url: &str,
        _body_json: &str,
        _auth: AuthHeader<'_>,
    ) -> Result<ForwardResult, ForwardError> {
        Ok(ForwardResult {
            status: 200,
            body: completion_json("replay", &self.take_next()),
        })
    }

    async fn get(
        &self,
        _endpoint_url: &str,
        _auth: AuthHeader<'_>,
    ) -> Result<ForwardResult, ForwardError> {
        Ok(ForwardResult {
            status: 200,
            body: serde_json::json!({"data": [{"id": "replay", "object": "model"}]}).to_string(),
        })
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
mod replay_tests {
    use super::*;

    fn content_of(body: &str) -> String {
        serde_json::from_str::<serde_json::Value>(body).unwrap()["choices"][0]["message"]["content"]
            .as_str()
            .unwrap()
            .to_string()
    }

    const SCRIPT: &str = r#"{"answers": ["first thing", "second thing"]}"#;

    #[tokio::test]
    async fn it_answers_in_order() {
        let f = ScriptedForwarder::from_json(SCRIPT).unwrap();
        assert_eq!(f.len(), 2);
        let a = content_of(&f.post("http://never", "{}", None).await.unwrap().body);
        let b = content_of(&f.post("http://never", "{}", None).await.unwrap().body);
        assert!(a.contains("first thing"), "{a}");
        assert!(b.contains("second thing"), "{b}");
        // The order is the point: a fixed string would satisfy any assertion about
        // "an answer came back", and prove nothing about a loop that runs twice.
        assert_ne!(a, b);
    }

    /// Every answer says what produced it, exactly as the echo provider does.
    #[tokio::test]
    async fn a_replayed_answer_cannot_pass_for_a_model() {
        let f = ScriptedForwarder::from_json(SCRIPT).unwrap();
        let a = content_of(&f.post("http://never", "{}", None).await.unwrap().body);
        assert!(a.starts_with(ECHO_MARKER), "{a}");
    }

    /// The case this shape exists for. A loop that asks a third time has done
    /// something the script did not expect, and the answer has to say so rather
    /// than repeat the last one, which would read as a loop behaving itself.
    #[tokio::test]
    async fn running_out_of_script_is_said_out_loud() {
        let f = ScriptedForwarder::from_json(SCRIPT).unwrap();
        for _ in 0..2 {
            let _ = f.post("http://never", "{}", None).await.unwrap();
        }
        let third = content_of(&f.post("http://never", "{}", None).await.unwrap().body);
        assert!(third.contains("ran out"), "{third}");
        assert!(third.contains("request 3"), "{third}");
        assert!(!third.contains("second thing"), "it must not repeat the last: {third}");
    }

    #[test]
    fn a_script_that_answers_nothing_is_refused() {
        // Loaded-but-empty and never-loaded look identical at the call site, and
        // the difference would surface as a strange answer components away.
        assert!(ScriptedForwarder::from_json(r#"{"answers": []}"#).is_err());
        assert!(ScriptedForwarder::from_json(r#"{"nope": 1}"#).is_err());
        assert!(ScriptedForwarder::from_json("not json").is_err());
    }

    #[tokio::test]
    async fn the_model_list_names_the_replay_provider() {
        let f = ScriptedForwarder::from_json(SCRIPT).unwrap();
        let body = f.get("http://never", None).await.unwrap().body;
        assert!(body.contains("replay"), "{body}");
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
        assert!(r.body.contains(ECHO_MARKER), "{}", r.body);
    }

    /// The guarantee that matters most: **no input produces an answer that could
    /// pass for a model's.** Not the empty body, not a malformed one, not a
    /// request with no messages, not a normal one. A fixture that can answer
    /// anonymously is an absence wearing the costume of a success, which is the
    /// failure this whole evening was spent removing - one layer up.
    ///
    /// Shown to fail before being trusted: dropping the marker from one branch of
    /// `completion` turns this red.
    #[tokio::test]
    async fn no_input_produces_an_anonymous_answer() {
        let bodies = [
            "",
            "not json",
            "{}",
            r#"{"messages":[]}"#,
            r#"{"messages":[{"role":"user","content":"hello"}]}"#,
            r#"{"messages":[{"role":"user"}]}"#,
        ];
        for body in bodies {
            let r = EchoForwarder::new().post("http://x", body, None).await.unwrap();
            let v: serde_json::Value = serde_json::from_str(&r.body).expect("valid json");
            let content = v["choices"][0]["message"]["content"].as_str().unwrap_or("");
            assert!(
                content.starts_with(ECHO_MARKER),
                "an answer that does not open with the marker could pass for a model: {content}"
            );
            assert_eq!(v["model"], "echo", "the model field must name the echo too");
        }
    }
}
