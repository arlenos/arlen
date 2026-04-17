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
}

impl ConfigFile {
    fn filename(self) -> &'static str {
        match self {
            Self::Appearance => "appearance.toml",
            Self::Compositor => "compositor.toml",
            Self::Shell => "shell.toml",
            Self::Notifications => "notifications.toml",
            Self::Modules => "modules.toml",
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

/// Write a value at a dot-notation key, preserving other sections.
#[tauri::command]
pub fn config_set(
    file: ConfigFile,
    key: String,
    value: serde_json::Value,
) -> Result<(), String> {
    let mut doc = read_file(file)?;
    set_path(&mut doc, &key, json_to_toml(value))?;
    write_file(file, &doc)
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
            let mut doc = read_file(file)?;
            remove_path(&mut doc, k)?;
            write_file(file, &doc)
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

[window]
corner_radius = 8
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
}
