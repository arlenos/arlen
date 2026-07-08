//! `arlen-ai-proxy` daemon entry point.
//!
//! Wires the policy core (`ProxyService`) into a real outbound layer
//! (`ReqwestForwarder`) and exposes `org.arlen.AIProxy1` on the
//! session D-Bus. Foundation §8.4.6 forbids any AI traffic from
//! leaving the host through any other path.

use std::sync::Arc;

use arlen_ai_proxy::allowlist::Allowlist;
use arlen_ai_proxy::audit::{AuditSink, LedgerAuditSink};
use arlen_ai_proxy::catalog::ProviderCatalog;
use arlen_ai_proxy::forward::ReqwestForwarder;
use arlen_ai_proxy::peer_auth::{self, PeerAuthError, PeerAuthMap};
use arlen_ai_proxy::service::{
    CallerAllowlist, ForwardRequest, ProxyError, ProxyService,
};
use zbus::Connection;

/// The `ai-routing.toml` path under the user config dir (`$XDG_CONFIG_HOME/arlen` or
/// `~/.config/arlen`). Absent, the proxy runs on its built-in catalog.
fn catalog_config_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from(".config"));
    base.join("arlen").join("ai-routing.toml")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let forwarder = Arc::new(ReqwestForwarder::new()?);
    let audit_sink: Arc<dyn AuditSink> = Arc::new(LedgerAuditSink::at_default_socket());
    let catalog_path = catalog_config_path();
    tracing::info!(path = %catalog_path.display(), "loading the provider catalog");
    let service = Arc::new(ProxyService::new(
        Allowlist::default_arlen(),
        ProviderCatalog::load_or_default(&catalog_path)?,
        CallerAllowlist::default_arlen(),
        forwarder,
        audit_sink,
    ));

    let peer_map = Arc::new(PeerAuthMap::default_arlen());
    let dbus = ProxyInterface {
        service: service.clone(),
        peer_map: peer_map.clone(),
    };

    let _connection = zbus::connection::Builder::session()?
        .name("org.arlen.AIProxy1")?
        .serve_at("/org/arlen/AIProxy1", dbus)?
        .build()
        .await?;

    tracing::info!("arlen-ai-proxy: serving org.arlen.AIProxy1");

    tokio::signal::ctrl_c().await?;
    tracing::info!("arlen-ai-proxy: shutting down");
    Ok(())
}

/// D-Bus surface (`org.arlen.AIProxy1`).
struct ProxyInterface {
    service: Arc<ProxyService>,
    peer_map: Arc<PeerAuthMap>,
}

#[zbus::interface(name = "org.arlen.AIProxy1")]
impl ProxyInterface {
    /// Forward a completion request through the named provider's
    /// catalogued endpoint. The proxy uses its own provider catalog
    /// for endpoint lookup; the caller never supplies a URL.
    #[zbus(name = "forward_completion")]
    async fn forward_completion(
        &self,
        provider_name: &str,
        body_json: &str,
        audit_token: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<String> {
        let sender = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::AccessDenied("no sender".to_string()))?
            .to_string();
        let caller = peer_auth::resolve(&sender, connection, &self.peer_map)
            .await
            .map_err(map_peer_auth_error)?;
        let req = ForwardRequest {
            provider_name: provider_name.to_string(),
            body_json: body_json.to_string(),
            audit_token: audit_token.to_string(),
        };
        match self.service.forward(&caller, req).await {
            Ok(outcome) => Ok(serde_json::json!({
                "upstream_status": outcome.upstream_status,
                "body": outcome.body,
            })
            .to_string()),
            Err(err) => Err(map_error(err)),
        }
    }

    /// Return the catalogued provider names. Lists what callers may
    /// pass to `forward_completion`; it does *not* expose the
    /// underlying endpoint URLs.
    #[zbus(name = "list_allowed_providers")]
    async fn list_allowed_providers(&self) -> Vec<String> {
        self.service.allowed_providers()
    }

    /// Return the manager-surface provider catalog as a JSON array of
    /// `{ id, name, kind, configured, builtin }` (camelCase) - display metadata
    /// only, never the endpoint URL or any credential. Backs the daemon's
    /// `ai_providers_list` for the Settings AI-providers manager. Empty array on
    /// a serialization failure (the manager then shows no providers, fail-safe).
    #[zbus(name = "list_providers")]
    async fn list_providers(&self) -> String {
        serde_json::to_string(&self.service.provider_views())
            .unwrap_or_else(|_| "[]".to_string())
    }

    /// Test a catalogued provider's connectivity (the manager's "test"
    /// button + the capability-grant `validate_provider`). GETs the
    /// provider's catalogued model-list endpoint and returns the verdict
    /// as JSON `{ ok, httpStatus?, network? }`. The endpoint comes from
    /// the trusted catalog, never the caller, so there is no caller-URL
    /// egress-consent concern. The same caller allowlist + audit-before-
    /// egress gate as `forward_completion` apply; a policy refusal is a
    /// D-Bus error.
    #[zbus(name = "test_provider")]
    async fn test_provider(
        &self,
        provider_name: &str,
        audit_token: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<String> {
        let sender = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::AccessDenied("no sender".to_string()))?
            .to_string();
        let caller = peer_auth::resolve(&sender, connection, &self.peer_map)
            .await
            .map_err(map_peer_auth_error)?;
        match self
            .service
            .test_provider(&caller, provider_name, audit_token)
            .await
        {
            Ok(outcome) => serde_json::to_string(&outcome)
                .map_err(|e| zbus::fdo::Error::Failed(format!("serialize test outcome: {e}"))),
            Err(err) => Err(map_error(err)),
        }
    }
}

fn map_peer_auth_error(err: PeerAuthError) -> zbus::fdo::Error {
    match err {
        PeerAuthError::NoSender => zbus::fdo::Error::AccessDenied("no sender".to_string()),
        PeerAuthError::PidLookup(detail) => zbus::fdo::Error::AccessDenied(
            format!("peer PID lookup failed: {detail}"),
        ),
        PeerAuthError::ExeLookup { pid, error } => zbus::fdo::Error::AccessDenied(format!(
            "peer exe lookup failed for pid {pid}: {error}"
        )),
        PeerAuthError::ExeNotAllowed { path } => zbus::fdo::Error::AccessDenied(format!(
            "caller executable not allowed: {path}"
        )),
        PeerAuthError::NameOwnershipMismatch {
            name,
            sender,
            owner,
        } => zbus::fdo::Error::AccessDenied(format!(
            "caller {sender} does not own {name} (owner: {owner})"
        )),
    }
}

fn map_error(err: ProxyError) -> zbus::fdo::Error {
    let detail = err.to_string();
    match err.code() {
        "caller-not-allowed" => zbus::fdo::Error::AccessDenied(detail),
        "unknown-provider" => zbus::fdo::Error::InvalidArgs(detail),
        "invalid-url" | "missing-host" => zbus::fdo::Error::Failed(detail),
        "disallowed-scheme" | "host-not-allowed" => zbus::fdo::Error::AccessDenied(detail),
        "proxy-at-capacity" => zbus::fdo::Error::LimitsExceeded(detail),
        "upstream-error" => zbus::fdo::Error::Failed(detail),
        _ => zbus::fdo::Error::Failed(detail),
    }
}
