//! Trusted provider catalog.
//!
//! Foundation §8.4.6 requires that the proxy treat the *endpoint URL*
//! as proxy-owned configuration, never as caller input. If the
//! caller could supply an arbitrary URL and the proxy enforced only a
//! hostname allowlist, the proxy would become a POST gadget for any
//! port/path on an allowed host — Anthropic's `/v1/messages` could
//! turn into `/v1/anything-the-attacker-wants`.
//!
//! The catalog maps a provider *name* (the same key used in
//! `ai-routing.toml`) onto the exact `(endpoint_url, backend)` the
//! proxy will reach. Callers identify their target by name; the URL
//! comes from this catalog.

use std::collections::HashMap;

use serde::Deserialize;

/// The wire protocol the proxy shapes a request/response for. The OpenAI
/// chat-completions shape is the common case (~12 of 15 providers are pure
/// base-URL + Bearer swaps); Anthropic and Gemini have native shapes the proxy
/// transcodes. Promoted from the old free-string `backend` so dispatch keys on a
/// closed set (`ai-providers-plan.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WireFormat {
    /// OpenAI `/chat/completions` shape (the default; most providers, incl. the
    /// local Ollama OpenAI-compatible endpoint).
    #[default]
    Openai,
    /// Anthropic `/v1/messages` native shape (transcoded by the proxy).
    Anthropic,
    /// Google Gemini native shape (served via the OpenAI-compat shim for now).
    Gemini,
}

/// How the proxy authenticates to the backend at egress. The credential is
/// NEVER held here - it is injected from the Connections broker via
/// `credential_ref` (CONN-R3, the first credential-injecting path); this only
/// records WHICH header/scheme carries the injected key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthScheme {
    /// No auth - a local provider (Ollama, llama.cpp). The default.
    #[default]
    None,
    /// `Authorization: Bearer <key>` (OpenAI + most OpenAI-compat providers).
    Bearer,
    /// `x-api-key: <key>` (Anthropic).
    XApiKey,
    /// `api-key: <key>` (Azure OpenAI).
    AzureApiKey,
    /// `x-goog-api-key: <key>` (Google Gemini native).
    XGoogApiKey,
}

/// Catalogued provider entry. The proxy treats every field as proxy-owned
/// configuration, never caller input (the POST-gadget defense above). The cloud
/// fields carry the multi-provider build; all the new ones are `#[serde(default)]`
/// so an existing `{endpoint_url, backend}` config still parses.
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogEntry {
    /// Full upstream endpoint URL (scheme + host + path). The proxy
    /// will POST `body_json` to this URL verbatim.
    pub endpoint_url: String,
    /// Backend identifier (`ollama`, `anthropic`, `openai`), retained for audit
    /// log lines. Dispatch now keys on `wire_format`; this stays for logging.
    pub backend: String,
    /// The wire protocol the proxy shapes for. Defaults to OpenAI
    /// chat-completions (the common case).
    #[serde(default)]
    pub wire_format: WireFormat,
    /// The egress auth scheme - which header carries the broker-injected key.
    /// Defaults to `none` (a local provider needs no key).
    #[serde(default)]
    pub auth_scheme: AuthScheme,
    /// URL template for a backend that needs path/query templating (Azure's
    /// `api-version` + deployment). `None` = use `endpoint_url` verbatim.
    #[serde(default)]
    pub url_template: Option<String>,
    /// Opaque handle into the Connections broker for this provider's credential
    /// (CONN-R3) - NEVER the key itself. `None` for a no-auth local provider.
    #[serde(default)]
    pub credential_ref: Option<String>,
    /// The provider's model-list endpoint (`GET /models`-class), for
    /// `validate_provider` and catalog/model refresh. `None` if not known.
    #[serde(default)]
    pub models_endpoint: Option<String>,
    /// Human-facing provider name for the picker. `None` falls back to the key.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Logo asset id for the picker. `None` = no logo.
    #[serde(default)]
    pub logo_id: Option<String>,
    /// A built-in Arlen preset (`true`) vs a user-added custom provider
    /// (`false`, the default for anything in a user config).
    #[serde(default)]
    pub builtin: bool,
}

/// Trusted provider catalog.
#[derive(Debug, Clone, Default)]
pub struct ProviderCatalog {
    entries: HashMap<String, CatalogEntry>,
}

impl ProviderCatalog {
    /// Build a catalog from an explicit map.
    pub fn new(entries: HashMap<String, CatalogEntry>) -> Self {
        Self { entries }
    }

