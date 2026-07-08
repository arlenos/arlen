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

/// The `ai-routing.toml` shape: a table of named providers, each a full catalog entry
/// (`[providers.<name>]`). The file is the proxy's own trusted configuration, never caller
/// input, so an entry here is as authoritative as a built-in default.
#[derive(Debug, Default, Deserialize)]
struct CatalogConfig {
    #[serde(default)]
    providers: HashMap<String, CatalogEntry>,
}

/// Why the provider catalog could not be loaded from disk.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    /// The catalog file could not be read.
    #[error("reading the provider catalog: {0}")]
    Read(#[from] std::io::Error),
    /// The catalog file was not valid TOML for the catalog shape.
    #[error("parsing the provider catalog: {0}")]
    Parse(#[from] toml::de::Error),
    /// A keyed provider (one whose `auth_scheme` sends an API key) is configured with a
    /// plaintext, non-loopback endpoint, which would leak the key on the wire. The SSRF
    /// floor guards the destination IP but not the scheme, so this is checked here.
    #[error("provider '{provider}' sends a key but its endpoint is not https or loopback")]
    InsecureEndpoint {
        /// The offending provider name.
        provider: String,
    },
}

/// Whether a catalog entry may be reached without leaking its credential: a provider that
/// sends no key is unrestricted (the SSRF floor still applies), and a keyed provider must
/// use `https` or a loopback host so the key never crosses the network in the clear.
fn endpoint_is_secure_enough(entry: &CatalogEntry) -> bool {
    if entry.auth_scheme == AuthScheme::None {
        return true;
    }
    match url::Url::parse(&entry.endpoint_url) {
        Ok(u) => u.scheme() == "https" || u.host_str().is_some_and(is_loopback_host),
        Err(_) => false,
    }
}

/// Whether a host names the local machine (so plaintext to it stays on the box).
fn is_loopback_host(host: &str) -> bool {
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
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

    /// Load the catalog from `ai-routing.toml` layered on the built-in defaults: an absent
    /// file yields the defaults alone, and each user entry adds a provider or overrides a
    /// built-in of the same name (the user-extensible path the plan asks for). A present but
    /// malformed file is an error rather than a silent fall-back, so a misconfiguration
    /// surfaces instead of quietly discarding the user's providers. The endpoints stay
    /// proxy-owned: callers still select a provider by name, and the dial is SSRF-pinned
    /// regardless of where the entry came from.
    pub fn load_or_default(path: &std::path::Path) -> Result<Self, CatalogError> {
        if !path.exists() {
            return Ok(Self::default_arlen());
        }
        let text = std::fs::read_to_string(path)?;
        let config: CatalogConfig = toml::from_str(&text)?;
        let mut entries = Self::default_arlen().entries;
        for (name, entry) in config.providers {
            entries.insert(name, entry);
        }
        for (name, entry) in &entries {
            if !endpoint_is_secure_enough(entry) {
                return Err(CatalogError::InsecureEndpoint { provider: name.clone() });
            }
        }
        Ok(Self::new(entries))
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

    fn tmp_catalog(name: &str, content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir()
            .join(format!("arlen-catalog-test-{}-{name}.toml", std::process::id()));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn load_absent_file_yields_the_defaults() {
        let path = std::env::temp_dir().join("arlen-catalog-test-absent-does-not-exist.toml");
        let cat = ProviderCatalog::load_or_default(&path).unwrap();
        assert!(cat.get("ollama-default").is_some());
    }

    #[test]
    fn load_empty_file_yields_the_defaults() {
        // a user who creates the config but adds nothing gets the built-ins, not an error
        let path = tmp_catalog("empty", "");
        let cat = ProviderCatalog::load_or_default(&path).unwrap();
        assert!(cat.get("ollama-default").is_some());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_adds_a_user_provider_over_the_defaults() {
        let path = tmp_catalog(
            "add",
            "[providers.my-llama]\n\
             endpoint_url = \"http://127.0.0.1:8080/v1/chat/completions\"\n\
             backend = \"llama\"\n\
             wire_format = \"openai\"\n",
        );
        let cat = ProviderCatalog::load_or_default(&path).unwrap();
        assert!(cat.get("ollama-default").is_some(), "built-in is kept");
        assert_eq!(
            cat.get("my-llama").unwrap().endpoint_url,
            "http://127.0.0.1:8080/v1/chat/completions"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_lets_a_user_override_a_builtin_by_name() {
        let path = tmp_catalog(
            "override",
            "[providers.ollama-default]\n\
             endpoint_url = \"http://127.0.0.1:9999/v1/chat/completions\"\n\
             backend = \"ollama\"\n",
        );
        let cat = ProviderCatalog::load_or_default(&path).unwrap();
        assert_eq!(
            cat.get("ollama-default").unwrap().endpoint_url,
            "http://127.0.0.1:9999/v1/chat/completions"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_malformed_config_is_an_error_not_a_silent_default() {
        let path = tmp_catalog("bad", "this = = not valid toml");
        assert!(ProviderCatalog::load_or_default(&path).is_err());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_rejects_a_keyed_provider_on_a_plaintext_endpoint() {
        // a cloud provider that sends a Bearer key over plaintext http would leak the key
        let path = tmp_catalog(
            "insecure",
            "[providers.cloud]\n\
             endpoint_url = \"http://api.example.com/v1/chat/completions\"\n\
             backend = \"openai\"\n\
             auth_scheme = \"bearer\"\n",
        );
        let err = ProviderCatalog::load_or_default(&path).unwrap_err();
        assert!(matches!(err, CatalogError::InsecureEndpoint { .. }));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_accepts_a_keyed_provider_over_https_or_loopback() {
        for (name, url) in [
            ("https", "https://api.example.com/v1/chat/completions"),
            ("loopback", "http://127.0.0.1:8080/v1/chat/completions"),
        ] {
            let path = tmp_catalog(
                name,
                &format!(
                    "[providers.keyed]\n\
                     endpoint_url = \"{url}\"\n\
                     backend = \"openai\"\n\
                     auth_scheme = \"bearer\"\n"
                ),
            );
            assert!(ProviderCatalog::load_or_default(&path).is_ok(), "{name} should load");
            std::fs::remove_file(&path).ok();
        }
    }

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
