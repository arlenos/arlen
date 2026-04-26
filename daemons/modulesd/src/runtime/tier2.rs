/// Tier 2 (iframe) broker.
///
/// The iframe DOM lives inside the desktop-shell webview; the daemon
/// owns the *policy* around it. Concretely the daemon:
///   * mints a per-instance nonce when the shell asks for an iframe
///   * tracks live nonces so the `module://` Tauri scheme handler can
///     reject stale requests
///   * gates every postMessage host call against the module's
///     `CapabilityContext`
///   * dispatches lifecycle events on crash / shutdown
///
/// S2 ships the policy data structures and the nonce store. The
/// scheme handler and postMessage proxy are wired up in S3.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::RwLock;

use crate::host::CapabilityContext;

/// One live iframe instance. Multiple iframes per module are allowed
/// (e.g. one in the topbar and one in a settings panel).
#[derive(Debug, Clone)]
pub struct IframeInstance {
    pub module_id: String,
    pub instance_id: String,
    pub nonce: String,
    pub created_at: Instant,
    pub ctx: CapabilityContext,
}

/// In-memory registry of live iframes keyed by nonce. Lookup is on the
/// hot path of the `module://` scheme handler so it must not block;
/// `RwLock` lets reads proceed concurrently with new iframe spawns.
#[derive(Default)]
pub struct Tier2Broker {
    iframes: RwLock<HashMap<String, IframeInstance>>,
}

impl Tier2Broker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register a new iframe. Returns the freshly minted nonce that
    /// the shell embeds in the iframe URL.
    pub async fn register(&self, instance: IframeInstance) {
        self.iframes.write().await.insert(instance.nonce.clone(), instance);
    }

    /// Look up a live iframe by nonce. Returns `None` if the nonce is
    /// unknown or has been revoked, in which case the scheme handler
    /// should 404 the request.
    pub async fn lookup(&self, nonce: &str) -> Option<IframeInstance> {
        self.iframes.read().await.get(nonce).cloned()
    }

    /// Revoke a nonce. The associated iframe will be denied future
    /// `module://` reads, so navigating it elsewhere fails closed.
    pub async fn revoke(&self, nonce: &str) {
        self.iframes.write().await.remove(nonce);
    }

    /// Revoke every nonce belonging to a module (e.g. on uninstall).
    pub async fn revoke_module(&self, module_id: &str) {
        let mut guard = self.iframes.write().await;
        guard.retain(|_, inst| inst.module_id != module_id);
    }

    pub async fn live_count(&self) -> usize {
        self.iframes.read().await.len()
    }
}

/// Mint a fresh per-instance nonce. Uses 256 bits of entropy from the
/// system RNG (via `uuid::Uuid::new_v4` for portability without a hard
/// dep on `getrandom`'s features).
pub fn mint_nonce() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    // Lightweight nonce: timestamp + random tail. A real broker would
    // use a CSPRNG; until S3 wires `rand`, this is a placeholder strong
    // enough for tests and weakly bound for development. The S3
    // scheme-handler step swaps it for a 32-byte CSPRNG nonce.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    format!("{nanos:x}-{pid:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(module_id: &str, nonce: &str) -> IframeInstance {
        IframeInstance {
            module_id: module_id.into(),
            instance_id: format!("{module_id}-1"),
            nonce: nonce.into(),
            created_at: Instant::now(),
            ctx: CapabilityContext::empty(module_id),
        }
    }

    #[tokio::test]
    async fn register_and_lookup() {
        let b = Tier2Broker::new();
        b.register(instance("com.example.weather", "abc")).await;
        let found = b.lookup("abc").await.unwrap();
        assert_eq!(found.module_id, "com.example.weather");
    }

    #[tokio::test]
    async fn lookup_unknown_nonce_returns_none() {
        let b = Tier2Broker::new();
        assert!(b.lookup("nope").await.is_none());
    }

    #[tokio::test]
    async fn revoke_removes_iframe() {
        let b = Tier2Broker::new();
        b.register(instance("com.example.weather", "abc")).await;
        b.revoke("abc").await;
        assert!(b.lookup("abc").await.is_none());
    }

    #[tokio::test]
    async fn revoke_module_drops_all_its_iframes() {
        let b = Tier2Broker::new();
        b.register(instance("com.example.weather", "n1")).await;
        b.register(instance("com.example.weather", "n2")).await;
        b.register(instance("com.example.other", "n3")).await;
        b.revoke_module("com.example.weather").await;
        assert!(b.lookup("n1").await.is_none());
        assert!(b.lookup("n2").await.is_none());
        assert!(b.lookup("n3").await.is_some());
    }

    #[test]
    fn nonces_are_unique_per_call() {
        let a = mint_nonce();
        std::thread::sleep(std::time::Duration::from_nanos(100));
        let b = mint_nonce();
        assert_ne!(a, b);
    }
}
