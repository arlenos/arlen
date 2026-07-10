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

use crate::usage::UsageLimits;

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

/// The access class of a provider: how a caller obtains the credential and what it costs.
/// Orthogonal both to [`AuthScheme`] (the header the token rides in) and to the sovereignty
/// tier - a free or subscription provider can still be a closed-jurisdiction one that trains
/// on you, and the info line governs that independently (`ai-providers-plan.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMethod {
    /// A paid or freemium per-token API key (the common case; the existing preset list).
    /// The default.
    #[default]
    ApiKey,
    /// An existing OAuth subscription reused with no per-token cost (e.g. Claude Code,
    /// Codex, GitHub Copilot, Cursor). Rides the Connections OAuth broker rather than a
    /// stored key. Lets a user start with what they already pay for.
    SubscriptionLogin,
    /// Free or no-auth access (e.g. Kiro, OpenCode, Vertex free credits). Zero API-key
    /// friction; a convenience, not a sovereignty win.
    Free,
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
    /// The access class: how the credential is obtained and what it costs (a per-token API
    /// key, an existing OAuth subscription, or free/no-auth). A first-class catalog
    /// dimension, orthogonal to `auth_scheme` and the sovereignty tier. Defaults to
    /// `api-key`.
    #[serde(default)]
    pub auth_method: AuthMethod,
    /// Whether this provider's access path is unofficial: reverse-engineered, ToS-violating,
    /// account-suspension-risky (e.g. the subscription-login paths for Claude Code / Codex /
    /// Cursor / Copilot). The UI must surface a clear warning and never offer it silently;
    /// the official free/subscription paths (Kiro, OpenCode, Vertex) are not flagged.
    #[serde(default)]
    pub unofficial: bool,
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
    /// The hand-curated sovereignty facts surfaced in the picker's per-provider
    /// info line (jurisdiction / trains-on-you / open-weight + honesty flags).
    /// Defaults to the conservative un-curated stub, so an entry that omits it
    /// renders honestly rather than fabricating a guarantee.
    #[serde(default)]
    pub sovereignty: crate::sovereignty::SovereigntyInfo,
}

