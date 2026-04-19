/// Module manifest parser and validation for Lunaris OS.
///
/// Modules extend the shell via well-defined extension points (Waypointer
/// search, top bar indicators, settings panels, etc.). Each module has a
/// `manifest.toml` describing its metadata, extensions, and capabilities.
///
/// See `docs/architecture/module-system.md`.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(String),
    #[error("validation: {0}")]
    Validation(String),
}

/// Non-fatal warnings from manifest validation.
#[derive(Debug, Clone)]
pub struct ValidationWarning {
    pub field: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Module type
// ---------------------------------------------------------------------------

/// Trust tier for a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModuleType {
    System,
    FirstParty,
    ThirdParty,
}

impl ModuleType {
    /// Default priority for this tier (lower = higher priority).
    pub fn default_priority(self) -> u32 {
        match self {
            Self::System => 0,
            Self::FirstParty => 10,
            Self::ThirdParty => 100,
        }
    }
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

/// Parsed `manifest.toml` for a Lunaris module.
#[derive(Debug, Clone, Deserialize)]
pub struct ModuleManifest {
    pub module: ModuleMeta,
    #[serde(default)]
    pub waypointer: Option<WaypointerConfig>,
    #[serde(default)]
    pub topbar: Option<TopbarConfig>,
    #[serde(default)]
    pub settings: Option<SettingsConfig>,
    #[serde(default)]
    pub capabilities: ModuleCapabilities,
    /// Declared permission requests. Mirrored onto the runtime
    /// `sdk/permissions::PermissionProfile` by the install daemon.
    #[serde(default)]
    pub permissions: ModulePermissions,
    /// Static keybindings the module ships. Written to
    /// `~/.config/lunaris/compositor.d/keybindings.d/<module-id>.toml`
    /// at install time; removed on uninstall.
    #[serde(default, rename = "keybinding")]
    pub keybindings: Vec<ModuleKeybinding>,
}

/// Module metadata section.
#[derive(Debug, Clone, Deserialize)]
pub struct ModuleMeta {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "type", default = "default_module_type")]
    pub module_type: ModuleType,
    /// Entry point relative to the module directory.
    #[serde(default = "default_entry")]
    pub entry: String,
    #[serde(default)]
    pub icon: String,
}

fn default_module_type() -> ModuleType {
    ModuleType::ThirdParty
}

fn default_entry() -> String {
    "index.js".into()
}

// ---------------------------------------------------------------------------
// Permissions requested by the manifest
// ---------------------------------------------------------------------------

/// Manifest-level declaration of permission requests. Kept deliberately
/// minimal; the runtime enforcement side lives in `sdk/permissions`.
/// The install daemon reads this to decide whether to honour or strip
/// entries in the shipped `[[keybinding]]` list.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ModulePermissions {
    /// Input subsystem permissions. Recognised strings:
    /// * `"register_focused_bindings"` — fires only while the module's
    ///   UI is focused.
    /// * `"register_global_bindings"` — fires regardless of focus;
    ///   reserved for first-party modules.
    #[serde(default)]
    pub input: Vec<String>,
}

impl ModulePermissions {
    /// Does the manifest declare permission for the named input category?
    pub fn has_input(&self, name: &str) -> bool {
        self.input.iter().any(|p| p == name)
    }

    /// Convenience for the common check at install time.
    pub fn can_register_global_bindings(&self) -> bool {
        self.has_input("register_global_bindings")
    }
}

// ---------------------------------------------------------------------------
// Keybindings
// ---------------------------------------------------------------------------

