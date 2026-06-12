//! Arlen files app backend host: the thin command layer over the
//! shared file-browser core (`file-manager-ui-plan.md`). The FM is
//! the unconfined surface, so the commands resolve absolute UI paths
//! against one ambient root capability. The KG commands answer with
//! empty shapes until the structured reads land; the UI mocks them
//! richly in the meantime. Filesystem mutations (`files_op`) arrive
//! with the operations UI.

mod capability;
mod thumbnail;

use std::path::Path;

use arlen_file_browser_core::{
    breadcrumb, list_dir, ops, properties, search, sort_entries, Crumb, FileEntry, SortKey,
};
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use serde::Serialize;

/// Whether the app runs under the Arlen shell (the event-bus socket
/// exists): the UI then leaves its chrome to the global topbar and
/// hides the local fallback toolbar.
#[tauri::command]
fn shell_present() -> bool {
    std::path::Path::new("/run/arlen/event-bus-producer.sock").exists()
}

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

fn conflict_policy(policy: Option<&str>) -> ops::ConflictPolicy {
    match policy {
        Some("replace") => ops::ConflictPolicy::Replace,
        Some("skip") => ops::ConflictPolicy::Skip,
        Some("rename") => ops::ConflictPolicy::Rename,
        _ => ops::ConflictPolicy::Fail,
    }
}

/// The home trash capability (`~/.local/share/Trash` with `files/` and
/// `info/`), created on first use per the freedesktop trash spec.
fn trash_dir() -> Result<Dir, String> {
    let base = dirs::data_local_dir()
        .ok_or("no data dir")?
        .join("Trash");
    std::fs::create_dir_all(base.join("files")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(base.join("info")).map_err(|e| e.to_string())?;
    Dir::open_ambient_dir(&base, ambient_authority()).map_err(|e| e.to_string())
}

fn split_parent(path: &str) -> Result<(String, String), String> {
    let p = Path::new(path);
    let name = p
        .file_name()
        .ok_or_else(|| format!("path has no name: {path}"))?
        .to_string_lossy()
        .into_owned();
    let parent = p
        .parent()
        .map(|x| x.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_string());
    Ok((parent, name))
}

/// One filesystem mutation (contract: kind, sources, destination).
/// `policy` is the conflict policy for copy/move ("fail" when absent);
/// it is a proposed contract extension, flagged, not silently invented.
#[tauri::command]
fn files_op(
    kind: String,
    src: Vec<String>,
    dst: Option<String>,
    policy: Option<String>,
) -> Result<(), String> {
    let dir = root()?;
    let pol = conflict_policy(policy.as_deref());
    match kind.as_str() {
        "new_folder" => {
            let parent = dst.ok_or("new_folder needs the destination folder")?;
            let name = src.first().ok_or("new_folder needs a name")?;
            ops::new_folder(&dir, rel(&parent), name)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        "rename" => {
            let from = src.first().ok_or("rename needs a source")?;
            let to = dst.ok_or("rename needs the new name")?;
            let (parent, from_name) = split_parent(from)?;
            ops::rename(&dir, rel(&parent), &from_name, &to)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        "trash" => {
            let trash = trash_dir()?;
            for s in &src {
                ops::trash_entry(&dir, rel(s), &trash, Path::new(s))
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        "copy" | "move" => {
            let dest_dir = dst.ok_or("copy and move need the destination folder")?;
            for s in &src {
                let (_, name) = split_parent(s)?;
                let target = format!("{}/{}", dest_dir.trim_end_matches('/'), name);
                let r = if kind == "copy" {
                    ops::copy_entry(&dir, rel(s), &dir, rel(&target), pol)
                } else {
                    ops::move_entry(&dir, rel(s), &dir, rel(&target), pol)
                };
                r.map(|_| ()).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        "duplicate" => {
            for s in &src {
                ops::copy_entry(&dir, rel(s), &dir, rel(s), ops::ConflictPolicy::Rename)
                    .map(|_| ())
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        "delete" => {
            for s in &src {
                ops::delete_permanent(&dir, rel(s)).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        other => Err(format!("unknown operation: {other}")),
    }
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

/// Persistent FM state in `~/.config/arlen/files.toml` (the TOML
/// rule). Today that is the bookmark list; defaults stay in Settings.
#[derive(serde::Deserialize, Serialize, Default)]
struct FilesConfig {
    #[serde(default)]
    bookmarks: Vec<String>,
}

fn files_config_path() -> Result<std::path::PathBuf, String> {
    Ok(dirs::config_dir()
        .ok_or("no config dir")?
        .join("arlen")
        .join("files.toml"))
}

fn read_files_config() -> FilesConfig {
    files_config_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_files_config(config: &FilesConfig) -> Result<(), String> {
    let path = files_config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let body = toml::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(path, body).map_err(|e| e.to_string())
}

/// The pinned folders, as places (label = folder name).
#[tauri::command]
fn files_bookmarks() -> Vec<Place> {
    read_files_config()
        .bookmarks
        .iter()
        .map(|p| Place {
            label: Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.clone()),
            icon: "bookmark".to_string(),
            path: p.clone(),
        })
        .collect()
}

/// Pin a folder; idempotent.
#[tauri::command]
fn files_bookmark_add(path: String) -> Result<(), String> {
    let mut config = read_files_config();
    if !config.bookmarks.contains(&path) {
        config.bookmarks.push(path);
        write_files_config(&config)?;
    }
    Ok(())
}

/// Unpin a folder; idempotent.
#[tauri::command]
fn files_bookmark_remove(path: String) -> Result<(), String> {
    let mut config = read_files_config();
    config.bookmarks.retain(|p| p != &path);
    write_files_config(&config)
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
        .plugin(tauri_plugin_arlen_shell::init())
        .manage(thumbnail::ThumbnailLimiter::new())
        .invoke_handler(tauri::generate_handler![
            shell_present,
            frontend_log,
            files_list,
            files_breadcrumb,
            files_places,
            files_info,
            files_search,
            files_op,
            files_bookmarks,
            files_bookmark_add,
            files_bookmark_remove,
            files_open,
            files_projects,
            files_saved_searches,
            thumbnail::files_thumbnail,
            capability::ai_capability
        ])
        .run(tauri::generate_context!())
        .expect("error while running arlen-files");
}
