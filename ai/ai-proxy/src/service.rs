//! Proxy service core.
//!
//! [`ProxyService`] holds the allowlist, the trusted provider
//! catalog, the caller allowlist, the audit sink, and the outbound
//! forwarder. The D-Bus surface in `main.rs` is a thin wrapper that
//! converts D-Bus method calls into [`ProxyService::forward`] calls
//! and back. Keeping the service detached from the D-Bus layer keeps
//! every policy decision exercised in unit tests.
//!
//! ## Trust boundaries (Foundation §8.4.6)
//!
//! 1. **The caller does not supply the endpoint URL.** Callers
//!    identify the upstream by a provider *name*. The URL is looked
//!    up from the proxy-owned [`ProviderCatalog`].
//! 2. **The proxy verifies its callers.** Only the
//!    [`CallerAllowlist`] (defaulted to `ai-daemon` + `ai-agent`)
//!    may invoke the proxy. Anything else is rejected before any
//!    outbound work.
//! 3. **The allowlist hostname check still runs**, applied to the
//!    catalogued URL, so a misconfigured catalog cannot smuggle a
//!    new host past the proxy without an explicit allowlist update.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::allowlist::{Allowlist, AllowlistDecision, RejectReason};
use crate::audit::{AuditOutcome, AuditRecord, AuditSink};
use crate::catalog::{ProviderCatalog, WireFormat};
use crate::forward::{ForwardError, Forwarder};

/// Stable error codes returned to D-Bus callers. The audit log
/// records the same `code()` string so logs and caller-side errors
/// align.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    /// Caller is not in the proxy's caller allowlist.
    #[error("caller not allowed: {caller}")]
    CallerNotAllowed {
        /// Unique caller identifier as reported by D-Bus.
        caller: String,
    },
    /// `provider_name` not present in the trusted catalog.
    #[error("unknown provider: {provider}")]
    UnknownProvider {
        /// Provider name from the request.
        provider: String,
    },
    /// Allowlist rejected the catalogued URL.
    #[error("allowlist: {0:?}")]
    Allowlist(RejectReason),
    /// The proxy is already forwarding its maximum number of
    /// concurrent upstream calls.
    #[error("proxy at concurrency capacity")]
    AtCapacity,
    /// The pre-forward audit entry could not be committed, so the
    /// outbound call was refused before the request left the host.
    /// Foundation §8.4.6: no un-audited AI network activity.
    #[error("audit log unavailable")]
    AuditUnavailable,
    /// Upstream call failed transport-side.
    #[error("upstream: {0}")]
    Upstream(#[from] ForwardError),
    /// The catalogued provider uses a wire format the proxy cannot yet shape.
    /// Only the OpenAI chat-completions shape is forwarded verbatim today; an
    /// Anthropic/Gemini entry is refused fail-closed until its transcoder lands,
    /// rather than POST an OpenAI-shaped body to a native endpoint.
    #[error("provider {provider} uses an unsupported wire format")]
    WireFormatUnsupported {
        /// Provider name from the request.
        provider: String,
    },
    /// The request body could not be transcoded into the provider's native
    /// wire format (it was not valid OpenAI chat-completions JSON), so the
    /// call was refused before a malformed body could leave the host.
    #[error("provider {provider} request could not be transcoded")]
    TranscodeFailed {
        /// Provider name from the request.
        provider: String,
    },
    /// The catalogued provider has no model-list endpoint, so a
    /// connection test (`test_provider`) cannot run against it.
    #[error("provider {provider} has no model-list endpoint to test")]
    NoModelsEndpoint {
        /// Provider name from the request.
        provider: String,
    },
    /// Every provider in the requested combo has reached its spending cap this window, so
    /// there is nothing left to try without exceeding a configured limit.
    #[error("every provider in combo '{combo}' has reached its spending cap")]
    SpendingCapReached {
        /// The combo whose members are all capped.
        combo: String,
    },
    /// The keyed provider's credential could not be resolved from the Connections
    /// daemon, so the forward was refused rather than dialled without the key (an
    /// un-authenticated call would leak the prompt upstream for nothing).
    #[error("credential unavailable for provider {provider}")]
    CredentialUnavailable {
        /// Provider name from the request.
        provider: String,
    },
}

impl ProxyError {
    /// Stable kebab-case error code used in audit records and as the
    /// `org.arlen.AIProxy1.<Code>` D-Bus error name.
    pub fn code(&self) -> &'static str {
        match self {
            ProxyError::CallerNotAllowed { .. } => "caller-not-allowed",
            ProxyError::UnknownProvider { .. } => "unknown-provider",
            ProxyError::Allowlist(RejectReason::InvalidUrl) => "invalid-url",
            ProxyError::Allowlist(RejectReason::MissingHost) => "missing-host",
            ProxyError::Allowlist(RejectReason::DisallowedScheme { .. }) => "disallowed-scheme",
            ProxyError::Allowlist(RejectReason::HostNotAllowed { .. }) => "host-not-allowed",
            ProxyError::AtCapacity => "proxy-at-capacity",
            ProxyError::AuditUnavailable => "audit-unavailable",
            ProxyError::Upstream(_) => "upstream-error",
            ProxyError::WireFormatUnsupported { .. } => "wire-format-unsupported",
            ProxyError::TranscodeFailed { .. } => "transcode-failed",
            ProxyError::NoModelsEndpoint { .. } => "no-models-endpoint",
            ProxyError::SpendingCapReached { .. } => "spending-cap-reached",
            ProxyError::CredentialUnavailable { .. } => "credential-unavailable",
        }
    }
}

/// Default ceiling on concurrent upstream forwards. A backstop so
/// that even if a daemon's own in-flight accounting is bypassed
/// (for example by submit/cancel churn) the proxy still bounds the
/// real outbound work it performs.
pub const DEFAULT_MAX_INFLIGHT: usize = 8;

/// RAII slot in the proxy's concurrency counter. Decrements on drop,
/// so every `forward` return path releases its slot.
struct InflightGuard(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Caller identity passed into [`ProxyService::forward`] by the D-Bus
/// layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerIdentity {
    /// Well-known bus name of the caller, if it owns one. The D-Bus
    /// layer fills this in from the message header.
    pub well_known_bus_name: Option<String>,
    /// Unique bus name (`":1.42"`) of the caller. Always present
    /// because every connection has one.
    pub unique_bus_name: String,
}

impl CallerIdentity {
    /// Compact identifier used in audit records and error messages.
    pub fn label(&self) -> &str {
        self.well_known_bus_name
            .as_deref()
            .unwrap_or(&self.unique_bus_name)
    }
}

/// Set of bus names permitted to invoke the proxy.
#[derive(Debug, Clone)]
pub struct CallerAllowlist {
    well_known_names: BTreeSet<String>,
}

impl CallerAllowlist {
    /// Build from any iterable of well-known names. The unique bus
    /// names (`":1.NN"`) are not allowlisted; only well-known names
    /// are stable across daemon restarts.
    pub fn new<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            well_known_names: names.into_iter().map(Into::into).collect(),
        }
    }

    /// The default Arlen caller allowlist: only the AI daemons.
    pub fn default_arlen() -> Self {
        Self::new(["org.arlen.AI1", "org.arlen.AIAgent1"])
    }

    /// Whether the caller is permitted.
    pub fn permits(&self, caller: &CallerIdentity) -> bool {
        match &caller.well_known_bus_name {
            Some(name) => self.well_known_names.contains(name),
            None => false,
        }
    }

    /// Iterator over allowed well-known names. Used by
    /// `list_allowed_endpoints` only for logging; callers cannot
    /// enumerate this list over D-Bus.
    pub fn allowed_names(&self) -> impl Iterator<Item = &str> {
        self.well_known_names.iter().map(String::as_str)
    }
}

/// Input to a single forwarded call.
#[derive(Debug, Clone)]
pub struct ForwardRequest {
    /// Provider catalog key. Maps onto a trusted endpoint URL.
    pub provider_name: String,
    /// JSON body to POST. The proxy does not re-serialise it.
    pub body_json: String,
    /// Capability token presented by the caller. Recorded in the
    /// audit log; not interpreted here.
    pub audit_token: String,
}

/// Output of a forwarded call.
#[derive(Debug, Clone)]
pub struct ForwardOutcome {
    /// HTTP status the upstream returned.
    pub upstream_status: u16,
    /// Upstream response body.
    pub body: String,
}

