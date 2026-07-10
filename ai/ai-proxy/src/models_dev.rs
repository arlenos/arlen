//! Seed the provider catalog from a models.dev `api.json` snapshot
//! (`anomalyco/models.dev`, MIT). models.dev is the machine-readable, auto-updated
//! source for the plumbing: each provider carries `id`, `name`, `api` (base URL),
//! `env` (auth env-var names), `npm` (the @ai-sdk adapter -> our wire format), and
//! a model map whose `open_weights` bool feeds the one sovereignty fact that IS
//! machine-derivable. The jurisdiction / trains-on-you / residency facts are NOT
//! in models.dev; they are hand-curated and layered on top separately, so this
//! parser leaves them at their conservative `Unknown` default.
//!
//! This is the parser only (pure `serde_json::Value` -> `CatalogEntry`); vendoring
//! a snapshot and wiring it into `default_arlen()` is a separate step.

use serde_json::Value;

use crate::catalog::{AuthMethod, AuthScheme, CatalogEntry, WireFormat};
use crate::sovereignty::SovereigntyInfo;

/// Map a models.dev `npm` adapter id to our wire format. The OpenAI-compatible
/// shape is the spine and the default; only Anthropic and Google get their own.
fn wire_format_from_npm(npm: &str) -> WireFormat {
    match npm {
        "@ai-sdk/anthropic" => WireFormat::Anthropic,
        "@ai-sdk/google" | "@ai-sdk/google-vertex" => WireFormat::Gemini,
        _ => WireFormat::Openai,
    }
}

/// The egress auth header a wire format uses when the provider declares an auth
/// env var. Anthropic uses `x-api-key`; everything else on the OpenAI-compatible
/// spine uses `Authorization: Bearer`.
fn auth_scheme_for(wire_format: WireFormat, has_env: bool) -> AuthScheme {
    if !has_env {
        return AuthScheme::None;
    }
    match wire_format {
        WireFormat::Anthropic => AuthScheme::XApiKey,
        _ => AuthScheme::Bearer,
    }
}

/// Build a [`CatalogEntry`] from one models.dev provider entry (the value under
/// its provider-id key). Returns `None` when the entry lacks the `api` base URL
/// (the ~24 adapter-default providers models.dev does not pin a URL for - not
/// seeded from here) so the catalog never carries an un-dispatchable endpoint.
///
/// The sovereignty `open_weight` chip is derived (any served model is
/// open-weight); jurisdiction and trains-on-you stay `Unknown` for the
/// hand-curated overlay.
pub fn catalog_entry_from_models_dev(id: &str, provider: &Value) -> Option<CatalogEntry> {
    let api = provider.get("api")?.as_str()?.trim_end_matches('/');
    if api.is_empty() {
        return None;
    }
    let name = provider
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(id)
        .to_string();
    let npm = provider.get("npm").and_then(Value::as_str).unwrap_or("");
    let wire_format = wire_format_from_npm(npm);
    let has_env = provider
        .get("env")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().any(|v| v.as_str().is_some_and(|s| !s.is_empty())));
    let open_weight = provider
        .get("models")
        .and_then(Value::as_object)
        .is_some_and(|models| {
            models
                .values()
                .any(|m| m.get("open_weights").and_then(Value::as_bool).unwrap_or(false))
        });

    Some(CatalogEntry {
        endpoint_url: format!("{api}/chat/completions"),
        backend: id.to_string(),
        wire_format,
        auth_scheme: auth_scheme_for(wire_format, has_env),
        auth_method: AuthMethod::ApiKey,
        url_template: None,
        credential_ref: None,
        models_endpoint: Some(format!("{api}/models")),
        display_name: Some(name),
        logo_id: None,
        unofficial: false,
        builtin: true,
        sovereignty: SovereigntyInfo {
            open_weight,
            ..Default::default()
        },
    })
}

/// Build catalog entries for every provider in a models.dev `api.json` document
/// that carries an `api` base URL (the seedable set), each keyed by its provider
/// id. Providers models.dev pins no URL for are skipped (the adapter-default long
/// tail, reachable via the "+ Add provider" escape hatch or a later slice). The
/// entries carry only the machine-derivable facts; the hand-curated sovereignty
/// overlay is applied on top separately.
pub fn seed_from_models_dev(doc: &Value) -> Vec<(String, CatalogEntry)> {
    doc.as_object()
        .map(|providers| {
            providers
                .iter()
                .filter_map(|(id, p)| catalog_entry_from_models_dev(id, p).map(|e| (id.clone(), e)))
                .collect()
        })
        .unwrap_or_default()
}

