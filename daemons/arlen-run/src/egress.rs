//! The egress-enforcer seam.
//!
//! A profile that declared `NetworkPolicy::FilteredHosts` must have its egress
//! held to that host set. The real enforcement (Strand 2's `EgressProxy` in a
//! launcher-owned network namespace whose only route is the forwarding proxy)
//! is on-kernel machinery; this module is the seam the launcher calls at one
//! site so the two strands compose without a hard ordering dependency.
//!
//! Until the real enforcer is wired, the launcher uses [`DenyUnlessEmpty`]: a
//! non-empty host set cannot be honoured, so the launch is **refused** rather
//! than run with unfiltered network. This is the house rule made concrete: no
//! half-open default. `NetworkPolicy::None` (no network) and `Unrestricted` (no
//! filter by design) never reach the enforcer at all; only `FilteredHosts` does.

use std::error::Error;
use std::fmt;

use arlen_net_guard::{EgressAllowlist, EgressProxy};
use tokio_util::sync::CancellationToken;

/// A handle whose `Drop` tears down an installed egress restriction. The real
/// enforcer holds the proxy's runtime + cancellation token here; the stand-in /
/// empty case holds nothing. The launcher holds it for the launch's lifetime and
/// reads [`Self::proxy_port`] to point the confined app's `*_proxy` env at it.
#[must_use = "the egress restriction is torn down when the guard drops; hold it for the whole launch"]
pub struct EgressGuard {
    // The real enforcer holds the running proxy here; the stand-in / empty case
    // holds `None`.
    proxy: Option<ProxyHandle>,
}

/// The live forwarding proxy behind a real guard: its own runtime (a worker
/// thread drives `serve` while the launcher blocks on the app), the token that
/// stops it, and the host-loopback port the app dials through the netns.
// `runtime` is held only for its Drop (it shuts the serve loop's threads down);
// the enforcer that constructs this is wired into `main` in the next slice.
#[allow(dead_code)]
struct ProxyHandle {
    // Dropped last, after `cancel`, so the (now-stopping) serve loop and its
    // runtime shut down without hanging.
    runtime: tokio::runtime::Runtime,
    cancel: CancellationToken,
    port: u16,
}

impl EgressGuard {
    /// A guard that restricts nothing: the empty-allowlist case, and the
    /// stand-in's only success.
    fn noop() -> Self {
        Self { proxy: None }
    }

    /// The host-loopback port the forwarding proxy bound, or `None` for a
    /// restriction-less guard. The launcher points `http_proxy`/`https_proxy` at
    /// the mapped-loopback gateway on this port.
    #[allow(dead_code)] // read by `main` when the enforcer is wired (next slice)
    pub fn proxy_port(&self) -> Option<u16> {
        self.proxy.as_ref().map(|p| p.port)
    }
}

impl Drop for EgressGuard {
    fn drop(&mut self) {
        // Stop the serve loop first; the runtime then shuts down promptly instead
        // of blocking on an accept loop that would never return.
        if let Some(p) = &self.proxy {
            p.cancel.cancel();
        }
    }
}

impl fmt::Debug for EgressGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EgressGuard").field("proxy_port", &self.proxy_port()).finish()
    }
}

/// A failure to install an egress restriction. The launcher maps it to the
/// `EGRESS` exit code and refuses the launch (fail-closed: a host-restricted app
/// must never run with unfiltered network).
#[derive(Debug, PartialEq, Eq)]
pub enum EgressError {
    /// No real enforcer is wired, so a non-empty host allowlist cannot be
    /// enforced. Refuse rather than run unfiltered.
    NoEnforcer,
    /// The declared host set is not a valid allowlist (bad `host:port` syntax).
    Allowlist(String),
    /// The proxy runtime or its listener could not be set up. Refuse rather than
    /// run with a half-installed filter.
    Setup(String),
}

impl fmt::Display for EgressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EgressError::NoEnforcer => {
                write!(f, "the egress filter is not yet wired; refusing a filtered-host launch")
            }
            EgressError::Allowlist(e) => write!(f, "invalid egress allowlist: {e}"),
            EgressError::Setup(e) => write!(f, "could not install the egress proxy: {e}"),
        }
    }
}

impl Error for EgressError {}

