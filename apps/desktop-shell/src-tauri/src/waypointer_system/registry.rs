/// Waypointer plugin registry.
///
/// Written by the shell at startup to `~/.local/share/lunaris/waypointer-plugins.toml`
/// so the Settings app can list built-in plugins in its Extensions
/// panel without needing cross-process Tauri IPC. The user's disabled
/// list lives in `~/.config/lunaris/modules.toml` under `[waypointer]`,
/// shared between the shell (filters at startup) and Settings (toggles
/// on click).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::manager::PluginManager;
use super::plugin::PluginDescriptor;

// ── Paths ───────────────────────────────────────────────────────────────

/// Where the shell writes the list of registered plugins so the
/// Settings app can read it.
pub fn registry_path() -> PathBuf {
    if let Ok(p) = std::env::var("LUNARIS_WAYPOINTER_REGISTRY") {
        return PathBuf::from(p);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lunaris/waypointer-plugins.toml")
}

/// The shared modules config. We read `[waypointer].disabled_plugins`
/// from it; existing `[disabled].modules` entries (filesystem modules)
/// are left untouched.
pub fn modules_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("LUNARIS_MODULES_CONFIG") {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lunaris/modules.toml")
}

// ── Registry file schema ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegistryFile {
    #[serde(default)]
    pub plugin: Vec<PluginDescriptor>,
}

/// Writes the current plugin roster to the registry file. Failures are
/// logged but not propagated so shell startup is never blocked by a
/// missing data directory.
pub fn write_registry(mgr: &PluginManager) {
    write_registry_to(&registry_path(), mgr);
}

/// Explicit-path variant used by tests. Also callable by anything else
/// that needs to redirect the registry location (eg. sandbox setups).
pub fn write_registry_to(path: &Path, mgr: &PluginManager) {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::warn!("waypointer registry: could not create dir: {e}");
            return;
        }
    }

    let file = RegistryFile {
        plugin: mgr.plugin_descriptors(),
    };

    let content = match toml::to_string_pretty(&file) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("waypointer registry: serialize failed: {e}");
            return;
        }
    };

    if let Err(e) = std::fs::write(path, content) {
        log::warn!("waypointer registry: write failed ({}): {e}", path.display());
        return;
    }
    log::info!(
        "waypointer registry: wrote {} plugins to {}",
        file.plugin.len(),
        path.display()
    );
}

// ── Disabled-list schema ────────────────────────────────────────────────

/// Minimal view of `modules.toml` that only cares about the
/// `[waypointer].disabled_plugins` list. Other sections (e.g.
/// `[disabled].modules`) are preserved on read because we do not
/// round-trip the full document.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ModulesConfigView {
    #[serde(default)]
    waypointer: WaypointerSection,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct WaypointerSection {
    #[serde(default)]
    disabled_plugins: Vec<String>,
}

/// Reads the list of disabled built-in plugin IDs from `modules.toml`.
/// Missing file or missing section yields an empty list.
pub fn load_disabled_plugins() -> Vec<String> {
    load_disabled_plugins_from(&modules_config_path())
}

/// Explicit-path variant used by tests.
pub fn load_disabled_plugins_from(path: &Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    match toml::from_str::<ModulesConfigView>(&content) {
        Ok(v) => v.waypointer.disabled_plugins,
        Err(e) => {
            log::warn!("waypointer registry: modules.toml parse failed: {e}");
            Vec::new()
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_file_roundtrip() {
        let desc = PluginDescriptor {
            id: "core.calculator".into(),
            name: "Calculator".into(),
            description: "Evaluates math expressions".into(),
            priority: 0,
            prefix: Some("=".into()),
            pattern: None,
        };
        let file = RegistryFile { plugin: vec![desc.clone()] };
        let toml_str = toml::to_string_pretty(&file).unwrap();
        let parsed: RegistryFile = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.plugin.len(), 1);
        assert_eq!(parsed.plugin[0].id, desc.id);
        assert_eq!(parsed.plugin[0].prefix, desc.prefix);
    }

    #[test]
    fn disabled_list_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("does-not-exist.toml");
        assert!(load_disabled_plugins_from(&path).is_empty());
    }

    #[test]
    fn disabled_list_reads_waypointer_section() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("modules.toml");
        std::fs::write(
            &cfg,
            "[disabled]\nmodules = [\"com.example.foo\"]\n\n\
             [waypointer]\ndisabled_plugins = [\"core.calculator\", \"core.unicode\"]\n",
        )
        .unwrap();
        assert_eq!(
            load_disabled_plugins_from(&cfg),
            vec!["core.calculator", "core.unicode"]
        );
    }

    #[test]
    fn disabled_list_ignores_other_sections() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("modules.toml");
        std::fs::write(&cfg, "[disabled]\nmodules = [\"com.example.foo\"]\n").unwrap();
        assert!(load_disabled_plugins_from(&cfg).is_empty());
    }
}
