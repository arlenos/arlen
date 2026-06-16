//! The mapping interpreter: one inbound foreign-plugin message + a validated
//! [`BridgeConfig`] -> a concrete [`UpsertPlan`] (the node to upsert and any
//! per-link edges). Pure and fail-closed: it produces only what the rule
//! authorises, never trusts the message for the type/key/edge names (those
//! come from the config), and treats the message purely as a value source.
//!
//! The plan is then written by the daemon through the app-tier entity-write
//! socket under the bridge's macaroon-scoped namespace, schema-validated and
//! audited. This module does no I/O.

use serde_json::{Map, Value};
use thiserror::Error;

use crate::bridge::{msg_field_ref, BridgeConfig};

/// A resolved upsert: the typed node plus the edges to create from it.
#[derive(Debug, Clone, PartialEq)]
pub struct UpsertPlan {
    /// The qualified entity type (from the rule, never the message).
    pub qualified_type: String,
    /// The stable external key resolved from the message's key field.
    pub external_key: String,
    /// The projected entity fields (only the message fields the rule's `set`
    /// referenced and that were present).
    pub fields: Map<String, Value>,
    /// Edges to create from this node (from `for_each_link`).
    pub links: Vec<LinkPlan>,
}

/// One edge the plan creates: `(this node) -[edge]-> (node keyed to_key)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPlan {
    /// The relation type (from the rule).
    pub edge: String,
    /// The source node's external key (this upsert's key).
    pub from_key: String,
    /// The target node's external key (resolved per `links` element).
    pub to_key: String,
}

/// An interpretation failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InterpretError {
    /// The message type has no rule in this bridge's map.
    #[error("no mapping rule for message type {0:?}")]
    UnknownType(String),
    /// The message's key field was absent, not a string, or empty.
    #[error("message is missing a usable key field {0:?}")]
    MissingKey(String),
}

/// Interpret one inbound message (`msg_type` + its flat object `msg`) against
/// the bridge config into an [`UpsertPlan`]. Fail-closed: an unmapped type or
/// a missing/blank key is refused; `set` fields absent from the message are
/// skipped (a partial sync sets only what it carries).
pub fn interpret_message(
    config: &BridgeConfig,
    msg_type: &str,
    msg: &Map<String, Value>,
) -> Result<UpsertPlan, InterpretError> {
    let rule = config
        .map
        .get(msg_type)
        .ok_or_else(|| InterpretError::UnknownType(msg_type.to_string()))?;

    // The external key: the message's key field, which must be a non-empty
    // string. The idempotency anchor, so it cannot be absent or blank.
    let external_key = msg
        .get(&rule.key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|k| !k.is_empty())
        .ok_or_else(|| InterpretError::MissingKey(rule.key.clone()))?;

    // Field projection: resolve each `$.<field>` ref against the message,
    // keeping only present fields.
    let mut fields = Map::new();
    for (target, reference) in &rule.set {
        let Some(src) = msg_field_ref(reference) else {
            continue; // validated at parse, defensive here
        };
        if let Some(value) = msg.get(src) {
            fields.insert(target.clone(), value.clone());
        }
    }

    // Edges: for each element of the message's `links` array, a target key is
    // either the element (a bare string) or its `to_key` field.
    let mut links = Vec::new();
    if let Some(link_rule) = &rule.for_each_link {
        if let Some(Value::Array(elements)) = msg.get("links") {
            for element in elements {
                let to_key = match element {
                    Value::String(s) => Some(s.clone()),
                    Value::Object(o) => {
                        o.get(&link_rule.to_key).and_then(Value::as_str).map(str::to_string)
                    }
                    _ => None,
                };
                if let Some(to_key) = to_key.filter(|k| !k.is_empty()) {
                    links.push(LinkPlan {
                        edge: link_rule.edge.clone(),
                        from_key: external_key.clone(),
                        to_key,
                    });
                }
            }
        }
    }

    Ok(UpsertPlan {
        qualified_type: rule.upsert.clone(),
        external_key,
        fields,
        links,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config() -> BridgeConfig {
        BridgeConfig::parse(
            r#"
[bridge]
allowed_plugin_id = "maria-obsidian-bridge"
[map."note.upsert"]
upsert = "md.obsidian.Note"
key    = "path"
set    = { title = "$.title", tags = "$.tags" }
for_each_link = { edge = "LINKS_TO", to_key = "path" }
"#,
        )
        .unwrap()
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn maps_a_note_upsert_with_fields_and_links() {
        let msg = obj(json!({
            "path": "notes/a.md",
            "title": "A",
            "tags": ["x", "y"],
            "links": [{ "path": "notes/b.md" }, "notes/c.md"]
        }));
        let plan = interpret_message(&config(), "note.upsert", &msg).unwrap();
        assert_eq!(plan.qualified_type, "md.obsidian.Note");
        assert_eq!(plan.external_key, "notes/a.md");
        assert_eq!(plan.fields.get("title").unwrap(), &json!("A"));
        assert_eq!(plan.fields.get("tags").unwrap(), &json!(["x", "y"]));
        assert_eq!(
            plan.links,
            vec![
                LinkPlan {
                    edge: "LINKS_TO".into(),
                    from_key: "notes/a.md".into(),
                    to_key: "notes/b.md".into(),
                },
                LinkPlan {
                    edge: "LINKS_TO".into(),
                    from_key: "notes/a.md".into(),
                    to_key: "notes/c.md".into(),
                },
            ]
        );
    }

    #[test]
    fn unknown_type_is_refused() {
        let msg = obj(json!({ "path": "x" }));
        assert_eq!(
            interpret_message(&config(), "note.delete", &msg),
            Err(InterpretError::UnknownType("note.delete".into()))
        );
    }

    #[test]
    fn missing_or_blank_key_is_refused() {
        assert_eq!(
            interpret_message(&config(), "note.upsert", &obj(json!({ "title": "A" }))),
            Err(InterpretError::MissingKey("path".into()))
        );
        assert_eq!(
            interpret_message(&config(), "note.upsert", &obj(json!({ "path": "" }))),
            Err(InterpretError::MissingKey("path".into()))
        );
    }

    #[test]
    fn absent_set_fields_are_skipped_not_invented() {
        // Only `path` + `title` present: `tags` is simply not set.
        let msg = obj(json!({ "path": "notes/a.md", "title": "A" }));
        let plan = interpret_message(&config(), "note.upsert", &msg).unwrap();
        assert!(plan.fields.contains_key("title"));
        assert!(!plan.fields.contains_key("tags"));
        assert!(plan.links.is_empty());
    }

    #[test]
    fn the_message_cannot_override_the_type_or_edge() {
        // A hostile message carrying its own `upsert`/`edge` fields is ignored:
        // the type + edge come from the config, never the message.
        let msg = obj(json!({
            "path": "n", "upsert": "system.Secret", "edge": "OWNS",
            "links": [{ "path": "t" }]
        }));
        let plan = interpret_message(&config(), "note.upsert", &msg).unwrap();
        assert_eq!(plan.qualified_type, "md.obsidian.Note");
        assert_eq!(plan.links[0].edge, "LINKS_TO");
    }
}
