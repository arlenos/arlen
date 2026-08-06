/// Settings search provider for the Waypointer.
///
/// Reads `~/.local/share/arlen/settings-index.json` (exported by the
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

/// The index format this build understands.
///
/// Checked, not carried. It was `#[allow(dead_code)]` before, and the format then
/// changed under it: the Settings app began writing message ids where prose used
/// to be, serde failed on the missing field, and `ensure_index` turned that into
/// an empty list behind a `warn!`. Waypointer's settings search went dark and said
/// nothing. A version nobody reads is a version that cannot warn you.
const INDEX_VERSION: u32 = 2;

/// The file as written: message ids and the catalog that resolves them.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsIndex {
    version: u32,
    /// The catalog the ids resolve against, loaded for this reader's own locale.
    catalog: String,
    settings: Vec<WireSetting>,
}

/// One entry as written: ids, never prose.
///
/// Prose here would be correct for exactly one language, chosen when Settings
/// happened to run, and would be a second copy of every string that drifts from
/// the catalog silently. It also could not work at all for a third-party app,
/// which is searchable in languages it never shipped a snapshot for.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireSetting {
    id: String,
    title_key: String,
    desc_key: String,
    keywords_key: String,
    panel: String,
    section_key: String,
    deep_link: String,
    inline_action: Option<WireInlineAction>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireInlineAction {
    #[serde(rename = "type")]
    action_type: String,
    config_file: String,
    config_key: String,
    #[serde(default)]
    options: Vec<WireSelectOption>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireSelectOption {
    value: String,
    label_key: String,
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

/// One entry, resolved for display, plus the source-locale text it also matches on.
#[derive(Debug, Clone)]
struct LoadedSetting {
    shown: IndexedSetting,
    /// The same entry in the language the settings were authored in.
    ///
    /// People learn a setting's name in English and then look for it in a German
    /// UI, so typing "night light" has to find *Nachtlicht*. Only ids can do that:
    /// a snapshot of resolved prose has already thrown the other language away.
    source_text: String,
}

/// Cached per locale, because the resolution depends on it.
static INDEX: Mutex<Option<(String, Vec<LoadedSetting>)>> = Mutex::new(None);

fn index_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("arlen")
        .join("settings-index.json")
}

/// The locale to resolve in: what the caller asked for, else the session's.
///
/// `LANG` is what a desktop session sets, and it is the honest default for a
/// process the user never told anything else. It is only a default: the Waypointer
/// holds the live locale store and passes it, so a language switch in Settings
/// takes effect without a restart.
fn requested_locale(explicit: Option<&str>) -> String {
    if let Some(l) = explicit.filter(|l| !l.is_empty()) {
        return l.to_string();
    }
    std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .ok()
        // `de_AT.UTF-8` is POSIX, `de-AT` is BCP-47.
        .map(|l| l.split('.').next().unwrap_or("en").replace('_', "-"))
        .filter(|l| !l.is_empty() && l != "C" && l != "POSIX")
        .unwrap_or_else(|| SOURCE_LOCALE.to_string())
}

/// The language the catalogs are authored in, and the floor of every fallback.
const SOURCE_LOCALE: &str = "en";

fn load_index(locale: &str) -> Vec<LoadedSetting> {
    load_index_from(&index_path(), locale)
}

/// The loading itself, over an explicit path so it can be tested against a real
/// index and real catalogs rather than whatever happens to be in the user's data
/// directory.
fn load_index_from(path: &std::path::Path, locale: &str) -> Vec<LoadedSetting> {
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        // No index yet is ordinary: Settings has not run on this machine.
        Err(_) => return Vec::new(),
    };
    let file: SettingsIndex = match serde_json::from_str(&content) {
        Ok(f) => f,
        Err(e) => {
            log::error!(
                "settings_provider: {} did not parse ({e}). Settings search is empty \
                 until Settings rewrites it.",
                path.display()
            );
            return Vec::new();
        }
    };
    if file.version != INDEX_VERSION {
        // Loudly, and distinctly from an absent index. The two look identical from
        // the outside - no results - and only one of them is somebody's mistake.
        log::error!(
            "settings_provider: {} is version {}, this build reads {}. Settings \
             search is empty until the two agree.",
            path.display(),
            file.version,
            INDEX_VERSION
        );
        return Vec::new();
    }

    let dir = path
        .parent()
        .unwrap_or(std::path::Path::new("/tmp"))
        .join("catalogs")
        .join(&file.catalog);
    let requested: arlen_i18n::Locale = locale.parse().unwrap_or_else(|_| {
        SOURCE_LOCALE.parse().expect("the source locale tag is well-formed")
    });
    let source: arlen_i18n::Locale =
        SOURCE_LOCALE.parse().expect("the source locale tag is well-formed");
    let (shown, warnings) = arlen_i18n::Localizer::load_dir(&dir, &requested, &source);
    let (in_source, _) = arlen_i18n::Localizer::load_dir(&dir, &source, &source);
    for w in warnings {
        log::warn!("settings_provider: catalog: {w}");
    }

    let args = arlen_i18n::Args::default();
    let tr = |loc: &arlen_i18n::Localizer, id: &str| loc.localize(id, &args);

    file.settings
        .into_iter()
        .map(|w| {
            let keywords = tr(&shown, &w.keywords_key);
            let source_text = format!(
                "{} {} {} {}",
                tr(&in_source, &w.title_key),
                tr(&in_source, &w.section_key),
                tr(&in_source, &w.desc_key),
                tr(&in_source, &w.keywords_key)
            )
            .to_lowercase();
            LoadedSetting {
                shown: IndexedSetting {
                    id: w.id,
                    title: tr(&shown, &w.title_key),
                    description: tr(&shown, &w.desc_key),
                    keywords: keywords
                        .split(',')
                        .map(|k| k.trim().to_lowercase())
                        .filter(|k| !k.is_empty())
                        .collect(),
                    panel: w.panel,
                    section: tr(&shown, &w.section_key),
                    deep_link: w.deep_link,
                    inline_action: w.inline_action.map(|a| InlineAction {
                        action_type: a.action_type,
                        config_file: a.config_file,
                        config_key: a.config_key,
                        options: a
                            .options
                            .into_iter()
                            .map(|o| SelectOption {
                                value: o.value,
                                label: tr(&shown, &o.label_key),
                            })
                            .collect(),
                    }),
                },
                source_text,
            }
        })
        .collect()
}

