//! The declarative connections grant config (connections-plan.md §2, property 1).
//!
//! A grant says an app may request a credential for a named connection up to a
//! scope ceiling: it is the standing capability the broker checks. This config is
//! the persisted declarative source the daemon reads (the daemon owns its own
//! config, like the account daemon's account configs); the first-use consent that
//! ADDS a grant interactively is the consent-system integration, and rendering a
//! grant as a KG Grant node is the capability-browser (CONN-R6). Everything fails
//! closed: a missing or unparseable file yields no grants, so the broker denies
//! every request until grants are configured, and an entry with an invalid
//! connection id is dropped rather than coerced.

use std::path::PathBuf;

use serde::Deserialize;

use crate::broker::{ConnectionGrant, ConnectionId};

/// The parsed connections config: a list of per-app grant entries.
#[derive(Debug, Default, Deserialize)]
pub struct ConnectionsConfig {
    /// Each `[[grant]]` table in the file.
    #[serde(default, rename = "grant")]
    grants: Vec<GrantEntry>,
}

/// One `[[grant]]` entry as written in the file.
#[derive(Debug, Deserialize)]
struct GrantEntry {
    /// The app the grant is for (the kernel-attested id the broker matches).
    app_id: String,
    /// The connection the app may reach.
    connection: String,
    /// The scope ceiling (a request may ask for any subset). Absent means an
    /// empty ceiling: the app may name the connection but request no scope.
    #[serde(default)]
    max_scope: Vec<String>,
    /// The per-connection egress endpoint allowlist (CONN-R3): the hosts this app
    /// may reach for this connection. Absent means no host is authorized, so no
    /// egress capability token can be minted (fail-closed).
    #[serde(default)]
    allowed_hosts: Vec<String>,
}

impl ConnectionsConfig {
    /// Parse a config from TOML text. A parse failure yields an empty config
    /// (fail-closed: the broker then denies every request).
    pub fn parse(text: &str) -> Self {
        toml::from_str(text).unwrap_or_default()
    }

    /// The validated grants: each entry whose connection id parses becomes a
    /// [`ConnectionGrant`]; an entry with an invalid connection id is dropped
    /// (never coerced into a valid-looking grant).
    pub fn grants(&self) -> Vec<ConnectionGrant> {
        self.grants
            .iter()
            .filter_map(|e| {
                ConnectionId::new(&e.connection).map(|connection_id| ConnectionGrant {
                    app_id: e.app_id.clone(),
                    connection_id,
                    max_scope: e.max_scope.clone(),
                    allowed_hosts: e.allowed_hosts.clone(),
                })
            })
            .collect()
    }
}

/// `~/.config/arlen/connections.toml`, or `None` if no config dir resolves.
pub fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("arlen").join("connections.toml"))
}

/// Load the config from [`config_path`], or an empty config (deny-all) when the
/// file is absent or unreadable. The daemon re-reads this per request so a
/// revoked grant takes effect without a restart (the account-daemon convention).
pub fn load() -> ConnectionsConfig {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| ConnectionsConfig::parse(&t))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_grants() {
        let cfg = ConnectionsConfig::parse(
            r#"
            [[grant]]
            app_id = "com.example.app"
            connection = "github"
            max_scope = ["repo", "read:user"]
            allowed_hosts = ["api.github.com"]

            [[grant]]
            app_id = "com.example.app"
            connection = "google-drive"
            "#,
        );
        let grants = cfg.grants();
        assert_eq!(grants.len(), 2);
        assert_eq!(grants[0].app_id, "com.example.app");
        assert_eq!(grants[0].connection_id.as_str(), "github");
        assert_eq!(grants[0].max_scope, vec!["repo".to_string(), "read:user".to_string()]);
        assert_eq!(grants[0].allowed_hosts, vec!["api.github.com".to_string()]);
        // A grant with no max_scope is an empty ceiling (name-only).
        assert_eq!(grants[1].connection_id.as_str(), "google-drive");
        assert!(grants[1].max_scope.is_empty());
        // Absent allowed_hosts is an empty allowlist (no egress token mintable).
        assert!(grants[1].allowed_hosts.is_empty());
    }

    #[test]
    fn an_invalid_connection_id_is_dropped() {
        // "Bad/Conn" is not a valid connection id -> the entry is dropped, not
        // coerced (fail-closed: no partially-valid grant slips through).
        let cfg = ConnectionsConfig::parse(
            r#"
            [[grant]]
            app_id = "com.example.app"
            connection = "Bad/Conn"
            max_scope = ["repo"]

            [[grant]]
            app_id = "com.example.app"
            connection = "github"
            "#,
        );
        let grants = cfg.grants();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].connection_id.as_str(), "github");
    }

    #[test]
    fn malformed_toml_is_empty_deny_all() {
        let cfg = ConnectionsConfig::parse("this is not = valid = toml [[[");
        assert!(cfg.grants().is_empty());
    }

    #[test]
    fn no_grants_section_is_empty() {
        let cfg = ConnectionsConfig::parse("# just a comment\n");
        assert!(cfg.grants().is_empty());
    }
}
