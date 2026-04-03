/// .desktop file parser and app index.
///
/// Scans standard freedesktop application directories on startup, parses
/// `.desktop` files, and exposes the results via Tauri commands.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;

/// A single application entry parsed from a `.desktop` file.
#[derive(Clone, Debug, Serialize)]
pub struct AppEntry {
    /// Human-readable name (Name= key).
    pub name: String,
    /// Command to execute (Exec= key, placeholders stripped).
    pub exec: String,
    /// Icon name or path (Icon= key, not yet resolved to data URL).
    pub icon_name: String,
    /// Short description (Comment= key).
    pub description: String,
    /// Semicolon-separated categories (Categories= key).
    pub categories: Vec<String>,
}

/// Shared app index managed by Tauri.
pub type AppIndex = Arc<Mutex<Vec<AppEntry>>>;

/// Directories to scan for `.desktop` files.
fn app_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/usr/share/applications")];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/share/applications"));
        // Flatpak (user)
        dirs.push(home.join(".local/share/flatpak/exports/share/applications"));
    }
    // System paths
    let extra = [
        "/usr/local/share/applications",
        "/var/lib/flatpak/exports/share/applications",
    ];
    for p in &extra {
        if Path::new(p).is_dir() {
            dirs.push(PathBuf::from(p));
        }
    }
    dirs
}

/// Builds the app index by scanning all directories.
pub fn build_index() -> Vec<AppEntry> {
    let mut entries = Vec::new();
    let mut seen_names: HashMap<String, usize> = HashMap::new();

    for dir in app_dirs() {
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            if let Some(app) = parse_desktop_file(&path) {
                // Deduplicate: later directories override earlier ones
                // (user apps override system apps).
                if let Some(&idx) = seen_names.get(&app.name) {
                    entries[idx] = app.clone();
                } else {
                    seen_names.insert(app.name.clone(), entries.len());
                    entries.push(app);
                }
            }
        }
    }

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    log::info!("app_index: indexed {} applications", entries.len());
    entries
}

/// Parses a single `.desktop` file into an `AppEntry`.
/// Returns `None` if the file should be hidden or is invalid.
fn parse_desktop_file(path: &Path) -> Option<AppEntry> {
    let content = std::fs::read_to_string(path).ok()?;

    let mut in_desktop_entry = false;
    let mut fields: HashMap<String, String> = HashMap::new();

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            fields.insert(key.trim().to_string(), value.trim().to_string());
        }
    }

    // Must be an Application type.
    let entry_type = fields.get("Type").map(|s| s.as_str()).unwrap_or("");
    if entry_type != "Application" {
        return None;
    }

    // Skip hidden or NoDisplay entries.
    if fields.get("NoDisplay").map(|s| s.as_str()) == Some("true") {
        return None;
    }
    if fields.get("Hidden").map(|s| s.as_str()) == Some("true") {
        return None;
    }

    let name = fields.get("Name")?.trim().to_string();
    if name.is_empty() || name.starts_with('_') {
        return None;
    }

    // Skip entries without an Exec command.
    let exec = fields.get("Exec").map(|s| s.trim().to_string()).unwrap_or_default();
    if exec.is_empty() {
        return None;
    }

    // Skip entries marked as not shown in the current desktop.
    if let Some(only_show) = fields.get("OnlyShowIn") {
        // We are not GNOME, KDE, etc. -- skip entries restricted to other desktops.
        // Unless they list "Lunaris" (future-proofing).
        if !only_show.contains("Lunaris") {
            return None;
        }
    }
    let icon_name = fields.get("Icon").unwrap_or(&String::new()).to_string();
    let description = fields.get("Comment").unwrap_or(&String::new()).to_string();
    let categories = fields
        .get("Categories")
        .map(|s| {
            s.split(';')
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect()
        })
        .unwrap_or_default();

    Some(AppEntry {
        name,
        exec: strip_exec_placeholders(&exec),
        icon_name,
        description,
        categories,
    })
}

/// Strips freedesktop Exec placeholders (%u, %U, %f, %F, %i, %c, %k, etc.).
fn strip_exec_placeholders(exec: &str) -> String {
    let mut result = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            // Skip the placeholder character.
            chars.next();
        } else {
            result.push(c);
        }
    }
    result.trim().to_string()
}

/// Returns the full app index.
#[tauri::command]
pub fn get_apps(index: tauri::State<AppIndex>) -> Vec<AppEntry> {
    index.lock().unwrap().clone()
}

/// Launches an application by running its Exec command via `sh -c`.
#[tauri::command]
pub fn launch_app(exec: String) {
    if exec.is_empty() {
        return;
    }
    log::info!("app_index: launching: {exec}");
    std::thread::spawn(move || {
        let _ = std::process::Command::new("sh")
            .arg("-c")
            .arg(&exec)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    });
}