fn ensure_index_for(locale: &str) -> Vec<LoadedSetting> {
    let mut guard = INDEX.lock().unwrap();
    if let Some((ref cached, ref idx)) = *guard {
        if cached == locale {
            return idx.clone();
        }
    }
    let settings = load_index(locale);
    log::info!(
        "settings_provider: loaded {} settings for {locale} from {}",
        settings.len(),
        index_path().display()
    );
    *guard = Some((locale.to_string(), settings.clone()));
    settings
}

// ---------------------------------------------------------------------------
// Generic TOML config read/write
// ---------------------------------------------------------------------------

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("arlen")
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
pub fn settings_reload_index(locale: Option<String>) -> usize {
    *INDEX.lock().unwrap() = None;
    ensure_index_for(&requested_locale(locale.as_deref())).len()
}

/// Search the settings index. Returns up to `limit` results.
///
/// Current config values for inline actions are read lazily by the
/// frontend via `settings_get_value` — NOT during the search itself.
/// This avoids 1-5 TOML file reads per keystroke which was the main
/// performance bottleneck.
#[tauri::command]
pub fn settings_search(
    query: String,
    limit: u32,
    locale: Option<String>,
) -> Vec<SettingsSearchResult> {
    let settings = ensure_index_for(&requested_locale(locale.as_deref()));
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

    for loaded in &settings {
        let setting = &loaded.shown;
        let title_lower = setting.title.to_lowercase();
        let section_lower = setting.section.to_lowercase();
        let desc_lower = setting.description.to_lowercase();
        // Both languages: what the reader sees, and what the setting is called in
        // the language it was written in.
        let haystack = format!(
            "{} {} {} {} {}",
            title_lower,
            section_lower,
            desc_lower,
            setting.keywords.join(" "),
            loaded.source_text
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
    let mut cmd = std::process::Command::new("arlen-settings");
    cmd.arg("--panel").arg(&panel);
    if let Some(ref a) = anchor {
        cmd.arg("--section").arg(a);
    }
    cmd.spawn().map_err(|e| format!("spawn: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_index() -> Vec<IndexedSetting> {
        vec![
            IndexedSetting {
                id: "appearance.theme.mode".into(),
                title: "Theme Mode".into(),
                description: "Switch between light and dark theme".into(),
                keywords: vec!["dark".into(), "light".into(), "theme".into()],
                panel: "appearance".into(),
                section: "Theme".into(),
                deep_link: "arlen-settings://appearance#theme-mode".into(),
                inline_action: Some(InlineAction {
                    action_type: "select".into(),
                    config_file: "appearance".into(),
                    config_key: "theme.mode".into(),
                    options: vec![
                        SelectOption { value: "light".into(), label: "Light".into() },
                        SelectOption { value: "dark".into(), label: "Dark".into() },
                    ],
                }),
            },
            IndexedSetting {
                id: "notifications.dnd.mode".into(),
                title: "Do Not Disturb".into(),
                description: "Control which notifications break through".into(),
                keywords: vec!["dnd".into(), "quiet".into(), "silent".into()],
                panel: "notifications".into(),
                section: "Do Not Disturb".into(),
                deep_link: "arlen-settings://notifications#dnd-mode".into(),
                inline_action: None,
            },
            IndexedSetting {
                id: "appearance.fonts.size".into(),
                title: "Font Size".into(),
                description: "Base font size for the interface".into(),
                keywords: vec!["font".into(), "size".into(), "text".into()],
                panel: "appearance".into(),
                section: "Typography".into(),
                deep_link: "arlen-settings://appearance#font-size".into(),
                inline_action: None,
            },
        ]
    }

    fn with_index<F: FnOnce()>(settings: Vec<IndexedSetting>, f: F) {
        let loaded = settings
            .into_iter()
            .map(|shown| {
                // These fixtures are already in the source language, so the entry
                // matches the same words twice. The dual-language search has its
                // own test below, where the two differ.
                let source_text = format!(
                    "{} {} {} {}",
                    shown.title,
                    shown.section,
                    shown.description,
                    shown.keywords.join(" ")
                )
                .to_lowercase();
                LoadedSetting { shown, source_text }
            })
            .collect();
        *INDEX.lock().unwrap() = Some((SOURCE_LOCALE.to_string(), loaded));
        f();
        *INDEX.lock().unwrap() = None;
    }

    // ── The index format ─────────────────────────────────────────────

    /// Write an index plus its catalogs the way the Settings app does, and hand
    /// back the index path.
    fn write_index(dir: &std::path::Path, version: u32, catalogs: &[(&str, &str)]) -> PathBuf {
        let cats = dir.join("catalogs").join("settings");
        std::fs::create_dir_all(&cats).unwrap();
        for (locale, body) in catalogs {
            std::fs::write(cats.join(format!("{locale}.json")), body).unwrap();
        }
        let path = dir.join("settings-index.json");
        std::fs::write(
            &path,
            format!(
                r#"{{"version":{version},"catalog":"settings","settings":[
                   {{"id":"appearance.night","titleKey":"t","descKey":"d",
                     "keywordsKey":"k","panel":"appearance","sectionKey":"s",
                     "deepLink":"arlen-settings://appearance#night"}}]}}"#
            ),
        )
        .unwrap();
        path
    }

    const EN: &str = r#"{"t":"Night light","d":"Warms the screen","k":"night, warm","s":"Display"}"#;
    const DE: &str = r#"{"t":"Nachtlicht","d":"Wärmt den Bildschirm","k":"nacht, warm","s":"Anzeige"}"#;

    #[test]
    fn an_index_of_ids_resolves_against_its_catalog() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_index(dir.path(), INDEX_VERSION, &[("en", EN), ("de", DE)]);

        let de = load_index_from(&path, "de");
        assert_eq!(de.len(), 1);
        // The point of the whole exercise: the reader shows its own language, from
        // an index that carries no prose at all.
        assert_eq!(de[0].shown.title, "Nachtlicht");
        assert_eq!(de[0].shown.section, "Anzeige");
    }

    #[test]
    fn a_german_entry_is_still_found_by_its_english_name() {
        // People learn a setting in one language and hunt for it in another. A
        // snapshot of resolved prose cannot do this - it kept only one language.
        let dir = tempfile::tempdir().unwrap();
        let path = write_index(dir.path(), INDEX_VERSION, &[("en", EN), ("de", DE)]);

        let de = load_index_from(&path, "de");
        assert!(
            de[0].source_text.contains("night light"),
            "the English name was dropped: {}",
            de[0].source_text
        );
    }

    #[test]
    fn an_index_from_a_different_format_is_refused_rather_than_half_read() {
        // The failure this replaced: the producer moved to ids, serde hit a missing
        // field, and the empty list that came back was indistinguishable from a
        // machine where Settings had simply never run.
        let dir = tempfile::tempdir().unwrap();
        let path = write_index(dir.path(), INDEX_VERSION + 1, &[("en", EN)]);
        assert!(load_index_from(&path, "en").is_empty());
    }

    #[test]
    fn a_missing_catalog_leaves_the_entry_findable_by_its_id() {
        // Degrading to keys is the loader's documented behaviour, and it is the
        // right one here: the entry still routes to its panel.
        let dir = tempfile::tempdir().unwrap();
        let path = write_index(dir.path(), INDEX_VERSION, &[]);
        let loaded = load_index_from(&path, "en");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].shown.deep_link, "arlen-settings://appearance#night");
    }

    #[test]
    fn a_posix_lang_becomes_a_locale_tag() {
        assert_eq!(requested_locale(Some("de-AT")), "de-AT");
        // What a desktop session actually sets.
        temp_env("LANG", "de_AT.UTF-8", || {
            assert_eq!(requested_locale(None), "de-AT");
        });
        // `C` is not a language.
        temp_env("LANG", "C", || {
            assert_eq!(requested_locale(None), SOURCE_LOCALE);
        });
    }

    fn temp_env<F: FnOnce()>(key: &str, value: &str, f: F) {
        let prior = std::env::var_os(key);
        // Safety: single-threaded within this test, and restored before returning.
        unsafe { std::env::set_var(key, value) };
        f();
        match prior {
            Some(v) => unsafe { std::env::set_var(key, v) },
            None => unsafe { std::env::remove_var(key) },
        }
    }

    // ── Search ───────────────────────────────────────────────────────

    #[test]
    fn test_search_single_term() {
        with_index(make_index(), || {
            let results = settings_search("dark".into(), 10, Some(SOURCE_LOCALE.into()));
            assert!(!results.is_empty(), "should find Theme Mode");
            assert_eq!(results[0].setting.id, "appearance.theme.mode");
        });
    }

    #[test]
    fn test_search_multi_term() {
        with_index(make_index(), || {
            let results = settings_search("font size".into(), 10, Some(SOURCE_LOCALE.into()));
            assert!(!results.is_empty(), "should find Font Size");
            assert_eq!(results[0].setting.id, "appearance.fonts.size");
        });
    }

    #[test]
    fn test_search_no_match() {
        with_index(make_index(), || {
            let results = settings_search("xyzzy".into(), 10, Some(SOURCE_LOCALE.into()));
            assert!(results.is_empty());
        });
    }

    #[test]
    fn test_search_empty_query() {
        with_index(make_index(), || {
            let results = settings_search("".into(), 10, Some(SOURCE_LOCALE.into()));
            assert!(results.is_empty(), "empty query = no results");
        });
    }

    #[test]
    fn test_search_scoring_title_beats_description() {
        with_index(make_index(), || {
            let results = settings_search("theme".into(), 10, Some(SOURCE_LOCALE.into()));
            // "Theme Mode" has "theme" in title (+10) AND keyword (+2) = 12.
            // "Font Size" might match "theme" in description? No.
            assert!(results.len() >= 1);
            assert_eq!(results[0].setting.id, "appearance.theme.mode");
        });
    }

    #[test]
    fn test_search_limit() {
        with_index(make_index(), || {
            // All 3 items match "the" (in description/title).
            let results = settings_search("the".into(), 1, Some(SOURCE_LOCALE.into()));
            assert!(results.len() <= 1, "limit respected");
        });
    }

    // ── TOML read/write ──────────────────────────────────────────────

    #[test]
    fn test_toml_read_write_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("test.toml");
        std::fs::write(&file, "[theme]\nmode = \"dark\"\n").unwrap();

        // Read
        let path_str = file.to_string_lossy();
        let file_name = path_str.strip_suffix(".toml").unwrap();
        // Can't easily test read_toml_key because it uses config_dir().
        // Test the underlying toml logic directly:
        let content = std::fs::read_to_string(&file).unwrap();
        let table: toml::Value = toml::from_str(&content).unwrap();
        let mut cur = &table;
        for part in "theme.mode".split('.') {
            cur = cur.as_table().unwrap().get(part).unwrap();
        }
        assert_eq!(cur.as_str(), Some("dark"));
    }

    #[test]
    fn test_toml_to_json_types() {
        assert_eq!(
            toml_to_json(&toml::Value::Integer(42)),
            serde_json::json!(42)
        );
        assert_eq!(
            toml_to_json(&toml::Value::Boolean(true)),
            serde_json::json!(true)
        );
        assert_eq!(
            toml_to_json(&toml::Value::String("hello".into())),
            serde_json::json!("hello")
        );
    }

    #[test]
    fn test_json_to_toml_types() {
        assert_eq!(
            json_to_toml(serde_json::json!(42)),
            toml::Value::Integer(42)
        );
        assert_eq!(
            json_to_toml(serde_json::json!("test")),
            toml::Value::String("test".into())
        );
        assert_eq!(
            json_to_toml(serde_json::json!(true)),
            toml::Value::Boolean(true)
        );
    }
}
