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

/// `[bridge]` — who may talk to this bridge's host endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct BridgeMeta {
    /// The single permitted foreign plugin id (mutual id-pin, no wildcard).
    /// An inbound connection whose declared id is not this is refused.
    pub allowed_plugin_id: String,
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

    /// Fail-closed structural checks: a non-empty plugin id, and every map
    /// rule with a non-empty `upsert`/`key` and `$.<field>` `set` refs.
    pub fn validate(&self) -> Result<(), BridgeError> {
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

    const SAMPLE: &str = r#"
[bridge]
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
        let t = "[bridge]\nallowed_plugin_id = \"\"\n";
        assert_eq!(BridgeConfig::parse(t).unwrap_err(), BridgeError::EmptyPluginId);
    }

    #[test]
    fn rejects_a_non_reference_set_value() {
        let t = r#"
[bridge]
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
        let t = "[bridge]\nallowed_plugin_id = \"x\"\n[map.\"n\"]\nupsert = \"\"\nkey = \"k\"\n";
        assert!(matches!(
            BridgeConfig::parse(t),
            Err(BridgeError::InvalidRule { .. })
        ));
    }

    #[test]
    fn msg_field_ref_only_accepts_single_segment() {
        assert_eq!(msg_field_ref("$.title"), Some("title"));
        assert_eq!(msg_field_ref("$.a.b"), None);
        assert_eq!(msg_field_ref("title"), None);
        assert_eq!(msg_field_ref("$."), None);
    }
}
