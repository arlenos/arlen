//! Extensions panel backend.
//!
//! Discovers modules in:
//!   * `/usr/share/lunaris/modules/`    — system (read-only)
//!   * `~/.local/share/lunaris/modules/` — user-installed (removable)
//!
//! Enabled state is persisted in `~/.config/lunaris/modules.toml` as a
//! flat list of disabled module IDs. This matches the shell's
//! `ModuleLoader` (`desktop-shell/src-tauri/src/modules.rs`) so both
//! processes see the same source of truth.
//!
//! Parsing uses the shared `lunaris-modules` crate so the manifest
//! schema stays in sync with the SDK.

use std::path::{Path, PathBuf};

use lunaris_modules::{load_manifest, parse_manifest, ModuleManifest, ModuleType};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types exposed to the frontend
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModuleSource {
    System,
    User,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    /// "system" / "first-party" / "third-party"
    pub module_type: String,
    pub source: ModuleSource,
    pub enabled: bool,
    pub has_waypointer: bool,
    pub has_topbar: bool,
    pub has_settings: bool,
    pub icon: String,
    /// Absolute filesystem path (used for uninstall). Not shown in UI.
    pub path: String,
    /// Non-fatal manifest validation warnings ("bad semver", etc.).
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

fn system_modules_dir() -> PathBuf {
    std::env::var("LUNARIS_SYSTEM_MODULES")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/usr/share/lunaris/modules"))
}

fn user_modules_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lunaris/modules")
}

fn modules_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lunaris/modules.toml")
}

// ---------------------------------------------------------------------------
// Disabled-list persistence
// ---------------------------------------------------------------------------

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

fn load_disabled_list() -> Vec<String> {
    let path = modules_config_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|c| toml::from_str::<ModulesConfig>(&c).ok())
        .map(|c| c.disabled.modules)
        .unwrap_or_default()
}

fn save_disabled_list(disabled: Vec<String>) -> Result<(), String> {
    let path = modules_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
    }
    let cfg = ModulesConfig {
        disabled: DisabledSection { modules: disabled },
    };
    let toml_str = toml::to_string_pretty(&cfg).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(&path, toml_str).map_err(|e| format!("write: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn module_type_str(t: ModuleType) -> &'static str {
    match t {
        ModuleType::System => "system",
        ModuleType::FirstParty => "first-party",
        ModuleType::ThirdParty => "third-party",
    }
}

/// Try every `manifest.toml` under `dir`. We use `parse_manifest` rather
/// than `load_manifest` so a missing `entry` file is just a warning —
/// the Settings app should show invalid modules too, not hide them.
fn scan_dir(dir: &Path, source: ModuleSource, disabled: &[String]) -> Vec<ModuleSummary> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join("manifest.toml");
        if !manifest_path.exists() {
            continue;
        }
        if let Some(summary) = load_one(&manifest_path, &path, source, disabled) {
            out.push(summary);
        }
    }
    out
}

fn load_one(
    manifest_path: &Path,
    module_dir: &Path,
    source: ModuleSource,
    disabled: &[String],
) -> Option<ModuleSummary> {
    let content = std::fs::read_to_string(manifest_path).ok()?;
    let manifest: ModuleManifest = match parse_manifest(&content) {
        Ok(m) => m,
        Err(e) => {
            log::warn!(
                "modules: failed to parse {}: {e}",
                manifest_path.display()
            );
            return None;
        }
    };

    let mut warnings: Vec<String> = lunaris_modules::validate_manifest(&manifest)
        .into_iter()
        .map(|w| format!("{}: {}", w.field, w.message))
        .collect();

    // Fall back to a "try-load" to surface missing-entry errors as
    // an extra warning; a missing entry file doesn't block the module
    // from appearing in the list.
    if let Err(e) = load_manifest(manifest_path) {
        if !matches!(e, lunaris_modules::ManifestError::Io(_)) {
            warnings.push(format!("load: {e}"));
        }
    }

    let enabled = !disabled.contains(&manifest.module.id);

    // We don't have an author field in the SDK struct today; fall back
    // to empty string. The UI shows a placeholder in that case.
    let author = String::new();

    Some(ModuleSummary {
        id: manifest.module.id.clone(),
        name: manifest.module.name,
        version: manifest.module.version,
        description: manifest.module.description,
        author,
        module_type: module_type_str(manifest.module.module_type).into(),
        source,
        enabled,
        has_waypointer: manifest.waypointer.is_some(),
        has_topbar: manifest.topbar.is_some(),
        has_settings: manifest.settings.is_some(),
        icon: manifest.module.icon,
        path: module_dir.to_string_lossy().into_owned(),
        warnings,
    })
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// List all discovered modules, merged with the disabled state.
/// User modules with the same id override system modules.
#[tauri::command]
pub fn modules_list() -> Vec<ModuleSummary> {
    let disabled = load_disabled_list();
    let mut map: std::collections::HashMap<String, ModuleSummary> =
        std::collections::HashMap::new();

    for m in scan_dir(&system_modules_dir(), ModuleSource::System, &disabled) {
        map.insert(m.id.clone(), m);
    }
    for m in scan_dir(&user_modules_dir(), ModuleSource::User, &disabled) {
        map.insert(m.id.clone(), m);
    }

    let mut out: Vec<_> = map.into_values().collect();
    out.sort_by(|a, b| {
        // Enabled first, then by name.
        b.enabled
            .cmp(&a.enabled)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

/// Toggle enabled state. Persists to `modules.toml`.
/// Returns `true` so the frontend can show a "restart required" banner.
#[tauri::command]
pub fn modules_set_enabled(id: String, enabled: bool) -> Result<bool, String> {
    let mut disabled = load_disabled_list();
    disabled.retain(|d| d != &id);
    if !enabled {
        disabled.push(id);
    }
    disabled.sort();
    disabled.dedup();
    save_disabled_list(disabled)?;
    Ok(true)
}

/// Uninstall a user module by deleting its directory.
/// System modules are refused — they are managed by the OS package.
#[tauri::command]
pub fn modules_uninstall(id: String) -> Result<(), String> {
    let user_dir = user_modules_dir();
    let list = modules_list();
    let entry = list
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("module not found: {id}"))?;

    if entry.source != ModuleSource::User {
        return Err(format!(
            "'{id}' is a system module and cannot be uninstalled from the Settings app"
        ));
    }

    let target = PathBuf::from(&entry.path);
    // Sanity check: must be inside user_modules_dir.
    if !target.starts_with(&user_dir) {
        return Err(format!(
            "refusing to delete '{}' — outside the user modules directory",
            target.display()
        ));
    }
    std::fs::remove_dir_all(&target).map_err(|e| format!("remove: {e}"))?;

    // Also clean up the disabled list so a reinstall of the same id
    // starts fresh.
    let mut disabled = load_disabled_list();
    disabled.retain(|d| d != &id);
    save_disabled_list(disabled)?;

    Ok(())
}
