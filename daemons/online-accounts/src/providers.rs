//! OA-R2: the OAuth provider registry.
//!
//! An `AddAccount` for a provider needs that provider's OAuth endpoints, its
//! registered client id and the requested scope. These live in a plaintext
//! config (`$XDG_CONFIG_HOME/arlen/oauth-providers.toml`), NOT the vault: the
//! `client_id` is a PUBLIC RFC-8252 native-app identifier (PKCE replaces a
//! client secret, so there is no secret to protect), and the endpoints are the
//! provider's published URLs. The operator installs this file with the real
//! client ids registered with each provider.

use std::path::PathBuf;

use serde::Deserialize;

use crate::flow::ProviderConfig;

/// One registered OAuth provider.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OwnedProvider {
    /// The provider name (`google`, `nextcloud`, ...) - the `AddAccount` key.
    pub name: String,
    /// The authorization endpoint the browser is sent to.
    pub authorization_endpoint: String,
    /// The token endpoint the code + refresh are exchanged at.
    pub token_endpoint: String,
    /// The public (RFC-8252) client id registered with the provider.
    pub client_id: String,
    /// The space-delimited requested scopes.
    #[serde(default)]
    pub scope: String,
}

impl OwnedProvider {
    /// Borrow as the flow's [`ProviderConfig`].
    pub fn as_config(&self) -> ProviderConfig<'_> {
        ProviderConfig {
            authorization_endpoint: &self.authorization_endpoint,
            token_endpoint: &self.token_endpoint,
            client_id: &self.client_id,
            scope: &self.scope,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ProvidersFile {
    #[serde(default)]
    provider: Vec<OwnedProvider>,
}

/// The provider registry path: `$XDG_CONFIG_HOME/arlen/oauth-providers.toml`,
/// else `$HOME/.config/arlen/oauth-providers.toml`. `None` when neither is set.
pub fn providers_config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("arlen").join("oauth-providers.toml"))
}

/// Parse the provider registry from `toml_text`. Fail-closed: a malformed file
/// is an error (no partial registry); a provider without a name or client id, or
/// with a non-`https` endpoint, is refused (an OAuth flow needs the id and the
/// token/authorization endpoints must not be plaintext).
pub fn parse_providers(toml_text: &str) -> Result<Vec<OwnedProvider>, String> {
    let file: ProvidersFile = toml::from_str(toml_text).map_err(|e| e.to_string())?;
    for p in &file.provider {
        if p.name.trim().is_empty() || p.client_id.trim().is_empty() {
            return Err(format!("provider {:?} is missing a name or client_id", p.name));
        }
        if !p.authorization_endpoint.starts_with("https://")
            || !p.token_endpoint.starts_with("https://")
        {
            return Err(format!("provider {} endpoints must be https", p.name));
        }
    }
    Ok(file.provider)
}

/// Look up a provider by name in a parsed registry.
pub fn find_provider<'a>(providers: &'a [OwnedProvider], name: &str) -> Option<&'a OwnedProvider> {
    providers.iter().find(|p| p.name == name)
}

/// Load + parse the provider registry from the default config path. A missing
/// config home or a missing file yields an empty registry (an `AddAccount` then
/// fails cleanly for an unknown provider); a present-but-malformed file is an
/// error (fail-closed, no partial registry).
pub fn load_providers() -> Result<Vec<OwnedProvider>, String> {
    let Some(path) = providers_config_path() else {
        return Ok(Vec::new());
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_providers(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("read provider registry {}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        [[provider]]
        name = "google"
        authorization_endpoint = "https://accounts.google.com/o/oauth2/v2/auth"
        token_endpoint = "https://oauth2.googleapis.com/token"
        client_id = "123.apps.googleusercontent.com"
        scope = "openid email https://www.googleapis.com/auth/calendar.readonly"

        [[provider]]
        name = "nextcloud"
        authorization_endpoint = "https://cloud.example.com/apps/oauth2/authorize"
        token_endpoint = "https://cloud.example.com/apps/oauth2/api/v1/token"
        client_id = "arlen-desktop"
    "#;

    #[test]
    fn parses_providers_and_borrows_as_config() {
        let providers = parse_providers(SAMPLE).unwrap();
        assert_eq!(providers.len(), 2);
        let google = find_provider(&providers, "google").unwrap();
        let cfg = google.as_config();
        assert_eq!(cfg.token_endpoint, "https://oauth2.googleapis.com/token");
        assert_eq!(cfg.client_id, "123.apps.googleusercontent.com");
        assert!(cfg.scope.contains("calendar.readonly"));
        // A provider that omitted scope defaults to empty (no scope requested).
        assert_eq!(find_provider(&providers, "nextcloud").unwrap().scope, "");
        assert!(find_provider(&providers, "absent").is_none());
    }

    #[test]
    fn a_provider_without_a_client_id_is_refused() {
        let bad = r#"
            [[provider]]
            name = "google"
            authorization_endpoint = "https://a/x"
            token_endpoint = "https://a/t"
            client_id = ""
        "#;
        assert!(parse_providers(bad).is_err());
    }

    #[test]
    fn a_non_https_endpoint_is_refused() {
        let bad = r#"
            [[provider]]
            name = "google"
            authorization_endpoint = "http://a/x"
            token_endpoint = "https://a/t"
            client_id = "cid"
        "#;
        assert!(parse_providers(bad).is_err());
    }

    #[test]
    fn an_empty_registry_parses_to_no_providers() {
        assert!(parse_providers("").unwrap().is_empty());
    }
}