/// A single keybinding shipped by a module. Translated into a TOML
/// fragment under `~/.config/lunaris/compositor.d/keybindings.d/` at
/// install time.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModuleKeybinding {
    /// Stable identifier within the module, e.g. `"open_picker"`.
    /// Used to build the compositor action string when `action` is
    /// absent: `module:<module_id>:<id>`.
    pub id: String,
    /// Human-readable label shown in the Settings keybinding list.
    pub label: String,
    /// Accelerator string parsed by the compositor, e.g. `"Super+Shift+C"`.
    pub default_binding: String,
    /// Optional pre-composed action string. If `None`, the install
    /// daemon synthesises `module:<module_id>:<id>`.
    #[serde(default)]
    pub action: Option<String>,
    /// Optional description shown as a subtitle in Settings.
    #[serde(default)]
    pub description: Option<String>,
    /// `"global"` (fires anywhere) or `"focused"` (fires only when the
    /// module's own window has keyboard focus). Defaults to `"global"`.
    #[serde(default = "default_keybinding_scope")]
    pub scope: String,
}

fn default_keybinding_scope() -> String {
    "global".into()
}

impl ModuleKeybinding {
    /// Compute the action string that should be written to the
    /// compositor fragment for this binding, given the owning module id.
    pub fn effective_action(&self, module_id: &str) -> String {
        self.action
            .clone()
            .unwrap_or_else(|| format!("module:{module_id}:{}", self.id))
    }
}

// ---------------------------------------------------------------------------
// Extension points
// ---------------------------------------------------------------------------

