//! Input (keybindings, mouse, touchpad) commands.
//!
//! All three surfaces read and write `~/.config/lunaris/compositor.toml`.
//! The compositor hot-reloads the file via notify, so writes take effect
//! without a restart.
//!
//! The keybinding commands layer three concepts on top of the raw TOML:
//!
//! 1. A curated catalogue of known actions with human-readable labels
//!    and category membership (`CATALOGUE`). This is what the Keyboard
//!    panel lists, even for actions the user has not bound.
//! 2. Default bindings (mirrored from the compositor source). Used for
//!    the "reset to default" path and to distinguish user customisations
//!    from built-in values.
//! 3. Conflict detection: any two entries with the same `Super+...` key
//!    combination.
//!
//! Custom entries (user-added `spawn:` / `shell:` actions) are stored in
//! the same `[keybindings]` table; they show up under a synthetic
//! "custom" category because they are not in the catalogue.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

fn compositor_toml_path() -> PathBuf {
    // `LUNARIS_CONFIG_DIR` overrides the lookup for tests and for
    // packagers that need a non-standard location. Matches the
    // pattern used by installd's `LUNARIS_USER_*_DIR` overrides.
    let dir = std::env::var("LUNARIS_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join("lunaris")
        });
    let _ = std::fs::create_dir_all(&dir);
    dir.join("compositor.toml")
}

fn read_doc() -> Result<toml::Value, String> {
    let path = compositor_toml_path();
    if !path.exists() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    toml::from_str(&content).map_err(|e| format!("parse {}: {e}", path.display()))
}

