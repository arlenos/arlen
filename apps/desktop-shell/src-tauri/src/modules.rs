/// Module loader: discovers, loads, and manages Lunaris extension modules.
///
/// System modules: `/usr/share/lunaris/modules/{id}/`
/// User modules: `~/.local/share/lunaris/modules/{id}/`
/// User modules override system modules with the same ID.
///
/// Enabled state persisted in `~/.config/lunaris/modules.toml`.
///
/// See `docs/architecture/module-system.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Where a module was loaded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModuleSource {
    System,
    User,
}

/// A discovered and loaded module.
#[derive(Debug, Clone)]
pub struct LoadedModule {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub module_type: String,
    pub path: PathBuf,
    pub source: ModuleSource,
    pub enabled: bool,
    pub has_waypointer: bool,
    pub has_topbar: bool,
    pub has_settings: bool,
    pub icon: String,
}

/// Summary for the frontend list.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub module_type: String,
    pub source: ModuleSource,
    pub enabled: bool,
    pub has_waypointer: bool,
    pub has_topbar: bool,
    pub has_settings: bool,
    pub icon: String,
}

/// Internal manifest structure (subset of lunaris-modules::ModuleManifest,
/// kept simple to avoid cross-crate dependency for now).
#[derive(Debug, Clone, Deserialize)]
struct ManifestFile {
    module: ManifestMeta,
    #[serde(default)]
    waypointer: Option<toml::Value>,
    #[serde(default)]
    topbar: Option<toml::Value>,
    #[serde(default)]
    settings: Option<toml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct ManifestMeta {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(rename = "type", default = "default_type")]
    module_type: String,
    #[serde(default)]
    icon: String,
}

fn default_type() -> String {
    "third-party".into()
}

/// Disabled modules config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ModulesConfig {
    #[serde(default)]
    disabled: DisabledSection,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DisabledSection {
    #[serde(default)]
    modules: Vec<String>,
}

// ---------------------------------------------------------------------------
// Module loader
// ---------------------------------------------------------------------------

/// Manages discovered modules.
pub struct ModuleLoader {
    modules: HashMap<String, LoadedModule>,
    config_path: PathBuf,
}

impl ModuleLoader {
    /// Create a loader and discover all modules.
    pub fn new() -> Self {
        let config_path = modules_config_path();
        let disabled = load_disabled_list(&config_path);
        let mut modules = HashMap::new();

        // System modules first.
        for m in scan_directory(&system_modules_dir(), ModuleSource::System, &disabled) {
            modules.insert(m.id.clone(), m);
        }

        // User modules override system.
        for m in scan_directory(&user_modules_dir(), ModuleSource::User, &disabled) {
            modules.insert(m.id.clone(), m);
        }

        Self {
            modules,
            config_path,
        }
    }

    /// List all modules.
    pub fn list(&self) -> Vec<&LoadedModule> {
        let mut list: Vec<_> = self.modules.values().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// Get a module by ID.
    pub fn get(&self, id: &str) -> Option<&LoadedModule> {
        self.modules.get(id)
    }

    /// Set enabled state and persist.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        if let Some(m) = self.modules.get_mut(id) {
            m.enabled = enabled;
        }
        self.save_disabled_list();
    }

    /// Re-scan modules from disk.
    pub fn refresh(&mut self) {
        let disabled = load_disabled_list(&self.config_path);
        let mut modules = HashMap::new();

        for m in scan_directory(&system_modules_dir(), ModuleSource::System, &disabled) {
            modules.insert(m.id.clone(), m);
        }
        for m in scan_directory(&user_modules_dir(), ModuleSource::User, &disabled) {
            modules.insert(m.id.clone(), m);
        }

        self.modules = modules;
    }

    fn save_disabled_list(&self) {
        let disabled: Vec<String> = self
            .modules
            .values()
            .filter(|m| !m.enabled)
            .map(|m| m.id.clone())
            .collect();

        let config = ModulesConfig {
            disabled: DisabledSection { modules: disabled },
        };

        if let Some(parent) = self.config_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(toml) = toml::to_string_pretty(&config) {
            let _ = std::fs::write(&self.config_path, toml);
        }
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn system_modules_dir() -> PathBuf {
    std::env::var("LUNARIS_SYSTEM_MODULES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/share/lunaris/modules"))
}

fn user_modules_dir() -> PathBuf {
    let data = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|_| {
            std::env::var("HOME").map(|h| PathBuf::from(h).join(".local/share"))
        })
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    data.join("lunaris/modules")
}

fn modules_config_path() -> PathBuf {
    let config = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|_| {
            std::env::var("HOME").map(|h| PathBuf::from(h).join(".config"))
        })
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    config.join("lunaris/modules.toml")
}

fn load_disabled_list(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|c| toml::from_str::<ModulesConfig>(&c).ok())
        .map(|c| c.disabled.modules)
        .unwrap_or_default()
}

fn scan_directory(
    dir: &Path,
    source: ModuleSource,
    disabled: &[String],
) -> Vec<LoadedModule> {
    let mut modules = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return modules,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("manifest.toml");
        if !manifest_path.exists() {
            continue;
        }
        if let Some(m) = load_single_module(&manifest_path, &path, source, disabled) {
            modules.push(m);
        }
    }

    modules
}

fn load_single_module(
    manifest_path: &Path,
    module_dir: &Path,
    source: ModuleSource,
    disabled: &[String],
) -> Option<LoadedModule> {
    let content = std::fs::read_to_string(manifest_path).ok()?;
    let manifest: ManifestFile = toml::from_str(&content).ok()?;

    let enabled = !disabled.contains(&manifest.module.id);

    Some(LoadedModule {
        id: manifest.module.id.clone(),
        name: manifest.module.name,
        version: manifest.module.version,
        description: manifest.module.description,
        module_type: manifest.module.module_type,
        path: module_dir.to_path_buf(),
        source,
        enabled,
        has_waypointer: manifest.waypointer.is_some(),
        has_topbar: manifest.topbar.is_some(),
        has_settings: manifest.settings.is_some(),
        icon: manifest.module.icon,
    })
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

use std::sync::Mutex;

/// Shared module loader state.
pub type ModuleLoaderState = Mutex<ModuleLoader>;

#[tauri::command]
pub fn list_modules(state: tauri::State<'_, ModuleLoaderState>) -> Vec<ModuleSummary> {
    let loader = state.lock().unwrap();
    loader
        .list()
        .into_iter()
        .map(|m| ModuleSummary {
            id: m.id.clone(),
            name: m.name.clone(),
            version: m.version.clone(),
            description: m.description.clone(),
            module_type: m.module_type.clone(),
            source: m.source,
            enabled: m.enabled,
            has_waypointer: m.has_waypointer,
            has_topbar: m.has_topbar,
            has_settings: m.has_settings,
            icon: m.icon.clone(),
        })
        .collect()
}

#[tauri::command]
pub fn set_module_enabled(
    id: String,
    enabled: bool,
    state: tauri::State<'_, ModuleLoaderState>,
) -> Result<(), String> {
    let mut loader = state.lock().unwrap();
    if loader.get(&id).is_none() {
        return Err(format!("module not found: {id}"));
    }
    loader.set_enabled(&id, enabled);
    Ok(())
}