    /// The default Arlen catalog.
    ///
    /// Phase 9-α ships only the local Ollama provider. The cloud
    /// providers (OpenAI, Anthropic) are deliberately absent: the
    /// proxy does not yet attach API-key authentication or
    /// backend-specific request shaping, so a cloud route would fail
    /// with a provider-side 401/400 rather than work. They are added
    /// in Phase 9-β/γ together with keyring-backed credentials. A
    /// half-working cloud entry would violate the "no stubs, no
    /// for-now" project rule, so it stays out until it functions.
    pub fn default_arlen() -> Self {
        let mut entries = HashMap::new();
        entries.insert(
            "ollama-default".to_string(),
            CatalogEntry {
                // 127.0.0.1, not localhost: a local llama-server/ollama binds IPv4
                // loopback, but `localhost` resolves to ::1 (IPv6) first, so the
                // forward races onto a port nothing listens on and fails. The
                // explicit IPv4 literal is unambiguous and matches the bind.
                endpoint_url: "http://127.0.0.1:11434/v1/chat/completions".to_string(),
                backend: "ollama".to_string(),
                wire_format: WireFormat::Openai,
                auth_scheme: AuthScheme::None,
                url_template: None,
                credential_ref: None,
                models_endpoint: Some("http://127.0.0.1:11434/v1/models".to_string()),
                display_name: Some("Ollama (local)".to_string()),
                logo_id: None,
                builtin: true,
            },
        );
        Self::new(entries)
    }

    /// Look up a provider by name.
    pub fn get(&self, provider_name: &str) -> Option<&CatalogEntry> {
        self.entries.get(provider_name)
    }

    /// Iterator over the registered provider names. Used by
    /// `list_allowed_endpoints` on the D-Bus interface.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// The manager-surface view of every catalogued provider, sorted by id for a
    /// stable order. Carries only display metadata (no endpoint URL, no
    /// credential), so it is safe to hand to the Settings AI-providers manager
    /// (via the daemon's `ai_providers_list`).
    pub fn views(&self) -> Vec<ProviderView> {
        let mut views: Vec<ProviderView> = self
            .entries
            .iter()
            .map(|(id, entry)| ProviderView {
                id: id.clone(),
                name: entry.display_name.clone().unwrap_or_else(|| id.clone()),
                kind: entry.kind(),
                configured: entry.is_configured(),
                builtin: entry.builtin,
            })
            .collect();
        views.sort_by(|a, b| a.id.cmp(&b.id));
        views
    }
}

impl CatalogEntry {
    /// Whether this is a local provider (no key needed, e.g. Ollama/llama.cpp) or
    /// a cloud one. Keyed on the auth scheme: `none` is local, any key-bearing
    /// scheme is cloud.
    pub fn kind(&self) -> ProviderKind {
        match self.auth_scheme {
            AuthScheme::None => ProviderKind::Local,
            _ => ProviderKind::Cloud,
        }
    }

    /// Whether the provider is ready to use: a local provider needs no key, a
    /// cloud one is configured once its credential is held in the broker
    /// (`credential_ref` set). A cloud entry with no `credential_ref` is
    /// catalogued-but-not-yet-configured.
    pub fn is_configured(&self) -> bool {
        self.auth_scheme == AuthScheme::None || self.credential_ref.is_some()
    }
}

/// Whether a catalogued provider runs locally (no key) or is a cloud service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// A local provider (Ollama, llama.cpp) - no credential needed.
    Local,
    /// A cloud provider reached over the network with a broker-held credential.
    Cloud,
}