fn write_doc(doc: &toml::Value) -> Result<(), String> {
    let path = compositor_toml_path();
    let content = toml::to_string_pretty(doc).map_err(|e| format!("serialize: {e}"))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("write tmp: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}

fn keybindings_table_mut(doc: &mut toml::Value) -> &mut toml::map::Map<String, toml::Value> {
    if !doc.is_table() {
        *doc = toml::Value::Table(toml::map::Map::new());
    }
    let root = doc.as_table_mut().unwrap();
    let entry = root
        .entry("keybindings".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    if !entry.is_table() {
        *entry = toml::Value::Table(toml::map::Map::new());
    }
    entry.as_table_mut().unwrap()
}

// -----------------------------------------------------------------------
// Catalogue of known actions
// -----------------------------------------------------------------------

/// A single row for the Settings keyboard list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindingEntry {
    /// Stable identifier. For catalogue actions this is the action string
    /// itself (e.g. `"focus_left"`); for custom entries it is the
    /// accelerator string, since the action is opaque to us.
    pub id: String,
    /// The raw action string that the compositor parses
    /// (e.g. `"focus_left"`, `"spawn:foot"`, `"workspace_switch:3"`).
    pub action: String,
    /// Current binding (accelerator), or `None` if the action is listed
    /// in the catalogue but not bound.
    pub binding: Option<String>,
    /// Default binding for this action, if any.
    pub default_binding: Option<String>,
    /// True if the user has overridden or added this entry.
    pub is_custom: bool,
    /// Grouping key used by the UI to render collapsible sections.
    pub category: String,
    /// Human-readable label (English for now).
    pub label: String,
    /// Optional one-line explanation.
    pub description: Option<String>,
    /// Owning module id for entries coming from
    /// `compositor.d/keybindings.d/*.toml` fragments. `None` for
    /// catalogue, custom, and user entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub binding: String,
    pub actions: Vec<String>,
}

struct CatalogueRow {
    action: &'static str,
    category: &'static str,
    label: &'static str,
    description: Option<&'static str>,
    default_binding: Option<&'static str>,
}

/// Curated list of actions the Settings UI shows, whether or not they
/// are currently bound. The catalogue is the single source of truth for
/// label + category + default. Keep in sync with the compositor's
/// `default_keybindings()` and `action_from_str()`.
const CATALOGUE: &[CatalogueRow] = &[
    // Window
    CatalogueRow {
        action: "close_window",
        category: "window",
        label: "Close window",
        description: None,
        default_binding: Some("Super+Q"),
    },
    CatalogueRow {
        action: "fullscreen",
        category: "window",
        label: "Toggle fullscreen",
        description: None,
        default_binding: Some("Super+F"),
    },
    CatalogueRow {
        action: "maximize",
        category: "window",
        label: "Toggle maximize",
        description: None,
        default_binding: None,
    },
    CatalogueRow {
        action: "minimize",
        category: "window",
        label: "Minimize window",
        description: None,
        default_binding: None,
    },
    // Focus
    CatalogueRow {
        action: "focus_left",
        category: "focus",
        label: "Focus window to the left",
        description: None,
        default_binding: Some("Super+H"),
    },
    CatalogueRow {
        action: "focus_right",
        category: "focus",
        label: "Focus window to the right",
        description: None,
        default_binding: Some("Super+L"),
    },
    CatalogueRow {
        action: "focus_up",
        category: "focus",
        label: "Focus window above",
        description: None,
        default_binding: Some("Super+K"),
    },
    CatalogueRow {
        action: "focus_down",
        category: "focus",
        label: "Focus window below",
        description: None,
        default_binding: Some("Super+J"),
    },
    // Move
    CatalogueRow {
        action: "move_left",
        category: "move",
        label: "Move window left",
        description: None,
        default_binding: Some("Super+Shift+H"),
    },
    CatalogueRow {
        action: "move_right",
        category: "move",
        label: "Move window right",
        description: None,
        default_binding: Some("Super+Shift+L"),
    },
    CatalogueRow {
        action: "move_up",
        category: "move",
        label: "Move window up",
        description: None,
        default_binding: Some("Super+Shift+K"),
    },
    CatalogueRow {
        action: "move_down",
        category: "move",
        label: "Move window down",
        description: None,
        default_binding: Some("Super+Shift+J"),
    },
    // Tiling
    CatalogueRow {
        action: "toggle_tiling",
        category: "tiling",
        label: "Toggle tiling mode",
        description: None,
        default_binding: Some("Super+T"),
    },
    CatalogueRow {
        action: "toggle_window_floating",
        category: "tiling",
        label: "Toggle window floating",
        description: None,
        default_binding: Some("Super+Shift+Space"),
    },
    CatalogueRow {
        action: "toggle_monocle",
        category: "tiling",
        label: "Toggle monocle mode",
        description: None,
        default_binding: Some("Super+M"),
    },
    CatalogueRow {
        action: "scratchpad_toggle",
        category: "tiling",
        label: "Toggle scratchpad",
        description: None,
        default_binding: Some("Super+Minus"),
    },
    CatalogueRow {
        action: "scratchpad_move",
        category: "tiling",
        label: "Move window to scratchpad",
        description: None,
        default_binding: Some("Super+Shift+Minus"),
    },
    CatalogueRow {
        action: "swap_window",
        category: "tiling",
        label: "Swap windows",
        description: None,
        default_binding: None,
    },
    // Workspace switch
    CatalogueRow {
        action: "workspace_switch:1",
        category: "workspace",
        label: "Switch to workspace 1",
        description: None,
        default_binding: Some("Super+1"),
    },
    CatalogueRow {
        action: "workspace_switch:2",
        category: "workspace",
        label: "Switch to workspace 2",
        description: None,
        default_binding: Some("Super+2"),
    },
    CatalogueRow {
        action: "workspace_switch:3",
        category: "workspace",
        label: "Switch to workspace 3",
        description: None,
        default_binding: Some("Super+3"),
    },
    CatalogueRow {
        action: "workspace_switch:4",
        category: "workspace",
        label: "Switch to workspace 4",
        description: None,
        default_binding: Some("Super+4"),
    },
    CatalogueRow {
        action: "workspace_switch:5",
        category: "workspace",
        label: "Switch to workspace 5",
        description: None,
        default_binding: Some("Super+5"),
    },
    CatalogueRow {
        action: "workspace_switch:6",
        category: "workspace",
        label: "Switch to workspace 6",
        description: None,
        default_binding: Some("Super+6"),
    },
    CatalogueRow {
        action: "workspace_switch:7",
        category: "workspace",
        label: "Switch to workspace 7",
        description: None,
        default_binding: Some("Super+7"),
    },
    CatalogueRow {
        action: "workspace_switch:8",
        category: "workspace",
        label: "Switch to workspace 8",
        description: None,
        default_binding: Some("Super+8"),
    },
    CatalogueRow {
        action: "workspace_switch:9",
        category: "workspace",
        label: "Switch to workspace 9",
        description: None,
        default_binding: Some("Super+9"),
    },
    CatalogueRow {
        action: "workspace_next",
        category: "workspace",
        label: "Next workspace",
        description: None,
        default_binding: None,
    },
    CatalogueRow {
        action: "workspace_prev",
        category: "workspace",
        label: "Previous workspace",
        description: None,
        default_binding: None,
    },
    // Workspace move
    CatalogueRow {
        action: "workspace_move:1",
        category: "workspace_move",
        label: "Move window to workspace 1",
        description: None,
        default_binding: Some("Super+Shift+1"),
    },
    CatalogueRow {
        action: "workspace_move:2",
        category: "workspace_move",
        label: "Move window to workspace 2",
        description: None,
        default_binding: Some("Super+Shift+2"),
    },
    CatalogueRow {
        action: "workspace_move:3",
        category: "workspace_move",
        label: "Move window to workspace 3",
        description: None,
        default_binding: Some("Super+Shift+3"),
    },
    CatalogueRow {
        action: "workspace_move:4",
        category: "workspace_move",
        label: "Move window to workspace 4",
        description: None,
        default_binding: Some("Super+Shift+4"),
    },
    CatalogueRow {
        action: "workspace_move:5",
        category: "workspace_move",
        label: "Move window to workspace 5",
        description: None,
        default_binding: Some("Super+Shift+5"),
    },
    CatalogueRow {
        action: "workspace_move:6",
        category: "workspace_move",
        label: "Move window to workspace 6",
        description: None,
        default_binding: Some("Super+Shift+6"),
    },
    CatalogueRow {
        action: "workspace_move:7",
        category: "workspace_move",
        label: "Move window to workspace 7",
        description: None,
        default_binding: Some("Super+Shift+7"),
    },
    CatalogueRow {
        action: "workspace_move:8",
        category: "workspace_move",
        label: "Move window to workspace 8",
        description: None,
        default_binding: Some("Super+Shift+8"),
    },
    CatalogueRow {
        action: "workspace_move:9",
        category: "workspace_move",
        label: "Move window to workspace 9",
        description: None,
        default_binding: Some("Super+Shift+9"),
    },
    // Shell + apps (baseline)
    CatalogueRow {
        action: "shell:waypointer_open",
        category: "shell",
        label: "Open Waypointer",
        description: Some("Launcher and inline command palette"),
        default_binding: Some("Super+Space"),
    },
    CatalogueRow {
        action: "shell:workspace_map_open",
        category: "workspace_map",
        label: "Open Workspace Map",
        description: Some(
            "Horizontal overview of all workspaces with window cards; \
             cycles focus on repeat. Inside the Map, see the \
             \"Workspace Map\" category for navigation keys.",
        ),
        default_binding: Some("Super+Tab"),
    },
    CatalogueRow {
        action: "spawn:foot",
        category: "apps",
        label: "Open terminal (foot)",
        description: None,
        default_binding: Some("Super+Return"),
    },
    // Keyboard layout switching. No default — the common accelerators
    // (Super+Space / Alt+Shift) are already taken by Waypointer and
    // the XKB `grp:*` options respectively; picking one of those as a
    // default would surprise existing users. The row stays visible so
    // the Shortcuts UI can list the actions for manual binding.
    CatalogueRow {
        action: "keyboard_layout_next",
        category: "keyboard",
        label: "Next keyboard layout",
        description: Some("Cycle to the next configured XKB layout"),
        default_binding: None,
    },
    CatalogueRow {
        action: "keyboard_layout_prev",
        category: "keyboard",
        label: "Previous keyboard layout",
        description: Some("Cycle to the previous configured XKB layout"),
        default_binding: None,
    },
];

fn classify_custom(action: &str) -> &'static str {
    if action.starts_with("spawn:") {
        "apps"
    } else if action.starts_with("shell:") {
        "shell"
    } else {
        "custom"
    }
}

fn label_for_custom(action: &str) -> String {
    if let Some(cmd) = action.strip_prefix("spawn:") {
        format!("Launch {cmd}")
    } else if let Some(event) = action.strip_prefix("shell:") {
        format!("Shell: {event}")
    } else {
        action.to_string()
    }
}

/// Read `[keybindings]` as a map `accelerator -> action_string`.
fn read_bindings() -> Result<BTreeMap<String, String>, String> {
    let doc = read_doc()?;
    let Some(table) = doc.get("keybindings").and_then(|v| v.as_table()) else {
        return Ok(BTreeMap::new());
    };
    let mut out = BTreeMap::new();
    for (key, val) in table {
        if let Some(s) = val.as_str() {
            out.insert(key.clone(), s.to_string());
        }
    }
    Ok(out)
}

// -----------------------------------------------------------------------
// Public commands
// -----------------------------------------------------------------------

/// Return one entry per catalogue action, plus one entry per custom
/// binding found in the TOML that is not in the catalogue.
/// Action aliases: old name → new canonical name. When looking up a
/// user's existing TOML binding for the new name, we fall back to the
/// old name so renamed-actions keep their binding without forcing the
/// user to rebind. Also used by the custom-orphan detection loop so
/// an old-name binding doesn't show up twice (once as catalogue entry,
/// once as custom orphan).
const ACTION_ALIASES: &[(&str, &str)] = &[
    // Workspace Overlay → Workspace Map rename (2026-04).
    ("shell:workspace_overlay_open", "shell:workspace_map_open"),
    ("shell:workspace_overlay_toggle", "shell:workspace_map_open"),
];

fn alias_for(action: &str) -> Option<&'static str> {
    ACTION_ALIASES.iter().find_map(
        |(old, new)| if *new == action { Some(*old) } else { None },
    )
}