/// Outcome of a provider connection test (`test_provider`). Serialised to
/// camelCase JSON for the Settings AI-providers manager: `{ ok, httpStatus?,
/// network? }`. `ok` is true only on a 2xx from the model-list endpoint; a
/// non-2xx carries the `httpStatus` (401/403/429 are the meaningful
/// auth/rate-limit signals); a dial that never completed carries `network`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestOutcome {
    /// The provider's model-list endpoint answered with a 2xx.
    pub ok: bool,
    /// The upstream HTTP status, when the probe reached the provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    /// A transport-level failure detail, when the dial never reached the
    /// provider (DNS, connection refused, TLS, timeout).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
}

/// Proxy service. Holds the policy plus the wired-in dependencies
/// (forwarder + audit sink).
pub struct ProxyService {
    allowlist: Allowlist,
    catalog: ProviderCatalog,
    caller_allowlist: CallerAllowlist,
    forwarder: Arc<dyn Forwarder>,
    audit_sink: Arc<dyn AuditSink>,
    /// Resolves the auth header to inject for a keyed provider (from the Connections
    /// daemon). Defaults to a no-op source that injects nothing, so a build without
    /// a wired source behaves exactly as before (key-less providers only).
    credential_source: Arc<dyn crate::connections_client::EgressCredentialSource>,
    /// Per-provider token usage over the configured window, for spending caps + the
    /// transparency surface. Behind a mutex; the accrue is a brief sync lock, never held
    /// across an await.
    usage: Arc<std::sync::Mutex<crate::usage::UsageLedger>>,
    inflight: Arc<std::sync::atomic::AtomicUsize>,
    max_inflight: usize,
}

/// A credential source that injects nothing: the default until the real
/// Connections-backed source is wired in `main`. It returns `Ok(None)` for every
/// request, so a keyed provider simply gets no auth header (the pre-injection
/// behaviour), never a spurious credential.
struct NoCredentialSource;

#[async_trait::async_trait]
impl crate::connections_client::EgressCredentialSource for NoCredentialSource {
    async fn credential_header(
        &self,
        _connection: &str,
        _host: &str,
        _scheme: crate::catalog::AuthScheme,
    ) -> Result<Option<(String, String)>, crate::connections_client::CredentialError> {
        Ok(None)
    }
}

