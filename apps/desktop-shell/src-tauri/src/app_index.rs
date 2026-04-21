/// .desktop file parser and app index.
///
/// Scans standard freedesktop application directories on startup, parses
/// `.desktop` files, resolves icons, and exposes the results via Tauri commands.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;

/// A single application entry parsed from a `.desktop` file.
///
/// The `*_lower` fields are precomputed at index-build time so the
/// waypointer app-search plugin can skip `.to_lowercase()` allocations
/// on every keystroke. At 500+ apps with two lowercase conversions
/// per entry per keystroke, the difference is 1000 allocations per
/// keystroke vs zero — small per keystroke but compounding across
/// typing bursts.
#[derive(Clone, Debug, Serialize)]
pub struct AppEntry {
    /// Human-readable name (Name= key).
    pub name: String,
    /// Command to execute (Exec= key, placeholders stripped).
    pub exec: String,
    /// Icon name or path (Icon= key).
    pub icon_name: String,
    /// Base64 data URL for the icon, or None if not resolved.
    pub icon_data: Option<String>,
    /// Short description (Comment= key).
    pub description: String,
    /// Semicolon-separated categories (Categories= key).
    pub categories: Vec<String>,
    /// Precomputed lowercase of `name`. Not serialised to the frontend.
    #[serde(skip)]
    pub name_lower: String,
    /// Precomputed lowercase of `description`. Not serialised.
    #[serde(skip)]
    pub description_lower: String,
}

/// Shared app index managed by Tauri.
pub type AppIndex = Arc<Mutex<Vec<AppEntry>>>;

/// Directories to scan for `.desktop` files.
fn app_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/usr/share/applications")];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/share/applications"));
        dirs.push(home.join(".local/share/flatpak/exports/share/applications"));
    }
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

/// Builds the app index by scanning all directories and resolving icons.
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
                if let Some(&idx) = seen_names.get(&app.name) {
                    entries[idx] = app.clone();
                } else {
                    seen_names.insert(app.name.clone(), entries.len());
                    entries.push(app);
                }
            }
        }
    }

    // Resolve icons for all entries.
    let icon_start = std::time::Instant::now();
    let mut icon_resolved = 0u32;
    for entry in &mut entries {
        if !entry.icon_name.is_empty() {
            entry.icon_data =
                crate::shell_overlay_client::resolve_app_icon(entry.icon_name.clone());
            if entry.icon_data.is_some() {
                icon_resolved += 1;
            }
        }
    }
    log::info!(
        "app_index: icon resolution took {:?} ({icon_resolved}/{} resolved)",
        icon_start.elapsed(),
        entries.len()
    );

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    log::info!("app_index: indexed {} applications", entries.len());
    entries
}

/// Parses a single `.desktop` file into an `AppEntry`.
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

    if fields.get("Type").map(|s| s.as_str()) != Some("Application") {
        return None;
    }
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

    let exec = fields
        .get("Exec")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if exec.is_empty() {
        return None;
    }

    if let Some(only_show) = fields.get("OnlyShowIn") {
        if !only_show.contains("Lunaris") {
            return None;
        }
    }

    let icon_name = fields
        .get("Icon")
        .unwrap_or(&String::new())
        .to_string();
    let description = fields
        .get("Comment")
        .unwrap_or(&String::new())
        .to_string();
    let categories = fields
        .get("Categories")
        .map(|s| {
            s.split(';')
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let name_lower = name.to_lowercase();
    let description_lower = description.to_lowercase();
    Some(AppEntry {
        name,
        exec: strip_exec_placeholders(&exec),
        icon_name,
        icon_data: None,
        description,
        categories,
        name_lower,
        description_lower,
    })
}

/// Strips freedesktop Exec placeholders (%u, %U, %f, %F, %i, %c, %k, etc.).
fn strip_exec_placeholders(exec: &str) -> String {
    let mut result = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            chars.next();
        } else {
            result.push(c);
        }
    }
    result.trim().to_string()
}

/// Returns the full app index (with icons pre-resolved).
#[tauri::command]
pub fn get_apps(index: tauri::State<AppIndex>) -> Vec<AppEntry> {
    index.lock().unwrap().clone()
}

/// Searches the app index by query string. Returns max 20 results.
///
/// Case-insensitive substring matching on name and description.
/// Empty query returns the first 20 apps alphabetically.
#[tauri::command]
pub fn search_apps(index: tauri::State<AppIndex>, query: String) -> Vec<AppEntry> {
    let index = index.lock().unwrap();
    let query = query.trim().to_lowercase();

    if query.is_empty() {
        return index.iter().take(8).cloned().collect();
    }

    let mut scored: Vec<(usize, &AppEntry)> = index
        .iter()
        .filter_map(|app| {
            let name_lower = app.name.to_lowercase();
            let desc_lower = app.description.to_lowercase();

            // Exact name prefix gets highest score.
            if name_lower.starts_with(&query) {
                return Some((0, app));
            }
            // Name contains query.
            if name_lower.contains(&query) {
                return Some((1, app));
            }
            // Word boundary match in name.
            if name_lower.split_whitespace().any(|w| w.starts_with(&query)) {
                return Some((2, app));
            }
            // Description contains query.
            if desc_lower.contains(&query) {
                return Some((3, app));
            }
            // Category match.
            if app.categories.iter().any(|c| c.to_lowercase().contains(&query)) {
                return Some((4, app));
            }
            None
        })
        .collect();

    scored.sort_by_key(|(score, _)| *score);
    scored.into_iter().take(8).map(|(_, app)| app.clone()).collect()
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