fn is_aliased_to_catalogue(action: &str) -> bool {
    ACTION_ALIASES.iter().any(|(old, _)| *old == action)
}

#[tauri::command]
pub fn keybindings_get_all() -> Result<Vec<KeybindingEntry>, String> {
    let user = read_bindings()?;
    // Reverse lookup: action string -> accelerator(s) currently bound.
    let mut action_to_accel: BTreeMap<String, String> = BTreeMap::new();
    for (accel, action) in &user {
        action_to_accel.insert(action.clone(), accel.clone());
    }

    let mut out = Vec::new();
    // Catalogue rows.
    for row in CATALOGUE {
        // Three-step binding resolution:
        // 1. User's explicit override for this action
        // 2. User's explicit override for a deprecated alias
        //    (migration path for renamed actions)
        // 3. The built-in default from the catalogue
        // Previously this only used steps 1+2 (defaults only when the
        // whole [keybindings] section was absent), so new catalogue
        // entries landed on screen as "Not set" for users who had any
        // prior customisation.
        let binding = action_to_accel
            .get(row.action)
            .cloned()
            .or_else(|| {
                alias_for(row.action)
                    .and_then(|old| action_to_accel.get(old).cloned())
            })
            .or_else(|| row.default_binding.map(|s| s.to_string()));
        let is_custom = match (&binding, &row.default_binding) {
            (Some(b), Some(d)) => b != d,
            (Some(_), None) => true,
            (None, _) => false,
        };
        out.push(KeybindingEntry {
            id: row.action.to_string(),
            action: row.action.to_string(),
            binding,
            default_binding: row.default_binding.map(|s| s.to_string()),
            is_custom,
            category: row.category.to_string(),
            label: row.label.to_string(),
            description: row.description.map(|s| s.to_string()),
            module_id: None,
        });
    }

    // Custom entries: anything in TOML whose action is not in the
    // catalogue. Keyed by accelerator because two custom entries could
    // share the same action string (unlikely, but technically legal).
    // Deprecated-alias actions are treated as "known" here so a user's
    // pre-migration TOML entry (e.g. `shell:workspace_overlay_open`)
    // doesn't render alongside the renamed catalogue entry.
    let known: std::collections::BTreeSet<&str> =
        CATALOGUE.iter().map(|r| r.action).collect();
    for (accel, action) in &user {
        if known.contains(action.as_str()) {
            continue;
        }
        if is_aliased_to_catalogue(action) {
            continue;
        }
        out.push(KeybindingEntry {
            id: accel.clone(),
            action: action.clone(),
            binding: Some(accel.clone()),
            default_binding: None,
            is_custom: true,
            category: classify_custom(action).to_string(),
            label: label_for_custom(action),
            description: None,
            module_id: None,
        });
    }

    // Module-shipped entries from compositor.d/keybindings.d/*.toml.
    // These are surfaced read-only: the user can see them and rebind,
    // but deleting a module fragment here has no effect — the fragment
    // is regenerated by installd. The UI treats them as a separate
    // "Modules" section grouped by `module_id`.
    for entry in scan_fragments() {
        out.push(entry);
    }

    Ok(out)
}

