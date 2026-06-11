//! TOML handler via `toml_edit`: format-preserving by construction.
//!
//! `read` walks the parsed [`DocumentMut`] collecting every scalar leaf path in
//! document order. `set` does an INDEXED ASSIGN (`doc[a][b] = value`), not
//! `.insert()`, so the key's surrounding comment decor survives, the exact
//! technique `apps/settings/src-tauri/src/toml_writer.rs` uses (its load-bearing
//! rule: `table[key] = value` mutates in place and keeps the comment block,
//! `.insert()` replaces the entry whole and resets its decor). Insert of a new
//! key falls out of the same indexed assign.

use toml_edit::{DocumentMut, Item, Value};

use crate::error::{EditError, ParseError};
use crate::model::{ConfigModel, ConfigValue, KeyPath};
use crate::{Format, FormatHandler};

/// The TOML format handler.
#[derive(Debug, Default, Clone, Copy)]
pub struct TomlHandler;

impl FormatHandler for TomlHandler {
    fn format(&self) -> Format {
        Format::Toml
    }

    fn read(&self, text: &str) -> Result<ConfigModel, ParseError> {
        if text.len() > crate::MAX_CONFIG_BYTES {
            return Err(ParseError::TooLarge);
        }
        let doc: DocumentMut = text.parse().map_err(|e| ParseError::Malformed {
            format: "toml",
            detail: format!("{e}"),
        })?;
        let mut entries = Vec::new();
        collect_leaves(doc.as_table().iter(), "", &mut entries, 0)?;
        Ok(ConfigModel::from_entries(entries))
    }

    fn set(&self, text: &str, key: &str, value: &ConfigValue) -> Result<String, EditError> {
        let mut doc: DocumentMut = text
            .parse()
            .map_err(|e| EditError::Failed(format!("parse: {e}")))?;
        let segments: Vec<&str> = key.split('.').collect();
        if segments.iter().any(|s| s.is_empty()) {
            return Err(EditError::Failed(format!("empty key segment in {key:?}")));
        }
        let new_value = to_toml_value(value)
            .ok_or_else(|| EditError::OpaqueTarget { key: key.to_string() })?;
        assign_nested(&mut doc, &segments, new_value)
            .map_err(EditError::Failed)?;
        Ok(doc.to_string())
    }

    fn remove(&self, text: &str, key: &str) -> Result<String, EditError> {
        let mut doc: DocumentMut = text
            .parse()
            .map_err(|e| EditError::Failed(format!("parse: {e}")))?;
        let segments: Vec<&str> = key.split('.').collect();
        remove_nested(&mut doc, &segments);
        Ok(doc.to_string())
    }
}

/// Walk a TOML table's items, appending every scalar leaf as a dotted key-path.
/// A nested table recurses with the path extended; an inline table is walked the
/// same way (its keys nest under the dotted path). An array, array-of-tables, or
/// any other non-scalar leaf at a path is reported as [`ConfigValue::Opaque`] so
/// the self-check refuses to flatten it.
fn collect_leaves<'a>(
    items: impl Iterator<Item = (&'a str, &'a Item)>,
    prefix: &str,
    out: &mut Vec<(KeyPath, ConfigValue)>,
    depth: usize,
) -> Result<(), ParseError> {
    if depth > crate::MAX_DEPTH {
        return Err(ParseError::TooDeep(crate::MAX_DEPTH));
    }
    for (key, item) in items {
        let path = join_path(prefix, key);
        match item {
            Item::Value(Value::InlineTable(t)) => {
                recurse_inline(t, &path, out, depth + 1)?;
            }
            Item::Table(t) => {
                collect_leaves(t.iter(), &path, out, depth + 1)?;
            }
            Item::Value(v) => {
                out.push((path, value_of(v)));
            }
            // An array-of-tables is a structured value: report Opaque so it is
            // never treated as a scalar leaf.
            Item::ArrayOfTables(_) => out.push((path, ConfigValue::Opaque)),
            Item::None => {}
        }
    }
    Ok(())
}

