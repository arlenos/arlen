//! Format-preserving dotted-key writes on a `toml_edit::DocumentMut`, plus the
//! serde_json -> toml_edit conversion the settings config-set path uses. Split out
//! of the settings host (`commands/config.rs`) so this write logic - the layer that
//! must preserve user comments and must not silently drop array-of-object configs
//! like `layout.window_rules` - is unit-tested in CI (the src-tauri host is not).
//! Pairs with [`crate::toml_writer`], which owns the atomic file write these
//! helpers feed.
/// Walk a dot-notation path inside a `toml_edit::DocumentMut`,
/// creating intermediate tables as needed, and assign the final
/// value with `Index`-style assign so the existing key's leading
/// comment decor (if any) is preserved.
///
/// Only `IndexMut::index_mut` (i.e. `table[key] = value`) keeps the
/// per-key decor intact; `Table::insert` resets it. That's why the
/// final write goes through `as_table_mut()` (concrete `Table`) and
/// not the `TableLike` trait — only `Table` impls `IndexMut<&str>`.
pub fn set_dotted_in_doc(
    doc: &mut toml_edit::DocumentMut,
    key: &str,
    value: toml_edit::Item,
) -> Result<(), String> {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        return Err("empty key".into());
    }

    // Single-level: doc[k] = value, decor preserved by IndexMut.
    if parts.len() == 1 {
        doc[parts[0]] = value;
        return Ok(());
    }

    // First level: ensure top-level table exists.
    let first = parts[0];
    if doc.get(first).is_none() {
        doc[first] = toml_edit::Item::Table(toml_edit::Table::new());
    }

    // Walk through middle parts (everything between first and last).
    // Each step needs a fresh `&mut Table` that we then index into.
    let mut cur_table: &mut toml_edit::Table = doc[first]
        .as_table_mut()
        .ok_or_else(|| format!("path component '{first}' is not a table"))?;

    for part in &parts[1..parts.len() - 1] {
        if cur_table.get(part).is_none() {
            cur_table[part] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        cur_table = cur_table[part]
            .as_table_mut()
            .ok_or_else(|| format!("path component '{part}' is not a table"))?;
    }

    // Final assign — IndexMut keeps the existing key's comment decor.
    let last = parts[parts.len() - 1];
    cur_table[last] = value;
    Ok(())
}

/// Remove a dot-notation key. No-op if any path component is missing.
pub fn remove_dotted_in_doc(doc: &mut toml_edit::DocumentMut, key: &str) {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        return;
    }
    if parts.len() == 1 {
        doc.remove(parts[0]);
        return;
    }

    let first = parts[0];
    let Some(item) = doc.get_mut(first) else {
        return;
    };
    let Some(mut cur_table) = item.as_table_mut() else {
        return;
    };

    for part in &parts[1..parts.len() - 1] {
        let Some(next_item) = cur_table.get_mut(part) else {
            return;
        };
        let Some(next_table) = next_item.as_table_mut() else {
            return;
        };
        cur_table = next_table;
    }
    cur_table.remove(parts[parts.len() - 1]);
}

/// Convert serde_json::Value to a toml_edit::Item. Mirrors the
/// existing `json_to_toml` but produces toml_edit shapes so the
/// format-preserving writer can consume it.
///
/// Arrays of objects are critical for `layout.window_rules` and
/// other list-of-records configs. We use `json_to_toml_edit_value`
/// for array elements so each element becomes a Value (scalar OR
/// InlineTable) — a previous version called the same function as
/// the outer dispatcher and dropped object elements that came
/// back as `Item::Table` because `Item::Table.as_value()` is
/// always `None`. That silently truncated `window_rules` arrays
/// to empty on save, a data-loss bug found during Sprint B.
pub fn json_to_toml_edit(v: serde_json::Value) -> toml_edit::Item {
    use serde_json::Value as J;
    match v {
        J::Null => toml_edit::value(""),
        J::Bool(b) => toml_edit::value(b),
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml_edit::value(i)
            } else if let Some(f) = n.as_f64() {
                toml_edit::value(f)
            } else {
                toml_edit::value(n.to_string())
            }
        }
        J::String(s) => toml_edit::value(s),
        J::Array(arr) => {
            let mut a = toml_edit::Array::new();
            for item in arr {
                a.push(json_to_toml_edit_value(item));
            }
            toml_edit::value(a)
        }
        J::Object(obj) => {
            let mut t = toml_edit::Table::new();
            for (k, val) in obj {
                t.insert(&k, json_to_toml_edit(val));
            }
            toml_edit::Item::Table(t)
        }
    }
}