/// Scan `~/.config/lunaris/compositor.d/keybindings.d/*.toml` and
/// project each `"accelerator" = "action"` pair into a Settings
/// keybinding row. Filename without the extension is used as the
/// `module_id`, matching the convention `installd` writes to.
fn scan_fragments() -> Vec<KeybindingEntry> {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lunaris")
        .join("compositor.d")
        .join("keybindings.d");
    let Ok(read_dir) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("toml") {
            continue;
        }
        let Some(module_id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let module_id = module_id.to_string();
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(table) = toml::from_str::<toml::Value>(&content) else {
            continue;
        };
        let Some(kb_table) = table.get("keybindings").and_then(|v| v.as_table()) else {
            continue;
        };
        for (binding, action) in kb_table {
            let Some(action_str) = action.as_str() else { continue };
            out.push(KeybindingEntry {
                id: format!("module:{module_id}:{binding}"),
                action: action_str.to_string(),
                binding: Some(binding.clone()),
                default_binding: Some(binding.clone()),
                is_custom: false,
                category: "module".to_string(),
                label: label_for_module_action(action_str, &module_id),
                description: None,
                module_id: Some(module_id.clone()),
            });
        }
    }
    out
}

fn label_for_module_action(action: &str, module_id: &str) -> String {
    // `module:<id>:<action>` -> pretty "<id>: <action>".
    if let Some(rest) = action.strip_prefix("module:") {
        if let Some(idx) = rest.find(':') {
            return format!("{}: {}", &rest[..idx], &rest[idx + 1..]);
        }
    }
    format!("{module_id}: {action}")
}

/// Change the accelerator for a known action (from the catalogue).
///
/// Passing `binding = None` removes the action entirely, leaving it
/// listed in the catalogue but unbound.
#[tauri::command]
pub fn keybindings_set(action: String, binding: Option<String>) -> Result<(), String> {
    let mut doc = read_doc()?;
    // First: remove any existing accelerator that maps to this action
    // OR to a deprecated alias of it. The alias sweep keeps the TOML
    // clean when a renamed action is rebound — otherwise a pre-
    // migration entry for the old name would linger beside the new
    // one and both would fire.
    {
        let table = keybindings_table_mut(&mut doc);
        let aliases: Vec<&str> = ACTION_ALIASES
            .iter()
            .filter_map(|(old, new)| if *new == action { Some(*old) } else { None })
            .collect();
        let to_remove: Vec<String> = table
            .iter()
            .filter_map(|(k, v)| {
                v.as_str()
                    .filter(|s| *s == action || aliases.contains(s))
                    .map(|_| k.clone())
            })
            .collect();
        for k in to_remove {
            table.remove(&k);
        }
    }
    // Then: insert the new binding, if any.
    if let Some(b) = binding {
        let table = keybindings_table_mut(&mut doc);
        table.insert(b, toml::Value::String(action));
    }
    write_doc(&doc)
}

/// Insert or overwrite a custom binding. `action` is the full action
/// string (`"spawn:foo"`, `"shell:waypointer_open"`, etc.). The UI is
/// responsible for building this string from its tab selection.
#[tauri::command]
pub fn keybindings_add_custom(binding: String, action: String) -> Result<(), String> {
    if action.is_empty() {
        return Err("action is empty".into());
    }
    let mut doc = read_doc()?;
    let table = keybindings_table_mut(&mut doc);
    table.insert(binding, toml::Value::String(action));
    write_doc(&doc)
}

/// Remove a binding by its accelerator (for custom entries) or by its
/// action string (for catalogue entries).
#[tauri::command]
pub fn keybindings_remove(key_or_action: String) -> Result<(), String> {
    let mut doc = read_doc()?;
    let table = keybindings_table_mut(&mut doc);
    // Try accelerator first.
    if table.remove(&key_or_action).is_some() {
        return write_doc(&doc);
    }
    // Fall back: remove any entry whose value equals the action string.
    let to_remove: Vec<String> = table
        .iter()
        .filter_map(|(k, v)| {
            v.as_str()
                .filter(|s| *s == key_or_action)
                .map(|_| k.clone())
        })
        .collect();
    for k in to_remove {
        table.remove(&k);
    }
    write_doc(&doc)
}

/// Detect duplicated accelerators. In the TOML layout each accelerator
/// is a table key, so true duplicates cannot exist at the file level —
/// but the UI can stage an in-memory change that would collide, and this
/// is the function that surfaces it.
///
/// The current on-disk state may still contain *one* conflict if the
/// user hand-edited the file and accidentally collided with a default
/// mapping that is not present explicitly. In that case we compare
/// against the union of TOML + catalogue defaults.
#[tauri::command]
pub fn keybindings_get_conflicts() -> Result<Vec<Conflict>, String> {
    let user = read_bindings()?;
    // Build the effective map. If the user has explicit entries, the
    // TOML wins (mirrors the compositor). Otherwise catalogue defaults
    // apply.
    let effective: BTreeMap<String, Vec<String>> = if user.is_empty() {
        let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for row in CATALOGUE {
            if let Some(b) = row.default_binding {
                m.entry(b.to_string()).or_default().push(row.action.to_string());
            }
        }
        m
    } else {
        let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (accel, action) in user {
            m.entry(accel).or_default().push(action);
        }
        m
    };

    Ok(effective
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(binding, actions)| Conflict { binding, actions })
        .collect())
}

/// Expose the catalogue + defaults so the UI can render a reset button.
#[tauri::command]
pub fn keybindings_get_defaults() -> Result<Vec<KeybindingEntry>, String> {
    Ok(CATALOGUE
        .iter()
        .map(|row| KeybindingEntry {
            id: row.action.to_string(),
            action: row.action.to_string(),
            binding: row.default_binding.map(|s| s.to_string()),
            default_binding: row.default_binding.map(|s| s.to_string()),
            is_custom: false,
            category: row.category.to_string(),
            label: row.label.to_string(),
            description: row.description.map(|s| s.to_string()),
            module_id: None,
        })
        .collect())
}

