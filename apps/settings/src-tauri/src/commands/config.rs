//! Generic config CRUD commands.
//!
//! All commands operate on TOML files under `~/.config/lunaris/<file>.toml`
//! using dot-notation keys (e.g. `theme.mode` -> `[theme] mode = ...`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Logical config file name, mapped to a path under `~/.config/lunaris/`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigFile {
    Appearance,
    Compositor,
    Shell,
    Notifications,
    Modules,
    /// Knowledge daemon project-watch config (`graph.toml`).
    /// Settings UI for Focus Mode + Knowledge Graph pages writes
    /// `[projects]` here. The daemon reads this file on startup
    /// only — Settings warns the user that changes need a daemon
    /// restart until live-reload lands in a follow-up sprint.
    Graph,
    /// Quick Settings tile layout (`quicksettings.toml`). Schema:
    /// `[[tile]] id, visible, size`. The desktop-shell reads this on
    /// QS-panel mount via `qs_layout_get`; Settings writes it through
    /// the generic config API.
    QuickSettings,
}

impl ConfigFile {
    fn filename(self) -> &'static str {
        match self {
            Self::Appearance => "appearance.toml",
            Self::Compositor => "compositor.toml",
            Self::Shell => "shell.toml",
            Self::Notifications => "notifications.toml",
            Self::Modules => "modules.toml",
            Self::Graph => "graph.toml",
            Self::QuickSettings => "quicksettings.toml",
        }
    }

    fn path(self) -> PathBuf {
        let dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("lunaris");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(self.filename())
    }
}

/// Read the file and parse as a generic TOML value. Returns an empty
/// table if the file does not exist yet.
fn read_file(file: ConfigFile) -> Result<toml::Value, String> {
    let path = file.path();
    if !path.exists() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    toml::from_str(&content).map_err(|e| format!("parse {}: {e}", path.display()))
}

/// Write a TOML value to disk atomically (write to .tmp, rename).
fn write_file(file: ConfigFile, value: &toml::Value) -> Result<(), String> {
    let path = file.path();
    let content = toml::to_string_pretty(value).map_err(|e| format!("serialize: {e}"))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("write tmp: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}

/// Walk a dot-notation path on a TOML value and return a reference.
fn get_path<'a>(value: &'a toml::Value, key: &str) -> Option<&'a toml::Value> {
    let mut cur = value;
    for part in key.split('.') {
        cur = cur.as_table()?.get(part)?;
    }
    Some(cur)
}

/// Walk a dot-notation path, creating intermediate tables as needed,
/// and set the final value.
fn set_path(value: &mut toml::Value, key: &str, new_value: toml::Value) -> Result<(), String> {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        return Err("empty key".into());
    }

    // Ensure the root is a table.
    if !value.is_table() {
        *value = toml::Value::Table(toml::map::Map::new());
    }

    let mut cur = value;
    for part in &parts[..parts.len() - 1] {
        let table = cur
            .as_table_mut()
            .ok_or_else(|| format!("path component '{part}' is not a table"))?;
        let entry = table
            .entry(part.to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        if !entry.is_table() {
            *entry = toml::Value::Table(toml::map::Map::new());
        }
        cur = entry;
    }

    let last = parts[parts.len() - 1];
    cur.as_table_mut()
        .ok_or_else(|| "final path is not a table".to_string())?
        .insert(last.to_string(), new_value);
    Ok(())
}

/// Remove a dot-notation key. No-op if the key does not exist.
fn remove_path(value: &mut toml::Value, key: &str) -> Result<(), String> {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        return Err("empty key".into());
    }
    let mut cur = value;
    for part in &parts[..parts.len() - 1] {
        let Some(next) = cur.as_table_mut().and_then(|t| t.get_mut(*part)) else {
            return Ok(());
        };
        cur = next;
    }
    if let Some(t) = cur.as_table_mut() {
        t.remove(parts[parts.len() - 1]);
    }
    Ok(())
}

/// Convert a serde_json::Value to a toml::Value.
fn json_to_toml(v: serde_json::Value) -> toml::Value {
    match v {
        serde_json::Value::Null => toml::Value::String(String::new()),
        serde_json::Value::Bool(b) => toml::Value::Boolean(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                toml::Value::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => toml::Value::String(s),
        serde_json::Value::Array(arr) => {
            toml::Value::Array(arr.into_iter().map(json_to_toml).collect())
        }
        serde_json::Value::Object(obj) => {
            let mut map = toml::map::Map::new();
            for (k, val) in obj {
                map.insert(k, json_to_toml(val));
            }
            toml::Value::Table(map)
        }
    }
}

/// Convert a toml::Value to serde_json::Value for the frontend.
fn toml_to_json(v: &toml::Value) -> serde_json::Value {
    match v {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_json::Value::from(*i),
        toml::Value::Float(f) => serde_json::Value::from(*f),
        toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
        toml::Value::Datetime(dt) => serde_json::Value::String(dt.to_string()),
        toml::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(toml_to_json).collect())
        }
        toml::Value::Table(t) => {
            let mut map = serde_json::Map::new();
            for (k, val) in t {
                map.insert(k.clone(), toml_to_json(val));
            }
            serde_json::Value::Object(map)
        }
    }
}

