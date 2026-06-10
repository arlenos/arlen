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

/// A handle whose `Drop` tears down an installed egress restriction. The real
/// enforcer stores the proxy's cancellation token and the network-namespace
/// handle here and tears them down on drop; the stand-in's guard restricts
/// nothing. The launcher holds it for the launch's lifetime.
#[derive(Debug)]
#[must_use = "the egress restriction is torn down when the guard drops; hold it for the whole launch"]
pub struct EgressGuard {
    // The real enforcer will hold the proxy CancellationToken + netns handle
    // here; the stand-in holds nothing.
    _private: (),
}

impl EgressGuard {
    /// A guard that restricts nothing: the empty-allowlist case, and the
    /// stand-in's only success.
    fn noop() -> Self {
        Self { _private: () }
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
}

impl fmt::Display for EgressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EgressError::NoEnforcer => {
                write!(f, "the egress filter is not yet wired; refusing a filtered-host launch")
            }
        }
    }
}

impl Error for EgressError {}

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
}
