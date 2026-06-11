//! Arlen files app backend host: the thin command layer over the
//! shared file-browser core (`file-manager-ui-plan.md`). The FM is
//! the unconfined surface, so the commands resolve absolute UI paths
//! against one ambient root capability. The KG commands answer with
//! empty shapes until the structured reads land; the UI mocks them
//! richly in the meantime. Filesystem mutations (`files_op`) arrive
//! with the operations UI.

mod capability;

use std::path::Path;

use arlen_file_browser_core::{
    breadcrumb, list_dir, properties, search, sort_entries, Crumb, FileEntry, SortKey,
};
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use serde::Serialize;

/// Route a log line from the frontend into the Rust logger so it shows
/// up in the same stdout stream as backend logs.
#[tauri::command]
fn frontend_log(level: String, msg: String) {
    match level.as_str() {
        "warn" => log::warn!("[frontend] {msg}"),
        "error" => log::error!("[frontend] {msg}"),
        _ => log::info!("[frontend] {msg}"),
    }
}

fn root() -> Result<Dir, String> {
    Dir::open_ambient_dir("/", ambient_authority()).map_err(|e| e.to_string())
}

/// Absolute UI path to a root-relative core path (`.` for `/`).
fn rel(path: &str) -> String {
    let r = path.trim_start_matches('/');
    if r.is_empty() {
        ".".to_string()
    } else {
        r.to_string()
    }
}

fn sort_key(key: &str) -> SortKey {
    match key {
        "size" => SortKey::Size,
        "modified" => SortKey::Modified,
        "type" => SortKey::Type,
        _ => SortKey::Name,
    }
}

/// List a directory, sorted.
#[tauri::command]
fn files_list(
    path: String,
    sort: String,
    folders_first: bool,
    ascending: bool,
) -> Result<Vec<FileEntry>, String> {
    let dir = root()?;
    let mut entries = list_dir(&dir, rel(&path)).map_err(|e| e.to_string())?;
    sort_entries(&mut entries, sort_key(&sort), folders_first, ascending);
    Ok(entries)
}

/// Decompose a path into clickable crumbs.
#[tauri::command]
fn files_breadcrumb(path: String) -> Vec<Crumb> {
    breadcrumb(Path::new(&path))
}

/// A navigable place in the sidebar (contract shape: label, icon key,
/// path). The icon key is resolved by the UI's icon map.
#[derive(Serialize, Clone)]
struct Place {
    label: String,
    icon: String,
    path: String,
}

/// The sidebar place groups.
#[derive(Serialize)]
struct Places {
    orte: Vec<Place>,
    geraete: Vec<Place>,
}

/// The standard places (xdg user dirs that exist) and devices.
#[tauri::command]
fn files_places() -> Places {
    let mut orte = Vec::new();
    let mut push = |label: &str, icon: &str, dir: Option<std::path::PathBuf>| {
        if let Some(p) = dir {
            if p.is_dir() {
                orte.push(Place {
                    label: label.to_string(),
                    icon: icon.to_string(),
                    path: p.to_string_lossy().into_owned(),
                });
            }
        }
    };
    push("Home", "home", dirs::home_dir());
    push("Documents", "documents", dirs::document_dir());
    push("Downloads", "downloads", dirs::download_dir());
    push("Pictures", "pictures", dirs::picture_dir());
    push("Music", "music", dirs::audio_dir());
    push("Videos", "videos", dirs::video_dir());
    push("Desktop", "desktop", dirs::desktop_dir());

    let geraete = vec![Place {
        label: "System".to_string(),
        icon: "system".to_string(),
        path: "/".to_string(),
    }];
    Places { orte, geraete }
}

/// One provenance line in the info panel (KG shape; empty until the
/// structured reads land).
#[derive(Serialize)]
struct ProvenanceEntry {
    label: String,
    detail: String,
}

/// One relationship line in the info panel (KG shape).
#[derive(Serialize)]
struct Relation {
    label: String,
    target: String,
}

/// The capability view in the info panel (KG shape).
#[derive(Serialize)]
struct Zugriff {
    readable_by: Vec<String>,
    manage_link: String,
}

/// The info panel payload: conventional metadata from the core plus
/// the KG sections.
#[derive(Serialize)]
struct Info {
    conventional: arlen_file_browser_core::Properties,
    woher: Vec<ProvenanceEntry>,
    verwandt: Vec<Relation>,
    zugriff: Zugriff,
}

/// Get-Info for one path: real metadata, empty KG sections for now.
#[tauri::command]
fn files_info(path: String) -> Result<Info, String> {
    let dir = root()?;
    let conventional = properties(&dir, rel(&path)).map_err(|e| e.to_string())?;
    Ok(Info {
        conventional,
        woher: Vec::new(),
        verwandt: Vec::new(),
        zugriff: Zugriff {
            readable_by: Vec::new(),
            manage_link: "settings://permissions".to_string(),
        },
    })
}

/// Bounded name search under a path.
#[tauri::command]
fn files_search(path: String, query: String) -> Result<search::SearchOutcome, String> {
    let dir = root()?;
    let scope = dir.open_dir(rel(&path)).map_err(|e| e.to_string())?;
    let opts = search::SearchOptions {
        query,
        match_names: true,
        match_content: false,
        ..Default::default()
    };
    Ok(search::search(&scope, &opts))
}

/// Open a path with the default handler.
#[tauri::command]
fn files_open(path: String) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(&path)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The Projekte sidebar section (KG; empty until structured reads land).
#[derive(Serialize)]
struct Project {
    id: String,
    name: String,
    path: String,
}

#[tauri::command]
fn files_projects() -> Vec<Project> {
    Vec::new()
}

/// The Suchen sidebar section (KG; empty until structured reads land).
#[derive(Serialize)]
struct SavedSearch {
    id: String,
    name: String,
    query: String,
}

#[tauri::command]
fn files_saved_searches() -> Vec<SavedSearch> {
    Vec::new()
}

/// Tauri application entry point invoked from `main.rs`.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            frontend_log,
            files_list,
            files_breadcrumb,
            files_places,
            files_info,
            files_search,
            files_open,
            files_projects,
            files_saved_searches,
            capability::ai_capability
        ])
        .run(tauri::generate_context!())
        .expect("error while running arlen-files");
}