/// The real enforcer: binds Strand 2's forwarding [`EgressProxy`] on host
/// loopback with the declared allowlist and serves it on a dedicated runtime for
/// the launch's lifetime. The launcher runs the app in a route-absent netns
/// (see [`crate::netns`]) whose only reachable peer is this proxy, so the
/// allowlist is the app's whole egress. An empty host set restricts nothing (a
/// noop guard); any parse/bind failure is a fail-closed [`EgressError`].
#[allow(dead_code)] // swapped in for `DenyUnlessEmpty` at the `main` site next slice
pub struct ProxyEgressEnforcer;

impl EgressEnforcer for ProxyEgressEnforcer {
    fn install(&self, hosts: &[String]) -> Result<EgressGuard, EgressError> {
        if hosts.is_empty() {
            return Ok(EgressGuard::noop());
        }
        let allowlist =
            EgressAllowlist::parse(hosts).map_err(|e| EgressError::Allowlist(e.to_string()))?;
        // A dedicated worker thread drives the proxy while the launcher blocks on
        // the app; a current-thread runtime would never be polled (nobody drives
        // it once the launcher waits), so serve must run on its own thread.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| EgressError::Setup(e.to_string()))?;
        // Bind on host loopback (a dynamic port); the netns maps its gateway to
        // this loopback, so the app reaches it there.
        let bind = crate::netns::proxy_bind_addr(0);
        let proxy = runtime
            .block_on(EgressProxy::bind(bind, allowlist))
            .map_err(|e| EgressError::Setup(e.to_string()))?;
        let port = proxy
            .listen_addr()
            .map_err(|e| EgressError::Setup(e.to_string()))?
            .port();
        let cancel = CancellationToken::new();
        let serve_cancel = cancel.clone();
        runtime.spawn(async move { proxy.serve(serve_cancel).await });
        Ok(EgressGuard {
            proxy: Some(ProxyHandle { runtime, cancel, port }),
        })
    }
}

/// Installs and tears down a per-launch egress allowlist. The real
/// implementation (Strand 2's `EgressProxy` in a launcher-owned netns) slots in
/// at the single construction site in `main`; this trait keeps the launcher
/// decoupled from that on-kernel machinery.
pub trait EgressEnforcer {
    /// Restrict the launch's egress to `hosts` (each `host:port`). Returns a
    /// guard whose `Drop` removes the restriction.
    fn install(&self, hosts: &[String]) -> Result<EgressGuard, EgressError>;
}

/// The fail-closed stand-in used until Strand 2's real enforcer is wired. A
/// non-empty host set cannot be honoured, so the launch is refused; an empty set
/// (nothing to restrict) passes.
pub struct DenyUnlessEmpty;

impl EgressEnforcer for DenyUnlessEmpty {
    fn install(&self, hosts: &[String]) -> Result<EgressGuard, EgressError> {
        if hosts.is_empty() {
            Ok(EgressGuard::noop())
        } else {
            Err(EgressError::NoEnforcer)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stand_in_refuses_a_non_empty_host_set() {
        let r = DenyUnlessEmpty.install(&["api.example.org:443".to_string()]);
        assert_eq!(r.unwrap_err(), EgressError::NoEnforcer);
    }

    #[test]
    fn stand_in_passes_an_empty_host_set() {
        assert!(DenyUnlessEmpty.install(&[]).is_ok());
    }

    #[test]
    fn proxy_enforcer_binds_reports_a_port_and_tears_down() {
        // A real guard binds the forwarding proxy and exposes its port; dropping
        // it must return promptly (cancel stops serve, the runtime shuts down) -
        // if teardown hung, this test would never finish.
        let guard = ProxyEgressEnforcer
            .install(&["example.org:443".to_string()])
            .expect("bind the proxy for a valid allowlist");
        let port = guard.proxy_port().expect("a real guard exposes its proxy port");
        assert_ne!(port, 0, "the proxy bound a real dynamic port");
        drop(guard);
    }

    #[test]
    fn proxy_enforcer_empty_hosts_restricts_nothing() {
        let guard = ProxyEgressEnforcer.install(&[]).unwrap();
        assert!(guard.proxy_port().is_none(), "an empty host set is a noop guard");
    }

    #[test]
    fn proxy_enforcer_refuses_a_malformed_allowlist_fail_closed() {
        match ProxyEgressEnforcer.install(&["noport".to_string()]) {
            Err(EgressError::Allowlist(_)) => {}
            other => panic!("expected a fail-closed Allowlist error, got {other:?}"),
        }
    }
}
