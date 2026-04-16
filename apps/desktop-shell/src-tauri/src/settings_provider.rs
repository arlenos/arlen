/// Settings search provider for the Waypointer.
///
/// Reads `~/.local/share/lunaris/settings-index.json` (exported by the
/// Settings app on startup), searches it by query, and provides generic
/// config read/write commands so inline actions can toggle settings
/// without opening the Settings app.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Index types (mirroring the Settings app's export format)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsIndex {
    #[allow(dead_code)]
    version: u32,
    settings: Vec<IndexedSetting>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexedSetting {
    pub id: String,
    pub title: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub panel: String,
    pub section: String,
    pub deep_link: String,
    pub inline_action: Option<InlineAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineAction {
    #[serde(rename = "type")]
    pub action_type: String,
    pub config_file: String,
    pub config_key: String,
    #[serde(default)]
    pub options: Vec<SelectOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
}

/// Search result returned to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSearchResult {
    pub setting: IndexedSetting,
    pub score: u32,
    /// Current value of the setting (if it has an inline action and
    /// the config file is readable). `null` if not actionable or if
    /// the read failed.
    pub current_value: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Cached index
// ---------------------------------------------------------------------------

static INDEX: Mutex<Option<Vec<IndexedSetting>>> = Mutex::new(None);

fn index_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lunaris")
        .join("settings-index.json")
}

fn ensure_index() -> Vec<IndexedSetting> {
    let mut guard = INDEX.lock().unwrap();
    if let Some(ref idx) = *guard {
        return idx.clone();
    }
    let path = index_path();
    let settings = match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<SettingsIndex>(&content) {
            Ok(idx) => idx.settings,
            Err(e) => {
                log::warn!("settings_provider: parse index failed: {e}");
                Vec::new()
            }
        },
        Err(_) => Vec::new(),
    };
    log::info!(
        "settings_provider: loaded {} settings from {}",
        settings.len(),
        path.display()
    );
    *guard = Some(settings.clone());
    settings
}

// ---------------------------------------------------------------------------
// Generic TOML config read/write
// ---------------------------------------------------------------------------

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lunaris")
}

/// Map a logical config file name to the actual filename.
fn config_file_path(file: &str) -> PathBuf {
    config_dir().join(format!("{file}.toml"))
}

fn read_toml_key(file: &str, key: &str) -> Option<serde_json::Value> {
    let path = config_file_path(file);
    let content = std::fs::read_to_string(&path).ok()?;
    let table: toml::Value = toml::from_str(&content).ok()?;
    let mut cur = &table;
    for part in key.split('.') {
        cur = cur.as_table()?.get(part)?;
    }
    Some(toml_to_json(cur))
}

fn write_toml_key(
    file: &str,
    key: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    let path = config_file_path(file);
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let mut table: toml::Value = toml::from_str(&content).unwrap_or_else(|_| {
        toml::Value::Table(toml::map::Map::new())
    });

    // Walk the dot-path, creating tables as needed.
    let parts: Vec<&str> = key.split('.').collect();
    let mut cur = &mut table;
    for part in &parts[..parts.len() - 1] {
        let t = cur
            .as_table_mut()
            .ok_or_else(|| format!("'{part}' is not a table"))?;
        let entry = t
            .entry(part.to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        cur = entry;
    }
    let last = parts[parts.len() - 1];
    cur.as_table_mut()
        .ok_or_else(|| "target is not a table".to_string())?
        .insert(last.to_string(), json_to_toml(value));

    let out = toml::to_string_pretty(&table).map_err(|e| format!("serialize: {e}"))?;

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Atomic write: tmp + rename.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, &out).map_err(|e| format!("write tmp: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}

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
            let mut m = serde_json::Map::new();
            for (k, val) in t {
                m.insert(k.clone(), toml_to_json(val));
            }
            serde_json::Value::Object(m)
        }
    }
}

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

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Reload the settings index from disk. Called when the Waypointer
/// opens so it always has a fresh copy.
#[tauri::command]
pub fn settings_reload_index() -> usize {
    *INDEX.lock().unwrap() = None;
    let idx = ensure_index();
    idx.len()
}

/// Search the settings index. Returns up to `limit` results.
///
/// Current config values for inline actions are read lazily by the
/// frontend via `settings_get_value` — NOT during the search itself.
/// This avoids 1-5 TOML file reads per keystroke which was the main
/// performance bottleneck.
#[tauri::command]
pub fn settings_search(query: String, limit: u32) -> Vec<SettingsSearchResult> {
    let settings = ensure_index();
    let terms: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect();

    if terms.is_empty() {
        return Vec::new();
    }

    let mut results: Vec<SettingsSearchResult> = Vec::new();

    for setting in &settings {
        let title_lower = setting.title.to_lowercase();
        let section_lower = setting.section.to_lowercase();
        let desc_lower = setting.description.to_lowercase();
        let haystack = format!(
            "{} {} {} {}",
            title_lower,
            section_lower,
            desc_lower,
            setting.keywords.join(" ")
        );

        if !terms.iter().all(|t| haystack.contains(t.as_str())) {
            continue;
        }

        let mut score: u32 = 0;
        for term in &terms {
            if title_lower.contains(term.as_str()) {
                score += 10;
            }
            if section_lower.contains(term.as_str()) {
                score += 5;
            }
            if desc_lower.contains(term.as_str()) {
                score += 3;
            }
            if setting.keywords.iter().any(|k| k.contains(term.as_str())) {
                score += 2;
            }
        }

        results.push(SettingsSearchResult {
            setting: setting.clone(),
            score,
            current_value: None,
        });
    }

    results.sort_by(|a, b| b.score.cmp(&a.score));
    results.truncate(limit as usize);
    results
}

/// Read a single config value. Called by the frontend lazily when
/// rendering an inline action, NOT during bulk search.
#[tauri::command]
pub fn settings_get_value(
    config_file: String,
    config_key: String,
) -> Option<serde_json::Value> {
    read_toml_key(&config_file, &config_key)
}

/// Write a config value for an inline action. The file watchers in
/// the daemon / shell / compositor pick up the change automatically.
#[tauri::command]
pub fn settings_set_value(
    config_file: String,
    config_key: String,
    value: serde_json::Value,
) -> Result<(), String> {
    write_toml_key(&config_file, &config_key, value)
}

/// Open the Settings app with a deep-link to a specific panel/anchor.
#[tauri::command]
pub fn settings_open_deep_link(panel: String, anchor: Option<String>) -> Result<(), String> {
    let mut cmd = std::process::Command::new("lunaris-settings");
    cmd.arg("--panel").arg(&panel);
    if let Some(ref a) = anchor {
        cmd.arg("--section").arg(a);
    }
    cmd.spawn().map_err(|e| format!("spawn: {e}"))?;
    Ok(())
}
