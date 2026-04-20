//! Built-in Waypointer plugin bridge.
//!
//! The desktop-shell compiles its Waypointer plugins directly into the
//! binary and exposes their metadata through the on-disk registry at
//! `~/.local/share/lunaris/waypointer-plugins.toml`. This module reads
//! that file and surfaces each entry in the Extensions panel alongside
//! filesystem modules.
//!
//! Toggle state is persisted in `~/.config/lunaris/modules.toml` under
//! `[waypointer] disabled_plugins`. The shell reads the same section on
//! startup and skips disabled plugins, so toggling here requires a
//! shell restart to take effect (the Extensions UI shows the same
//! "restart required" banner).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types surfaced to the frontend
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Always `"builtin"`. Lets the frontend distinguish from modules.
    pub source: String,
    pub enabled: bool,
    pub priority: u32,
    pub prefix: Option<String>,
    pub pattern: Option<String>,
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

fn registry_path() -> PathBuf {
    if let Ok(p) = std::env::var("LUNARIS_WAYPOINTER_REGISTRY") {
        return PathBuf::from(p);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lunaris/waypointer-plugins.toml")
}

fn modules_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("LUNARIS_MODULES_CONFIG") {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lunaris/modules.toml")
}

// ---------------------------------------------------------------------------
// Registry file schema
// ---------------------------------------------------------------------------

/// Mirrors the shell's `registry::PluginDescriptor`. We deserialize here
/// instead of depending on the shell crate so the Settings app stays
/// decoupled; the TOML format is the contract.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryEntry {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    priority: u32,
    #[serde(default)]
    prefix: Option<String>,
    #[serde(default)]
    pattern: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RegistryFile {
    #[serde(default)]
    plugin: Vec<RegistryEntry>,
}

// ---------------------------------------------------------------------------
// modules.toml: round-tripping preserves the filesystem-module section
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ModulesConfig {
    /// Disabled filesystem modules. Preserved verbatim — this module
    /// does not read or write the list, only `disabled_plugins` below.
    #[serde(default, skip_serializing_if = "DisabledSection::is_empty")]
    disabled: DisabledSection,
    #[serde(default, skip_serializing_if = "WaypointerSection::is_empty")]
    waypointer: WaypointerSection,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DisabledSection {
    #[serde(default)]
    modules: Vec<String>,
}

impl DisabledSection {
    fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct WaypointerSection {
    #[serde(default)]
    disabled_plugins: Vec<String>,
}

impl WaypointerSection {
    fn is_empty(&self) -> bool {
        self.disabled_plugins.is_empty()
    }
}

fn read_modules_config(path: &std::path::Path) -> ModulesConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|c| toml::from_str(&c).ok())
        .unwrap_or_default()
}

fn write_modules_config(path: &std::path::Path, cfg: &ModulesConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
    }
    let content = toml::to_string_pretty(cfg).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(path, content).map_err(|e| format!("write: {e}"))
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Lists the built-in Waypointer plugins registered by the shell.
/// Returns an empty list if the shell has not yet written the registry
/// (e.g. first run after install, or shell not started).
#[tauri::command]
pub fn waypointer_list_plugins() -> Vec<PluginSummary> {
    let path = registry_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => {
            log::debug!(
                "waypointer: registry not present at {} — shell probably not running",
                path.display()
            );
            return Vec::new();
        }
    };

    let file: RegistryFile = match toml::from_str(&content) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("waypointer: registry parse error: {e}");
            return Vec::new();
        }
    };

    let disabled = read_modules_config(&modules_config_path())
        .waypointer
        .disabled_plugins;

    let mut out: Vec<PluginSummary> = file
        .plugin
        .into_iter()
        .map(|e| PluginSummary {
            enabled: !disabled.contains(&e.id),
            id: e.id,
            name: e.name,
            description: e.description,
            source: "builtin".into(),
            priority: e.priority,
            prefix: e.prefix,
            pattern: e.pattern,
        })
        .collect();

    // Enabled first, then by priority, then by name.
    out.sort_by(|a, b| {
        b.enabled
            .cmp(&a.enabled)
            .then_with(|| a.priority.cmp(&b.priority))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// Toggle a built-in plugin's enabled state. Persists to
/// `modules.toml`. Returns `true` so the frontend can show a
/// "restart required" banner.
#[tauri::command]
pub fn waypointer_set_plugin_enabled(id: String, enabled: bool) -> Result<bool, String> {
    let path = modules_config_path();
    let mut cfg = read_modules_config(&path);

    let list = &mut cfg.waypointer.disabled_plugins;
    list.retain(|p| p != &id);
    if !enabled {
        list.push(id);
    }
    list.sort();
    list.dedup();

    write_modules_config(&path, &cfg)?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_parse_minimal() {
        let toml_str = r#"
[[plugin]]
id = "core.calculator"
name = "Calculator"
description = "Math"
priority = 0
prefix = "="
"#;
        let file: RegistryFile = toml::from_str(toml_str).unwrap();
        assert_eq!(file.plugin.len(), 1);
        assert_eq!(file.plugin[0].id, "core.calculator");
        assert_eq!(file.plugin[0].prefix.as_deref(), Some("="));
    }

    #[test]
    fn modules_config_roundtrip_preserves_unrelated() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("modules.toml");
        std::fs::write(
            &path,
            "[disabled]\nmodules = [\"com.example.foo\"]\n\n\
             [waypointer]\ndisabled_plugins = [\"core.unicode\"]\n",
        )
        .unwrap();

        let mut cfg = read_modules_config(&path);
        // Toggle core.unicode on, core.calculator off.
        cfg.waypointer.disabled_plugins.retain(|p| p != "core.unicode");
        cfg.waypointer.disabled_plugins.push("core.calculator".into());
        write_modules_config(&path, &cfg).unwrap();

        let reread = std::fs::read_to_string(&path).unwrap();
        // Filesystem-module section survives.
        assert!(reread.contains("com.example.foo"));
        // Waypointer section was updated.
        assert!(reread.contains("core.calculator"));
        assert!(!reread.contains("core.unicode"));
    }

    #[test]
    fn modules_config_default_is_empty_both_sections() {
        let cfg = ModulesConfig::default();
        let s = toml::to_string_pretty(&cfg).unwrap();
        // Both sections omit themselves when empty -> tiny/empty output.
        assert!(!s.contains("[disabled]"));
        assert!(!s.contains("[waypointer]"));
    }
}
