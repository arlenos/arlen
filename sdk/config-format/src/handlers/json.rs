//! JSON / JSONC handler via `jsonc-parser`'s comment-preserving CST.
//!
//! Plain `serde_json` is disqualified for the edit path: it is lossy on
//! round-trip (drops comments, reorders, reformats). `jsonc-parser` parses to a
//! CST that owns trivia (comments + whitespace) and supports minimal in-place
//! value replacement and member insertion, so a `set` replaces only the target
//! value node and an insert appends one member to the containing object, leaving
//! every comment and the key ordering intact. JSONC `//` and `/* */` comments
//! are preserved the same way.
//!
//! `read` walks the CST collecting scalar leaf paths in document order. The
//! key-path nests through objects (`a.b.c` -> `obj["a"]["b"]["c"]`); a scalar
//! standing where an object is needed, or an array on the path, is reported as
//! [`ConfigValue::Opaque`] / refused so it is never flattened.

use jsonc_parser::cst::{CstInputValue, CstNode, CstObject, CstRootNode};
use jsonc_parser::ParseOptions;

use crate::error::{EditError, ParseError};
use crate::model::{ConfigModel, ConfigValue, KeyPath};
use crate::{Format, FormatHandler};

/// The JSON / JSONC format handler.
#[derive(Debug, Default, Clone, Copy)]
pub struct JsonHandler;

/// Parse options: JSONC-friendly (comments, trailing commas) so a real-world
/// `.json`/`.jsonc` config parses, while still rejecting genuinely malformed
/// input.
fn options() -> ParseOptions {
    ParseOptions {
        allow_comments: true,
        allow_loose_object_property_names: true,
        allow_trailing_commas: true,
        ..Default::default()
    }
}

impl FormatHandler for JsonHandler {
    fn format(&self) -> Format {
        Format::Json
    }

    fn read(&self, text: &str) -> Result<ConfigModel, ParseError> {
        if text.len() > crate::MAX_CONFIG_BYTES {
            return Err(ParseError::TooLarge);
        }
        let root = parse_root(text)?;
        let mut entries = Vec::new();
        if let Some(value) = root.value() {
            collect(&value, &mut String::new(), &mut entries, 0)?;
        }
        Ok(ConfigModel::from_entries(entries))
    }

    fn set(&self, text: &str, key: &str, value: &ConfigValue) -> Result<String, EditError> {
        let root = parse_root(text).map_err(parse_to_edit)?;
        let segments: Vec<&str> = key.split('.').collect();
        if segments.iter().any(|s| s.is_empty()) {
            return Err(EditError::Failed(format!("empty key segment in {key:?}")));
        }
        let input = to_input_value(value)
            .ok_or_else(|| EditError::OpaqueTarget { key: key.to_string() })?;

        // The document root must be an object to hold a keyed setting.
        let obj = root
            .object_value_or_create()
            .ok_or_else(|| EditError::Failed("document root is not an object".to_string()))?;
        set_in_object(&obj, &segments, input).map_err(EditError::Failed)?;
        Ok(root.to_string())
    }

    fn remove(&self, text: &str, key: &str) -> Result<String, EditError> {
        let root = parse_root(text).map_err(parse_to_edit)?;
        let segments: Vec<&str> = key.split('.').collect();
        let Some(obj) = root.object_value() else {
            // No object root: nothing to remove.
            return Ok(root.to_string());
        };
        remove_in_object(&obj, &segments);
        Ok(root.to_string())
    }
}

/// Parse the text to a CST root, mapping a syntax error to [`ParseError`].
fn parse_root(text: &str) -> Result<CstRootNode, ParseError> {
    CstRootNode::parse(text, &options()).map_err(|e| ParseError::Malformed {
        format: "json",
        detail: format!("{e}"),
    })
}

/// Surface a parse failure during an edit as [`EditError::Failed`].
fn parse_to_edit(e: ParseError) -> EditError {
    EditError::Failed(format!("parse: {e}"))
}

/// Walk a CST value node, appending scalar leaves under `prefix` in document
/// order. An object recurses (depth-bounded); an array is reported as
/// [`ConfigValue::Opaque`] at its own path (a structured value, never flattened).
fn collect(
    node: &CstNode,
    prefix: &mut String,
    out: &mut Vec<(KeyPath, ConfigValue)>,
    depth: usize,
) -> Result<(), ParseError> {
    if depth > crate::MAX_DEPTH {
        return Err(ParseError::TooDeep(crate::MAX_DEPTH));
    }
    if let Some(obj) = node.as_object() {
        for prop in obj.properties() {
            let Some(name) = prop.name().and_then(|n| n.decoded_value().ok()) else {
                continue;
            };
            let Some(value) = prop.value() else { continue };
            let path = join_path(prefix, &name);
            if value.as_object().is_some() {
                let mut p = path;
                collect(&value, &mut p, out, depth + 1)?;
            } else {
                out.push((path, classify(&value)));
            }
        }
        return Ok(());
    }
    Ok(())
}

