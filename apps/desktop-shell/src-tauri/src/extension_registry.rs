/// Extension point registry for Lunaris modules.
///
/// Tracks which modules provide which extension points, resolves conflicts,
/// and returns sorted extension lists by priority.
///
/// See `docs/architecture/module-system.md`.

use std::collections::HashMap;

use serde::Serialize;

use crate::modules::LoadedModule;

// ---------------------------------------------------------------------------
// Extension point enum
// ---------------------------------------------------------------------------

/// Known extension points in the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionPoint {
    WaypointerSearch,
    WaypointerAction,
    TopbarIndicator,
    TopbarApplet,
    SettingsPanel,
    FilesHandler,
    ContextAction,
}

// ---------------------------------------------------------------------------
// Extension config (parsed from manifest TOML values)
// ---------------------------------------------------------------------------

/// Extension-point-specific configuration extracted from the manifest.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExtensionConfig {
    WaypointerSearch {
        prefix: Option<String>,
        detect_pattern: Option<String>,
    },
    WaypointerAction {
        name: String,
        icon: String,
    },
    TopbarIndicator {
        slot: String,
        order: u32,
        polling_interval: u32,
    },
    TopbarApplet {
        title: String,
        icon: String,
    },
    SettingsPanel {
        title: String,
        icon: String,
        category: String,
    },
    FilesHandler {
        // Future: file type patterns, etc.
    },
    ContextAction {
        // Future: context menu actions.
    },
}

// ---------------------------------------------------------------------------
// Registered extension
// ---------------------------------------------------------------------------

/// A single registered extension from one module.
#[derive(Debug, Clone, Serialize)]
pub struct RegisteredExtension {
    pub module_id: String,
    pub module_name: String,
    pub extension_point: ExtensionPoint,
    pub priority: u32,
    pub config: ExtensionConfig,
}

/// A conflict between two extensions.
#[derive(Debug, Clone, Serialize)]
pub struct Conflict {
    pub extension_point: ExtensionPoint,
    pub field: String,
    pub value: String,
    pub module_a: String,
    pub module_b: String,
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Central registry of all active extension points.
pub struct ExtensionRegistry {
    extensions: HashMap<ExtensionPoint, Vec<RegisteredExtension>>,
}

impl ExtensionRegistry {
    pub fn new() -> Self {
        Self {
            extensions: HashMap::new(),
        }
    }

    /// Register all extensions declared by a module.
    pub fn register(&mut self, module: &LoadedModule) {
        if !module.enabled {
            return;
        }

        // Parse extension configs from the module's manifest path.
        let manifest_path = module.path.join("manifest.toml");
        let content = match std::fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let raw: toml::Value = match toml::from_str(&content) {
            Ok(v) => v,
            Err(_) => return,
        };

        let base_priority = match module.module_type.as_str() {
            "system" => 0,
            "first-party" => 10,
            _ => 100,
        };

        // Waypointer search.
        if let Some(wp) = raw.get("waypointer").and_then(|w| w.get("search")) {
            let priority = wp
                .get("priority")
                .and_then(|v| v.as_integer())
                .unwrap_or(base_priority as i64) as u32;
            self.insert(RegisteredExtension {
                module_id: module.id.clone(),
                module_name: module.name.clone(),
                extension_point: ExtensionPoint::WaypointerSearch,
                priority,
                config: ExtensionConfig::WaypointerSearch {
                    prefix: wp.get("prefix").and_then(|v| v.as_str()).map(String::from),
                    detect_pattern: wp
                        .get("detect_pattern")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                },
            });
        }

        // Waypointer action.
        if let Some(wa) = raw.get("waypointer").and_then(|w| w.get("action")) {
            self.insert(RegisteredExtension {
                module_id: module.id.clone(),
                module_name: module.name.clone(),
                extension_point: ExtensionPoint::WaypointerAction,
                priority: base_priority,
                config: ExtensionConfig::WaypointerAction {
                    name: wa
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .into(),
                    icon: wa
                        .get("icon")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .into(),
                },
            });
        }

        // Topbar indicator.
        if let Some(ti) = raw.get("topbar").and_then(|t| t.get("indicator")) {
            self.insert(RegisteredExtension {
                module_id: module.id.clone(),
                module_name: module.name.clone(),
                extension_point: ExtensionPoint::TopbarIndicator,
                priority: base_priority,
                config: ExtensionConfig::TopbarIndicator {
                    slot: ti
                        .get("slot")
                        .and_then(|v| v.as_str())
                        .unwrap_or("temp")
                        .into(),
                    order: ti
                        .get("order")
                        .and_then(|v| v.as_integer())
                        .unwrap_or(50) as u32,
                    polling_interval: ti
                        .get("polling_interval")
                        .and_then(|v| v.as_integer())
                        .unwrap_or(30) as u32,
                },
            });
        }

        // Topbar applet.
        if let Some(ta) = raw.get("topbar").and_then(|t| t.get("applet")) {
            self.insert(RegisteredExtension {
                module_id: module.id.clone(),
                module_name: module.name.clone(),
                extension_point: ExtensionPoint::TopbarApplet,
                priority: base_priority,
                config: ExtensionConfig::TopbarApplet {
                    title: ta
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .into(),
                    icon: ta
                        .get("icon")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .into(),
                },
            });
        }

        // Settings panel.
        if let Some(sp) = raw.get("settings").and_then(|s| s.get("panel")) {
            self.insert(RegisteredExtension {
                module_id: module.id.clone(),
                module_name: module.name.clone(),
                extension_point: ExtensionPoint::SettingsPanel,
                priority: base_priority,
                config: ExtensionConfig::SettingsPanel {
                    title: sp
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .into(),
                    icon: sp
                        .get("icon")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .into(),
                    category: sp
                        .get("category")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .into(),
                },
            });
        }
    }