/// Layer the hand-curated sovereignty facts (`sovereignty::curated_for`) over a
/// machine-derived seed. The curated jurisdiction / trains-on-you / residency /
/// honesty flags are authoritative and win; the seed's `open_weight` is kept
/// (it is derived from the provider's actual model set, the one sovereignty fact
/// models.dev supplies reliably). An un-curated provider keeps its seed stub.
pub fn apply_curated_overlay(entries: &mut [(String, CatalogEntry)]) {
    for (id, entry) in entries.iter_mut() {
        if let Some(curated) = crate::sovereignty::curated_for(id) {
            entry.sovereignty = SovereigntyInfo {
                open_weight: entry.sovereignty.open_weight,
                ..curated
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn openai_compatible_provider_maps_to_the_bearer_spine() {
        // Mistral's real models.dev shape: OpenAI-compatible adapter, an env key,
        // an open-weight model in the mix.
        let p = json!({
            "id": "mistral",
            "name": "Mistral",
            "npm": "@ai-sdk/mistral",
            "env": ["MISTRAL_API_KEY"],
            "api": "https://api.mistral.ai/v1",
            "models": {
                "mistral-large": { "open_weights": false },
                "mistral-small": { "open_weights": true }
            }
        });
        let e = catalog_entry_from_models_dev("mistral", &p).expect("has api");
        assert_eq!(e.endpoint_url, "https://api.mistral.ai/v1/chat/completions");
        assert_eq!(e.models_endpoint.as_deref(), Some("https://api.mistral.ai/v1/models"));
        assert_eq!(e.wire_format, WireFormat::Openai);
        assert_eq!(e.auth_scheme, AuthScheme::Bearer);
        assert_eq!(e.display_name.as_deref(), Some("Mistral"));
        assert_eq!(e.backend, "mistral");
        assert!(e.builtin);
        // At least one served model is open-weight -> the chip is set.
        assert!(e.sovereignty.open_weight);
        // The hand-curated facts are NOT derived here.
        assert_eq!(e.sovereignty.jurisdiction, crate::sovereignty::Jurisdiction::Unknown);
    }

    #[test]
    fn anthropic_maps_to_the_native_shape_and_x_api_key() {
        let p = json!({
            "id": "anthropic",
            "name": "Anthropic",
            "npm": "@ai-sdk/anthropic",
            "env": ["ANTHROPIC_API_KEY"],
            "api": "https://api.anthropic.com/v1",
            "models": { "claude": { "open_weights": false } }
        });
        let e = catalog_entry_from_models_dev("anthropic", &p).expect("has api");
        assert_eq!(e.wire_format, WireFormat::Anthropic);
        assert_eq!(e.auth_scheme, AuthScheme::XApiKey);
        assert!(!e.sovereignty.open_weight);
    }

    #[test]
    fn trailing_slash_on_the_api_url_is_not_doubled() {
        let p = json!({ "name": "X", "npm": "", "env": ["K"], "api": "https://x.example/v1/", "models": {} });
        let e = catalog_entry_from_models_dev("x", &p).expect("has api");
        assert_eq!(e.endpoint_url, "https://x.example/v1/chat/completions");
    }

    #[test]
    fn a_provider_without_an_api_url_is_skipped() {
        // The ~24 adapter-default providers models.dev pins no URL for.
        let p = json!({ "id": "x", "name": "X", "npm": "@ai-sdk/x", "env": ["K"], "models": {} });
        assert!(catalog_entry_from_models_dev("x", &p).is_none());
    }

    #[test]
    fn seed_collects_only_the_providers_with_an_api_url() {
        let doc = json!({
            "mistral": {
                "name": "Mistral", "npm": "@ai-sdk/mistral", "env": ["K"],
                "api": "https://api.mistral.ai/v1", "models": {}
            },
            // No api URL: an adapter-default provider, skipped by the seed.
            "adapter-only": {
                "name": "Adapter Only", "npm": "@ai-sdk/x", "env": ["K"], "models": {}
            }
        });
        let mut seeded = seed_from_models_dev(&doc);
        seeded.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(seeded.len(), 1);
        assert_eq!(seeded[0].0, "mistral");
        assert_eq!(seeded[0].1.display_name.as_deref(), Some("Mistral"));
    }

    #[test]
    fn curated_overlay_wins_governance_but_keeps_the_seed_open_weight() {
        use crate::sovereignty::{Jurisdiction, Residency, TrainsOnYou};
        // Seed mistral (machine-derived: open-weight true) + an un-curated provider.
        let doc = json!({
            "mistral": {
                "name": "Mistral", "npm": "@ai-sdk/mistral", "env": ["K"],
                "api": "https://api.mistral.ai/v1",
                "models": { "m": { "open_weights": true } }
            },
            "obscure": {
                "name": "Obscure", "npm": "@ai-sdk/x", "env": ["K"],
                "api": "https://obscure.example/v1",
                "models": { "m": { "open_weights": false } }
            }
        });
        let mut seeded = seed_from_models_dev(&doc);
        apply_curated_overlay(&mut seeded);
        let mistral = &seeded.iter().find(|(id, _)| id == "mistral").unwrap().1;
        // Curated governance facts win.
        assert_eq!(mistral.sovereignty.jurisdiction, Jurisdiction::Eu);
        assert_eq!(mistral.sovereignty.residency, Residency::EuPolicyDefault);
        assert_eq!(mistral.sovereignty.trains_on_you, TrainsOnYou::PaidApiOnly);
        // The seed's machine-derived open-weight is kept.
        assert!(mistral.sovereignty.open_weight);
        // The un-curated provider keeps its Unknown stub.
        let obscure = &seeded.iter().find(|(id, _)| id == "obscure").unwrap().1;
        assert_eq!(obscure.sovereignty.jurisdiction, Jurisdiction::Unknown);
    }

    #[test]
    fn no_env_means_no_auth_scheme() {
        let p = json!({ "name": "Local", "npm": "", "env": [], "api": "http://127.0.0.1:8080/v1", "models": {} });
        let e = catalog_entry_from_models_dev("local", &p).expect("has api");
        assert_eq!(e.auth_scheme, AuthScheme::None);
    }
}