/// Read the whole file (when `key` is None) or a single dot-notation key.
#[tauri::command]
pub fn config_get(
    file: ConfigFile,
    key: Option<String>,
) -> Result<serde_json::Value, String> {
    let doc = read_file(file)?;
    match key.as_deref() {
        None | Some("") => Ok(toml_to_json(&doc)),
        Some(k) => match get_path(&doc, k) {
            Some(v) => Ok(toml_to_json(v)),
            None => Ok(serde_json::Value::Null),
        },
    }
}

/// Write a value at a dot-notation key, preserving other sections
/// AND user comments + formatting in the file. Goes through
/// `toml_writer` (toml_edit-backed, atomic tmp+rename) so a save
/// from the Settings app doesn't erase the user's hand-edited
/// notes in `compositor.toml` / `shell.toml`.
#[tauri::command]
pub fn config_set(
    file: ConfigFile,
    key: String,
    value: serde_json::Value,
) -> Result<(), String> {
    let path = file.path();
    let item = json_to_toml_edit(value);
    crate::toml_writer::update(&path, |doc| {
        set_dotted_in_doc(doc, &key, item)
    })?;
    Ok(())
}

/// Reset a single key (delete it) or the whole file.
#[tauri::command]
pub fn config_reset(file: ConfigFile, key: Option<String>) -> Result<(), String> {
    match key.as_deref() {
        None | Some("") => {
            let path = file.path();
            if path.exists() {
                std::fs::remove_file(&path)
                    .map_err(|e| format!("remove {}: {e}", path.display()))?;
            }
            Ok(())
        }
        Some(k) => {
            let path = file.path();
            crate::toml_writer::update(&path, |doc| {
                remove_dotted_in_doc(doc, k);
                Ok(())
            })?;
            Ok(())
        }
    }
}

