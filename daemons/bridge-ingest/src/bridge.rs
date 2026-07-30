//! The declarative `bridge.toml` schema: the per-bridge mapping a foreign
//! plugin's messages are interpreted against (foreign-app-bridges.md §1).
//!
//! A bridge ships no code; this file is data. It names the single permitted
//! foreign plugin id (mutual id-pin) and, per inbound message type, which
//! entity to upsert, the stable key field, the field projection, and any
//! per-link edge. The interpreter ([`crate::interpret`]) turns one inbound
//! message + this config into a concrete upsert plan; the daemon validates it
//! against the registered `entities.toml`, writes it idempotently under the
//! bridge's macaroon-scoped namespace, and audits it.

use std::collections::BTreeMap;

use serde::Deserialize;
use thiserror::Error;

/// A parsed, validated `bridge.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct BridgeConfig {
    /// `[bridge]` — the bridge's identity + access control.
    pub bridge: BridgeMeta,
    /// `[map."<message.type>"]` — one rule per inbound message type.
    #[serde(default)]
    pub map: BTreeMap<String, MapRule>,
}

/// `[bridge]` — who this bridge IS, what it may write, and who may talk to it.
#[derive(Debug, Clone, Deserialize)]
pub struct BridgeMeta {
    /// The bridge's identity, which is also the namespace it writes under
    /// (`md.obsidian`). ONE field rather than a separate id and namespace,
    /// because two could disagree and the disagreement would have to be
    /// resolved somewhere - and whichever won, the other would be a lie the
    /// config kept stating.
    ///
    /// Every installed bridge runs the same binary, so this is what
    /// distinguishes them. It derives the per-bridge app id ([`app_id`]) that
    /// a per-bridge profile is keyed under, replacing the single shared
    /// `bridge-ingest` identity every bridge writes as today.
    pub id: String,
    /// The single permitted foreign plugin id (mutual id-pin, no wildcard).
    /// An inbound connection whose declared id is not this is refused.
    pub allowed_plugin_id: String,
}

impl BridgeMeta {
    /// The per-bridge app id: `bridge.<id>`.
    ///
    /// Prefixed rather than bare so a bridge can never collide with a
    /// first-party app id - a bridge declaring `id = "settings"` would
    /// otherwise name the Settings app and inherit whatever is keyed to it.
    pub fn app_id(&self) -> String {
        format!("bridge.{}", self.id)
    }
}

/// One `[map."<type>"]` rule: how an inbound message of that type becomes a
/// node upsert (+ optional per-link edges).
#[derive(Debug, Clone, Deserialize)]
pub struct MapRule {
    /// The canonical qualified entity type to upsert (e.g.
    /// `md.obsidian.Note`). Must be in the bridge's own declared namespace;
    /// the daemon's write path refuses `system.*`/`shared.*`.
    pub upsert: String,
    /// The message field whose value is the stable external key (the
    /// idempotency key, so a re-sync strengthens rather than duplicates).
    pub key: String,
    /// Field projection: target entity field -> a `$.<msgfield>` reference
    /// into the inbound message. Absent message fields are skipped.
    #[serde(default)]
    pub set: BTreeMap<String, String>,
    /// Optional: for each element of the message's `links` array, create an
    /// edge of this type to the node keyed by the element's `to_key`.
    #[serde(default)]
    pub for_each_link: Option<LinkRule>,
}

/// `for_each_link = { edge = "LINKS_TO", to_key = "path" }`.
#[derive(Debug, Clone, Deserialize)]
pub struct LinkRule {
    /// The edge (relation) type to create.
    pub edge: String,
    /// On each `links` element, the field holding the target node's external
    /// key (or, if the element is a bare string, that string is the key).
    pub to_key: String,
}