/// Replace the entire `[keybindings]` section with the defaults.
#[tauri::command]
pub fn keybindings_reset_all() -> Result<(), String> {
    let mut doc = read_doc()?;
    let table = keybindings_table_mut(&mut doc);
    table.clear();
    for row in CATALOGUE {
        if let Some(b) = row.default_binding {
            table.insert(b.to_string(), toml::Value::String(row.action.to_string()));
        }
    }
    write_doc(&doc)
}

/// Drop the whole `[keybindings]` section from `compositor.toml` so
/// the compositor falls back to its built-in defaults. Distinct from
/// `keybindings_reset_all`, which writes the defaults explicitly:
/// removing the section means future default changes propagate
/// automatically.
#[tauri::command]
pub fn keybindings_reset_all_to_defaults() -> Result<(), String> {
    let mut doc = read_doc()?;
    if let Some(table) = doc.as_table_mut() {
        table.remove("keybindings");
    }
    write_doc(&doc)
}

/// Remove every module-shipped keybinding fragment in
/// `~/.config/lunaris/compositor.d/keybindings.d/`. Useful as a
/// panic button when a module has registered a problematic shortcut
/// and the user cannot uninstall the module right away.
#[tauri::command]
pub fn keybindings_reset_module_fragments() -> Result<u32, String> {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lunaris")
        .join("compositor.d")
        .join("keybindings.d");
    if !dir.exists() {
        return Ok(0);
    }
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("read_dir: {e}"))?;
    let mut removed = 0u32;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("toml") {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

// -----------------------------------------------------------------------
// D-Bus live conflict queries
// -----------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveConflict {
    pub binding: String,
    pub existing_action: String,
    pub existing_scope: String,
    pub existing_owner: String,
}

/// Ask `org.lunaris.InputManager1` for every dynamic registration
/// that would collide with the given accelerator. Used by the
/// Key-Capture modal so the user sees the full conflict picture
/// (static catalogue + running apps) before committing.
///
/// Fails softly: a broken D-Bus connection returns an empty list
/// with a logged warning, so the UI can keep working even when the
/// compositor is not running.
#[tauri::command]
pub async fn keybindings_query_live_conflicts(
    binding: String,
) -> Result<Vec<LiveConflict>, String> {
    use lunaris_input_client::{zbus, InputManagerClient};

    let conn = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(err) => {
            log::warn!("live conflicts: no session bus: {err}");
            return Ok(Vec::new());
        }
    };
    let client = match InputManagerClient::new(&conn).await {
        Ok(c) => c,
        Err(err) => {
            log::warn!("live conflicts: no InputManager proxy: {err}");
            return Ok(Vec::new());
        }
    };
    let conflicts = client.query_conflicts(&binding).await.map_err(|e| {
        log::warn!("live conflicts: QueryConflicts failed: {e}");
        e.to_string()
    })?;
    Ok(conflicts
        .into_iter()
        .map(|c| LiveConflict {
            binding: c.binding,
            existing_action: c.existing_action,
            existing_scope: c.existing_scope,
            existing_owner: c.existing_owner,
        })
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictGroup {
    pub binding: String,
    pub entries: Vec<KeybindingEntry>,
}

/// Static conflict scan over the effective keybinding set Settings
/// can see (catalogue + user + modules + customs). Returns one
/// [`ConflictGroup`] per accelerator that has more than one entry.
#[tauri::command]
pub fn keybindings_get_all_conflicts() -> Result<Vec<ConflictGroup>, String> {
    let all = keybindings_get_all()?;
    let mut by_binding: std::collections::BTreeMap<String, Vec<KeybindingEntry>> =
        std::collections::BTreeMap::new();
    for entry in all {
        if let Some(ref binding) = entry.binding {
            by_binding.entry(binding.clone()).or_default().push(entry);
        }
    }
    Ok(by_binding
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(binding, entries)| ConflictGroup { binding, entries })
        .collect())
}

// -----------------------------------------------------------------------
// Keyboard layouts
// -----------------------------------------------------------------------

fn xkb_section(doc: &toml::Value) -> Option<&toml::map::Map<String, toml::Value>> {
    doc.get("xkb_config").and_then(|v| v.as_table())
}

/// Read layouts from the TOML. `layouts = [...]` wins; falls back to
/// the legacy single-string `layout = "de"` form; empties to `["us"]`.
/// Matches the compositor's own parser exactly so Settings never shows
/// a different set than what the compositor loaded.
#[tauri::command]
pub fn keyboard_get_layouts() -> Result<Vec<String>, String> {
    let doc = read_doc()?;
    let Some(xkb) = xkb_section(&doc) else {
        return Ok(vec!["us".into()]);
    };
    if let Some(list) = xkb.get("layouts").and_then(|v| v.as_array()) {
        let out: Vec<String> = list
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        if !out.is_empty() {
            return Ok(out);
        }
    }
    if let Some(s) = xkb.get("layout").and_then(|v| v.as_str()) {
        // Support both the array form and a comma-separated single
        // string, since the compositor accepts both on the way in.
        let out: Vec<String> = s
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        if !out.is_empty() {
            return Ok(out);
        }
    }
    Ok(vec!["us".into()])
}