/// Walk a dot-notation path inside a `toml_edit::DocumentMut`,
/// creating intermediate tables as needed, and assign the final
/// value with `Index`-style assign so the existing key's leading
/// comment decor (if any) is preserved.
///
/// Only `IndexMut::index_mut` (i.e. `table[key] = value`) keeps the
/// per-key decor intact; `Table::insert` resets it. That's why the
/// final write goes through `as_table_mut()` (concrete `Table`) and
/// not the `TableLike` trait — only `Table` impls `IndexMut<&str>`.
fn set_dotted_in_doc(
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
fn remove_dotted_in_doc(doc: &mut toml_edit::DocumentMut, key: &str) {
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
/// to empty on save, a data-loss bug flagged in Codex review of
/// Sprint B.
fn json_to_toml_edit(v: serde_json::Value) -> toml_edit::Item {
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
fn json_to_toml_edit_value(v: serde_json::Value) -> toml_edit::Value {
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

/// Return the built-in default for a config file (or a single key).
/// This is what the user sees when they "reset to default".
#[tauri::command]
pub fn config_get_default(
    file: ConfigFile,
    key: Option<String>,
) -> Result<serde_json::Value, String> {
    let doc = default_for(file);
    match key.as_deref() {
        None | Some("") => Ok(toml_to_json(&doc)),
        Some(k) => match get_path(&doc, k) {
            Some(v) => Ok(toml_to_json(v)),
            None => Ok(serde_json::Value::Null),
        },
    }
}

fn default_for(file: ConfigFile) -> toml::Value {
    let raw = match file {
        ConfigFile::Appearance => DEFAULT_APPEARANCE,
        ConfigFile::Notifications => DEFAULT_NOTIFICATIONS,
        _ => return toml::Value::Table(toml::map::Map::new()),
    };
    toml::from_str::<toml::Value>(raw)
        .unwrap_or_else(|_| toml::Value::Table(toml::map::Map::new()))
}

/// Default appearance.toml shipped with the settings app. Matches the
/// dark theme values used by desktop-shell.
const DEFAULT_APPEARANCE: &str = r##"
[theme]
active = "dark"
mode = "dark"

[overrides]
# `radius_intensity` is the user's global radius multiplier (0.0..=2.0).
# Applied at emit time to chip/button/input/card/modal tokens; `full`
# and the per-corner window outline stay categorical. See
# docs/architecture/theme-system.md.
# radius_intensity = 1.0

[window]
border_width = 1
gap_size = 8

[fonts]
interface = "Inter Variable"
monospace = "JetBrains Mono"
size = 14

[accessibility]
reduce_motion = false
"##;

/// Default notifications.toml shipped with the settings app. Mirrors
/// the daemon's `notification_daemon::config::Config::default()`.
const DEFAULT_NOTIFICATIONS: &str = r##"
[general]
toast_duration_normal = 4000
toast_duration_high = 8000
max_visible_toasts = 5

[dnd]
mode = "off"
suppress_fullscreen = false
always_suppress = []
always_allow = []

[dnd.schedule]
start = "22:00"
end = "07:00"
days = []
mode = "priority"

[history]
enabled = true
max_age_days = 30
max_count = 1000

[grouping]
by_app = true
stack_similar = true
auto_collapse_after = 3
"##;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── get_path / set_path / remove_path ────────────────────────────

    #[test]
    fn test_get_path_top_level() {
        let v: toml::Value = toml::from_str("[theme]\nmode = \"dark\"").unwrap();
        let r = get_path(&v, "theme.mode").unwrap();
        assert_eq!(r.as_str(), Some("dark"));
    }

    #[test]
    fn test_get_path_deeply_nested() {
        let v: toml::Value =
            toml::from_str("[window.border]\nfocused = \"$accent\"").unwrap();
        let r = get_path(&v, "window.border.focused").unwrap();
        assert_eq!(r.as_str(), Some("$accent"));
    }

    #[test]
    fn test_get_path_missing_returns_none() {
        let v: toml::Value = toml::from_str("[theme]\nmode = \"dark\"").unwrap();
        assert!(get_path(&v, "theme.accent").is_none());
        assert!(get_path(&v, "nonexistent").is_none());
        assert!(get_path(&v, "theme.mode.sub").is_none());
    }

    #[test]
    fn test_set_path_creates_intermediate_tables() {
        let mut v = toml::Value::Table(toml::map::Map::new());
        set_path(
            &mut v,
            "window.border.focused",
            toml::Value::String("$accent".into()),
        )
        .unwrap();
        assert_eq!(
            get_path(&v, "window.border.focused")
                .and_then(|v| v.as_str()),
            Some("$accent")
        );
    }

    #[test]
    fn test_set_path_preserves_siblings() {
        let mut v: toml::Value =
            toml::from_str("[theme]\nmode = \"dark\"\naccent = \"#fff\"").unwrap();
        set_path(
            &mut v,
            "theme.mode",
            toml::Value::String("light".into()),
        )
        .unwrap();
        assert_eq!(
            get_path(&v, "theme.mode").and_then(|v| v.as_str()),
            Some("light"),
            "updated key"
        );
        assert_eq!(
            get_path(&v, "theme.accent").and_then(|v| v.as_str()),
            Some("#fff"),
            "sibling preserved"
        );
    }

    #[test]
    fn test_remove_path_existing() {
        let mut v: toml::Value =
            toml::from_str("[theme]\nmode = \"dark\"\naccent = \"#fff\"").unwrap();
        remove_path(&mut v, "theme.accent").unwrap();
        assert!(get_path(&v, "theme.accent").is_none());
        assert!(get_path(&v, "theme.mode").is_some(), "sibling intact");
    }

    #[test]
    fn test_remove_path_missing_is_noop() {
        let mut v: toml::Value = toml::from_str("[theme]\nmode = \"dark\"").unwrap();
        remove_path(&mut v, "theme.nonexistent").unwrap();
        assert_eq!(get_path(&v, "theme.mode").and_then(|v| v.as_str()), Some("dark"));
    }

    // ── json ↔ toml conversions ──────────────────────────────────────

    #[test]
    fn test_json_to_toml_primitives() {
        assert_eq!(
            json_to_toml(serde_json::json!(42)),
            toml::Value::Integer(42)
        );
        assert_eq!(
            json_to_toml(serde_json::json!(true)),
            toml::Value::Boolean(true)
        );
        assert_eq!(
            json_to_toml(serde_json::json!("hello")),
            toml::Value::String("hello".into())
        );
    }

    #[test]
    fn test_toml_to_json_roundtrip() {
        let original = toml::Value::String("test".into());
        let json = toml_to_json(&original);
        let back = json_to_toml(json);
        assert_eq!(original, back);
    }

    // ── default configs ──────────────────────────────────────────────

    #[test]
    fn test_default_appearance_is_valid_toml() {
        let v: Result<toml::Value, _> = toml::from_str(DEFAULT_APPEARANCE);
        assert!(v.is_ok(), "DEFAULT_APPEARANCE parse error: {:?}", v.err());
        let table = v.unwrap();
        assert!(
            get_path(&table, "theme.mode").is_some(),
            "theme.mode must exist"
        );
    }

    #[test]
    fn test_default_notifications_is_valid_toml() {
        let v: Result<toml::Value, _> = toml::from_str(DEFAULT_NOTIFICATIONS);
        assert!(
            v.is_ok(),
            "DEFAULT_NOTIFICATIONS parse error: {:?}",
            v.err()
        );
        let table = v.unwrap();
        assert!(
            get_path(&table, "dnd.mode").is_some(),
            "dnd.mode must exist"
        );
        assert!(
            get_path(&table, "history.enabled").is_some(),
            "history.enabled must exist"
        );
    }

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

    /// Critical regression test for the Codex Sprint B review: an
    /// array of objects (e.g. `layout.window_rules`) must survive a
    /// round-trip through `config_set`. The previous implementation
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