    /// Remove all extensions from a module.
    pub fn unregister(&mut self, module_id: &str) {
        for extensions in self.extensions.values_mut() {
            extensions.retain(|e| e.module_id != module_id);
        }
    }

    /// Get all extensions for a point, sorted by priority (lower = higher priority).
    pub fn get_extensions(&self, point: ExtensionPoint) -> Vec<&RegisteredExtension> {
        let mut exts: Vec<_> = self
            .extensions
            .get(&point)
            .map(|v| v.iter().collect())
            .unwrap_or_default();
        exts.sort_by_key(|e| e.priority);
        exts
    }

    /// Check for conflicts a module would introduce if registered.
    pub fn check_conflicts(&self, module: &LoadedModule) -> Vec<Conflict> {
        let mut conflicts = Vec::new();

        // Build a temporary registry entry to check against.
        let manifest_path = module.path.join("manifest.toml");
        let content = match std::fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(_) => return conflicts,
        };
        let raw: toml::Value = match toml::from_str(&content) {
            Ok(v) => v,
            Err(_) => return conflicts,
        };

        // Check Waypointer prefix conflicts.
        if let Some(new_prefix) = raw
            .get("waypointer")
            .and_then(|w| w.get("search"))
            .and_then(|s| s.get("prefix"))
            .and_then(|v| v.as_str())
        {
            for existing in self.get_extensions(ExtensionPoint::WaypointerSearch) {
                if let ExtensionConfig::WaypointerSearch {
                    prefix: Some(ref ep),
                    ..
                } = existing.config
                {
                    if ep == new_prefix {
                        conflicts.push(Conflict {
                            extension_point: ExtensionPoint::WaypointerSearch,
                            field: "prefix".into(),
                            value: new_prefix.into(),
                            module_a: existing.module_id.clone(),
                            module_b: module.id.clone(),
                        });
                    }
                }
            }
        }

        // Check topbar slot+order conflicts.
        if let Some(ti) = raw.get("topbar").and_then(|t| t.get("indicator")) {
            let slot = ti.get("slot").and_then(|v| v.as_str()).unwrap_or("temp");
            let order = ti.get("order").and_then(|v| v.as_integer()).unwrap_or(50) as u32;

            for existing in self.get_extensions(ExtensionPoint::TopbarIndicator) {
                if let ExtensionConfig::TopbarIndicator {
                    slot: ref es,
                    order: eo,
                    ..
                } = existing.config
                {
                    if es == slot && eo == order {
                        conflicts.push(Conflict {
                            extension_point: ExtensionPoint::TopbarIndicator,
                            field: "slot+order".into(),
                            value: format!("{slot}:{order}"),
                            module_a: existing.module_id.clone(),
                            module_b: module.id.clone(),
                        });
                    }
                }
            }
        }

        conflicts
    }

    /// Number of registered extensions across all points.
    pub fn total_count(&self) -> usize {
        self.extensions.values().map(|v| v.len()).sum()
    }