/// Waypointer search/action extension.
#[derive(Debug, Clone, Deserialize)]
pub struct WaypointerConfig {
    #[serde(default)]
    pub search: Option<WaypointerSearchConfig>,
    #[serde(default)]
    pub action: Option<WaypointerActionConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WaypointerSearchConfig {
    #[serde(default = "default_priority")]
    pub priority: u32,
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub detect_pattern: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WaypointerActionConfig {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub shortcut: Option<String>,
}

/// Top bar indicator extension.
#[derive(Debug, Clone, Deserialize)]
pub struct TopbarConfig {
    #[serde(default)]
    pub indicator: Option<TopbarIndicatorConfig>,
    #[serde(default)]
    pub applet: Option<TopbarAppletConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TopbarIndicatorConfig {
    #[serde(default = "default_slot")]
    pub slot: String,
    #[serde(default = "default_order")]
    pub order: u32,
    #[serde(default = "default_polling")]
    pub polling_interval: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TopbarAppletConfig {
    pub title: String,
    #[serde(default)]
    pub icon: String,
}

/// Settings panel extension.
#[derive(Debug, Clone, Deserialize)]
pub struct SettingsConfig {
    pub panel: Option<SettingsPanelConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingsPanelConfig {
    pub title: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub category: String,
}

fn default_priority() -> u32 {
    100
}
fn default_slot() -> String {
    "temp".into()
}
fn default_order() -> u32 {
    50
}
fn default_polling() -> u32 {
    30
}

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

/// Module capability requests (subset of full PermissionProfile).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModuleCapabilities {
    #[serde(default)]
    pub network: Option<NetworkCapability>,
    #[serde(default)]
    pub storage: Option<StorageCapability>,
    #[serde(default)]
    pub notifications: bool,
    #[serde(default)]
    pub clipboard: Option<ClipboardCapability>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkCapability {
    #[serde(default)]
    pub allowed_domains: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageCapability {
    #[serde(default = "default_storage_quota")]
    pub quota_mb: u32,
}

fn default_storage_quota() -> u32 {
    50
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClipboardCapability {
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub write: bool,
}

// ---------------------------------------------------------------------------
// Parsing and loading
// ---------------------------------------------------------------------------

/// Parse a manifest from a TOML string.
pub fn parse_manifest(toml_str: &str) -> Result<ModuleManifest, ManifestError> {
    toml::from_str(toml_str).map_err(|e| ManifestError::Parse(e.to_string()))
}

/// Load a manifest from a file path.
pub fn load_manifest(path: &Path) -> Result<ModuleManifest, ManifestError> {
    let content = std::fs::read_to_string(path)?;
    let manifest = parse_manifest(&content)?;

    // Validate entry file exists relative to the manifest directory.
    if let Some(dir) = path.parent() {
        let entry_path = dir.join(&manifest.module.entry);
        if !entry_path.exists() {
            return Err(ManifestError::Validation(format!(
                "entry file not found: {}",
                entry_path.display()
            )));
        }
    }

    Ok(manifest)
}

/// Validate a manifest and return non-fatal warnings.
pub fn validate_manifest(manifest: &ModuleManifest) -> Vec<ValidationWarning> {
    let mut warnings = Vec::new();

    // ID must be reverse-domain.
    if !is_reverse_domain(&manifest.module.id) {
        warnings.push(ValidationWarning {
            field: "module.id".into(),
            message: format!("'{}' is not valid reverse-domain notation", manifest.module.id),
        });
    }

    // Version should be semver-like.
    if !is_semver_like(&manifest.module.version) {
        warnings.push(ValidationWarning {
            field: "module.version".into(),
            message: format!("'{}' is not a valid semver version", manifest.module.version),
        });
    }

    // Name should not be empty.
    if manifest.module.name.trim().is_empty() {
        warnings.push(ValidationWarning {
            field: "module.name".into(),
            message: "module name is empty".into(),
        });
    }

    // Waypointer search: prefix should be short.
    if let Some(wp) = &manifest.waypointer {
        if let Some(search) = &wp.search {
            if let Some(prefix) = &search.prefix {
                if prefix.len() > 5 {
                    warnings.push(ValidationWarning {
                        field: "waypointer.search.prefix".into(),
                        message: "prefix is unusually long (>5 chars)".into(),
                    });
                }
            }
        }
    }

    // Network domains should not be wildcards.
    if let Some(net) = &manifest.capabilities.network {
        for domain in &net.allowed_domains {
            if domain == "*" || domain.starts_with("*.") {
                warnings.push(ValidationWarning {
                    field: "capabilities.network.allowed_domains".into(),
                    message: format!("wildcard domain '{domain}' is not allowed"),
                });
            }
        }
    }

    // Input permission strings must be recognised. Unknown entries are
    // forward-compat: we keep the manifest valid but log a warning so
    // old installers don't silently discard new permission kinds.
    const KNOWN_INPUT_PERMS: &[&str] = &[
        "register_focused_bindings",
        "register_global_bindings",
    ];
    for perm in &manifest.permissions.input {
        if !KNOWN_INPUT_PERMS.contains(&perm.as_str()) {
            warnings.push(ValidationWarning {
                field: "permissions.input".into(),
                message: format!("unknown input permission '{perm}'"),
            });
        }
    }

    // Keybindings.
    for (i, kb) in manifest.keybindings.iter().enumerate() {
        let prefix = format!("keybinding[{i}]");
        if kb.id.trim().is_empty() {
            warnings.push(ValidationWarning {
                field: format!("{prefix}.id"),
                message: "keybinding id must not be empty".into(),
            });
        }
        if kb.label.trim().is_empty() {
            warnings.push(ValidationWarning {
                field: format!("{prefix}.label"),
                message: "keybinding label must not be empty".into(),
            });
        }
        if kb.default_binding.trim().is_empty() {
            warnings.push(ValidationWarning {
                field: format!("{prefix}.default_binding"),
                message: "keybinding default_binding must not be empty".into(),
            });
        } else if !is_valid_binding_format(&kb.default_binding) {
            warnings.push(ValidationWarning {
                field: format!("{prefix}.default_binding"),
                message: format!(
                    "'{}' uses an unknown modifier; expected any of Super/Ctrl/Alt/Shift",
                    kb.default_binding
                ),
            });
        }
        if kb.scope != "global" && kb.scope != "focused" {
            warnings.push(ValidationWarning {
                field: format!("{prefix}.scope"),
                message: format!("scope must be 'global' or 'focused', got '{}'", kb.scope),
            });
        }
        if kb.scope == "global"
            && !manifest
                .permissions
                .can_register_global_bindings()
        {
            warnings.push(ValidationWarning {
                field: format!("{prefix}"),
                message: "global keybinding declared without \
                     permissions.input = [\"register_global_bindings\"]"
                    .into(),
            });
        }
    }

    warnings
}

/// Quick syntactic check for accelerator strings like `"Super+Shift+H"`.
///
/// Accepted modifiers (case-insensitive): `Super`, `Logo`, `Mod4`,
/// `Shift`, `Ctrl`, `Control`, `Alt`, `Mod1`. The final token is the
/// key name and may be any non-empty string — the compositor does the
/// full keysym resolution at dispatch time.
fn is_valid_binding_format(binding: &str) -> bool {
    const VALID_MODIFIERS: &[&str] = &[
        "super", "logo", "mod4", "shift", "ctrl", "control", "alt", "mod1",
    ];
    let parts: Vec<&str> = binding.split('+').collect();
    if parts.is_empty() {
        return false;
    }
    let last = match parts.last() {
        Some(p) if !p.is_empty() => *p,
        _ => return false,
    };
    for part in &parts[..parts.len() - 1] {
        let lower = part.to_lowercase();
        if !VALID_MODIFIERS.contains(&lower.as_str()) {
            return false;
        }
    }
    !last.is_empty()
}

/// Check if a string looks like reverse-domain (e.g. "com.example.app").
fn is_reverse_domain(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() >= 2
        && parts.iter().all(|p| {
            !p.is_empty() && p.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        })
}

/// Check if a string looks like semver (X.Y.Z with optional pre-release).
fn is_semver_like(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() >= 2
        && parts.len() <= 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_digit() || c == '-' || c.is_alphanumeric())
        })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const FULL_MANIFEST: &str = r#"
[module]
id = "com.example.calculator"
name = "Calculator"
version = "1.0.0"
description = "Quick calculations in Waypointer"
type = "third-party"
entry = "index.js"
icon = "calculator"

[waypointer.search]
priority = 100
prefix = "="

[waypointer.action]
name = "Calculate"
description = "Open calculator"
icon = "calculator"

[topbar.indicator]
slot = "temp"
order = 50
polling_interval = 30

[settings.panel]
title = "Calculator Settings"
icon = "calculator"
category = "modules"

[capabilities]
notifications = true

[capabilities.network]
allowed_domains = ["api.example.com"]

[capabilities.clipboard]
read = true
write = false

[capabilities.storage]
quota_mb = 100
"#;

    #[test]
    fn test_parse_full_manifest() {
        let m = parse_manifest(FULL_MANIFEST).unwrap();
        assert_eq!(m.module.id, "com.example.calculator");
        assert_eq!(m.module.name, "Calculator");
        assert_eq!(m.module.version, "1.0.0");
        assert_eq!(m.module.module_type, ModuleType::ThirdParty);
        assert_eq!(m.module.entry, "index.js");
    }

    #[test]
    fn test_waypointer_config() {
        let m = parse_manifest(FULL_MANIFEST).unwrap();
        let wp = m.waypointer.unwrap();
        let search = wp.search.unwrap();
        assert_eq!(search.priority, 100);
        assert_eq!(search.prefix.as_deref(), Some("="));
        let action = wp.action.unwrap();
        assert_eq!(action.name, "Calculate");
    }

    #[test]
    fn test_topbar_config() {
        let m = parse_manifest(FULL_MANIFEST).unwrap();
        let tb = m.topbar.unwrap();
        let ind = tb.indicator.unwrap();
        assert_eq!(ind.slot, "temp");
        assert_eq!(ind.order, 50);
        assert_eq!(ind.polling_interval, 30);
    }

    #[test]
    fn test_settings_config() {
        let m = parse_manifest(FULL_MANIFEST).unwrap();
        let s = m.settings.unwrap();
        let panel = s.panel.unwrap();
        assert_eq!(panel.title, "Calculator Settings");
    }

    #[test]
    fn test_capabilities() {
        let m = parse_manifest(FULL_MANIFEST).unwrap();
        assert!(m.capabilities.notifications);
        let net = m.capabilities.network.unwrap();
        assert_eq!(net.allowed_domains, vec!["api.example.com"]);
        let clip = m.capabilities.clipboard.unwrap();
        assert!(clip.read);
        assert!(!clip.write);
        let storage = m.capabilities.storage.unwrap();
        assert_eq!(storage.quota_mb, 100);
    }

    #[test]
    fn test_minimal_manifest() {
        let toml = r#"
[module]
id = "com.test.minimal"
name = "Minimal"
version = "0.1.0"
"#;
        let m = parse_manifest(toml).unwrap();
        assert_eq!(m.module.module_type, ModuleType::ThirdParty); // default
        assert_eq!(m.module.entry, "index.js"); // default
        assert!(m.waypointer.is_none());
        assert!(m.topbar.is_none());
        assert!(m.settings.is_none());
        assert!(!m.capabilities.notifications);
    }

    #[test]
    fn test_system_module() {
        let toml = r#"
[module]
id = "org.lunaris.core-search"
name = "Core Search"
version = "1.0.0"
type = "system"
"#;
        let m = parse_manifest(toml).unwrap();
        assert_eq!(m.module.module_type, ModuleType::System);
        assert_eq!(ModuleType::System.default_priority(), 0);
    }

    #[test]
    fn test_validate_valid() {
        let m = parse_manifest(FULL_MANIFEST).unwrap();
        let warnings = validate_manifest(&m);
        assert!(warnings.is_empty(), "expected no warnings: {:?}", warnings);
    }

    #[test]
    fn test_validate_bad_id() {
        let toml = r#"
[module]
id = "bad"
name = "Bad"
version = "1.0.0"
"#;
        let m = parse_manifest(toml).unwrap();
        let warnings = validate_manifest(&m);
        assert!(warnings.iter().any(|w| w.field == "module.id"));
    }

    #[test]
    fn test_validate_bad_version() {
        let toml = r#"
[module]
id = "com.test.app"
name = "Test"
version = "not-a-version"
"#;
        let m = parse_manifest(toml).unwrap();
        let warnings = validate_manifest(&m);
        assert!(warnings.iter().any(|w| w.field == "module.version"));
    }

    #[test]
    fn test_validate_wildcard_domain() {
        let toml = r#"
[module]
id = "com.test.app"
name = "Test"
version = "1.0.0"

[capabilities.network]
allowed_domains = ["*.evil.com"]
"#;
        let m = parse_manifest(toml).unwrap();
        let warnings = validate_manifest(&m);
        assert!(warnings.iter().any(|w| w.field.contains("network")));
    }

    #[test]
    fn test_validate_long_prefix() {
        let toml = r#"
[module]
id = "com.test.app"
name = "Test"
version = "1.0.0"

[waypointer.search]
prefix = "longprefix"
"#;
        let m = parse_manifest(toml).unwrap();
        let warnings = validate_manifest(&m);
        assert!(warnings.iter().any(|w| w.field.contains("prefix")));
    }

    #[test]
    fn test_load_manifest_with_entry() {
        let dir = tempfile::TempDir::new().unwrap();
        // Write manifest.
        let manifest_path = dir.path().join("manifest.toml");
        let mut f = std::fs::File::create(&manifest_path).unwrap();
        f.write_all(
            br#"
[module]
id = "com.test.loader"
name = "Loader Test"
version = "1.0.0"
entry = "dist/index.js"
"#,
        )
        .unwrap();

        // Without entry file: should fail.
        assert!(load_manifest(&manifest_path).is_err());

        // Create entry file.
        std::fs::create_dir_all(dir.path().join("dist")).unwrap();
        std::fs::write(dir.path().join("dist/index.js"), "// module").unwrap();

        // Now should succeed.
        let m = load_manifest(&manifest_path).unwrap();
        assert_eq!(m.module.id, "com.test.loader");
    }

    #[test]
    fn test_is_reverse_domain() {
        assert!(is_reverse_domain("com.example.app"));
        assert!(is_reverse_domain("org.lunaris.core"));
        assert!(is_reverse_domain("com.my-app.v2"));
        assert!(!is_reverse_domain("app"));
        assert!(!is_reverse_domain(""));
        assert!(!is_reverse_domain("com..app"));
    }

    #[test]
    fn test_is_semver_like() {
        assert!(is_semver_like("1.0.0"));
        assert!(is_semver_like("0.1.0"));
        assert!(is_semver_like("2.0.0-beta1"));
        assert!(is_semver_like("1.0"));
        assert!(!is_semver_like(""));
        assert!(!is_semver_like("v1"));
    }

    #[test]
    fn test_default_priorities() {
        assert_eq!(ModuleType::System.default_priority(), 0);
        assert_eq!(ModuleType::FirstParty.default_priority(), 10);
        assert_eq!(ModuleType::ThirdParty.default_priority(), 100);
    }

    #[test]
    fn test_missing_name_and_version_errors() {
        // Only id provided — name and version are required String fields.
        let toml = r#"
[module]
id = "com.test.bare"
"#;
        let result = parse_manifest(toml);
        assert!(result.is_err(), "should fail without name/version");
    }

    #[test]
    fn test_missing_module_section_errors() {
        let result = parse_manifest("");
        assert!(result.is_err(), "empty manifest should fail");
    }

    #[test]
    fn test_unknown_fields_ignored() {
        let toml = r#"
[module]
id = "com.test.extra"
name = "Extra"
version = "1.0.0"
some_future_field = true

[unknown_section]
x = 1
"#;
        // Unknown fields should not cause a parse error.
        let m = parse_manifest(toml).unwrap();
        assert_eq!(m.module.id, "com.test.extra");
    }

    #[test]
    fn test_validate_empty_name_warns() {
        let toml = r#"
[module]
id = "com.test.app"
name = "  "
version = "1.0.0"
"#;
        let m = parse_manifest(toml).unwrap();
        let warnings = validate_manifest(&m);
        assert!(
            warnings.iter().any(|w| w.field == "module.name"),
            "whitespace-only name should warn"
        );
    }

    #[test]
    fn test_first_party_type() {
        let toml = r#"
[module]
id = "org.lunaris.search"
name = "Search"
version = "1.0.0"
type = "first-party"
"#;
        let m = parse_manifest(toml).unwrap();
        assert_eq!(m.module.module_type, ModuleType::FirstParty);
        assert_eq!(ModuleType::FirstParty.default_priority(), 10);
    }

    // -----------------------------------------------------------------
    // Keybindings + module permissions
    // -----------------------------------------------------------------

    #[test]
    fn parses_keybinding_section() {
        let toml = r#"
[module]
id = "com.example.picker"
name = "Color Picker"
version = "1.0.0"

[permissions]
input = ["register_global_bindings"]

[[keybinding]]
id = "open_picker"
label = "Open Color Picker"
default_binding = "Super+Shift+C"

[[keybinding]]
id = "pick_from_screen"
label = "Pick Color from Screen"
default_binding = "Super+Shift+P"
scope = "global"
description = "Click anywhere to read the pixel colour."
"#;
        let m = parse_manifest(toml).unwrap();
        assert_eq!(m.keybindings.len(), 2);
        assert_eq!(m.keybindings[0].id, "open_picker");
        assert_eq!(m.keybindings[0].scope, "global");
        assert_eq!(m.keybindings[0].default_binding, "Super+Shift+C");
        assert!(m.permissions.can_register_global_bindings());
    }

    #[test]
    fn keybinding_scope_defaults_to_global() {
        let toml = r#"
[module]
id = "com.example.a"
name = "A"
version = "1.0.0"

[permissions]
input = ["register_global_bindings"]

[[keybinding]]
id = "x"
label = "X"
default_binding = "Super+X"
"#;
        let m = parse_manifest(toml).unwrap();
        assert_eq!(m.keybindings[0].scope, "global");
    }

    #[test]
    fn effective_action_synthesises_module_prefix() {
        let kb = ModuleKeybinding {
            id: "save".into(),
            label: "Save".into(),
            default_binding: "Ctrl+S".into(),
            action: None,
            description: None,
            scope: "focused".into(),
        };
        assert_eq!(kb.effective_action("com.example.editor"), "module:com.example.editor:save");
    }

    #[test]
    fn effective_action_respects_explicit_override() {
        let kb = ModuleKeybinding {
            id: "foo".into(),
            label: "Foo".into(),
            default_binding: "Super+F".into(),
            action: Some("spawn:foot".into()),
            description: None,
            scope: "global".into(),
        };
        assert_eq!(kb.effective_action("any.module.id"), "spawn:foot");
    }

    #[test]
    fn validate_flags_global_binding_without_permission() {
        let toml = r#"
[module]
id = "com.example.bad"
name = "Bad"
version = "1.0.0"

[[keybinding]]
id = "x"
label = "X"
default_binding = "Super+X"
scope = "global"
"#;
        let m = parse_manifest(toml).unwrap();
        let warnings = validate_manifest(&m);
        assert!(warnings
            .iter()
            .any(|w| w.message.contains("register_global_bindings")));
    }

    #[test]
    fn validate_flags_empty_id_and_label() {
        let toml = r#"
[module]
id = "com.example.bad"
name = "Bad"
version = "1.0.0"

[permissions]
input = ["register_focused_bindings"]

[[keybinding]]
id = ""
label = ""
default_binding = "Ctrl+S"
scope = "focused"
"#;
        let m = parse_manifest(toml).unwrap();
        let warnings = validate_manifest(&m);
        assert!(warnings.iter().any(|w| w.field.ends_with(".id")));
        assert!(warnings.iter().any(|w| w.field.ends_with(".label")));
    }

    #[test]
    fn validate_flags_bad_modifier() {
        let toml = r#"
[module]
id = "com.example.bad"
name = "Bad"
version = "1.0.0"

[permissions]
input = ["register_focused_bindings"]

[[keybinding]]
id = "x"
label = "X"
default_binding = "Hyper+X"
scope = "focused"
"#;
        let m = parse_manifest(toml).unwrap();
        let warnings = validate_manifest(&m);
        assert!(warnings
            .iter()
            .any(|w| w.field == "keybinding[0].default_binding"));
    }

    #[test]
    fn validate_flags_unknown_scope() {
        let toml = r#"
[module]
id = "com.example.bad"
name = "Bad"
version = "1.0.0"

[permissions]
input = ["register_focused_bindings"]

[[keybinding]]
id = "x"
label = "X"
default_binding = "Ctrl+X"
scope = "everywhere"
"#;
        let m = parse_manifest(toml).unwrap();
        let warnings = validate_manifest(&m);
        assert!(warnings.iter().any(|w| w.field == "keybinding[0].scope"));
    }

    #[test]
    fn is_valid_binding_format_accepts_common_combos() {
        assert!(is_valid_binding_format("Super+H"));
        assert!(is_valid_binding_format("Ctrl+Shift+Space"));
        assert!(is_valid_binding_format("F4"));
        assert!(!is_valid_binding_format("Hyper+X"));
        assert!(!is_valid_binding_format(""));
        assert!(!is_valid_binding_format("Ctrl+"));
    }
}