/// Recurse into an inline table, collecting its scalar leaves under `prefix`.
fn recurse_inline(
    table: &toml_edit::InlineTable,
    prefix: &str,
    out: &mut Vec<(KeyPath, ConfigValue)>,
    depth: usize,
) -> Result<(), ParseError> {
    if depth > crate::MAX_DEPTH {
        return Err(ParseError::TooDeep(crate::MAX_DEPTH));
    }
    for (key, value) in table.iter() {
        let path = join_path(prefix, key);
        match value {
            Value::InlineTable(t) => {
                recurse_inline(t, &path, out, depth + 1)?;
            }
            v => out.push((path, value_of(v))),
        }
    }
    Ok(())
}

/// Join a dotted prefix with the next segment.
fn join_path(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

/// Classify a `toml_edit::Value` leaf into a [`ConfigValue`]. An array (or any
/// non-scalar) is [`ConfigValue::Opaque`].
fn value_of(v: &Value) -> ConfigValue {
    match v {
        Value::String(s) => ConfigValue::String(s.value().clone()),
        Value::Integer(i) => ConfigValue::Int(*i.value()),
        Value::Float(f) => ConfigValue::Float(*f.value()),
        Value::Boolean(b) => ConfigValue::Bool(*b.value()),
        // Datetimes, arrays and inline tables are non-scalar from the adapter's
        // point of view (its types are string/bool/int/float/enum).
        _ => ConfigValue::Opaque,
    }
}

/// Turn a [`ConfigValue`] into a `toml_edit::Value`, or `None` for
/// [`ConfigValue::Opaque`] (unsettable). A non-finite float is also rejected
/// (`None`) so a NaN/inf can never be serialized into the document.
fn to_toml_value(value: &ConfigValue) -> Option<Value> {
    Some(match value {
        ConfigValue::String(s) => Value::from(s.as_str()),
        ConfigValue::Bool(b) => Value::from(*b),
        ConfigValue::Int(i) => Value::from(*i),
        ConfigValue::Float(f) if f.is_finite() => Value::from(*f),
        ConfigValue::Float(_) => return None,
        ConfigValue::Opaque => return None,
    })
}

/// Assign `value` at the dotted path `segments`, creating intermediate tables as
/// needed. Uses indexed assign on the final segment so the key's comment decor is
/// preserved (the `toml_writer` rule). Refuses to descend through a non-table
/// intermediate (a scalar standing where a table is needed).
fn assign_nested(
    doc: &mut DocumentMut,
    segments: &[&str],
    value: Value,
) -> Result<(), String> {
    let (last, parents) = segments
        .split_last()
        .ok_or_else(|| "empty key".to_string())?;

    // Descend (creating tables) to the parent of the leaf.
    let mut current: &mut Item = doc.as_item_mut();
    for seg in parents {
        // If the segment is absent, create an implicit table; if it is a
        // non-table, refuse rather than clobber a scalar.
        let table = table_like_mut(current)
            .ok_or_else(|| format!("{seg:?} parent is not a table"))?;
        if !table.contains_key(seg) {
            table.insert_table(seg);
        }
        let next = table
            .get_item_mut(seg)
            .ok_or_else(|| format!("could not descend into {seg:?}"))?;
        if next.as_table_like().is_none() {
            return Err(format!("{seg:?} is not a table"));
        }
        current = next;
    }

    // Indexed assign on the leaf: mutate in place, keep the decor.
    let parent =
        table_like_mut(current).ok_or_else(|| "leaf parent is not a table".to_string())?;
    parent.assign_value(last, value);
    Ok(())
}

/// View an item as the [`TableLikeMut`] used by the nested assign, whether it is
/// a regular table or an inline table. Returns `None` for a scalar standing
/// where a table is needed. Borrows the item exactly once, so it composes in a
/// descent loop without the double-mutable-borrow a `map`/`or_else` chain hits.
fn table_like_mut(item: &mut Item) -> Option<&mut dyn TableLikeMut> {
    if item.is_table() {
        item.as_table_mut().map(|t| t as &mut dyn TableLikeMut)
    } else if item.is_inline_table() {
        item.as_inline_table_mut()
            .map(|t| t as &mut dyn TableLikeMut)
    } else {
        None
    }
}

/// Remove the leaf at `segments`. An absent path is a no-op (the caller treats a
/// missing key as nothing to remove).
fn remove_nested(doc: &mut DocumentMut, segments: &[&str]) {
    let Some((last, parents)) = segments.split_last() else {
        return;
    };
    let mut current: &mut Item = doc.as_item_mut();
    for seg in parents {
        // Descend through regular tables only; an inline table holds `Value`s,
        // not `Item`s, so a path running through one has no `Item` to remove
        // from and the absent path is a no-op, matching `assign_nested`.
        let Some(next) = current.as_table_mut().and_then(|t| t.get_mut(seg)) else {
            return;
        };
        current = next;
    }
    if let Some(t) = current.as_table_mut() {
        t.remove(last);
    } else if let Some(t) = current.as_inline_table_mut() {
        t.remove(last);
    }
}

/// A small trait unifying `toml_edit::Table` and `InlineTable` for the few
/// operations the nested assign needs, so the descent code is written once.
trait TableLikeMut {
    /// Whether the table has `key`.
    fn contains_key(&self, key: &str) -> bool;
    /// Insert an empty sub-table at `key` if absent.
    fn insert_table(&mut self, key: &str);
    /// Mutable item at `key`.
    fn get_item_mut(&mut self, key: &str) -> Option<&mut Item>;
    /// Indexed-assign a scalar value at `key`, preserving decor.
    fn assign_value(&mut self, key: &str, value: Value);
}

impl TableLikeMut for toml_edit::Table {
    fn contains_key(&self, key: &str) -> bool {
        self.contains_key(key)
    }
    fn insert_table(&mut self, key: &str) {
        self[key] = Item::Table(toml_edit::Table::new());
    }
    fn get_item_mut(&mut self, key: &str) -> Option<&mut Item> {
        self.get_mut(key)
    }
    fn assign_value(&mut self, key: &str, value: Value) {
        // Indexed assign keeps the existing key's leading comment decor.
        self[key] = Item::Value(value);
    }
}

impl TableLikeMut for toml_edit::InlineTable {
    fn contains_key(&self, key: &str) -> bool {
        self.contains_key(key)
    }
    fn insert_table(&mut self, key: &str) {
        self.insert(key, Value::InlineTable(toml_edit::InlineTable::new()));
    }
    fn get_item_mut(&mut self, _key: &str) -> Option<&mut Item> {
        // An inline table holds `Value`s, not `Item`s; nesting a new table
        // through an inline table is not supported (rare for config), so the
        // caller's table-descent over inline tables is not exercised. Returning
        // None makes the descent refuse rather than misbehave.
        None
    }
    fn assign_value(&mut self, key: &str, value: Value) {
        self.insert(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_collects_nested_leaf_paths() {
        let h = TomlHandler;
        let src = "\
title = \"arlen\"
[server]
host = \"localhost\"
port = 8080
ratio = 1.5
on = true
";
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
    fn set_preserves_comments() {
        let h = TomlHandler;
        let src = "\
# top
[server]
# the port
port = 8080
host = \"localhost\"
";
        let out = h.set(src, "server.port", &ConfigValue::Int(9090)).unwrap();
        assert!(out.contains("# top"));
        assert!(out.contains("# the port"));
        assert!(out.contains("port = 9090"));
        assert!(out.contains("host = \"localhost\""));
    }

    #[test]
    fn array_value_reported_opaque() {
        let h = TomlHandler;
        let m = h.read("ports = [1, 2, 3]\n").unwrap();
        assert_eq!(m.get("ports"), Some(&ConfigValue::Opaque));
    }
}