    /// Get topbar indicator extensions, sorted by order field.
    pub fn topbar_indicators(&self) -> Vec<&RegisteredExtension> {
        let mut exts = self.get_extensions(ExtensionPoint::TopbarIndicator);
        exts.sort_by_key(|e| {
            if let ExtensionConfig::TopbarIndicator { order, .. } = &e.config {
                *order
            } else {
                u32::MAX
            }
        });
        exts
    }

    fn insert(&mut self, ext: RegisteredExtension) {
        self.extensions
            .entry(ext.extension_point)
            .or_default()
            .push(ext);
    }
}

// ---------------------------------------------------------------------------
// Tauri state + commands
// ---------------------------------------------------------------------------

use std::sync::Mutex;

/// Shared extension registry state.
pub type ExtensionRegistryState = Mutex<ExtensionRegistry>;

/// Serializable indicator info for the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct TopbarIndicatorInfo {
    pub module_id: String,
    pub module_name: String,
    pub slot: String,
    pub order: u32,
    pub polling_interval: u32,
    pub priority: u32,
}

/// Get all registered topbar indicator extensions.
#[tauri::command]
pub fn get_topbar_indicators(
    state: tauri::State<'_, ExtensionRegistryState>,
) -> Vec<TopbarIndicatorInfo> {
    let registry = state.lock().unwrap();
    registry
        .topbar_indicators()
        .into_iter()
        .filter_map(|ext| {
            if let ExtensionConfig::TopbarIndicator {
                slot,
                order,
                polling_interval,
            } = &ext.config
            {
                Some(TopbarIndicatorInfo {
                    module_id: ext.module_id.clone(),
                    module_name: ext.module_name.clone(),
                    slot: slot.clone(),
                    order: *order,
                    polling_interval: *polling_interval,
                    priority: ext.priority,
                })
            } else {
                None
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn make_module(dir: &std::path::Path, id: &str, manifest: &str) -> LoadedModule {
        let module_dir = dir.join(id);
        std::fs::create_dir_all(&module_dir).unwrap();
        let manifest_path = module_dir.join("manifest.toml");
        let mut f = std::fs::File::create(&manifest_path).unwrap();
        f.write_all(manifest.as_bytes()).unwrap();

        LoadedModule {
            id: id.into(),
            name: id.into(),
            version: "1.0.0".into(),
            description: String::new(),
            module_type: "third-party".into(),
            path: module_dir,
            source: crate::modules::ModuleSource::User,
            enabled: true,
            has_waypointer: manifest.contains("waypointer"),
            has_topbar: manifest.contains("topbar"),
            has_settings: manifest.contains("settings"),
            icon: String::new(),
        }
    }

    #[test]
    fn test_register_waypointer_search() {
        let dir = tempfile::TempDir::new().unwrap();
        let module = make_module(
            dir.path(),
            "com.test.calc",
            r#"
[module]
id = "com.test.calc"
name = "Calc"
version = "1.0.0"

[waypointer.search]
priority = 50
prefix = "="
"#,
        );

        let mut reg = ExtensionRegistry::new();
        reg.register(&module);

        let exts = reg.get_extensions(ExtensionPoint::WaypointerSearch);
        assert_eq!(exts.len(), 1);
        assert_eq!(exts[0].module_id, "com.test.calc");
        assert_eq!(exts[0].priority, 50);
    }

    #[test]
    fn test_register_topbar_indicator() {
        let dir = tempfile::TempDir::new().unwrap();
        let module = make_module(
            dir.path(),
            "com.test.weather",
            r#"
[module]
id = "com.test.weather"
name = "Weather"
version = "1.0.0"

[topbar.indicator]
slot = "temp"
order = 30
polling_interval = 60
"#,
        );

        let mut reg = ExtensionRegistry::new();
        reg.register(&module);

        let exts = reg.get_extensions(ExtensionPoint::TopbarIndicator);
        assert_eq!(exts.len(), 1);
        if let ExtensionConfig::TopbarIndicator { slot, order, polling_interval } = &exts[0].config {
            assert_eq!(slot, "temp");
            assert_eq!(*order, 30);
            assert_eq!(*polling_interval, 60);
        } else {
            panic!("wrong config type");
        }
    }

    #[test]
    fn test_priority_sorting() {
        let dir = tempfile::TempDir::new().unwrap();
        let m1 = make_module(
            dir.path(),
            "com.low",
            r#"
[module]
id = "com.low"
name = "Low"
version = "1.0.0"

[waypointer.search]
priority = 200
prefix = "!"
"#,
        );
        let m2 = make_module(
            dir.path(),
            "com.high",
            r#"
[module]
id = "com.high"
name = "High"
version = "1.0.0"

[waypointer.search]
priority = 10
prefix = "@"
"#,
        );

        let mut reg = ExtensionRegistry::new();
        reg.register(&m1);
        reg.register(&m2);

        let exts = reg.get_extensions(ExtensionPoint::WaypointerSearch);
        assert_eq!(exts.len(), 2);
        assert_eq!(exts[0].module_id, "com.high"); // priority 10 first
        assert_eq!(exts[1].module_id, "com.low"); // priority 200 second
    }

    #[test]
    fn test_unregister() {
        let dir = tempfile::TempDir::new().unwrap();
        let module = make_module(
            dir.path(),
            "com.test.rem",
            r#"
[module]
id = "com.test.rem"
name = "Rem"
version = "1.0.0"

[waypointer.search]
prefix = "?"
"#,
        );

        let mut reg = ExtensionRegistry::new();
        reg.register(&module);
        assert_eq!(reg.total_count(), 1);

        reg.unregister("com.test.rem");
        assert_eq!(reg.total_count(), 0);
    }

    #[test]
    fn test_prefix_conflict() {
        let dir = tempfile::TempDir::new().unwrap();
        let m1 = make_module(
            dir.path(),
            "com.a",
            r#"
[module]
id = "com.a"
name = "A"
version = "1.0.0"

[waypointer.search]
prefix = "="
"#,
        );
        let m2 = make_module(
            dir.path(),
            "com.b",
            r#"
[module]
id = "com.b"
name = "B"
version = "1.0.0"

[waypointer.search]
prefix = "="
"#,
        );

        let mut reg = ExtensionRegistry::new();
        reg.register(&m1);

        let conflicts = reg.check_conflicts(&m2);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "prefix");
        assert_eq!(conflicts[0].value, "=");
    }

    #[test]
    fn test_slot_order_conflict() {
        let dir = tempfile::TempDir::new().unwrap();
        let m1 = make_module(
            dir.path(),
            "com.weather",
            r#"
[module]
id = "com.weather"
name = "Weather"
version = "1.0.0"

[topbar.indicator]
slot = "temp"
order = 50
"#,
        );
        let m2 = make_module(
            dir.path(),
            "com.stocks",
            r#"
[module]
id = "com.stocks"
name = "Stocks"
version = "1.0.0"

[topbar.indicator]
slot = "temp"
order = 50
"#,
        );

        let mut reg = ExtensionRegistry::new();
        reg.register(&m1);

        let conflicts = reg.check_conflicts(&m2);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "slot+order");
    }

    #[test]
    fn test_no_conflict_different_slot() {
        let dir = tempfile::TempDir::new().unwrap();
        let m1 = make_module(
            dir.path(),
            "com.w",
            r#"
[module]
id = "com.w"
name = "W"
version = "1.0.0"

[topbar.indicator]
slot = "temp"
order = 50
"#,
        );
        let m2 = make_module(
            dir.path(),
            "com.s",
            r#"
[module]
id = "com.s"
name = "S"
version = "1.0.0"

[topbar.indicator]
slot = "project"
order = 50
"#,
        );

        let mut reg = ExtensionRegistry::new();
        reg.register(&m1);

        let conflicts = reg.check_conflicts(&m2);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn test_disabled_module_not_registered() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut module = make_module(
            dir.path(),
            "com.disabled",
            r#"
[module]
id = "com.disabled"
name = "Disabled"
version = "1.0.0"

[waypointer.search]
prefix = "!"
"#,
        );
        module.enabled = false;

        let mut reg = ExtensionRegistry::new();
        reg.register(&module);
        assert_eq!(reg.total_count(), 0);
    }

    #[test]
    fn test_multiple_extension_points() {
        let dir = tempfile::TempDir::new().unwrap();
        let module = make_module(
            dir.path(),
            "com.full",
            r#"
[module]
id = "com.full"
name = "Full"
version = "1.0.0"

[waypointer.search]
prefix = "="

[topbar.indicator]
slot = "temp"

[settings.panel]
title = "Full Settings"
"#,
        );

        let mut reg = ExtensionRegistry::new();
        reg.register(&module);
        assert_eq!(reg.total_count(), 3);
        assert_eq!(reg.get_extensions(ExtensionPoint::WaypointerSearch).len(), 1);
        assert_eq!(reg.get_extensions(ExtensionPoint::TopbarIndicator).len(), 1);
        assert_eq!(reg.get_extensions(ExtensionPoint::SettingsPanel).len(), 1);
    }
}