/// Variant that always returns a `Value` so it can live inside an
/// `Array`. Object → `InlineTable`; nested arrays / scalars same as
/// the Item variant.
pub fn json_to_toml_edit_value(v: serde_json::Value) -> toml_edit::Value {
    use serde_json::Value as J;
    match v {
        J::Null => toml_edit::Value::from(""),
        J::Bool(b) => toml_edit::Value::from(b),
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml_edit::Value::from(i)
            } else if let Some(f) = n.as_f64() {
                toml_edit::Value::from(f)
            } else {
                toml_edit::Value::from(n.to_string())
            }
        }
        J::String(s) => toml_edit::Value::from(s),
        J::Array(arr) => {
            let mut a = toml_edit::Array::new();
            for item in arr {
                a.push(json_to_toml_edit_value(item));
            }
            toml_edit::Value::Array(a)
        }
        J::Object(obj) => {
            let mut t = toml_edit::InlineTable::new();
            for (k, val) in obj {
                t.insert(&k, json_to_toml_edit_value(val));
            }
            toml_edit::Value::InlineTable(t)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    // ── format-preserving dotted-set on toml_edit::DocumentMut ──────────

    /// Sprint A migrated `config_set` from `toml::to_string_pretty`
    /// (which loses user comments) to toml_edit. This test verifies
    /// the dotted-key walker on the new path keeps unrelated keys
    /// AND comments in place when a single setting changes.
    #[test]
    fn set_dotted_preserves_comments_and_siblings() {
        let initial = r#"
# User-authored top header.
[layout]
# Inner gap in pixels.
inner_gap = 8
outer_gap = 8

[workspaces]
workspace_layout = "Horizontal"
"#;
        let mut doc: toml_edit::DocumentMut = initial.parse().expect("parse");
        set_dotted_in_doc(&mut doc, "layout.inner_gap", toml_edit::value(12_i64))
            .expect("set");
        let written = doc.to_string();
        assert!(written.contains("# User-authored top header."));
        assert!(written.contains("# Inner gap in pixels."));
        assert!(
            written.contains("inner_gap = 12"),
            "value not updated: {written}"
        );
        assert!(
            written.contains("outer_gap = 8"),
            "sibling clobbered: {written}"
        );
        assert!(
            written.contains(r#"workspace_layout = "Horizontal""#),
            "unrelated section clobbered: {written}"
        );
    }

    /// Creating a new section + key on an empty document works.
    #[test]
    fn set_dotted_creates_intermediate_sections() {
        let mut doc = toml_edit::DocumentMut::new();
        set_dotted_in_doc(&mut doc, "system_actions.VolumeRaise", toml_edit::value("spawn:wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+"))
            .expect("set");
        let written = doc.to_string();
        assert!(written.contains("[system_actions]"));
        assert!(written.contains("VolumeRaise"));
        assert!(written.contains("wpctl set-volume"));
    }

    /// Removing a missing key is a no-op, not an error.
    #[test]
    fn remove_dotted_missing_is_noop() {
        let mut doc: toml_edit::DocumentMut =
            "[a]\nx = 1\n".parse().unwrap();
        remove_dotted_in_doc(&mut doc, "b.y");
        remove_dotted_in_doc(&mut doc, "a.nonexistent");
        let written = doc.to_string();
        assert!(written.contains("x = 1"));
    }

    /// Removing an existing dotted key drops just that key.
    #[test]
    fn remove_dotted_existing() {
        let mut doc: toml_edit::DocumentMut =
            "[a]\nx = 1\ny = 2\n".parse().unwrap();
        remove_dotted_in_doc(&mut doc, "a.x");
        let written = doc.to_string();
        assert!(!written.contains("x = 1"));
        assert!(written.contains("y = 2"));
    }

    /// Critical regression test from Sprint B: an array of objects
    /// (e.g. `layout.window_rules`) must survive a round-trip through
    /// `config_set`. The previous implementation
    /// silently dropped object entries because `Item::Table` returns
    /// `None` from `.as_value()`. This test would have caught it on
    /// the first save.
    #[test]
    fn set_dotted_preserves_array_of_objects() {
        let mut doc = toml_edit::DocumentMut::new();
        let payload = serde_json::json!([
            {
                "match": { "app_id": "firefox", "title": "Preferences" },
                "action": "float"
            },
            {
                "match": { "app_id": "pavucontrol" },
                "action": "float"
            }
        ]);
        let item = json_to_toml_edit(payload);
        set_dotted_in_doc(&mut doc, "layout.window_rules", item).expect("set");

        let written = doc.to_string();
        assert!(
            written.contains("firefox"),
            "first rule lost: {written}"
        );
        assert!(
            written.contains("pavucontrol"),
            "second rule lost: {written}"
        );
        assert!(
            written.contains(r#"action = "float""#),
            "action lost: {written}"
        );

        // Re-parse and verify the structural shape the compositor
        // looks for: array of tables, each with a `match` table and
        // an `action` string.
        let reparsed: toml::Value = toml::from_str(&written).expect("re-parse");
        let arr = reparsed
            .get("layout")
            .and_then(|v| v.get("window_rules"))
            .and_then(|v| v.as_array())
            .expect("window_rules is array");
        assert_eq!(arr.len(), 2, "rule count: {written}");
        for entry in arr {
            let table = entry.as_table().expect("entry is table");
            assert!(
                table.get("match").and_then(|v| v.as_table()).is_some(),
                "entry missing match table: {entry:?}"
            );
            assert!(
                table.get("action").and_then(|v| v.as_str()).is_some(),
                "entry missing action: {entry:?}"
            );
        }
    }

    /// Empty arrays still round-trip cleanly — clearing the window-
    /// rules list shouldn't crash or write a malformed value.
    #[test]
    fn set_dotted_handles_empty_array_of_objects() {
        let mut doc = toml_edit::DocumentMut::new();
        let payload = serde_json::json!([]);
        let item = json_to_toml_edit(payload);
        set_dotted_in_doc(&mut doc, "layout.window_rules", item).expect("set");

        let written = doc.to_string();
        let reparsed: toml::Value = toml::from_str(&written).expect("re-parse");
        let arr = reparsed
            .get("layout")
            .and_then(|v| v.get("window_rules"))
            .and_then(|v| v.as_array())
            .expect("window_rules is array");
        assert_eq!(arr.len(), 0);
    }
}
