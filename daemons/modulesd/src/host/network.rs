/// `lunaris:host/network` import implementation.
///
/// Network access is the most-watched capability for Lunaris modules.
/// Foundation §07 mandates that modules cannot reach hosts outside
/// their declared `network.allow` list and that denial returns a typed
/// error rather than a panic. This module is the choke point.
///
/// The actual HTTP client is intentionally minimalist (raw tokio +
/// rustls would pull in a heavy tree). For the first ship modulesd
/// proxies fetches through the existing system facilities; the
/// implementation here is the policy layer, not the wire layer.

use crate::error::{DaemonError, Result};
use crate::host::CapabilityContext;

/// Outcome of a `network::fetch` host call.
#[derive(Debug, Clone)]
pub struct FetchResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Decide whether a fetch is permitted, without performing it. Used by
/// host-import bindings that wrap the actual HTTP call: they call this
/// first and bail with `CapabilityDenied` before opening a socket.
pub fn check_fetch(ctx: &CapabilityContext, url: &str) -> Result<()> {
    if !ctx.allow_network(url) {
        return Err(DaemonError::CapabilityDenied {
            module_id: ctx.module_id.clone(),
            capability: format!("network.fetch({url})"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lunaris_modules::{ModuleCapabilities, NetworkCapability};

    #[test]
    fn check_denies_when_url_not_in_allowlist() {
        let mut caps = ModuleCapabilities::default();
        caps.network = Some(NetworkCapability {
            allowed_domains: vec!["api.example.com".into()],
        });
        let ctx = CapabilityContext::new("x", caps);
        let err = check_fetch(&ctx, "https://api.evil.com/x").unwrap_err();
        assert!(matches!(err, DaemonError::CapabilityDenied { .. }));
    }

    #[test]
    fn check_allows_matching_host() {
        let mut caps = ModuleCapabilities::default();
        caps.network = Some(NetworkCapability {
            allowed_domains: vec!["api.example.com".into()],
        });
        let ctx = CapabilityContext::new("x", caps);
        assert!(check_fetch(&ctx, "https://api.example.com/v1").is_ok());
    }

    #[test]
    fn check_denies_when_no_network_capability() {
        let ctx = CapabilityContext::empty("x");
        assert!(check_fetch(&ctx, "https://anything.com").is_err());
    }
}