/// A `bridge.toml` validation or parse failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BridgeError {
    /// The TOML did not parse.
    #[error("bridge.toml parse: {0}")]
    Parse(String),
    /// `allowed_plugin_id` was empty.
    #[error("bridge.toml: allowed_plugin_id must be non-empty")]
    EmptyPluginId,
    /// `id` was empty, malformed, or named a reserved namespace.
    #[error("bridge.toml: id {id:?} is not a valid bridge namespace: {why}")]
    InvalidId {
        /// The offending id.
        id: String,
        /// What was wrong.
        why: String,
    },
    /// A map rule wrote outside the bridge's own declared namespace.
    #[error("bridge.toml: map rule {rule:?} writes {upsert:?}, outside this bridge's {id:?}")]
    ForeignNamespace {
        /// The offending message-type key.
        rule: String,
        /// The type it tried to write.
        upsert: String,
        /// The bridge's own namespace.
        id: String,
    },
    /// A map rule was malformed (empty `upsert`, `key`, or a `set` value
    /// that is not a `$.<field>` reference).
    #[error("bridge.toml: map rule {rule:?} is invalid: {why}")]
    InvalidRule {
        /// The offending message-type key.
        rule: String,
        /// What was wrong.
        why: String,
    },
}

impl BridgeConfig {
    /// Parse + validate a `bridge.toml` from its text.
    pub fn parse(text: &str) -> Result<Self, BridgeError> {
        let config: BridgeConfig =
            toml::from_str(text).map_err(|e| BridgeError::Parse(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// Fail-closed structural checks: a valid non-reserved bridge `id`, a
    /// non-empty plugin id, and every map rule writing under the bridge's OWN
    /// namespace with a non-empty `key` and `$.<field>` `set` refs.
    ///
    /// The namespace containment is the point rather than a tidiness check.
    /// Every bridge runs one binary and so writes under one shared identity
    /// today, which means the daemon-side grant it is checked against is the
    /// union of EVERY installed bridge's namespace - install a second bridge
    /// and the first one's config could name the second one's types. Binding a
    /// bridge to its own declared namespace here means a bridge cannot even ask.
    ///
    /// This is a fail-fast at load, not the authority. The daemon's grant check
    /// (`write::permits_any`, which refuses `system.*`/`shared.*` structurally)
    /// remains the enforcement point, because it is the side that cannot be
    /// edited by whoever wrote the config.
    pub fn validate(&self) -> Result<(), BridgeError> {
        let id = self.bridge.id.trim();
        if let Some(why) = invalid_namespace(id) {
            return Err(BridgeError::InvalidId {
                id: self.bridge.id.clone(),
                why: why.to_string(),
            });
        }
        if self.bridge.allowed_plugin_id.trim().is_empty() {
            return Err(BridgeError::EmptyPluginId);
        }
        for (msg_type, rule) in &self.map {
            if rule.upsert.trim().is_empty() {
                return Err(BridgeError::InvalidRule {
                    rule: msg_type.clone(),
                    why: "empty upsert type".to_string(),
                });
            }
            if rule.key.trim().is_empty() {
                return Err(BridgeError::InvalidRule {
                    rule: msg_type.clone(),
                    why: "empty key field".to_string(),
                });
            }
            if !is_under(&rule.upsert, id) {
                return Err(BridgeError::ForeignNamespace {
                    rule: msg_type.clone(),
                    upsert: rule.upsert.clone(),
                    id: id.to_string(),
                });
            }
            for (field, reference) in &rule.set {
                if msg_field_ref(reference).is_none() {
                    return Err(BridgeError::InvalidRule {
                        rule: msg_type.clone(),
                        why: format!("set.{field} = {reference:?} is not a $.<field> reference"),
                    });
                }
            }
        }
        Ok(())
    }
}

/// The reserved namespaces no bridge may claim as its identity. A bridge is
/// third-party data by definition, so a `system`- or `shared`-named bridge would
/// be asserting authority over first-party facts. The daemon refuses these too
/// (`write::namespace_grant`); refusing at load means a misconfigured bridge
/// fails at startup with a clear reason rather than on its first write.
const RESERVED: &[&str] = &["system", "shared"];

/// Why `id` is not a usable bridge namespace, or `None` if it is fine.
///
/// Lowercase reverse-DNS: `[a-z0-9-]` segments joined by `.`, no empty segment.
/// Uppercase is excluded because an entity type is `{namespace}.{Type}` and the
/// capitalised final segment is what distinguishes the type from its namespace.
fn invalid_namespace(id: &str) -> Option<&'static str> {
    if id.is_empty() {
        return Some("empty");
    }
    if id.split('.').next().is_some_and(|first| RESERVED.contains(&first)) {
        return Some("system and shared are reserved for first-party facts");
    }
    for segment in id.split('.') {
        if segment.is_empty() {
            return Some("empty segment");
        }
        if !segment
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Some("segments must be lowercase [a-z0-9-]");
        }
    }
    None
}

/// Whether `ty` lies STRICTLY under namespace `ns`: `md.obsidian.Note` is under
/// `md.obsidian`, but `md.obsidian` itself is not (a namespace is not a type),
/// and neither is the sibling `md.obsidianvault.Note` - the check is on the
/// dotted boundary, so a prefix match alone cannot smuggle a neighbouring
/// namespace in.
fn is_under(ty: &str, ns: &str) -> bool {
    ty.strip_prefix(ns)
        .and_then(|rest| rest.strip_prefix('.'))
        .is_some_and(|tail| !tail.is_empty())
}

/// Resolve a `$.<field>` reference to its `<field>` name, or `None` if it is
/// not a well-formed single-segment message reference. Only top-level message
/// fields are addressable (no nested paths); the inbound message is a flat
/// object per the native-messaging contract.
pub fn msg_field_ref(reference: &str) -> Option<&str> {
    let field = reference.strip_prefix("$.")?;
    if field.is_empty() || field.contains('.') || field.contains(' ') {
        return None;
    }
    Some(field)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The namespace charset, in both directions. Mutation testing found the
    /// `||` chain could be narrowed so that only lowercase letters pass, which
    /// refuses a perfectly ordinary id - `md.obsidian2`, `com.slack-app` - and
    /// nothing caught it, because no test used a digit or a hyphen. Over-
    /// rejection is the safe direction, but a bridge that cannot declare its own
    /// name is still broken, and the charset is documented as `[a-z0-9-]`.
    #[test]
    fn the_namespace_charset_admits_digits_and_hyphens_and_nothing_else() {
        for ok in ["md.obsidian", "md.obsidian2", "com.slack-app", "a.b-2.c3"] {
            assert!(
                invalid_namespace(ok).is_none(),
                "{ok} is within the documented [a-z0-9-] charset"
            );
        }
        for bad in ["md.Obsidian", "md.obsidian_vault", "md.obsidian!", "md..obsidian", "md.obsidian "] {
            assert!(invalid_namespace(bad).is_some(), "{bad} must be refused");
        }
        // And the reserved roots, which are a different refusal entirely.
        for reserved in ["system.Thing", "shared.Person"] {
            assert!(invalid_namespace(reserved).is_some(), "{reserved} is reserved");
        }
    }

    const SAMPLE: &str = r#"
[bridge]
id = "md.obsidian"
allowed_plugin_id = "maria-obsidian-bridge"
[map."note.upsert"]
upsert = "md.obsidian.Note"
key    = "path"
set    = { title = "$.title", tags = "$.tags" }
for_each_link = { edge = "LINKS_TO", to_key = "path" }
"#;

    #[test]
    fn parses_the_sample_bridge() {
        let c = BridgeConfig::parse(SAMPLE).unwrap();
        assert_eq!(c.bridge.id, "md.obsidian");
        assert_eq!(c.bridge.app_id(), "bridge.md.obsidian");
        assert_eq!(c.bridge.allowed_plugin_id, "maria-obsidian-bridge");
        let rule = c.map.get("note.upsert").unwrap();
        assert_eq!(rule.upsert, "md.obsidian.Note");
        assert_eq!(rule.key, "path");
        assert_eq!(rule.set.get("title").unwrap(), "$.title");
        let link = rule.for_each_link.as_ref().unwrap();
        assert_eq!(link.edge, "LINKS_TO");
        assert_eq!(link.to_key, "path");
    }

    #[test]
    fn rejects_empty_plugin_id() {
        let t = "[bridge]\nid = \"md.obsidian\"\nallowed_plugin_id = \"\"\n";
        assert_eq!(BridgeConfig::parse(t).unwrap_err(), BridgeError::EmptyPluginId);
    }

    #[test]
    fn rejects_a_non_reference_set_value() {
        let t = r#"
[bridge]
id = "a.b"
allowed_plugin_id = "x"
[map."n"]
upsert = "a.b.C"
key = "k"
set = { title = "literal-not-a-ref" }
"#;
        assert!(matches!(
            BridgeConfig::parse(t),
            Err(BridgeError::InvalidRule { .. })
        ));
    }

    #[test]
    fn rejects_empty_upsert_or_key() {
        let t =
            "[bridge]\nid = \"a.b\"\nallowed_plugin_id = \"x\"\n[map.\"n\"]\nupsert = \"\"\nkey = \"k\"\n";
        assert!(matches!(
            BridgeConfig::parse(t),
            Err(BridgeError::InvalidRule { .. })
        ));
    }

    fn config_with(id: &str, upsert: &str) -> Result<BridgeConfig, BridgeError> {
        BridgeConfig::parse(&format!(
            "[bridge]\nid = {id:?}\nallowed_plugin_id = \"x\"\n[map.\"n\"]\nupsert = {upsert:?}\nkey = \"k\"\n"
        ))
    }

    /// The containment that makes a per-bridge identity worth having: with one
    /// shared identity the daemon-side grant is the union of every installed
    /// bridge's namespace, so a second installed bridge silently widens what the
    /// first one's config may name.
    #[test]
    fn a_bridge_cannot_map_another_bridges_namespace() {
        assert!(config_with("md.obsidian", "md.obsidian.Note").is_ok());
        assert!(matches!(
            config_with("md.obsidian", "com.slack.Message"),
            Err(BridgeError::ForeignNamespace { .. })
        ));
    }

    /// A prefix match alone would admit a neighbouring namespace, and the
    /// namespace itself is not a type.
    #[test]
    fn namespace_containment_is_on_the_dotted_boundary() {
        assert!(matches!(
            config_with("md.obsidian", "md.obsidianvault.Note"),
            Err(BridgeError::ForeignNamespace { .. })
        ));
        assert!(matches!(
            config_with("md.obsidian", "md.obsidian"),
            Err(BridgeError::ForeignNamespace { .. })
        ));
        // A deeper type is still this bridge's own.
        assert!(config_with("md.obsidian", "md.obsidian.vault.Note").is_ok());
    }

    /// A bridge is third-party data, so it can never name itself a first-party
    /// namespace. The daemon refuses these too; this fails at load instead.
    #[test]
    fn a_bridge_cannot_claim_a_reserved_namespace() {
        for id in ["system", "shared", "system.core"] {
            assert!(
                matches!(config_with(id, "x.Y"), Err(BridgeError::InvalidId { .. })),
                "reserved id {id} was accepted"
            );
        }
    }

    #[test]
    fn a_malformed_id_is_refused() {
        for id in ["", "md..obsidian", "MD.Obsidian", "md obsidian", "md.obsidian."] {
            assert!(
                matches!(config_with(id, "x.Y"), Err(BridgeError::InvalidId { .. })),
                "malformed id {id:?} was accepted"
            );
        }
    }

    /// The prefix keeps a bridge from naming a first-party app id and
    /// inheriting whatever is keyed under it.
    #[test]
    fn the_app_id_is_namespaced_away_from_first_party_ids() {
        let c = config_with("settings", "settings.Thing").unwrap();
        assert_eq!(c.bridge.app_id(), "bridge.settings");
    }

    #[test]
    fn msg_field_ref_only_accepts_single_segment() {
        assert_eq!(msg_field_ref("$.title"), Some("title"));
        assert_eq!(msg_field_ref("$.a.b"), None);
        assert_eq!(msg_field_ref("title"), None);
        assert_eq!(msg_field_ref("$."), None);
    }
}