/// Current wall-clock time as epoch seconds, for the usage ledger's window. A clock error
/// (system time before the epoch) floors to 0 rather than panicking.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl ProxyService {
    /// Build the service with the default concurrency ceiling.
    pub fn new(
        allowlist: Allowlist,
        catalog: ProviderCatalog,
        caller_allowlist: CallerAllowlist,
        forwarder: Arc<dyn Forwarder>,
        audit_sink: Arc<dyn AuditSink>,
    ) -> Self {
        Self::with_max_inflight(
            allowlist,
            catalog,
            caller_allowlist,
            forwarder,
            audit_sink,
            DEFAULT_MAX_INFLIGHT,
        )
    }

    /// Build with an explicit concurrency ceiling. Tests use a small
    /// ceiling to exercise the at-capacity path.
    pub fn with_max_inflight(
        allowlist: Allowlist,
        catalog: ProviderCatalog,
        caller_allowlist: CallerAllowlist,
        forwarder: Arc<dyn Forwarder>,
        audit_sink: Arc<dyn AuditSink>,
        max_inflight: usize,
    ) -> Self {
        let usage = Arc::new(std::sync::Mutex::new(crate::usage::UsageLedger::new(
            catalog.limits().window_secs,
            now_secs(),
        )));
        Self {
            allowlist,
            catalog,
            caller_allowlist,
            forwarder,
            audit_sink,
            credential_source: Arc::new(NoCredentialSource),
            usage,
            inflight: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            max_inflight,
        }
    }

    /// Attach the credential source that resolves a keyed provider's auth header at
    /// egress time (the Connections-backed source in production, a mock in tests).
    /// Without this, the service injects no credentials.
    pub fn with_credential_source(
        mut self,
        source: Arc<dyn crate::connections_client::EgressCredentialSource>,
    ) -> Self {
        self.credential_source = source;
        self
    }

    /// Names of catalogued providers the proxy will accept. Returned
    /// to the D-Bus surface for `list_allowed_endpoints`.
    pub fn allowed_providers(&self) -> Vec<String> {
        self.catalog.names().map(str::to_string).collect()
    }

    /// The manager-surface view of every ADDABLE provider (id, name, kind,
    /// configured, builtin, sovereignty) - display metadata only, no endpoint or
    /// credential. Backs the daemon's `ai_providers_list`.
    ///
    /// The picker shows the ACTIVE catalog (local builtins + the user's configured
    /// providers, which carry a credential and so read `configured: true`) merged
    /// with the models.dev available seed (the addable cloud long tail, shown
    /// `configured: false` until a key is attached). The active entry wins on id so
    /// a user-configured provider keeps its state; the forward path is untouched
    /// (it still routes only the active catalog, so no half-working cloud route).
    pub fn provider_views(&self) -> Vec<crate::catalog::ProviderView> {
        let active = self.catalog.views();
        let active_ids: std::collections::HashSet<String> =
            active.iter().map(|v| v.id.clone()).collect();
        let mut views = active;
        for v in ProviderCatalog::available_arlen().views() {
            if !active_ids.contains(&v.id) {
                views.push(v);
            }
        }
        views.sort_by(|a, b| a.id.cmp(&b.id));
        views
    }

    /// The current-window token usage for a provider, for the AI-transparency surface. Reads
    /// zero once the window has elapsed (and if the ledger mutex is poisoned, a safe default).
    pub fn provider_usage(&self, provider: &str) -> crate::usage::ProviderUsage {
        self.usage.lock().map(|led| led.usage_of(provider, now_secs())).unwrap_or_default()
    }

    /// A snapshot of every catalogued provider's current-window usage + configured cap, plus
    /// the shared window countdown, for the AI-transparency surface. Read-only, sorted by id.
    pub fn usage_report(&self) -> crate::usage::UsageReport {
        let now = now_secs();
        let names: Vec<String> = self.catalog.names().map(String::from).collect();
        let led = match self.usage.lock() {
            Ok(l) => l,
            Err(_) => return crate::usage::UsageReport::default(),
        };
        let mut providers: Vec<crate::usage::ProviderUsageView> = names
            .into_iter()
            .map(|id| {
                let usage = led.usage_of(&id, now);
                let cap = self.catalog.cap_for(&id);
                crate::usage::ProviderUsageView { id, usage, cap }
            })
            .collect();
        providers.sort_by(|a, b| a.id.cmp(&b.id));
        crate::usage::UsageReport { window_resets_in_secs: led.resets_in(now), providers }
    }

    /// Forward through a named fallback chain, or a single provider. When `req.provider_name`
    /// names a combo, its providers are tried in order: the first that returns a usable
    /// response serves, and the walk falls to the next only on a provider-availability signal -
    /// an unreachable transport failure, an upstream 429 or 503, or a member whose wire format
    /// this proxy cannot shape (a pre-egress refusal). A definitive response (a 2xx, or a 4xx
    /// the provider answered, including auth failures), a gateway-ambiguous 500/502/504, and any
    /// proxy-side refusal (caller-not-allowed, at-capacity, audit-unavailable, transcode-failed)
    /// return immediately, since falling either would not help or could double-bill. The 429/503
    /// and wire-format falls are pre-completion; the transport fall covers the down-provider case
    /// with one accepted residual - a rare post-send read timeout could bill a completing upstream
    /// the walk then retries. A non-combo name forwards to that single provider unchanged. Each
    /// attempt is audited by `forward`, so the ledger records the whole walk.
    pub async fn forward_combo(
        &self,
        caller: &CallerIdentity,
        req: ForwardRequest,
    ) -> Result<ForwardOutcome, ProxyError> {
        fn should_fall(outcome: &Result<ForwardOutcome, ProxyError>) -> bool {
            match outcome {
                // Unambiguously pre-completion upstream refusals: rate-limited (429) or
                // service-unavailable (503) means the request was rejected before any output
                // was generated, so a fall repeats no billable work. 500/502/504 are
                // gateway-ambiguous (the backend may have generated), so they do NOT fall.
                Ok(o) => matches!(o.upstream_status, 429 | 503),
                // Refused before any egress because this proxy cannot shape the member's wire
                // format; a later member may serve and nothing was sent, so it is safe to fall.
                Err(ProxyError::WireFormatUnsupported { .. }) => true,
                // An unreachable provider (connection refused / DNS / TLS / timeout). Falling
                // covers the down-provider case the combo exists for. Accepted residual: a
                // post-send read timeout is indistinguishable here from a pre-send failure, so
                // a rare timed-out-but-completing upstream could be billed and then retried - an
                // availability-over-cost tradeoff, not a pre-completion guarantee.
                Err(ProxyError::Upstream(ForwardError::Transport(_))) => true,
                // Any other definitive outcome returns immediately: a 2xx, a 4xx the provider
                // answered (incl. 401/403 auth and 400), a gateway-ambiguous 5xx, or a proxy-side
                // refusal (caller-not-allowed, at-capacity, audit-unavailable, transcode-failed).
                Err(_) => false,
            }
        }
        let order: Vec<String> = match self.catalog.combo(&req.provider_name) {
            Some(order) => order.to_vec(),
            None => return self.forward(caller, req).await,
        };
        let now = now_secs();
        let mut last: Option<Result<ForwardOutcome, ProxyError>> = None;
        let mut skipped_capped = false;
        for provider in order {
            // Skip a provider that has reached its spending cap this window: do not dial it
            // (stopping the spend is the point of the cap), and try the next member. A poisoned
            // ledger fails open (dial) rather than denying service on an unreachable error.
            if let Some(cap) = self.catalog.cap_for(&provider) {
                let capped = self
                    .usage
                    .lock()
                    .map(|led| led.reached_cap(&provider, cap, now))
                    .unwrap_or(false);
                if capped {
                    skipped_capped = true;
                    continue;
                }
            }
            let member_req = ForwardRequest { provider_name: provider, ..req.clone() };
            let outcome = self.forward(caller, member_req).await;
            if should_fall(&outcome) {
                last = Some(outcome);
                continue;
            }
            return outcome;
        }
        // Chain exhausted. Surface the last dialed attempt if there was one; else, if every
        // member was skipped for its cap, report that rather than a misleading unknown-provider.
        // The combo is validated non-empty at load, so one of these always applies.
        if let Some(outcome) = last {
            return outcome;
        }
        if skipped_capped {
            return Err(ProxyError::SpendingCapReached { combo: req.provider_name.clone() });
        }
        Err(ProxyError::UnknownProvider { provider: req.provider_name.clone() })
    }

    /// Run a single forward call to one catalogued provider. The audit sink is invoked
    /// regardless of outcome.
    pub async fn forward(
        &self,
        caller: &CallerIdentity,
        req: ForwardRequest,
    ) -> Result<ForwardOutcome, ProxyError> {
        // 1. Caller allowlist.
        if !self.caller_allowlist.permits(caller) {
            let err = ProxyError::CallerNotAllowed {
                caller: caller.label().to_string(),
            };
            self.audit_best_effort(
                &req,
                None,
                AuditOutcome::RejectedByPolicy {
                    code: err.code().to_string(),
                },
            )
            .await;
            return Err(err);
        }

        // 2. Catalog lookup.
        let entry = match self.catalog.get(&req.provider_name) {
            Some(entry) => entry,
            None => {
                let err = ProxyError::UnknownProvider {
                    provider: req.provider_name.clone(),
                };
                self.audit_best_effort(
                    &req,
                    None,
                    AuditOutcome::RejectedByPolicy {
                        code: err.code().to_string(),
                    },
                )
                .await;
                return Err(err);
            }
        };
        let endpoint_url = entry.endpoint_url.clone();
        let wire_format = entry.wire_format;
        let credential_ref = entry.credential_ref.clone();
        let auth_scheme = entry.auth_scheme;

        // 2b. Dispatch on the catalogued wire format. The OpenAI chat-completions
        //     shape is forwarded verbatim; an Anthropic entry is transcoded both
        //     ways around the POST (request before, response after on a 2xx);
        //     Gemini has no transcoder yet, so it is refused fail-closed rather
        //     than POST an OpenAI-shaped body to a native endpoint and leak a
        //     malformed request upstream. `transcode_anthropic` carries the
        //     decision down to the POST below.
        let transcode_anthropic = match wire_format {
            WireFormat::Openai => false,
            WireFormat::Anthropic => true,
            WireFormat::Gemini => {
                let err = ProxyError::WireFormatUnsupported {
                    provider: req.provider_name.clone(),
                };
                self.audit_best_effort(
                    &req,
                    None,
                    AuditOutcome::RejectedByPolicy {
                        code: err.code().to_string(),
                    },
                )
                .await;
                return Err(err);
            }
        };

        // Transcode the request body before it leaves the host. A body that is
        // not valid OpenAI chat-completions JSON cannot be reshaped, so the call
        // is refused fail-closed (no malformed body is POSTed upstream).
        let body_to_post = if transcode_anthropic {
            match crate::transcode::request_body_openai_to_anthropic(&req.body_json) {
                Some(body) => body,
                None => {
                    let err = ProxyError::TranscodeFailed {
                        provider: req.provider_name.clone(),
                    };
                    self.audit_best_effort(
                        &req,
                        None,
                        AuditOutcome::RejectedByPolicy {
                            code: err.code().to_string(),
                        },
                    )
                    .await;
                    return Err(err);
                }
            }
        } else {
            req.body_json.clone()
        };

        // 3. Allowlist on the catalogued URL (defence in depth).
        let host = match self.allowlist.check(&endpoint_url) {
            AllowlistDecision::Allowed { host } => host,
            AllowlistDecision::Rejected(reason) => {
                let err = ProxyError::Allowlist(reason);
                self.audit_best_effort(
                    &req,
                    None,
                    AuditOutcome::RejectedByPolicy {
                        code: err.code().to_string(),
                    },
                )
                .await;
                return Err(err);
            }
        };

        // 4. Reserve a concurrency slot before doing the real
        //    outbound work. The guard releases it on every return
        //    path below. If the proxy is already at its ceiling the
        //    call is refused here, so a flood of forwards cannot
        //    multiply real upstream traffic.
        let prev = self
            .inflight
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _slot = InflightGuard(self.inflight.clone());
        if prev >= self.max_inflight {
            let err = ProxyError::AtCapacity;
            self.audit_best_effort(
                &req,
                Some(&host),
                AuditOutcome::RejectedByPolicy {
                    code: err.code().to_string(),
                },
            )
            .await;
            return Err(err);
        }

        // 5. Audit-before-action gate (foundation §8.4.6). The proxy
        //    is the network egress chokepoint, so it must record the
        //    outbound call *before* it leaves the host and refuse the
        //    call if the ledger cannot record it. On this early return
        //    `_slot` drops and releases the concurrency slot.
        self.audit_forwarding_gate(&req, &host).await?;

        // 5b. Resolve the credential to inject for a keyed provider. The proxy reads
        //     the raw key from the Connections daemon (`credential_ref` names the
        //     connection) and injects it into the outbound request here, so the
        //     calling app never sees it. A key-less provider (`credential_ref: None`,
        //     e.g. a local model) injects nothing. A resolution failure refuses the
        //     forward closed rather than dial upstream un-authenticated.
        let injected = match credential_ref.as_deref() {
            Some(cref) => {
                let connection = cref.strip_prefix("conn:").unwrap_or(cref);
                match self
                    .credential_source
                    .credential_header(connection, &host, auth_scheme)
                    .await
                {
                    Ok(header) => header,
                    Err(_) => {
                        let err = ProxyError::CredentialUnavailable {
                            provider: req.provider_name.clone(),
                        };
                        self.audit_best_effort(
                            &req,
                            Some(&host),
                            AuditOutcome::RejectedByPolicy {
                                code: err.code().to_string(),
                            },
                        )
                        .await;
                        return Err(err);
                    }
                }
            }
            None => None,
        };
        let auth = injected.as_ref().map(|(n, v)| (n.as_str(), v.as_str()));

        // 6. Forward. The status entry is best-effort: the call has
        //    already happened, so a ledger hiccup here does not undo
        //    it; the pre-forward entry already satisfies §8.4.6.
        match self.forwarder.post(&endpoint_url, &body_to_post, auth).await {
            Ok(result) => {
                self.audit_best_effort(
                    &req,
                    Some(&host),
                    AuditOutcome::Forwarded {
                        upstream_status: result.status,
                    },
                )
                .await;
                // Transcode the response back to the OpenAI shape only for a 2xx
                // Anthropic completion; a non-2xx error body is passed through
                // verbatim so the caller sees the real upstream error, not a
                // fabricated empty completion.
                let body = if transcode_anthropic && (200..300).contains(&result.status) {
                    crate::transcode::response_body_anthropic_to_openai(&result.body)
                } else {
                    result.body
                };
                // Meter the tokens this provider spent (for spending caps + the transparency
                // surface). Only a body carrying a usage object accrues; anything else is a
                // no-op. A poisoned ledger mutex is skipped rather than panicking the forward.
                if let Some((prompt, completion)) = crate::usage::tokens_from_response_body(&body) {
                    if let Ok(mut ledger) = self.usage.lock() {
                        ledger.accrue(&req.provider_name, prompt, completion, now_secs());
                    }
                }
                Ok(ForwardOutcome {
                    upstream_status: result.status,
                    body,
                })
            }
            Err(err) => {
                let detail = err.to_string();
                self.audit_best_effort(
                    &req,
                    Some(&host),
                    AuditOutcome::UpstreamError { detail },
                )
                .await;
                Err(ProxyError::Upstream(err))
            }
        }
    }

    /// Run a connection test against a catalogued provider's model-list
    /// endpoint. A body-less `GET` probe that doubles as the
    /// capability-grant verification (`validate_provider`): it answers
    /// "can the proxy reach this provider, and does the provider accept
    /// our credentials". The endpoint URL is taken from the trusted
    /// catalog, never from the caller, so this is allowlist-safe with no
    /// egress-consent moment (unlike a user-supplied URL fetch).
    ///
    /// Policy failures (caller not allowed, unknown provider, no
    /// model-list endpoint, allowlist reject, audit gate down, capacity)
    /// return `Err`. Once the probe is dialed, the result IS the answer:
    /// a transport failure is `Ok(TestOutcome { network })`, a non-2xx is
    /// `Ok(TestOutcome { http_status })`, a 2xx is `Ok(ok: true)`.
    pub async fn test_provider(
        &self,
        caller: &CallerIdentity,
        provider_name: &str,
        audit_token: &str,
    ) -> Result<TestOutcome, ProxyError> {
        let audit = |host: Option<&str>, outcome: AuditOutcome| {
            let record = AuditRecord {
                audit_token: audit_token.to_string(),
                provider_name: provider_name.to_string(),
                host: host.map(str::to_string),
                outcome,
            };
            async move {
                if let Err(err) = self.audit_sink.submit(record.to_ingest_request()).await {
                    tracing::warn!("ai-proxy test audit submit failed: {err}");
                }
            }
        };

        // 1. Caller allowlist (same gate as forward).
        if !self.caller_allowlist.permits(caller) {
            let err = ProxyError::CallerNotAllowed {
                caller: caller.label().to_string(),
            };
            audit(None, AuditOutcome::RejectedByPolicy { code: err.code().to_string() }).await;
            return Err(err);
        }

        // 2. Catalog lookup + model-list endpoint resolution.
        let entry = match self.catalog.get(provider_name) {
            Some(entry) => entry,
            None => {
                let err = ProxyError::UnknownProvider {
                    provider: provider_name.to_string(),
                };
                audit(None, AuditOutcome::RejectedByPolicy { code: err.code().to_string() }).await;
                return Err(err);
            }
        };
        let credential_ref = entry.credential_ref.clone();
        let auth_scheme = entry.auth_scheme;
        let endpoint_url = match entry.models_endpoint.clone() {
            Some(url) => url,
            None => {
                let err = ProxyError::NoModelsEndpoint {
                    provider: provider_name.to_string(),
                };
                audit(None, AuditOutcome::RejectedByPolicy { code: err.code().to_string() }).await;
                return Err(err);
            }
        };

        // 3. Allowlist on the catalogued URL (defence in depth, same as forward).
        let host = match self.allowlist.check(&endpoint_url) {
            AllowlistDecision::Allowed { host } => host,
            AllowlistDecision::Rejected(reason) => {
                let err = ProxyError::Allowlist(reason);
                audit(None, AuditOutcome::RejectedByPolicy { code: err.code().to_string() }).await;
                return Err(err);
            }
        };

        // 4. Reserve a concurrency slot so a flood of tests cannot multiply egress.
        let prev = self
            .inflight
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _slot = InflightGuard(self.inflight.clone());
        if prev >= self.max_inflight {
            let err = ProxyError::AtCapacity;
            audit(Some(&host), AuditOutcome::RejectedByPolicy { code: err.code().to_string() }).await;
            return Err(err);
        }

        // 5. Audit-before-egress gate, fail-closed: a test is still outbound
        //    traffic (§8.4.6), so record it before the probe leaves the host
        //    and refuse if the ledger cannot.
        {
            let record = AuditRecord {
                audit_token: audit_token.to_string(),
                provider_name: provider_name.to_string(),
                host: Some(host.clone()),
                outcome: AuditOutcome::TestConnection,
            };
            self.audit_sink
                .submit(record.to_ingest_request())
                .await
                .map_err(|err| {
                    tracing::warn!("ai-proxy test refused: audit log unavailable: {err}");
                    ProxyError::AuditUnavailable
                })?;
        }

        // 5b. Inject the provider credential (a keyed provider's model-list probe
        //     needs the key to answer "does the provider accept our credentials").
        //     A resolution failure refuses the test closed.
        let injected = match credential_ref.as_deref() {
            Some(cref) => {
                let connection = cref.strip_prefix("conn:").unwrap_or(cref);
                match self
                    .credential_source
                    .credential_header(connection, &host, auth_scheme)
                    .await
                {
                    Ok(header) => header,
                    Err(_) => {
                        let err = ProxyError::CredentialUnavailable {
                            provider: provider_name.to_string(),
                        };
                        audit(Some(&host), AuditOutcome::RejectedByPolicy { code: err.code().to_string() }).await;
                        return Err(err);
                    }
                }
            }
            None => None,
        };
        let auth = injected.as_ref().map(|(n, v)| (n.as_str(), v.as_str()));

        // 6. Probe. The transport outcome IS the test result, not an error:
        //    a refused dial is the truthful "network" verdict, a non-2xx the
        //    "httpStatus" verdict. Body is discarded (only the status matters).
        match self.forwarder.get(&endpoint_url, auth).await {
            Ok(result) => {
                audit(
                    Some(&host),
                    AuditOutcome::TestConnectionResult { upstream_status: result.status },
                )
                .await;
                Ok(TestOutcome {
                    ok: (200..300).contains(&result.status),
                    http_status: Some(result.status),
                    network: None,
                })
            }
            Err(err) => {
                let detail = err.to_string();
                audit(Some(&host), AuditOutcome::UpstreamError { detail: detail.clone() }).await;
                Ok(TestOutcome {
                    ok: false,
                    http_status: None,
                    network: Some(detail),
                })
            }
        }
    }

    /// Commit the fail-closed pre-forward entry. Returns
    /// `Err(ProxyError::AuditUnavailable)` if the ledger cannot record
    /// it, so the caller refuses the forward rather than letting an
    /// unaudited request leave the host.
    async fn audit_forwarding_gate(
        &self,
        req: &ForwardRequest,
        host: &str,
    ) -> Result<(), ProxyError> {
        let record = AuditRecord {
            audit_token: req.audit_token.clone(),
            provider_name: req.provider_name.clone(),
            host: Some(host.to_string()),
            outcome: AuditOutcome::Forwarding,
        };
        self.audit_sink
            .submit(record.to_ingest_request())
            .await
            .map(|_| ())
            .map_err(|err| {
                tracing::warn!(
                    "ai-proxy forward refused: audit log unavailable: {err}"
                );
                ProxyError::AuditUnavailable
            })
    }

    /// Record one audit entry best-effort: a ledger failure is logged,
    /// not propagated. Used for rejections (nothing left the host) and
    /// for the post-forward status entry (the call already happened).
    async fn audit_best_effort(
        &self,
        req: &ForwardRequest,
        host: Option<&str>,
        outcome: AuditOutcome,
    ) {
        let record = AuditRecord {
            audit_token: req.audit_token.clone(),
            provider_name: req.provider_name.clone(),
            host: host.map(str::to_string),
            outcome,
        };
        if let Err(err) = self.audit_sink.submit(record.to_ingest_request()).await {
            tracing::warn!("ai-proxy audit submit failed: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::test_support::CollectingAuditSink;
    use crate::forward::test_support::StubForwarder;
    use crate::forward::ForwardResult;
    use async_trait::async_trait;

    fn ai_daemon_caller() -> CallerIdentity {
        CallerIdentity {
            well_known_bus_name: Some("org.arlen.AI1".to_string()),
            unique_bus_name: ":1.42".to_string(),
        }
    }

    fn service_with(
        forwarder: Arc<StubForwarder>,
        sink: Arc<CollectingAuditSink>,
    ) -> ProxyService {
        ProxyService::new(
            Allowlist::default_arlen(),
            ProviderCatalog::default_arlen(),
            CallerAllowlist::default_arlen(),
            forwarder as Arc<dyn Forwarder>,
            sink as Arc<dyn AuditSink>,
        )
    }

    #[tokio::test]
    async fn happy_path_forwards_via_catalog_url() {
        let forwarder = Arc::new(StubForwarder::new(vec![Ok(ForwardResult {
            status: 200,
            body: r#"{"ok":true}"#.to_string(),
        })]));
        let sink = Arc::new(CollectingAuditSink::new());
        let svc = service_with(forwarder.clone(), sink.clone());

        let out = svc
            .forward(
                &ai_daemon_caller(),
                ForwardRequest {
                    provider_name: "ollama-default".to_string(),
                    body_json: "{}".to_string(),
                    audit_token: "tok-1".to_string(),
                },
            )
            .await
            .expect("ok");
        assert_eq!(out.upstream_status, 200);

        let calls = forwarder.calls.lock().await;
        assert_eq!(calls.len(), 1);
        // The forwarder must have been called with the *catalogued*
        // URL, not with anything the caller supplied.
        assert_eq!(calls[0].0, "http://127.0.0.1:11434/v1/chat/completions");

        let records = sink.snapshot().await;
        // Two entries: the fail-closed pre-forward gate, then the
        // best-effort status entry.
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].structural.outcome, "forwarding");
        assert_eq!(records[0].structural.subject, "127.0.0.1");
        assert_eq!(records[1].structural.outcome, "forwarded-200");
    }

    #[tokio::test]
    async fn forward_meters_provider_token_usage() {
        // A 200 whose body carries a usage object accrues that provider's tokens.
        let forwarder = Arc::new(StubForwarder::new(vec![Ok(ForwardResult {
            status: 200,
            body: r#"{"choices":[],"usage":{"prompt_tokens":30,"completion_tokens":12,"total_tokens":42}}"#
                .to_string(),
        })]));
        let svc = service_with(forwarder, Arc::new(CollectingAuditSink::new()));
        svc.forward(
            &ai_daemon_caller(),
            ForwardRequest {
                provider_name: "ollama-default".to_string(),
                body_json: "{}".to_string(),
                audit_token: "tok".to_string(),
            },
        )
        .await
        .expect("ok");
        let u = svc.provider_usage("ollama-default");
        assert_eq!(u.prompt_tokens, 30);
        assert_eq!(u.completion_tokens, 12);
        assert_eq!(u.total_tokens, 42);
        assert_eq!(u.requests, 1);
        // an unused provider meters nothing
        assert_eq!(svc.provider_usage("nonexistent").total_tokens, 0);
    }

    #[tokio::test]
    async fn usage_report_snapshots_per_provider_usage_and_caps() {
        let forwarder = Arc::new(StubForwarder::new(vec![Ok(ForwardResult {
            status: 200,
            body: r#"{"usage":{"prompt_tokens":8,"completion_tokens":4,"total_tokens":12}}"#
                .to_string(),
        })]));
        let svc = service_with(forwarder, Arc::new(CollectingAuditSink::new()));
        svc.forward(
            &ai_daemon_caller(),
            ForwardRequest {
                provider_name: "ollama-default".to_string(),
                body_json: "{}".to_string(),
                audit_token: "tok".to_string(),
            },
        )
        .await
        .expect("ok");
        let report = svc.usage_report();
        let ollama =
            report.providers.iter().find(|p| p.id == "ollama-default").expect("in the report");
        assert_eq!(ollama.usage.total_tokens, 12);
        assert_eq!(ollama.cap, None, "the default catalog configures no cap");
        assert!(report.window_resets_in_secs > 0, "the window is counting down");
    }

    fn combo_catalog(name: &str) -> ProviderCatalog {
        // p1 and p2 share the loopback ollama URL (which passes the allowlist and needs no
        // key); the StubForwarder ignores the URL and returns its scripted results in order.
        let path = std::env::temp_dir().join(format!("arlen-proxy-{name}.toml"));
        std::fs::write(
            &path,
            "[providers.p1]\n\
             endpoint_url = \"http://127.0.0.1:11434/v1/chat/completions\"\n\
             backend = \"ollama\"\n\
             [providers.p2]\n\
             endpoint_url = \"http://127.0.0.1:11434/v1/chat/completions\"\n\
             backend = \"ollama\"\n\
             [combos]\n\
             chain = [\"p1\", \"p2\"]\n",
        )
        .unwrap();
        let catalog = ProviderCatalog::load_or_default(&path).unwrap();
        std::fs::remove_file(&path).ok();
        catalog
    }

    fn combo_service(
        catalog: ProviderCatalog,
        forwarder: Arc<StubForwarder>,
    ) -> ProxyService {
        ProxyService::new(
            Allowlist::default_arlen(),
            catalog,
            CallerAllowlist::default_arlen(),
            forwarder as Arc<dyn Forwarder>,
            Arc::new(CollectingAuditSink::new()) as Arc<dyn AuditSink>,
        )
    }

    fn chain_req() -> ForwardRequest {
        ForwardRequest {
            provider_name: "chain".to_string(),
            body_json: "{}".to_string(),
            audit_token: "tok".to_string(),
        }
    }

    #[tokio::test]
    async fn forward_combo_falls_over_an_unavailable_provider_to_the_next() {
        // p1 returns 503 (unavailable), so the combo walk falls to p2, which serves 200.
        let forwarder = Arc::new(StubForwarder::new(vec![
            Ok(ForwardResult { status: 503, body: "unavailable".to_string() }),
            Ok(ForwardResult { status: 200, body: "served".to_string() }),
        ]));
        let svc = combo_service(combo_catalog("combo-fall"), forwarder.clone());
        let out = svc
            .forward_combo(&ai_daemon_caller(), chain_req())
            .await
            .expect("p2 serves after p1 is unavailable");
        assert_eq!(out.upstream_status, 200);
        assert_eq!(out.body, "served");
        assert_eq!(forwarder.calls.lock().await.len(), 2, "tried p1 then p2");
    }

    #[tokio::test]
    async fn forward_combo_returns_a_definitive_response_without_falling() {
        // A 200 from the first provider serves immediately; the second is never tried.
        let forwarder = Arc::new(StubForwarder::new(vec![
            Ok(ForwardResult { status: 200, body: "first".to_string() }),
            Ok(ForwardResult { status: 200, body: "second".to_string() }),
        ]));
        let svc = combo_service(combo_catalog("combo-first"), forwarder.clone());
        let out = svc.forward_combo(&ai_daemon_caller(), chain_req()).await.expect("first serves");
        assert_eq!(out.body, "first");
        assert_eq!(forwarder.calls.lock().await.len(), 1, "second provider never tried");
    }

    #[tokio::test]
    async fn forward_combo_falls_over_a_transport_error() {
        // p1 is unreachable (a transport error); the walk falls to p2, which serves.
        let forwarder = Arc::new(StubForwarder::new(vec![
            Err(ForwardError::Transport("connection refused".to_string())),
            Ok(ForwardResult { status: 200, body: "served".to_string() }),
        ]));
        let svc = combo_service(combo_catalog("combo-transport"), forwarder.clone());
        let out = svc
            .forward_combo(&ai_daemon_caller(), chain_req())
            .await
            .expect("p2 serves after p1 is unreachable");
        assert_eq!(out.upstream_status, 200);
        assert_eq!(forwarder.calls.lock().await.len(), 2);
    }

    #[tokio::test]
    async fn forward_combo_surfaces_the_last_attempt_when_the_whole_chain_is_unavailable() {
        // Both members return 503; the walk exhausts the chain and surfaces the last outcome
        // (exercising the post-loop `last` path).
        let forwarder = Arc::new(StubForwarder::new(vec![
            Ok(ForwardResult { status: 503, body: "down".to_string() }),
            Ok(ForwardResult { status: 503, body: "also down".to_string() }),
        ]));
        let svc = combo_service(combo_catalog("combo-exhaust"), forwarder.clone());
        let out = svc.forward_combo(&ai_daemon_caller(), chain_req()).await.expect("last surfaced");
        assert_eq!(out.upstream_status, 503);
        assert_eq!(out.body, "also down");
        assert_eq!(forwarder.calls.lock().await.len(), 2, "both tried");
    }

    #[tokio::test]
    async fn forward_combo_does_not_fall_on_an_auth_failure() {
        // A 401 is a definitive provider answer, not an availability signal: return it and do
        // not fall, so a systemic auth misconfiguration is not masked by the next member.
        let forwarder = Arc::new(StubForwarder::new(vec![
            Ok(ForwardResult { status: 401, body: "unauthorized".to_string() }),
            Ok(ForwardResult { status: 200, body: "should-not-reach".to_string() }),
        ]));
        let svc = combo_service(combo_catalog("combo-auth"), forwarder.clone());
        let out = svc.forward_combo(&ai_daemon_caller(), chain_req()).await.expect("401 returned");
        assert_eq!(out.upstream_status, 401);
        assert_eq!(forwarder.calls.lock().await.len(), 1, "did not fall past the auth failure");
    }

    #[tokio::test]
    async fn forward_combo_falls_over_an_unshapeable_wire_format() {
        // p1's wire format the proxy cannot shape (Gemini) is a pre-egress refusal, so the walk
        // falls to the OpenAI member, which serves. The forwarder is only reached for p2.
        let path = std::env::temp_dir().join("arlen-proxy-combo-wire.toml");
        std::fs::write(
            &path,
            "[providers.gem]\n\
             endpoint_url = \"http://127.0.0.1:11434/v1/chat/completions\"\n\
             backend = \"gemini\"\n\
             wire_format = \"gemini\"\n\
             [providers.oai]\n\
             endpoint_url = \"http://127.0.0.1:11434/v1/chat/completions\"\n\
             backend = \"ollama\"\n\
             [combos]\n\
             chain = [\"gem\", \"oai\"]\n",
        )
        .unwrap();
        let catalog = ProviderCatalog::load_or_default(&path).unwrap();
        std::fs::remove_file(&path).ok();
        let forwarder =
            Arc::new(StubForwarder::new(vec![Ok(ForwardResult { status: 200, body: "oai".into() })]));
        let svc = combo_service(catalog, forwarder.clone());
        let out = svc
            .forward_combo(&ai_daemon_caller(), chain_req())
            .await
            .expect("the OpenAI member serves after the unshapeable Gemini member");
        assert_eq!(out.upstream_status, 200);
        assert_eq!(out.body, "oai");
        assert_eq!(forwarder.calls.lock().await.len(), 1, "only the OpenAI member reached egress");
    }

    fn capped_catalog(name: &str, limits_toml: &str) -> ProviderCatalog {
        let path = std::env::temp_dir().join(format!("arlen-proxy-{name}.toml"));
        std::fs::write(
            &path,
            format!(
                "[providers.p1]\n\
                 endpoint_url = \"http://127.0.0.1:11434/v1/chat/completions\"\n\
                 backend = \"ollama\"\n\
                 [providers.p2]\n\
                 endpoint_url = \"http://127.0.0.1:11434/v1/chat/completions\"\n\
                 backend = \"ollama\"\n\
                 [combos]\n\
                 chain = [\"p1\", \"p2\"]\n\
                 {limits_toml}"
            ),
        )
        .unwrap();
        let catalog = ProviderCatalog::load_or_default(&path).unwrap();
        std::fs::remove_file(&path).ok();
        catalog
    }

    #[tokio::test]
    async fn forward_combo_skips_a_capped_provider() {
        // p1's cap is 0 (no headroom this window), so the walk skips it WITHOUT dialing and
        // p2 serves - the spending cap triggering the fallback.
        let cat = capped_catalog("combo-capped", "[limits.caps]\np1 = 0\n");
        let forwarder =
            Arc::new(StubForwarder::new(vec![Ok(ForwardResult { status: 200, body: "p2".into() })]));
        let svc = combo_service(cat, forwarder.clone());
        let out = svc.forward_combo(&ai_daemon_caller(), chain_req()).await.expect("p2 serves");
        assert_eq!(out.body, "p2");
        assert_eq!(forwarder.calls.lock().await.len(), 1, "p1 skipped (not dialed), only p2 reached");
    }

    #[tokio::test]
    async fn forward_combo_reports_when_every_member_is_capped() {
        // both members capped at 0 -> nothing is dialed and the cap condition is reported.
        let cat = capped_catalog("combo-allcapped", "[limits.caps]\np1 = 0\np2 = 0\n");
        let forwarder = Arc::new(StubForwarder::new(vec![]));
        let svc = combo_service(cat, forwarder.clone());
        let err = svc.forward_combo(&ai_daemon_caller(), chain_req()).await.unwrap_err();
        assert!(matches!(err, ProxyError::SpendingCapReached { .. }), "got {err:?}");
        assert_eq!(forwarder.calls.lock().await.len(), 0, "nothing dialed when all are capped");
    }

    #[tokio::test]
    async fn test_provider_probes_the_catalogued_models_endpoint() {
        // A 2xx from the model-list endpoint is the `ok` verdict, and the
        // GET must hit the *catalogued* models URL, never a caller value.
        let forwarder = Arc::new(StubForwarder::new(vec![Ok(ForwardResult {
            status: 200,
            body: r#"{"data":[]}"#.to_string(),
        })]));
        let sink = Arc::new(CollectingAuditSink::new());
        let svc = service_with(forwarder.clone(), sink.clone());

        let out = svc
            .test_provider(&ai_daemon_caller(), "ollama-default", "tok-t")
            .await
            .expect("ok");
        assert!(out.ok);
        assert_eq!(out.http_status, Some(200));
        assert!(out.network.is_none());

        let calls = forwarder.calls.lock().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "http://127.0.0.1:11434/v1/models");
        assert_eq!(calls[0].1, ""); // a GET carries no body

        let records = sink.snapshot().await;
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].structural.outcome, "test-connection");
        assert_eq!(records[0].structural.subject, "127.0.0.1");
        assert_eq!(records[1].structural.outcome, "test-connection-200");
    }

    #[tokio::test]
    async fn test_provider_reports_a_non_2xx_status() {
        // A 401 is the meaningful "configured but unauthorized" signal: not an
        // error, a truthful verdict the manager surfaces.
        let forwarder = Arc::new(StubForwarder::new(vec![Ok(ForwardResult {
            status: 401,
            body: String::new(),
        })]));
        let sink = Arc::new(CollectingAuditSink::new());
        let svc = service_with(forwarder, sink);

        let out = svc
            .test_provider(&ai_daemon_caller(), "ollama-default", "tok-t")
            .await
            .expect("reached upstream");
        assert!(!out.ok);
        assert_eq!(out.http_status, Some(401));
    }

    #[tokio::test]
    async fn test_provider_rejects_an_unknown_provider() {
        let forwarder = Arc::new(StubForwarder::new(vec![]));
        let sink = Arc::new(CollectingAuditSink::new());
        let svc = service_with(forwarder, sink);

        let err = svc
            .test_provider(&ai_daemon_caller(), "ghost-provider", "tok-t")
            .await
            .expect_err("unknown provider");
        assert_eq!(err.code(), "unknown-provider");
    }

    #[tokio::test]
    async fn test_provider_refuses_a_non_allowlisted_caller() {
        let forwarder = Arc::new(StubForwarder::new(vec![]));
        let sink = Arc::new(CollectingAuditSink::new());
        let svc = service_with(forwarder, sink);
        let stranger = CallerIdentity {
            well_known_bus_name: Some("com.example.Stranger".to_string()),
            unique_bus_name: ":1.99".to_string(),
        };
        let err = svc
            .test_provider(&stranger, "ollama-default", "tok-t")
            .await
            .expect_err("caller not allowed");
        assert_eq!(err.code(), "caller-not-allowed");
    }

    #[tokio::test]
    async fn test_provider_refuses_a_provider_with_no_models_endpoint() {
        // A catalogued provider that declares no model-list endpoint must be
        // refused before any egress, not silently dialed at a wrong URL. The
        // forwarder carries no queued response: reaching it would be the bug.
        let forwarder = Arc::new(StubForwarder::new(vec![]));
        let sink = Arc::new(CollectingAuditSink::new());
        let svc = ProxyService::new(
            Allowlist::default_arlen(),
            catalog_with(
                "custom-cloud",
                "https://api.anthropic.com/v1/messages",
                WireFormat::Anthropic,
            ),
            CallerAllowlist::default_arlen(),
            forwarder.clone() as Arc<dyn Forwarder>,
            sink as Arc<dyn AuditSink>,
        );

        let err = svc
            .test_provider(&ai_daemon_caller(), "custom-cloud", "tok-t")
            .await
            .expect_err("a provider with no model-list endpoint is refused");
        assert_eq!(err.code(), "no-models-endpoint");
        // No egress: the refusal landed before the probe was dialed.
        assert!(forwarder.calls.lock().await.is_empty());
    }

    #[test]
    fn test_outcome_serializes_to_the_manager_camelcase_shape() {
        // The manager UI branches on these exact keys: `ok`, then `httpStatus`
        // for a reached-but-non-2xx provider, or `network` for a dial that never
        // landed. Skip-if-none keeps the absent field off the wire. A rename here
        // compiles and passes the verdict tests above yet breaks the manager.
        let ok = serde_json::to_value(TestOutcome { ok: true, http_status: None, network: None })
            .expect("serializes");
        assert_eq!(ok, serde_json::json!({ "ok": true }));

        let unauthorized = serde_json::to_value(TestOutcome {
            ok: false,
            http_status: Some(401),
            network: None,
        })
        .expect("serializes");
        assert_eq!(unauthorized, serde_json::json!({ "ok": false, "httpStatus": 401 }));

        let unreachable = serde_json::to_value(TestOutcome {
            ok: false,
            http_status: None,
            network: Some("connection refused".to_string()),
        })
        .expect("serializes");
        assert_eq!(
            unreachable,
            serde_json::json!({ "ok": false, "network": "connection refused" })
        );
    }

    /// Build a catalog with a single cloud provider under the given wire format.
    fn catalog_with(name: &str, url: &str, wire_format: WireFormat) -> ProviderCatalog {
        use std::collections::HashMap;
        let mut entries = HashMap::new();
        entries.insert(
            name.to_string(),
            crate::catalog::CatalogEntry {
                endpoint_url: url.to_string(),
                backend: name.to_string(),
                wire_format,
                auth_scheme: crate::catalog::AuthScheme::XApiKey,
                url_template: None,
                credential_ref: Some(format!("conn:{name}")),
                models_endpoint: None,
                display_name: None,
                logo_id: None,
                auth_method: crate::catalog::AuthMethod::ApiKey,
                unofficial: false,
                builtin: false,
                sovereignty: Default::default(),
            },
        );
        ProviderCatalog::new(entries)
    }

    #[tokio::test]
    async fn an_anthropic_provider_is_transcoded_both_ways() {
        // The upstream returns a native Anthropic completion; the proxy must POST
        // the request in Anthropic shape and hand the caller an OpenAI-shaped
        // response back.
        let forwarder = Arc::new(StubForwarder::new(vec![Ok(ForwardResult {
            status: 200,
            body: r#"{"id":"msg_1","type":"message","role":"assistant","model":"claude-x","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":1}}"#.to_string(),
        })]));
        let sink = Arc::new(CollectingAuditSink::new());
        let svc = ProxyService::new(
            Allowlist::default_arlen(),
            catalog_with("anthropic", "https://api.anthropic.com/v1/messages", WireFormat::Anthropic),
            CallerAllowlist::default_arlen(),
            forwarder.clone() as Arc<dyn Forwarder>,
            sink.clone() as Arc<dyn AuditSink>,
        );

        let out = svc
            .forward(
                &ai_daemon_caller(),
                ForwardRequest {
                    provider_name: "anthropic".to_string(),
                    body_json: r#"{"model":"claude-x","messages":[{"role":"system","content":"be terse"},{"role":"user","content":"hi"}]}"#.to_string(),
                    audit_token: "tok-1".to_string(),
                },
            )
            .await
            .expect("an anthropic provider forwards transcoded");
        assert_eq!(out.upstream_status, 200);

        // The posted body is the Anthropic request shape: system lifted to the
        // top level, max_tokens defaulted in, the user turn under `messages`.
        let calls = forwarder.calls.lock().await;
        assert_eq!(calls.len(), 1);
        let posted: serde_json::Value = serde_json::from_str(&calls[0].1).expect("posted JSON");
        assert_eq!(posted["system"], serde_json::json!("be terse"));
        assert!(posted["max_tokens"].is_number());
        assert_eq!(posted["messages"][0]["role"], serde_json::json!("user"));

        // The returned body is the OpenAI completion shape.
        let got: serde_json::Value = serde_json::from_str(&out.body).expect("response JSON");
        assert_eq!(got["object"], serde_json::json!("chat.completion"));
        assert_eq!(got["choices"][0]["message"]["content"], serde_json::json!("hello"));
        assert_eq!(got["choices"][0]["finish_reason"], serde_json::json!("stop"));
    }

    /// A scripted credential source for the injection tests.
    struct FixedCredentialSource(Result<Option<(String, String)>, ()>);

    #[async_trait::async_trait]
    impl crate::connections_client::EgressCredentialSource for FixedCredentialSource {
        async fn credential_header(
            &self,
            _connection: &str,
            _host: &str,
            _scheme: crate::catalog::AuthScheme,
        ) -> Result<Option<(String, String)>, crate::connections_client::CredentialError> {
            match &self.0 {
                Ok(v) => Ok(v.clone()),
                Err(()) => Err(crate::connections_client::CredentialError::Daemon("x".into())),
            }
        }
    }

    #[tokio::test]
    async fn a_keyed_provider_injects_the_resolved_header() {
        // A keyed provider (credential_ref set) must carry the credential the source
        // resolves as an auth header on the outbound POST; the calling app supplies
        // nothing.
        let forwarder = Arc::new(StubForwarder::new(vec![Ok(ForwardResult {
            status: 200,
            body: r#"{"id":"msg_1","type":"message","role":"assistant","model":"c","content":[{"type":"text","text":"hi"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#.to_string(),
        })]));
        let sink = Arc::new(CollectingAuditSink::new());
        let svc = ProxyService::new(
            Allowlist::default_arlen(),
            catalog_with("anthropic", "https://api.anthropic.com/v1/messages", WireFormat::Anthropic),
            CallerAllowlist::default_arlen(),
            forwarder.clone() as Arc<dyn Forwarder>,
            sink as Arc<dyn AuditSink>,
        )
        .with_credential_source(Arc::new(FixedCredentialSource(Ok(Some((
            "x-api-key".to_string(),
            "sk-ant-secret".to_string(),
        ))))));

        svc.forward(
            &ai_daemon_caller(),
            ForwardRequest {
                provider_name: "anthropic".to_string(),
                body_json: r#"{"model":"c","messages":[{"role":"user","content":"hi"}]}"#.to_string(),
                audit_token: "tok-1".to_string(),
            },
        )
        .await
        .expect("keyed provider forwards with the injected key");

        let headers = forwarder.auth_headers.lock().await;
        assert_eq!(
            headers[0],
            Some(("x-api-key".to_string(), "sk-ant-secret".to_string())),
            "the resolved credential must be injected as the auth header"
        );
    }

    #[tokio::test]
    async fn a_credential_source_failure_refuses_the_forward() {
        // If the Connections daemon cannot hand over the key, the proxy refuses the
        // forward closed rather than dial upstream un-authenticated.
        let forwarder = Arc::new(StubForwarder::new(vec![Ok(ForwardResult {
            status: 200,
            body: "{}".to_string(),
        })]));
        let sink = Arc::new(CollectingAuditSink::new());
        let svc = ProxyService::new(
            Allowlist::default_arlen(),
            catalog_with("anthropic", "https://api.anthropic.com/v1/messages", WireFormat::Anthropic),
            CallerAllowlist::default_arlen(),
            forwarder.clone() as Arc<dyn Forwarder>,
            sink as Arc<dyn AuditSink>,
        )
        .with_credential_source(Arc::new(FixedCredentialSource(Err(()))));

        let err = svc
            .forward(
                &ai_daemon_caller(),
                ForwardRequest {
                    provider_name: "anthropic".to_string(),
                    body_json: r#"{"model":"c","messages":[{"role":"user","content":"hi"}]}"#.to_string(),
                    audit_token: "tok-1".to_string(),
                },
            )
            .await
            .expect_err("a credential failure must refuse the forward");
        assert_eq!(err.code(), "credential-unavailable");
        // No dial happened: the key was never resolved, so nothing left the host.
        assert!(forwarder.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn a_malformed_request_body_is_refused_before_forwarding() {
        // A body that is not valid OpenAI JSON cannot be transcoded; the proxy
        // fails closed rather than POST garbage to the native endpoint.
        let forwarder = Arc::new(StubForwarder::new(vec![]));
        let sink = Arc::new(CollectingAuditSink::new());
        let svc = ProxyService::new(
            Allowlist::default_arlen(),
            catalog_with("anthropic", "https://api.anthropic.com/v1/messages", WireFormat::Anthropic),
            CallerAllowlist::default_arlen(),
            forwarder.clone() as Arc<dyn Forwarder>,
            sink.clone() as Arc<dyn AuditSink>,
        );

        let err = svc
            .forward(
                &ai_daemon_caller(),
                ForwardRequest {
                    provider_name: "anthropic".to_string(),
                    body_json: "not json".to_string(),
                    audit_token: "tok-1".to_string(),
                },
            )
            .await
            .expect_err("a malformed body is refused");
        assert_eq!(err.code(), "transcode-failed");
        assert!(forwarder.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn a_gemini_provider_is_refused_until_its_transcoder() {
        // Gemini has no transcoder yet, so it stays fail-closed.
        let forwarder = Arc::new(StubForwarder::new(vec![]));
        let sink = Arc::new(CollectingAuditSink::new());
        let svc = ProxyService::new(
            Allowlist::default_arlen(),
            catalog_with("gemini", "https://generativelanguage.googleapis.com/v1/x", WireFormat::Gemini),
            CallerAllowlist::default_arlen(),
            forwarder.clone() as Arc<dyn Forwarder>,
            sink.clone() as Arc<dyn AuditSink>,
        );

        let err = svc
            .forward(
                &ai_daemon_caller(),
                ForwardRequest {
                    provider_name: "gemini".to_string(),
                    body_json: "{}".to_string(),
                    audit_token: "tok-1".to_string(),
                },
            )
            .await
            .expect_err("gemini is refused until its transcoder");
        assert_eq!(err.code(), "wire-format-unsupported");
        assert!(forwarder.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn caller_not_in_allowlist_is_rejected() {
        let forwarder = Arc::new(StubForwarder::new(vec![]));
        let sink = Arc::new(CollectingAuditSink::new());
        let svc = service_with(forwarder.clone(), sink.clone());

        let unknown = CallerIdentity {
            well_known_bus_name: Some("com.example.evil".to_string()),
            unique_bus_name: ":1.7".to_string(),
        };
        let err = svc
            .forward(
                &unknown,
                ForwardRequest {
                    provider_name: "ollama-default".to_string(),
                    body_json: "{}".to_string(),
                    audit_token: "tok-2".to_string(),
                },
            )
            .await
            .expect_err("reject");
        assert_eq!(err.code(), "caller-not-allowed");
        assert!(forwarder.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn caller_with_no_well_known_name_is_rejected() {
        let forwarder = Arc::new(StubForwarder::new(vec![]));
        let sink = Arc::new(CollectingAuditSink::new());
        let svc = service_with(forwarder.clone(), sink.clone());

        let anon = CallerIdentity {
            well_known_bus_name: None,
            unique_bus_name: ":1.9".to_string(),
        };
        let err = svc
            .forward(
                &anon,
                ForwardRequest {
                    provider_name: "ollama-default".to_string(),
                    body_json: "{}".to_string(),
                    audit_token: "tok-3".to_string(),
                },
            )
            .await
            .expect_err("reject");
        assert_eq!(err.code(), "caller-not-allowed");
        assert!(forwarder.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn unknown_provider_is_rejected() {
        let forwarder = Arc::new(StubForwarder::new(vec![]));
        let sink = Arc::new(CollectingAuditSink::new());
        let svc = service_with(forwarder.clone(), sink.clone());

        let err = svc
            .forward(
                &ai_daemon_caller(),
                ForwardRequest {
                    provider_name: "imaginary".to_string(),
                    body_json: "{}".to_string(),
                    audit_token: "tok-4".to_string(),
                },
            )
            .await
            .expect_err("reject");
        assert_eq!(err.code(), "unknown-provider");
        assert!(forwarder.calls.lock().await.is_empty());
    }

    #[tokio::test]
    async fn upstream_transport_error_audits_upstream_error() {
        let forwarder = Arc::new(StubForwarder::new(vec![Err(ForwardError::Transport(
            "connection refused".to_string(),
        ))]));
        let sink = Arc::new(CollectingAuditSink::new());
        let svc = service_with(forwarder.clone(), sink.clone());

        let err = svc
            .forward(
                &ai_daemon_caller(),
                ForwardRequest {
                    provider_name: "ollama-default".to_string(),
                    body_json: "{}".to_string(),
                    audit_token: "tok-5".to_string(),
                },
            )
            .await
            .expect_err("fail");
        assert_eq!(err.code(), "upstream-error");
        let records = sink.snapshot().await;
        // Pre-forward gate entry, then the best-effort error entry.
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].structural.outcome, "forwarding");
        assert_eq!(records[1].structural.outcome, "upstream-error");
        assert_eq!(records[1].structural.subject, "127.0.0.1");
    }

    #[tokio::test]
    async fn allowed_providers_lists_catalog_entries() {
        let forwarder = Arc::new(StubForwarder::new(vec![]));
        let sink = Arc::new(CollectingAuditSink::new());
        let svc = service_with(forwarder, sink);
        let providers = svc.allowed_providers();
        // The default catalog ships only the local provider.
        assert_eq!(providers, vec!["ollama-default".to_string()]);
    }

    /// Forwarder that parks every call on a notify until released,
    /// so a test can hold a forward in flight deterministically.
    struct GatedForwarder {
        gate: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl Forwarder for GatedForwarder {
        async fn post(
            &self,
            _endpoint_url: &str,
            _body_json: &str,
            _auth: crate::forward::AuthHeader<'_>,
        ) -> Result<ForwardResult, ForwardError> {
            self.gate.notified().await;
            Ok(ForwardResult {
                status: 200,
                body: "{}".to_string(),
            })
        }

        async fn get(
            &self,
            _endpoint_url: &str,
            _auth: crate::forward::AuthHeader<'_>,
        ) -> Result<ForwardResult, ForwardError> {
            self.gate.notified().await;
            Ok(ForwardResult {
                status: 200,
                body: "{}".to_string(),
            })
        }
    }

    #[tokio::test]
    async fn concurrent_forwards_past_the_ceiling_are_refused() {
        // A ceiling of one: the first forward parks in the gated
        // forwarder holding the only slot; a second concurrent
        // forward must be refused rather than reaching upstream.
        let gate = Arc::new(tokio::sync::Notify::new());
        let svc = Arc::new(ProxyService::with_max_inflight(
            Allowlist::default_arlen(),
            ProviderCatalog::default_arlen(),
            CallerAllowlist::default_arlen(),
            Arc::new(GatedForwarder { gate: gate.clone() }) as Arc<dyn Forwarder>,
            Arc::new(CollectingAuditSink::new()) as Arc<dyn AuditSink>,
            1,
        ));

        let svc_a = svc.clone();
        let first = tokio::spawn(async move {
            svc_a
                .forward(
                    &ai_daemon_caller(),
                    ForwardRequest {
                        provider_name: "ollama-default".to_string(),
                        body_json: "{}".to_string(),
                        audit_token: "tok-a".to_string(),
                    },
                )
                .await
        });

        // Let the first forward enter the gated post() and hold the
        // slot before the second one runs.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let err = svc
            .forward(
                &ai_daemon_caller(),
                ForwardRequest {
                    provider_name: "ollama-default".to_string(),
                    body_json: "{}".to_string(),
                    audit_token: "tok-b".to_string(),
                },
            )
            .await
            .expect_err("second forward over ceiling");
        assert_eq!(err.code(), "proxy-at-capacity");

        // Release the first forward; once its slot frees, a new
        // forward is admitted again.
        gate.notify_one();
        first.await.unwrap().expect("first forward completes");
        gate.notify_one();
        svc.forward(
            &ai_daemon_caller(),
            ForwardRequest {
                provider_name: "ollama-default".to_string(),
                body_json: "{}".to_string(),
                audit_token: "tok-c".to_string(),
            },
        )
        .await
        .expect("forward admitted after slot freed");
    }

    #[tokio::test]
    async fn forward_is_refused_when_audit_is_unavailable() {
        // The audit ledger is down. The proxy must refuse the forward
        // before any request leaves the host (foundation §8.4.6),
        // even though the caller, provider, and allowlist all pass.
        let forwarder = Arc::new(StubForwarder::new(vec![Ok(ForwardResult {
            status: 200,
            body: "{}".to_string(),
        })]));
        let sink = Arc::new(CollectingAuditSink::failing());
        let svc = ProxyService::new(
            Allowlist::default_arlen(),
            ProviderCatalog::default_arlen(),
            CallerAllowlist::default_arlen(),
            forwarder.clone() as Arc<dyn Forwarder>,
            sink as Arc<dyn AuditSink>,
        );
        let err = svc
            .forward(
                &ai_daemon_caller(),
                ForwardRequest {
                    provider_name: "ollama-default".to_string(),
                    body_json: "{}".to_string(),
                    audit_token: "tok".to_string(),
                },
            )
            .await
            .expect_err("audit-unavailable must refuse the forward");
        assert_eq!(err.code(), "audit-unavailable");
        // The request never left the host: the forwarder was not called.
        assert!(forwarder.calls.lock().await.is_empty());
    }
}