/// The Settings AI-providers manager view of one catalogued provider: display
/// metadata only (no endpoint URL, no credential). `camelCase` on the wire to
/// match the manager-seam serialization (`ai_providers_list`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    /// The catalog key (the id the manager addresses + the picker uses).
    pub id: String,
    /// The human-facing name (the entry's `display_name`, else the id).
    pub name: String,
    /// Local vs cloud.
    pub kind: ProviderKind,
    /// Whether the provider is ready to use (local, or cloud with a held key).
    pub configured: bool,
    /// A built-in Arlen preset vs a user-added custom provider.
    pub builtin: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_catalog_ships_only_the_local_provider() {
        // Phase 9-α: cloud providers are intentionally absent until
        // keyring-backed auth lands.
        let cat = ProviderCatalog::default_arlen();
        let names: Vec<&str> = cat.names().collect();
        assert_eq!(names, vec!["ollama-default"]);
    }

    #[test]
    fn lookup_returns_full_url() {
        let cat = ProviderCatalog::default_arlen();
        let entry = cat.get("ollama-default").unwrap();
        assert_eq!(entry.endpoint_url, "http://127.0.0.1:11434/v1/chat/completions");
        assert_eq!(entry.backend, "ollama");
        // The local provider is OpenAI-compat, needs no key, and is a built-in.
        assert_eq!(entry.wire_format, WireFormat::Openai);
        assert_eq!(entry.auth_scheme, AuthScheme::None);
        assert!(entry.credential_ref.is_none());
        assert!(entry.builtin);
    }

    #[test]
    fn legacy_entry_deserializes_with_defaults() {
        // An existing `{endpoint_url, backend}` config (no cloud fields) still
        // parses: the new fields are `#[serde(default)]` (OpenAI / none / custom).
        let entry: CatalogEntry = serde_json::from_str(
            r#"{"endpoint_url":"http://localhost:11434/v1/chat/completions","backend":"ollama"}"#,
        )
        .expect("legacy entry parses");
        assert_eq!(entry.wire_format, WireFormat::Openai);
        assert_eq!(entry.auth_scheme, AuthScheme::None);
        assert!(entry.url_template.is_none());
        assert!(entry.credential_ref.is_none());
        assert!(!entry.builtin);

        // And the cloud schemes deserialize from their kebab/lowercase wire form.
        let anthropic: CatalogEntry = serde_json::from_str(
            r#"{"endpoint_url":"https://api.anthropic.com/v1/messages","backend":"anthropic","wire_format":"anthropic","auth_scheme":"x-api-key","credential_ref":"conn:anthropic"}"#,
        )
        .expect("anthropic entry parses");
        assert_eq!(anthropic.wire_format, WireFormat::Anthropic);
        assert_eq!(anthropic.auth_scheme, AuthScheme::XApiKey);
        assert_eq!(anthropic.credential_ref.as_deref(), Some("conn:anthropic"));
    }

    #[test]
    fn unknown_provider_returns_none() {
        let cat = ProviderCatalog::default_arlen();
        assert!(cat.get("missing-provider").is_none());
    }

    #[test]
    fn views_project_kind_and_configured_safely() {
        use std::collections::HashMap;
        let mut entries = HashMap::new();
        // A local provider: no key needed -> local + configured.
        entries.insert("ollama-default".to_string(), ProviderCatalog::default_arlen().get("ollama-default").unwrap().clone());
        // A cloud provider WITHOUT a credential -> cloud + not configured.
        entries.insert(
            "anthropic".to_string(),
            CatalogEntry {
                endpoint_url: "https://api.anthropic.com/v1/messages".to_string(),
                backend: "anthropic".to_string(),
                wire_format: WireFormat::Anthropic,
                auth_scheme: AuthScheme::XApiKey,
                url_template: None,
                credential_ref: None,
                models_endpoint: None,
                display_name: Some("Anthropic".to_string()),
                logo_id: None,
                builtin: true,
            },
        );
        let views = ProviderCatalog::new(entries).views();
        // Sorted by id: anthropic, then ollama-default.
        assert_eq!(views[0].id, "anthropic");
        assert_eq!(views[0].kind, ProviderKind::Cloud);
        assert!(!views[0].configured, "cloud with no credential is unconfigured");
        assert_eq!(views[0].name, "Anthropic");
        assert_eq!(views[1].id, "ollama-default");
        assert_eq!(views[1].kind, ProviderKind::Local);
        assert!(views[1].configured, "a local provider needs no key");
        // A cloud provider WITH a held credential is configured.
        let configured = CatalogEntry {
            endpoint_url: "https://api.anthropic.com/v1/messages".to_string(),
            backend: "anthropic".to_string(),
            wire_format: WireFormat::Anthropic,
            auth_scheme: AuthScheme::XApiKey,
            url_template: None,
            credential_ref: Some("conn:anthropic".to_string()),
            models_endpoint: None,
            display_name: None,
            logo_id: None,
            builtin: true,
        };
        assert!(configured.is_configured());
        assert_eq!(configured.kind(), ProviderKind::Cloud);
    }

    #[test]
    fn provider_view_serializes_to_the_manager_camelcase_shape() {
        // arlen-ui's `invoke` consumer reads these exact keys; a field rename
        // would pass every value test above yet silently break the manager UI.
        // Pin the wire shape: the key set, and that `kind` is the lowercase tag.
        let view = ProviderView {
            id: "anthropic".to_string(),
            name: "Anthropic".to_string(),
            kind: ProviderKind::Cloud,
            configured: false,
            builtin: true,
        };
        let v = serde_json::to_value(&view).expect("serializes");
        let obj = v.as_object().expect("a JSON object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["builtin", "configured", "id", "kind", "name"]);
        assert_eq!(obj["kind"], serde_json::json!("cloud"));
        assert_eq!(obj["configured"], serde_json::json!(false));
        // A local provider's kind tag is the lowercase counterpart.
        let local = serde_json::to_value(ProviderView {
            id: "ollama-default".to_string(),
            name: "Ollama".to_string(),
            kind: ProviderKind::Local,
            configured: true,
            builtin: true,
        })
        .expect("serializes");
        assert_eq!(local["kind"], serde_json::json!("local"));
    }
}
