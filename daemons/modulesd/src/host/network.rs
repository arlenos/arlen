/// `arlen:host/network` import implementation.
///
/// Network access is the most-watched capability for Arlen modules.
/// Foundation §07 mandates that modules cannot reach hosts outside
/// their declared `network.allow` list and that denial returns a typed
/// error rather than a panic. This module is the choke point.
///
/// Hardening rules (per ds#77 plan):
///   * **HTTPS only.** `http://` URLs are rejected even if the host
///     is in the allowlist. Modules that genuinely need plaintext
///     would need a future explicit `network.allow_http` capability;
///     until then, fail closed.
///   * **Re-check on every redirect.** Reqwest's default redirect
///     policy follows up to 10 hops without revisiting the original
///     allowlist. We override it with a custom policy that consults
///     `CapabilityContext` per hop and stops at the first denial.
///   * **Body size cap (10 MB) and total timeout (30 s).** Defends
///     against malicious or accidentally-pathological servers that
///     would otherwise flood RAM or wedge the daemon.
///   * **User-Agent.** Set to `Arlen-modulesd/<version> <module-id>`
///     so server-side logs can attribute requests and rate-limiters
///     have something to grip on.
///   * **Per-module concurrency cap.** A `Semaphore` of 4 concurrent
///     fetches per module, surfaced via the manager so a runaway
///     module cannot saturate the host's outbound socket pool.

use std::sync::Arc;
use std::time::Duration;

use arlen_net_guard::{resolve_and_pin, GuardError};
use reqwest::redirect::{Attempt, Policy};
use reqwest::Method;

use crate::error::{DaemonError, Result};
use crate::host::CapabilityContext;

const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

/// Outcome of a `network::fetch` host call.
#[derive(Debug, Clone)]
pub struct FetchResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Decide whether a fetch is permitted, without performing it.
/// Used by the bindings layer (`manager::handle_host_call`) so it
/// can fail fast before opening a socket.
pub fn check_fetch(ctx: &CapabilityContext, url: &str) -> Result<()> {
    let scheme = url.split_once("://").map(|(s, _)| s).unwrap_or("");
    if scheme != "https" {
        return Err(DaemonError::CapabilityDenied {
            module_id: ctx.module_id.clone(),
            capability: format!("network.fetch (non-https scheme: {scheme})"),
        });
    }
    if !ctx.allow_network(url) {
        return Err(DaemonError::CapabilityDenied {
            module_id: ctx.module_id.clone(),
            capability: format!("network.fetch({url})"),
        });
    }
    Ok(())
}

/// Build a `reqwest::Client` configured for sandboxed module use:
/// per-hop capability re-check, total timeout, no cookie jar, no
/// connection re-use across modules (one client per call keeps the
/// allowlist scoped tightly to the calling module).
///
/// `pin_resolution` is set to a (host, addr) pair we have already
/// verified isn't a blocked destination; reqwest is forced to use
/// that exact socket address rather than re-querying DNS, which
/// closes the rebinding window between our pre-check and the
/// connect.
fn build_client(
    ctx: Arc<CapabilityContext>,
    module_version: &str,
    pin_resolution: Option<(String, std::net::SocketAddr)>,
) -> Result<reqwest::Client> {
    let policy = Policy::custom(move |attempt: Attempt| {
        // attempt.url() is the URL the redirect would land on.
        let url = attempt.url().to_string();
        let scheme = attempt.url().scheme();
        if scheme != "https" {
            return attempt.error("redirect to non-https blocked");
        }
        if !ctx.allow_network(&url) {
            return attempt.error("redirect to non-allowlisted host blocked");
        }
        // Cap the chain at 5 hops; reqwest defaults to 10 but we
        // deliberately keep ours stricter. Returning an error here
        // (not `stop`) is intentional: `stop()` would surface the
        // last 3xx response as a successful body, hiding the fact
        // that the resource was never reached.
        if attempt.previous().len() > 5 {
            return attempt.error("redirect chain longer than 5 hops");
        }
        // Note: this hook is sync, so we cannot run an async DNS
        // resolution here. The initial URL is SSRF-checked in
        // `perform()` and pinned via `Client::resolve(...)`, which
        // closes the rebinding window for the same hostname. A
        // redirect to a *different* allowlisted host would re-resolve
        // through reqwest's own resolver; a follow-up iteration of
        // this code can disable redirects entirely and walk the
        // chain manually so the IP-range check fires per hop.
        attempt.follow()
    });

    let mut builder = reqwest::Client::builder()
        .redirect(policy)
        .timeout(TIMEOUT)
        .user_agent(format!(
            "Arlen-modulesd/{} {}",
            env!("CARGO_PKG_VERSION"),
            module_version,
        ))
        .https_only(true);

    if let Some((host, addr)) = pin_resolution {
        builder = builder.resolve(&host, addr);
    }

    builder
        .build()
        .map_err(|e| DaemonError::Internal(format!("reqwest builder: {e}")))
}