/// Write the layout list and remove the legacy single-layout key so
/// the two forms cannot drift out of sync. Empty input is rejected —
/// the compositor falls back to `us` but writing that intent is
/// explicit here.
#[tauri::command]
pub fn keyboard_set_layouts(layouts: Vec<String>) -> Result<(), String> {
    let clean: Vec<String> = layouts
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if clean.is_empty() {
        return Err("at least one layout required".into());
    }
    let mut doc = read_doc()?;
    if !doc.is_table() {
        doc = toml::Value::Table(toml::map::Map::new());
    }
    let root = doc.as_table_mut().unwrap();
    let xkb_entry = root
        .entry("xkb_config".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    if !xkb_entry.is_table() {
        *xkb_entry = toml::Value::Table(toml::map::Map::new());
    }
    let xkb = xkb_entry.as_table_mut().unwrap();
    xkb.insert(
        "layouts".to_string(),
        toml::Value::Array(clean.into_iter().map(toml::Value::String).collect()),
    );
    // Clear the scalar form so future edits through this command
    // cannot produce an ambiguous file.
    xkb.remove("layout");
    write_doc(&doc)
}

#[tauri::command]
pub fn keyboard_get_variants() -> Result<Vec<String>, String> {
    let doc = read_doc()?;
    let Some(xkb) = xkb_section(&doc) else {
        return Ok(Vec::new());
    };
    if let Some(list) = xkb.get("variants").and_then(|v| v.as_array()) {
        return Ok(list
            .iter()
            .map(|v| v.as_str().unwrap_or("").to_string())
            .collect());
    }
    if let Some(s) = xkb.get("variant").and_then(|v| v.as_str()) {
        if s.is_empty() {
            return Ok(Vec::new());
        }
        return Ok(s.split(',').map(|t| t.trim().to_string()).collect());
    }
    Ok(Vec::new())
}

#[tauri::command]
pub fn keyboard_set_variants(variants: Vec<String>) -> Result<(), String> {
    let mut doc = read_doc()?;
    if !doc.is_table() {
        doc = toml::Value::Table(toml::map::Map::new());
    }
    let root = doc.as_table_mut().unwrap();
    let xkb_entry = root
        .entry("xkb_config".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    if !xkb_entry.is_table() {
        *xkb_entry = toml::Value::Table(toml::map::Map::new());
    }
    let xkb = xkb_entry.as_table_mut().unwrap();
    // Empty vec → remove key entirely; saves a dead `variants = []`
    // line in the TOML.
    if variants.is_empty() {
        xkb.remove("variants");
        xkb.remove("variant");
    } else {
        xkb.insert(
            "variants".to_string(),
            toml::Value::Array(variants.into_iter().map(toml::Value::String).collect()),
        );
        xkb.remove("variant");
    }
    write_doc(&doc)
}

// -----------------------------------------------------------------------
// Mouse + touchpad
// -----------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseConfig {
    pub acceleration: f64,
    pub natural_scroll: bool,
    pub left_handed: bool,
    /// Linear multiplier on wheel scroll deltas. 1.0 = libinput
    /// default; clamped to 0.1..3.0 on the compositor side.
    pub scroll_speed: f64,
}

impl Default for MouseConfig {
    fn default() -> Self {
        Self {
            acceleration: 0.0,
            natural_scroll: false,
            left_handed: false,
            scroll_speed: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TouchpadConfig {
    pub tap_to_click: bool,
    pub natural_scroll: bool,
    pub two_finger_scroll: bool,
    pub disable_while_typing: bool,
    pub acceleration: f64,
    /// `"clickfinger"` (default) or `"areas"`. The compositor rejects
    /// unknown strings with a warning — the UI picker only offers
    /// the two documented values so this is a belt-and-braces check.
    pub click_method: String,
    /// Tap-and-hold to drag a window/selection. Requires
    /// `tap_to_click`.
    pub tap_drag: bool,
}

impl Default for TouchpadConfig {
    fn default() -> Self {
        Self {
            tap_to_click: true,
            natural_scroll: true,
            two_finger_scroll: true,
            disable_while_typing: true,
            acceleration: 0.0,
            click_method: "clickfinger".into(),
            tap_drag: true,
        }
    }
}

#[tauri::command]
pub fn mouse_get_config() -> Result<MouseConfig, String> {
    let doc = read_doc()?;
    let Some(section) = doc.get("mouse").and_then(|v| v.as_table()) else {
        return Ok(MouseConfig::default());
    };
    let default = MouseConfig::default();
    Ok(MouseConfig {
        acceleration: section
            .get("acceleration")
            .and_then(|v| v.as_float())
            .unwrap_or(default.acceleration),
        natural_scroll: section
            .get("natural_scroll")
            .and_then(|v| v.as_bool())
            .unwrap_or(default.natural_scroll),
        left_handed: section
            .get("left_handed")
            .and_then(|v| v.as_bool())
            .unwrap_or(default.left_handed),
        scroll_speed: section
            .get("scroll_speed")
            .and_then(|v| v.as_float())
            .unwrap_or(default.scroll_speed),
    })
}

#[tauri::command]
pub fn mouse_set_config(config: MouseConfig) -> Result<(), String> {
    let mut doc = read_doc()?;
    if !doc.is_table() {
        doc = toml::Value::Table(toml::map::Map::new());
    }
    let mut section = toml::map::Map::new();
    section.insert(
        "acceleration".to_string(),
        toml::Value::Float(config.acceleration.clamp(-1.0, 1.0)),
    );
    section.insert(
        "natural_scroll".to_string(),
        toml::Value::Boolean(config.natural_scroll),
    );
    section.insert(
        "left_handed".to_string(),
        toml::Value::Boolean(config.left_handed),
    );
    section.insert(
        "scroll_speed".to_string(),
        toml::Value::Float(config.scroll_speed.clamp(0.1, 3.0)),
    );
    doc.as_table_mut()
        .unwrap()
        .insert("mouse".to_string(), toml::Value::Table(section));
    write_doc(&doc)
}

#[tauri::command]
pub fn touchpad_get_config() -> Result<TouchpadConfig, String> {
    let doc = read_doc()?;
    let Some(section) = doc.get("touchpad").and_then(|v| v.as_table()) else {
        return Ok(TouchpadConfig::default());
    };
    let default = TouchpadConfig::default();
    Ok(TouchpadConfig {
        tap_to_click: section
            .get("tap_to_click")
            .and_then(|v| v.as_bool())
            .unwrap_or(default.tap_to_click),
        natural_scroll: section
            .get("natural_scroll")
            .and_then(|v| v.as_bool())
            .unwrap_or(default.natural_scroll),
        two_finger_scroll: section
            .get("two_finger_scroll")
            .and_then(|v| v.as_bool())
            .unwrap_or(default.two_finger_scroll),
        disable_while_typing: section
            .get("disable_while_typing")
            .and_then(|v| v.as_bool())
            .unwrap_or(default.disable_while_typing),
        acceleration: section
            .get("acceleration")
            .and_then(|v| v.as_float())
            .unwrap_or(default.acceleration),
        click_method: section
            .get("click_method")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or(default.click_method),
        tap_drag: section
            .get("tap_drag")
            .and_then(|v| v.as_bool())
            .unwrap_or(default.tap_drag),
    })
}

#[tauri::command]
pub fn touchpad_set_config(config: TouchpadConfig) -> Result<(), String> {
    let mut doc = read_doc()?;
    if !doc.is_table() {
        doc = toml::Value::Table(toml::map::Map::new());
    }
    let mut section = toml::map::Map::new();
    section.insert(
        "tap_to_click".to_string(),
        toml::Value::Boolean(config.tap_to_click),
    );
    section.insert(
        "natural_scroll".to_string(),
        toml::Value::Boolean(config.natural_scroll),
    );
    section.insert(
        "two_finger_scroll".to_string(),
        toml::Value::Boolean(config.two_finger_scroll),
    );
    section.insert(
        "disable_while_typing".to_string(),
        toml::Value::Boolean(config.disable_while_typing),
    );
    section.insert(
        "acceleration".to_string(),
        toml::Value::Float(config.acceleration.clamp(-1.0, 1.0)),
    );
    // Belt and braces: reject values the compositor would warn about
    // anyway. Keeps the TOML on disk clean for grep/debugging.
    let click = if config.click_method == "areas" {
        "areas"
    } else {
        "clickfinger"
    };
    section.insert(
        "click_method".to_string(),
        toml::Value::String(click.to_string()),
    );
    section.insert(
        "tap_drag".to_string(),
        toml::Value::Boolean(config.tap_drag),
    );
    doc.as_table_mut()
        .unwrap()
        .insert("touchpad".to_string(), toml::Value::Table(section));
    write_doc(&doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_has_unique_actions() {
        let mut seen = std::collections::HashSet::new();
        for row in CATALOGUE {
            assert!(seen.insert(row.action), "duplicate action {}", row.action);
        }
    }

    #[test]
    fn catalogue_categories_are_known() {
        // Keep in sync with the frontend `CATEGORIES` list in
        // src/lib/stores/keybindings.ts — the Settings UI only
        // renders known categories. Adding a new category here
        // requires the same id on the frontend.
        const VALID: &[&str] = &[
            "window",
            "focus",
            "move",
            "tiling",
            "workspace",
            "workspace_move",
            "workspace_map",
            "keyboard",
            "shell",
            "apps",
        ];
        for row in CATALOGUE {
            assert!(
                VALID.contains(&row.category),
                "unknown category {} for {}",
                row.category,
                row.action
            );
        }
    }

    #[test]
    fn alias_resolution_round_trip() {
        // User's existing TOML has the pre-rename action with a
        // binding. The catalogue has only the new name. alias_for
        // should bridge them so the UI doesn't render "Not set".
        let old = "shell:workspace_overlay_open";
        let new_ = "shell:workspace_map_open";
        assert_eq!(alias_for(new_), Some(old));
        assert!(is_aliased_to_catalogue(old));
        // Non-aliased actions return None.
        assert!(alias_for("shell:waypointer_open").is_none());
        assert!(!is_aliased_to_catalogue("shell:waypointer_open"));
    }

    #[test]
    fn classify_custom_matches_prefixes() {
        assert_eq!(classify_custom("spawn:foo"), "apps");
        assert_eq!(classify_custom("shell:waypointer_open"), "shell");
        assert_eq!(classify_custom("something_else"), "custom");
    }

    #[test]
    fn label_for_custom_renders() {
        assert_eq!(label_for_custom("spawn:firefox"), "Launch firefox");
        assert_eq!(label_for_custom("shell:lock"), "Shell: lock");
        assert_eq!(label_for_custom("bare"), "bare");
    }

    #[test]
    fn mouse_config_defaults_are_zero() {
        let c = MouseConfig::default();
        assert_eq!(c.acceleration, 0.0);
        assert!(!c.natural_scroll);
        assert!(!c.left_handed);
        // scroll_speed 1.0 is libinput's neutral factor — changing this
        // would silently multiply every existing user's scroll.
        assert_eq!(c.scroll_speed, 1.0);
    }

    #[test]
    fn touchpad_defaults_match_spec() {
        let c = TouchpadConfig::default();
        assert!(c.tap_to_click);
        assert!(c.natural_scroll);
        assert!(c.two_finger_scroll);
        assert!(c.disable_while_typing);
        assert_eq!(c.acceleration, 0.0);
        assert_eq!(c.click_method, "clickfinger");
        assert!(c.tap_drag);
    }

    #[test]
    fn catalogue_contains_keyboard_actions() {
        // The two layout-switch actions are deliberately defaultless
        // (Super+Space is Waypointer, Alt+Shift is an XKB-level toggle),
        // but the rows must exist so the Shortcuts UI can list them.
        let next = CATALOGUE
            .iter()
            .find(|r| r.action == "keyboard_layout_next")
            .expect("keyboard_layout_next in CATALOGUE");
        assert_eq!(next.category, "keyboard");
        assert!(next.default_binding.is_none());
        assert!(
            CATALOGUE
                .iter()
                .any(|r| r.action == "keyboard_layout_prev")
        );
    }

    #[test]
    fn label_for_module_action_pretty_prints_prefix_form() {
        assert_eq!(
            label_for_module_action("module:com.example:open", "com.example"),
            "com.example: open",
        );
    }

    #[test]
    fn label_for_module_action_falls_back_when_not_prefixed() {
        assert_eq!(
            label_for_module_action("spawn:foot", "com.example"),
            "com.example: spawn:foot",
        );
    }

    // ── Disk-backed roundtrip tests ──────────────────────────────────
    //
    // These tests flip `LUNARIS_CONFIG_DIR` to a tempdir so the real
    // `~/.config/lunaris/` is never touched. Because the env var is
    // process-global, concurrent runs would stomp on each other; the
    // `TEST_ENV_LOCK` mutex serialises just this family of tests so
    // the rest of the suite can still run in parallel. (Previously
    // the workaround was `-- --test-threads=1`, which forced the
    // whole app-settings suite to serialise.)

    static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_temp_config<F: FnOnce()>(f: F) {
        // `unwrap_or_else(|e| e.into_inner())`: a previous test that
        // poisoned the lock (panicked while holding it) shouldn't
        // block the rest — we recover the guard and keep going.
        let _guard = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::TempDir::new().unwrap();
        let prev = std::env::var_os("LUNARIS_CONFIG_DIR");
        std::env::set_var("LUNARIS_CONFIG_DIR", dir.path());
        f();
        match prev {
            Some(v) => std::env::set_var("LUNARIS_CONFIG_DIR", v),
            None => std::env::remove_var("LUNARIS_CONFIG_DIR"),
        }
    }

    #[test]
    fn keyboard_layouts_roundtrip() {
        with_temp_config(|| {
            keyboard_set_layouts(vec!["de".into(), "us".into()]).unwrap();
            assert_eq!(
                keyboard_get_layouts().unwrap(),
                vec!["de".to_string(), "us".to_string()]
            );

            // Overwrite with a different set — command replaces, not
            // merges.
            keyboard_set_layouts(vec!["fr".into()]).unwrap();
            assert_eq!(
                keyboard_get_layouts().unwrap(),
                vec!["fr".to_string()]
            );
        });
    }

    #[test]
    fn keyboard_layouts_empty_is_rejected() {
        with_temp_config(|| {
            assert!(keyboard_set_layouts(Vec::new()).is_err());
        });
    }

    #[test]
    fn keyboard_layouts_legacy_scalar_accepted() {
        with_temp_config(|| {
            // Old single-string `layout = "de,us"`: the getter must
            // still understand it so pre-migration configs don't lose
            // their layouts when Settings opens.
            std::fs::write(
                compositor_toml_path(),
                "[xkb_config]\nlayout = \"de,us\"\n",
            )
            .unwrap();
            assert_eq!(
                keyboard_get_layouts().unwrap(),
                vec!["de".to_string(), "us".to_string()]
            );
        });
    }

    #[test]
    fn keyboard_set_layouts_removes_legacy_scalar() {
        with_temp_config(|| {
            std::fs::write(
                compositor_toml_path(),
                "[xkb_config]\nlayout = \"de\"\n",
            )
            .unwrap();
            keyboard_set_layouts(vec!["us".into()]).unwrap();
            let content =
                std::fs::read_to_string(compositor_toml_path()).unwrap();
            assert!(!content
                .lines()
                .any(|l| l.trim_start().starts_with("layout = ")));
            assert!(content.contains("layouts = "));
        });
    }

    #[test]
    fn keyboard_variants_roundtrip_and_empty_clears() {
        with_temp_config(|| {
            keyboard_set_variants(vec!["".into(), "dvorak".into()]).unwrap();
            assert_eq!(
                keyboard_get_variants().unwrap(),
                vec!["".to_string(), "dvorak".to_string()]
            );
            keyboard_set_variants(Vec::new()).unwrap();
            let content =
                std::fs::read_to_string(compositor_toml_path()).unwrap();
            assert!(!content.contains("variants"));
            assert!(!content.contains("variant"));
        });
    }

    #[test]
    fn mouse_scroll_speed_clamp() {
        with_temp_config(|| {
            let mut cfg = MouseConfig::default();
            cfg.scroll_speed = 99.0;
            mouse_set_config(cfg).unwrap();
            let got = mouse_get_config().unwrap();
            assert!(got.scroll_speed <= 3.0);
            assert!(got.scroll_speed >= 0.1);

            let mut cfg = MouseConfig::default();
            cfg.scroll_speed = -1.0;
            mouse_set_config(cfg).unwrap();
            assert!(mouse_get_config().unwrap().scroll_speed >= 0.1);
        });
    }

    #[test]
    fn touchpad_click_method_normalizes_unknown_to_clickfinger() {
        with_temp_config(|| {
            let mut cfg = TouchpadConfig::default();
            cfg.click_method = "bogus".into();
            touchpad_set_config(cfg).unwrap();
            assert_eq!(
                touchpad_get_config().unwrap().click_method,
                "clickfinger"
            );
        });
    }

    #[test]
    fn touchpad_tap_drag_roundtrip() {
        with_temp_config(|| {
            let mut cfg = TouchpadConfig::default();
            cfg.tap_drag = false;
            touchpad_set_config(cfg).unwrap();
            assert!(!touchpad_get_config().unwrap().tap_drag);
        });
    }

    #[test]
    fn keybindings_reset_all_to_defaults_removes_section() {
        with_temp_config(|| {
            std::fs::write(
                compositor_toml_path(),
                "[keybindings]\n\"Super+Q\" = \"close_window\"\n",
            )
            .unwrap();
            keybindings_reset_all_to_defaults().unwrap();
            let content =
                std::fs::read_to_string(compositor_toml_path()).unwrap();
            assert!(!content.contains("[keybindings]"));
        });
    }
}