/// Classify a non-object CST value node into a scalar [`ConfigValue`], or
/// [`ConfigValue::Opaque`] for an array or anything non-scalar.
fn classify(node: &CstNode) -> ConfigValue {
    if let Some(s) = node.as_string_lit() {
        return match s.decoded_value() {
            Ok(v) => ConfigValue::String(v),
            Err(_) => ConfigValue::Opaque,
        };
    }
    if let Some(b) = node.as_boolean_lit() {
        return ConfigValue::Bool(b.value());
    }
    if node.as_null_keyword().is_some() {
        // JSON null maps to an empty string is wrong; carry it as Opaque so the
        // adapter (whose scalar types do not include null) treats it as
        // non-scalar rather than coercing.
        return ConfigValue::Opaque;
    }
    if let Some(n) = node.as_number_lit() {
        let raw = n.value();
        if let Ok(i) = raw.parse::<i64>() {
            return ConfigValue::Int(i);
        }
        if let Ok(f) = raw.parse::<f64>() {
            if f.is_finite() {
                return ConfigValue::Float(f);
            }
        }
        return ConfigValue::Opaque;
    }
    // Arrays and any other node: non-scalar.
    ConfigValue::Opaque
}

/// Join a dotted prefix with the next segment.
fn join_path(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

/// Convert a [`ConfigValue`] to a CST input value, or `None` for an unsettable
/// value (Opaque, or a non-finite float).
fn to_input_value(value: &ConfigValue) -> Option<CstInputValue> {
    Some(match value {
        ConfigValue::String(s) => CstInputValue::String(s.clone()),
        ConfigValue::Bool(b) => CstInputValue::Bool(*b),
        ConfigValue::Int(i) => CstInputValue::Number(i.to_string()),
        ConfigValue::Float(f) if f.is_finite() => CstInputValue::Number(format!("{f}")),
        ConfigValue::Float(_) => return None,
        ConfigValue::Opaque => return None,
    })
}

/// Set `value` at the dotted path `segments` inside `obj`, descending (and
/// creating) intermediate objects, then replacing or appending the leaf member.
/// Refuses to descend through a non-object intermediate.
fn set_in_object(obj: &CstObject, segments: &[&str], value: CstInputValue) -> Result<(), String> {
    let (last, parents) = segments
        .split_last()
        .ok_or_else(|| "empty key".to_string())?;

    let mut current = obj.clone();
    for seg in parents {
        // `object_value_or_create` makes the sub-object if absent; if the member
        // exists but is a non-object scalar, it returns None, so we refuse rather
        // than clobber.
        current = current
            .object_value_or_create(seg)
            .ok_or_else(|| format!("{seg:?} is not an object"))?;
    }

    // Replace the existing member's value, or append a new member minimally.
    match current.get(last) {
        Some(prop) => prop.set_value(value),
        None => {
            current.append(last, value);
        }
    }
    Ok(())
}

/// Remove the leaf member at the dotted path. An absent path is a no-op.
fn remove_in_object(obj: &CstObject, segments: &[&str]) {
    let Some((last, parents)) = segments.split_last() else {
        return;
    };
    let mut current = obj.clone();
    for seg in parents {
        match current.object_value(seg) {
            Some(child) => current = child,
            None => return,
        }
    }
    if let Some(prop) = current.get(last) {
        prop.remove();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_collects_nested_leaves() {
        let h = JsonHandler;
        let src = r#"{
  "title": "arlen",
  "server": {
    "host": "localhost",
    "port": 8080,
    "ratio": 1.5,
    "on": true
  }
}"#;
        let m = h.read(src).unwrap();
        assert_eq!(
            m.get("title"),
            Some(&ConfigValue::String("arlen".to_string()))
        );
        assert_eq!(
            m.get("server.host"),
            Some(&ConfigValue::String("localhost".to_string()))
        );
        assert_eq!(m.get("server.port"), Some(&ConfigValue::Int(8080)));
        assert_eq!(m.get("server.ratio"), Some(&ConfigValue::Float(1.5)));
        assert_eq!(m.get("server.on"), Some(&ConfigValue::Bool(true)));
    }

    #[test]
    fn set_preserves_jsonc_comments() {
        let h = JsonHandler;
        let src = r#"{
  // the homepage
  "homepage": "https://old",
  "page": 1
}"#;
        let out = h
            .set(
                src,
                "homepage",
                &ConfigValue::String("https://new".to_string()),
            )
            .unwrap();
        assert!(out.contains("// the homepage"), "comment kept; got:\n{out}");
        assert!(out.contains("https://new"));
        assert!(!out.contains("https://old"));
        assert!(out.contains("\"page\": 1"));
    }

    #[test]
    fn array_reported_opaque() {
        let h = JsonHandler;
        let m = h.read(r#"{"ports": [1, 2, 3]}"#).unwrap();
        assert_eq!(m.get("ports"), Some(&ConfigValue::Opaque));
    }
}