/// The `ai-routing.toml` shape: a table of named providers, each a full catalog entry
/// (`[providers.<name>]`). The file is the proxy's own trusted configuration, never caller
/// input, so an entry here is as authoritative as a built-in default.
#[derive(Debug, Default, Deserialize)]
struct CatalogConfig {
    #[serde(default)]
    providers: HashMap<String, CatalogEntry>,
    /// Named fallback chains: combo name -> the provider ids to try, in order.
    #[serde(default)]
    combos: HashMap<String, Vec<String>>,
    /// Per-provider token spending limits + the reset window.
    #[serde(default)]
    limits: Option<UsageLimits>,
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
    /// A combo names a provider absent from the catalog, so the fallback chain would
    /// dead-end at walk time. Caught at load rather than mid-request.
    #[error("combo '{combo}' references unknown provider '{provider}'")]
    UnknownComboProvider {
        /// The combo whose chain is broken.
        combo: String,
        /// The unknown provider id it referenced.
        provider: String,
    },
    /// A combo is empty or its name shadows a provider name, either of which makes the
    /// fallback chain ambiguous or dead. Caught at load.
    #[error("combo '{combo}' is invalid: {reason}")]
    InvalidCombo {
        /// The offending combo name.
        combo: String,
        /// Why it is invalid.
        reason: &'static str,
    },
    /// A spending cap names a provider absent from the catalog, so the cap would never fire.
    /// Caught at load.
    #[error("spending cap references unknown provider '{provider}'")]
    UnknownCapProvider {
        /// The unknown provider id the cap referenced.
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
    /// Named fallback chains (combo name -> ordered provider ids). Empty unless configured.
    combos: HashMap<String, Vec<String>>,
    /// Per-provider token spending limits + the reset window. Default = no caps.
    limits: UsageLimits,
}

/// The mainstream closed majors as available-to-add catalog entries. models.dev
/// pins no `api` base URL for them (they use the @ai-sdk adapter default), so they
/// are added here with their well-known endpoints, wire format and curated
/// sovereignty. Credential-less (`credential_ref: None`) so they list as needs-key
/// in the picker; the active-routing catalog never carries them until a key exists.
fn mainstream_available() -> Vec<(String, CatalogEntry)> {
    // (id, display, endpoint, models_endpoint, wire_format, auth_scheme)
    const SPECS: &[(&str, &str, &str, &str, WireFormat, AuthScheme)] = &[
        (
            "openai",
            "OpenAI",
            "https://api.openai.com/v1/chat/completions",
            "https://api.openai.com/v1/models",
            WireFormat::Openai,
            AuthScheme::Bearer,
        ),
        (
            "anthropic",
            "Anthropic",
            "https://api.anthropic.com/v1/messages",
            "https://api.anthropic.com/v1/models",
            WireFormat::Anthropic,
            AuthScheme::XApiKey,
        ),
        (
            "mistral",
            "Mistral",
            "https://api.mistral.ai/v1/chat/completions",
            "https://api.mistral.ai/v1/models",
            WireFormat::Openai,
            AuthScheme::Bearer,
        ),
    ];
    SPECS
        .iter()
        .map(|(id, name, endpoint, models, wire_format, auth_scheme)| {
            (
                id.to_string(),
                CatalogEntry {
                    endpoint_url: endpoint.to_string(),
                    backend: id.to_string(),
                    wire_format: *wire_format,
                    auth_scheme: *auth_scheme,
                    auth_method: AuthMethod::ApiKey,
                    url_template: None,
                    credential_ref: None,
                    models_endpoint: Some(models.to_string()),
                    display_name: Some(name.to_string()),
                    logo_id: None,
                    unofficial: false,
                    builtin: true,
                    sovereignty: crate::sovereignty::curated_for(id).unwrap_or_default(),
                },
            )
        })
        .collect()
}

impl ProviderCatalog {
    /// Build a catalog from an explicit map (no combos, no limits).
    pub fn new(entries: HashMap<String, CatalogEntry>) -> Self {
        Self { entries, combos: HashMap::new(), limits: UsageLimits::default() }
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
        // Validate every combo up front, so a broken fallback chain fails at load rather
        // than dead-ending a request mid-walk: non-empty, not shadowing a provider name,
        // and every member a known provider.
        for (combo, order) in &config.combos {
            if order.is_empty() {
                return Err(CatalogError::InvalidCombo {
                    combo: combo.clone(),
                    reason: "a combo must list at least one provider",
                });
            }
            if entries.contains_key(combo) {
                return Err(CatalogError::InvalidCombo {
                    combo: combo.clone(),
                    reason: "a combo name must not shadow a provider name",
                });
            }
            for provider in order {
                if !entries.contains_key(provider) {
                    return Err(CatalogError::UnknownComboProvider {
                        combo: combo.clone(),
                        provider: provider.clone(),
                    });
                }
            }
        }
        // Resolve + validate spending limits: every capped provider must be known, so a cap
        // on a typo'd provider fails at load rather than silently never firing.
        let limits = config.limits.unwrap_or_default();
        for provider in limits.caps.keys() {
            if !entries.contains_key(provider) {
                return Err(CatalogError::UnknownCapProvider { provider: provider.clone() });
            }
        }
        Ok(Self { entries, combos: config.combos, limits })
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
                // Ollama is a local, free, no-auth provider.
                auth_method: AuthMethod::Free,
                unofficial: false,
                builtin: true,
                // A local runtime: inference stays on the device, so the data
                // never leaves and there is no cloud jurisdiction or training on
                // it; the served models are open-weight. The most sovereign tier.
                sovereignty: crate::sovereignty::SovereigntyInfo {
                    jurisdiction: crate::sovereignty::Jurisdiction::Local,
                    trains_on_you: crate::sovereignty::TrainsOnYou::No,
                    open_weight: true,
                    ..Default::default()
                },
            },
        );
        Self::new(entries)
    }

    /// The full available-to-add catalog for the provider picker: the local active
    /// builtins ([`Self::default_arlen`]) plus every models.dev-seeded provider
    /// (sovereignty overlaid via the hand-curated table). Unlike `default_arlen`,
    /// this deliberately includes credential-less cloud providers - it is the
    /// SHOW-and-ADD surface, so a listed cloud provider reads as `configured:
    /// false` until the user attaches a key. The active-routing catalog stays
    /// `default_arlen` (local-only) so no half-working cloud route is ever taken.
    /// The seed is a vendored models.dev snapshot (MIT); a seeded entry never
    /// overrides a local builtin.
    pub fn available_arlen() -> Self {
        const SEED: &str = include_str!("../seed/models-dev-slim.json");
        let mut cat = Self::default_arlen();
        if let Ok(doc) = serde_json::from_str::<serde_json::Value>(SEED) {
            let mut seeded = crate::models_dev::seed_from_models_dev(&doc);
            crate::models_dev::apply_curated_overlay(&mut seeded);
            for (id, entry) in seeded {
                cat.entries.entry(id).or_insert(entry);
            }
        }
        // The mainstream closed majors carry no `api` base URL in models.dev (they
        // rely on the adapter default), so the seed omits them; add them explicitly
        // as available-to-add entries (credential-less -> configured:false) so the
        // picker surfaces the providers users most want, sovereignty-annotated.
        for (id, entry) in mainstream_available() {
            cat.entries.entry(id).or_insert(entry);
        }
        cat
    }

    /// Look up a provider by name.
    pub fn get(&self, provider_name: &str) -> Option<&CatalogEntry> {
        self.entries.get(provider_name)
    }

    /// The ordered provider ids of a named fallback chain, if the combo exists. The walk over
    /// these (falling to the next on a provider-availability signal) lives in
    /// `ProxyService::forward_combo`.
    pub fn combo(&self, name: &str) -> Option<&[String]> {
        self.combos.get(name).map(Vec::as_slice)
    }

    /// The configured spending limits (reset window + per-provider token caps).
    pub fn limits(&self) -> &UsageLimits {
        &self.limits
    }

    /// The token cap for a provider this window, if one is configured (else uncapped).
    pub fn cap_for(&self, provider: &str) -> Option<u64> {
        self.limits.caps.get(provider).copied()
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
                auth_method: entry.auth_method,
                unofficial: entry.unofficial,
                jurisdiction: entry.sovereignty.jurisdiction_value().map(str::to_string),
                trains_on_you: entry.sovereignty.trains_value().map(str::to_string),
                open_weight: entry.sovereignty.open_weight,
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
    /// The access class (api-key / subscription-login / free), for the UI's auth/cost chip.
    /// Serializes as `authMethod`.
    pub auth_method: AuthMethod,
    /// Whether this provider's access path is unofficial (reverse-engineered,
    /// suspension-risk); the UI shows a clear warning and never offers it silently.
    pub unofficial: bool,
    /// The jurisdiction chip (`"eu"`/`"us"`/`"cn"`), or absent for local/unknown.
    /// The picker renders it as a chip with a data-law tooltip. Facts to surface,
    /// never a verdict. Serializes as `jurisdiction`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<String>,
    /// The training posture (`"no"`/`"no-paid"`/`"yes"`), or absent when uncurated.
    /// `no-paid` is the load-bearing caveat: no on the paid API, the free tier
    /// trains. Serializes as `trainsOnYou`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trains_on_you: Option<String>,
    /// Whether the served models are open-weight (the escape hatch is visible).
    /// Serializes as `openWeight`.
    pub open_weight: bool,
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
    fn load_carries_the_auth_method_and_unofficial_flag() {
        let path = tmp_catalog(
            "authmethod",
            "[providers.claude-code]\n\
             endpoint_url = \"https://api.anthropic.com/v1/messages\"\n\
             backend = \"anthropic\"\n\
             auth_scheme = \"x-api-key\"\n\
             auth_method = \"subscription-login\"\n\
             unofficial = true\n\
             [providers.plain]\n\
             endpoint_url = \"https://api.example.com/v1/chat/completions\"\n\
             backend = \"openai\"\n\
             auth_scheme = \"bearer\"\n",
        );
        let cat = ProviderCatalog::load_or_default(&path).unwrap();
        let cc = cat.get("claude-code").unwrap();
        assert_eq!(cc.auth_method, AuthMethod::SubscriptionLogin);
        assert!(cc.unofficial, "the reverse-engineered subscription path must be flagged");
        // an entry that omits them defaults to a paid, official API key
        let plain = cat.get("plain").unwrap();
        assert_eq!(plain.auth_method, AuthMethod::ApiKey);
        assert!(!plain.unofficial);
        // the built-in local Ollama is free
        assert_eq!(cat.get("ollama-default").unwrap().auth_method, AuthMethod::Free);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_parses_a_provider_combo() {
        let path = tmp_catalog(
            "combo",
            "[providers.p1]\n\
             endpoint_url = \"https://a.example/v1/chat/completions\"\n\
             backend = \"openai\"\n\
             auth_scheme = \"bearer\"\n\
             [providers.p2]\n\
             endpoint_url = \"https://b.example/v1/chat/completions\"\n\
             backend = \"openai\"\n\
             auth_scheme = \"bearer\"\n\
             [combos]\n\
             sovereign = [\"ollama-default\", \"p1\", \"p2\"]\n",
        );
        let cat = ProviderCatalog::load_or_default(&path).unwrap();
        assert_eq!(
            cat.combo("sovereign").unwrap(),
            &["ollama-default".to_string(), "p1".to_string(), "p2".to_string()]
        );
        assert!(cat.combo("nonexistent").is_none());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_rejects_a_combo_referencing_an_unknown_provider() {
        let path = tmp_catalog("combobad", "[combos]\nx = [\"ollama-default\", \"ghost\"]\n");
        let err = ProviderCatalog::load_or_default(&path).unwrap_err();
        assert!(matches!(err, CatalogError::UnknownComboProvider { .. }), "got {err:?}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_rejects_an_empty_or_shadowing_combo() {
        let empty = tmp_catalog("comboempty", "[combos]\nx = []\n");
        assert!(
            matches!(
                ProviderCatalog::load_or_default(&empty).unwrap_err(),
                CatalogError::InvalidCombo { .. }
            ),
            "empty combo must be rejected"
        );
        std::fs::remove_file(&empty).ok();
        // a combo whose name equals a provider's would shadow it
        let shadow =
            tmp_catalog("comboshadow", "[combos]\n\"ollama-default\" = [\"ollama-default\"]\n");
        assert!(
            matches!(
                ProviderCatalog::load_or_default(&shadow).unwrap_err(),
                CatalogError::InvalidCombo { .. }
            ),
            "a combo shadowing a provider name must be rejected"
        );
        std::fs::remove_file(&shadow).ok();
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
    fn available_arlen_seeds_the_picker_catalog_but_keeps_the_local_builtin() {
        let cat = ProviderCatalog::available_arlen();
        // The local active builtin survives (seeded entries never override it).
        let ollama = cat.get("ollama-default").expect("local builtin kept");
        assert_eq!(ollama.kind(), ProviderKind::Local);
        // The vendored models.dev snapshot populated the addable cloud providers.
        assert!(
            cat.entries.len() > 100,
            "expected the ~130-provider models.dev seed"
        );
        // A seeded curated provider carries its hand-curated sovereignty overlay,
        // while its endpoint came from the machine seed.
        let deepseek = cat.get("deepseek").expect("deepseek seeded");
        assert_eq!(
            deepseek.sovereignty.jurisdiction,
            crate::sovereignty::Jurisdiction::Cn
        );
        assert!(deepseek.endpoint_url.ends_with("/chat/completions"));
        // The mainstream majors (api-less in models.dev) are added explicitly with
        // their well-known endpoints, wire format and curated sovereignty.
        let anthropic = cat.get("anthropic").expect("mainstream builtin");
        assert_eq!(anthropic.endpoint_url, "https://api.anthropic.com/v1/messages");
        assert_eq!(anthropic.wire_format, WireFormat::Anthropic);
        assert_eq!(
            anthropic.sovereignty.jurisdiction,
            crate::sovereignty::Jurisdiction::Us
        );
        // Credential-less -> the picker shows it as needs-key, not active.
        assert!(!anthropic.is_configured());
        let mistral = cat.get("mistral").expect("mainstream builtin");
        assert_eq!(
            mistral.sovereignty.residency,
            crate::sovereignty::Residency::EuPolicyDefault
        );
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
                auth_method: AuthMethod::ApiKey,
                unofficial: false,
                builtin: true,
                sovereignty: Default::default(),
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
            auth_method: AuthMethod::ApiKey,
            unofficial: false,
            builtin: true,
            sovereignty: Default::default(),
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
            auth_method: AuthMethod::ApiKey,
            unofficial: false,
            jurisdiction: Some("us".to_string()),
            trains_on_you: Some("no-paid".to_string()),
            open_weight: false,
        };
        let v = serde_json::to_value(&view).expect("serializes");
        let obj = v.as_object().expect("a JSON object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "authMethod",
                "builtin",
                "configured",
                "id",
                "jurisdiction",
                "kind",
                "name",
                "openWeight",
                "trainsOnYou",
                "unofficial"
            ]
        );
        assert_eq!(obj["kind"], serde_json::json!("cloud"));
        assert_eq!(obj["configured"], serde_json::json!(false));
        // the auth class serializes as a camelCase key with a kebab-case value
        assert_eq!(obj["authMethod"], serde_json::json!("api-key"));
        assert_eq!(obj["unofficial"], serde_json::json!(false));
        // the sovereignty facts match the picker's exact structured contract
        assert_eq!(obj["jurisdiction"], serde_json::json!("us"));
        assert_eq!(obj["trainsOnYou"], serde_json::json!("no-paid"));
        assert_eq!(obj["openWeight"], serde_json::json!(false));
        // A local provider's kind tag is the lowercase counterpart.
        let local = serde_json::to_value(ProviderView {
            id: "ollama-default".to_string(),
            name: "Ollama".to_string(),
            kind: ProviderKind::Local,
            configured: true,
            builtin: true,
            auth_method: AuthMethod::Free,
            unofficial: false,
            jurisdiction: None,
            trains_on_you: None,
            open_weight: true,
        })
        .expect("serializes");
        assert_eq!(local["kind"], serde_json::json!("local"));
        assert_eq!(local["authMethod"], serde_json::json!("free"));
    }
}