/// Issue a real HTTP GET. Capability is re-checked here in addition
/// to whatever the caller did, so even if the policy layer is
/// bypassed at some future call site, the fetch still fails closed.
pub async fn fetch(
    ctx: Arc<CapabilityContext>,
    url: &str,
    headers: &[(String, String)],
) -> Result<FetchResponse> {
    check_fetch(&ctx, url)?;
    perform(ctx, Method::GET, url, headers, None).await
}

/// Issue a real HTTP POST with a request body.
pub async fn post(
    ctx: Arc<CapabilityContext>,
    url: &str,
    body: Vec<u8>,
    headers: &[(String, String)],
) -> Result<FetchResponse> {
    check_fetch(&ctx, url)?;
    perform(ctx, Method::POST, url, headers, Some(body)).await
}

async fn perform(
    ctx: Arc<CapabilityContext>,
    method: Method,
    url: &str,
    headers: &[(String, String)],
    body: Option<Vec<u8>>,
) -> Result<FetchResponse> {
    let module_version = "1.0";

    // SSRF guard. Resolve the URL host ourselves, fail closed on any
    // private/loopback/link-local destination, and pin reqwest to
    // the verified socket address so DNS rebinding cannot swap the
    // target between our check and the connect.
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| DaemonError::Internal(format!("parse url: {e}")))?;
    let mut pin_resolution: Option<(String, std::net::SocketAddr)> = None;
    if let Some(host) = parsed.host_str() {
        let port = parsed.port_or_known_default().unwrap_or(443);
        match resolve_and_pin(host, port).await {
            Ok(addr) => pin_resolution = Some((host.to_string(), addr)),
            Err(blocked @ GuardError::Blocked { .. }) => {
                return Err(DaemonError::CapabilityDenied {
                    module_id: ctx.module_id.clone(),
                    capability: format!("network.fetch {blocked}"),
                });
            }
            Err(other) => return Err(DaemonError::Internal(format!("network.fetch {other}"))),
        }
    }

    let client = build_client(Arc::clone(&ctx), module_version, pin_resolution)?;

    let mut req = client.request(method, url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    if let Some(body) = body {
        req = req.body(body);
    }

    let resp = req.send().await.map_err(|e| {
        if e.is_timeout() {
            DaemonError::Internal(format!("network.fetch timeout: {url}"))
        } else if e.is_redirect() {
            DaemonError::CapabilityDenied {
                module_id: ctx.module_id.clone(),
                capability: format!("network.fetch redirect blocked: {e}"),
            }
        } else {
            DaemonError::Internal(format!("network.fetch: {e}"))
        }
    })?;

    let status = resp.status().as_u16();

    // Stream the body with a hard cap. We intentionally do not use
    // `bytes()` because that allocates the whole body even if it's
    // multiple GB; this loop bails as soon as the cap is hit.
    let mut body_buf: Vec<u8> = Vec::new();
    use futures_util::StreamExt;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            DaemonError::Internal(format!("network.fetch body stream: {e}"))
        })?;
        if body_buf.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(DaemonError::Internal(format!(
                "network.fetch body exceeds {MAX_BODY_BYTES} bytes"
            )));
        }
        body_buf.extend_from_slice(&chunk);
    }

    Ok(FetchResponse {
        status,
        body: body_buf,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_modules::{ModuleCapabilities, NetworkCapability};

    fn ctx_for(domains: &[&str]) -> Arc<CapabilityContext> {
        let mut caps = ModuleCapabilities::default();
        caps.network = Some(NetworkCapability {
            allowed_domains: domains.iter().map(|s| s.to_string()).collect(),
        });
        Arc::new(CapabilityContext::new("com.example.test", caps))
    }

    #[test]
    fn check_denies_non_https_even_when_host_is_allowlisted() {
        let ctx = ctx_for(&["api.example.com"]);
        let err = check_fetch(&ctx, "http://api.example.com/").unwrap_err();
        assert!(matches!(err, DaemonError::CapabilityDenied { .. }));
    }

    #[test]
    fn check_denies_when_host_not_allowlisted() {
        let ctx = ctx_for(&["api.example.com"]);
        let err = check_fetch(&ctx, "https://api.evil.com/").unwrap_err();
        assert!(matches!(err, DaemonError::CapabilityDenied { .. }));
    }

    #[test]
    fn check_allows_https_to_allowlisted_host() {
        let ctx = ctx_for(&["api.example.com"]);
        assert!(check_fetch(&ctx, "https://api.example.com/v1").is_ok());
    }

    #[test]
    fn check_denies_when_no_network_capability() {
        let ctx = Arc::new(CapabilityContext::empty("x"));
        assert!(check_fetch(&ctx, "https://anything.com").is_err());
    }
}
