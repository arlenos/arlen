//! Arlen files app backend host: the thin command layer over the
//! shared file-browser core (`file-manager-ui-plan.md`). The FM is
//! the unconfined surface, so the commands resolve absolute UI paths
//! against one ambient root capability. The KG commands answer with
//! empty shapes until the structured reads land; the UI mocks them
//! richly in the meantime. Filesystem mutations (`files_op`) arrive
//! with the operations UI.

mod archive;
mod capability;
mod devices;
mod thumbnail;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use arlen_file_browser_core::undo::{UndoStack, UndoableOp};
use arlen_file_browser_core::{
    breadcrumb, list_dir, ops, properties, search, sort_entries, Crumb, EntryKind, FileEntry,
    SortKey,
};
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use serde::Serialize;
use tauri::Emitter;

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

/// The "New from template" entries: the files in the XDG Templates directory
/// (`~/Templates`), sorted by name. The UI offers these in a New menu; creating
/// one is a copy of the chosen template's `path` into the target folder via the
/// existing `files_op` copy, so no separate create command is needed.
#[tauri::command]
fn files_templates() -> Vec<Place> {
    let Some(dir) = dirs::template_dir() else {
        return Vec::new();
    };
    let Ok(read_dir) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<Place> = read_dir
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let path = e.path();
            path.file_name()
                .and_then(|n| n.to_str())
                .map(|name| Place {
                    label: name.to_string(),
                    icon: "file".to_string(),
                    path: path.to_string_lossy().into_owned(),
                })
        })
        .collect();
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
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

/// Map a caller-scoped provenance view into the info panel's `woher` lines.
/// Only the caller's own identity is named; a foreign actor is summarised, never
/// named (the daemon already enforces the co-tenant no-leak, this preserves it).
fn provenance_to_woher(view: &os_sdk::graph::ProvenanceView) -> Vec<ProvenanceEntry> {
    let mut out: Vec<ProvenanceEntry> = view
        .actors
        .iter()
        .map(|actor| ProvenanceEntry {
            label: "Accessed by".to_string(),
            detail: actor.clone(),
        })
        .collect();
    if view.accessed_by_others {
        out.push(ProvenanceEntry {
            label: "Also accessed by".to_string(),
            detail: "another app".to_string(),
        });
    }
    out
}

/// Read a file's provenance from the knowledge graph (the caller-scoped 0x04
/// op). Best-effort: an out-of-scope object, an absent daemon or any error yields
/// no lines, so the info panel still shows the conventional metadata.
async fn read_woher(path: &str) -> Vec<ProvenanceEntry> {
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());
    // The File node id in the graph is the file's absolute path.
    match client.read_provenance(&abs(path)).await {
        Ok(Some(view)) => provenance_to_woher(&view),
        _ => Vec::new(),
    }
}

/// Escape a string for safe interpolation as a single-quoted Cypher literal:
/// backslash first (so an escaped quote is not double-escaped), then the quote.
/// The KG read path already denies writes and authority labels, so the bounded
/// risk is reading within the caller's own scope; escaping keeps a quote in a
/// real filename from breaking (or perturbing) the query.
fn escape_cypher_literal(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Map the project-membership rows (`{ name }`) into the info panel's
/// relationship lines. Pure, so the shaping is unit-tested without a daemon.
fn verwandt_from_rows(rows: &[std::collections::HashMap<String, serde_json::Value>]) -> Vec<Relation> {
    rows.iter()
        .filter_map(|r| r.get("name").and_then(|v| v.as_str()))
        .map(|name| Relation {
            label: "Part of project".to_string(),
            target: name.to_string(),
        })
        .collect()
}

/// Read a file's KG relationships (its `FILE_PART_OF` project membership) via
/// the structured read op. Best-effort: an out-of-scope object, an absent
/// daemon or any error yields no lines, so the info panel still shows the
/// conventional metadata and the provenance section.
async fn read_verwandt(path: &str) -> Vec<Relation> {
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());
    // The File node id in the graph is the file's absolute path. The id is
    // escaped before interpolation (filenames may contain a quote); the read
    // path denies writes + authority labels, so the bounded reach is the
    // caller's own read scope.
    let cypher = format!(
        "MATCH (f:File {{id: '{}'}})-[:FILE_PART_OF]->(p:Project) RETURN p.name AS name LIMIT 16",
        escape_cypher_literal(&abs(path))
    );
    match client.query_rows(&cypher).await {
        Ok(rows) => verwandt_from_rows(&rows),
        Err(_) => Vec::new(),
    }
}

/// Get-Info for one path: conventional metadata plus the KG provenance and
/// relationship sections. The capability section's reader enumeration stays
/// empty (the per-file authority read is system-scoped, denied to the FM); the
/// manage-access deep-link is the honest non-enumerable entry point.
#[tauri::command]
async fn files_info(path: String) -> Result<Info, String> {
    // Conventional metadata first (the cap-std dir is dropped before the await).
    let conventional = {
        let dir = root()?;
        properties(&dir, rel(&path)).map_err(|e| e.to_string())?
    };
    let woher = read_woher(&path).await;
    let verwandt = read_verwandt(&path).await;
    Ok(Info {
        conventional,
        woher,
        verwandt,
        zugriff: Zugriff {
            readable_by: Vec::new(),
            manage_link: "settings://permissions".to_string(),
        },
    })
}

/// Bounded search under a path. `match_content` opts into the heavier
/// in-file-contents walk (the core supports it; the UI toggles it).
#[tauri::command]
fn files_search(
    path: String,
    query: String,
    match_content: bool,
) -> Result<search::SearchOutcome, String> {
    let dir = root()?;
    let scope = dir.open_dir(rel(&path)).map_err(|e| e.to_string())?;
    let opts = search::SearchOptions {
        query,
        match_names: true,
        match_content,
        ..Default::default()
    };
    Ok(search::search(&scope, &opts))
}

/// The home trash contents (paired `files/` + `info/` entries) for the Trash
/// view, each with its recorded original path + deletion date.
#[tauri::command]
fn files_trash_list() -> Result<Vec<ops::TrashedItem>, String> {
    let trash = trash_dir()?;
    ops::list_trash(&trash).map_err(|e| e.to_string())
}

/// Empty the home trash; returns the number of entries cleared.
#[tauri::command]
fn files_trash_empty() -> Result<usize, String> {
    let trash = trash_dir()?;
    ops::empty_trash(&trash).map_err(|e| e.to_string())
}

/// Restore a trashed entry to a host-resolved destination: its recorded original
/// path reanchored to the FM root capability, NEVER the untrusted `.trashinfo`
/// path used to drive the write (cap-std confines the move to the root and
/// refuses an escaping path). A name conflict gets a fresh name, so a restore
/// never overwrites an existing file. The caller passes `trashed_name` +
/// `original_path` straight from a `files_trash_list` entry. Completes the
/// list/empty/restore trash trio.
#[tauri::command]
fn files_trash_restore(trashed_name: String, original_path: String) -> Result<(), String> {
    let trash = trash_dir()?;
    let dir = root()?;
    let dest_rel = rel(&original_path);
    ops::restore_entry(
        &trash,
        &trashed_name,
        &dir,
        std::path::Path::new(&dest_rel),
        ops::ConflictPolicy::Rename,
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Create a symbolic link `name` under `parent` pointing at `target` (the
/// link's verbatim contents; it may be absolute, relative, or dangling).
#[tauri::command]
fn files_symlink(parent: String, name: String, target: String) -> Result<(), String> {
    let dir = root()?;
    ops::create_symlink(&dir, rel(&parent), &name, &target)
        .map(|_| ())
        .map_err(|e| e.to_string())
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
    undo: tauri::State<'_, Mutex<UndoStack>>,
) -> Result<(), String> {
    let dir = root()?;
    let pol = conflict_policy(policy.as_deref());
    // The undoable ops this call produced; recorded as one batch on full success
    // (a permanent delete records nothing - it has no inverse). A `Skipped`
    // outcome contributes no undo entry.
    let mut undoable: Vec<UndoableOp> = Vec::new();
    match kind.as_str() {
        "new_folder" => {
            let parent = dst.ok_or("new_folder needs the destination folder")?;
            let name = src.first().ok_or("new_folder needs a name")?;
            if let ops::OpOutcome::Created { target } =
                ops::new_folder(&dir, rel(&parent), name).map_err(|e| e.to_string())?
            {
                undoable.push(UndoableOp::Created { path: target });
            }
        }
        "rename" => {
            let from = src.first().ok_or("rename needs a source")?;
            let to = dst.ok_or("rename needs the new name")?;
            let (parent, from_name) = split_parent(from)?;
            ops::rename(&dir, rel(&parent), &from_name, &to).map_err(|e| e.to_string())?;
            undoable.push(UndoableOp::Renamed {
                parent: PathBuf::from(rel(&parent)),
                from_name,
                to_name: to,
            });
        }
        "trash" => {
            let trash = trash_dir()?;
            for s in &src {
                let trashed = ops::trash_entry(&dir, rel(s), &trash, Path::new(s))
                    .map_err(|e| e.to_string())?;
                undoable.push(UndoableOp::Trashed {
                    trashed_name: trashed.trashed_name,
                    original: PathBuf::from(rel(s)),
                });
            }
        }
        "copy" | "move" => {
            let dest_dir = dst.ok_or("copy and move need the destination folder")?;
            for s in &src {
                let (_, name) = split_parent(s)?;
                let target = format!("{}/{}", dest_dir.trim_end_matches('/'), name);
                let outcome = if kind == "copy" {
                    ops::copy_entry(&dir, rel(s), &dir, rel(&target), pol)
                } else {
                    ops::move_entry(&dir, rel(s), &dir, rel(&target), pol)
                }
                .map_err(|e| e.to_string())?;
                match outcome {
                    ops::OpOutcome::Created { target } if kind == "copy" => {
                        undoable.push(UndoableOp::Created { path: target });
                    }
                    ops::OpOutcome::Renamed { target } if kind == "move" => {
                        let (orig_parent, _) = split_parent(s)?;
                        undoable.push(UndoableOp::Moved {
                            current: target,
                            original_parent: PathBuf::from(rel(&orig_parent)),
                        });
                    }
                    _ => {} // Skipped (or a mismatched shape): nothing to undo.
                }
            }
        }
        "duplicate" => {
            for s in &src {
                if let ops::OpOutcome::Created { target } =
                    ops::copy_entry(&dir, rel(s), &dir, rel(s), ops::ConflictPolicy::Rename)
                        .map_err(|e| e.to_string())?
                {
                    undoable.push(UndoableOp::Created { path: target });
                }
            }
        }
        "delete" => {
            for s in &src {
                ops::delete_permanent(&dir, rel(s)).map_err(|e| e.to_string())?;
            }
            // A permanent delete has no inverse; nothing is recorded.
        }
        other => return Err(format!("unknown operation: {other}")),
    }
    if let Ok(mut stack) = undo.lock() {
        stack.record(undoable);
    }
    Ok(())
}

/// Set the Unix permission bits of `path` to `mode` (the editable half of the
/// info panel's metadata, `chmod`). `path` is an absolute path the panel holds
/// for the inspected entry; it is reanchored relative to the root capability,
/// so the write stays inside it. `mode` is masked to the permission bits in the
/// core op. Not yet undoable - the panel shows the current mode, so a change is
/// re-editable; recording an inverse for `Ctrl+Z` is a follow-up.
#[tauri::command]
fn files_set_permissions(path: String, mode: u32) -> Result<(), String> {
    let dir = root()?;
    ops::set_permissions(&dir, rel(&path), mode).map_err(|e| e.to_string())
}

/// Write the editable EXIF string tags (description/artist/copyright) of a JPEG
/// `path`, the media half of the info panel's editable metadata (permissions
/// and rename are the other two). The write is fail-safe: the core reads the
/// original, splices the tags into an in-memory copy, verifies it on readback,
/// then atomically swaps it over the original, so a failed write never corrupts
/// the file. A non-JPEG path or an all-empty edit is refused before any write.
/// `path` is reanchored relative to the root capability. A `None` field leaves
/// that tag untouched.
#[tauri::command]
fn files_set_exif_tags(
    path: String,
    description: Option<String>,
    artist: Option<String>,
    copyright: Option<String>,
) -> Result<(), String> {
    let dir = root()?;
    let edits = arlen_file_browser_core::metadata::ExifEdits {
        description,
        artist,
        copyright,
    };
    arlen_file_browser_core::metadata::write_exif_tags(&dir, rel(&path), &edits)
        .map_err(|e| e.to_string())
}

/// Read the editable EXIF string tags (description/artist/copyright) of a JPEG
/// `path`, so the info panel can show the current values before an edit. A
/// non-JPEG path, or a JPEG with no EXIF, yields an empty editor (all fields
/// `None`) rather than an error. `path` is reanchored relative to the root
/// capability. The read counterpart of `files_set_exif_tags`.
#[tauri::command]
fn files_get_exif_tags(
    path: String,
) -> Result<arlen_file_browser_core::metadata::ExifEdits, String> {
    let dir = root()?;
    arlen_file_browser_core::metadata::read_exif_tags(&dir, rel(&path)).map_err(|e| e.to_string())
}

/// Undo the most recent file operation (`Ctrl+Z`). Pops the last recorded batch
/// and applies each inverse through the ops; `Ok(false)` when there is nothing to
/// undo. A permanent delete was never recorded, so it is never offered as undo.
#[tauri::command]
fn files_undo(undo: tauri::State<'_, Mutex<UndoStack>>) -> Result<bool, String> {
    let dir = root()?;
    let trash = trash_dir()?;
    let mut stack = undo.lock().map_err(|e| e.to_string())?;
    stack.undo(&dir, &trash).map_err(|e| e.to_string())
}

/// A root-relative core path to the real absolute filesystem path. The FM core
/// addresses everything relative to the host root (`/`, see [`rel`]); `xdg-open`
/// needs a real absolute path, since a relative one resolves against the FM
/// process's own cwd and fails ("folder not found"). Idempotent for a path the UI
/// already sent absolute; `.`/empty/`/` map to the root.
fn abs(path: &str) -> String {
    let trimmed = path.trim_start_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        "/".to_string()
    } else {
        format!("/{trimmed}")
    }
}

/// Extract a tar-family archive (`.tar`, `.tar.gz`, `.tgz`) into `dest`.
///
/// Runs off the listing path (extraction can be slow). Entries are written
/// through a cap-std capability opened at `dest`, so a traversing or escaping
/// entry is refused by the capability; only regular files and directories are
/// extracted (symlinks/special files are skipped) and the total size and entry
/// count are bounded. `dest` is created if absent.
#[tauri::command]
async fn files_extract(archive: String, dest: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let name = Path::new(&archive)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| "archive has no name".to_string())?
            .to_string();
        if !archive::is_extractable(&name) {
            return Err(format!("unsupported archive format: {name}"));
        }
        let r = root()?;
        let file = r.open(rel(&archive)).map_err(|e| e.to_string())?;
        r.create_dir_all(rel(&dest)).map_err(|e| e.to_string())?;
        let dest_dir = r.open_dir(rel(&dest)).map_err(|e| e.to_string())?;
        if archive::is_zip(&name) {
            archive::zip_extract(file, &dest_dir)
        } else {
            archive::extract_named(&name, file, &dest_dir)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Compress `sources` into the tar-family archive at `dest` (`.tar`, `.tar.gz`,
/// `.tgz`, chosen by `dest`'s extension).
///
/// Runs off the listing path. Each source is stored under its basename, so
/// extraction restores the selected items without their absolute prefix. Sources
/// are read through the root capability; symlinks and special files are skipped.
#[tauri::command]
async fn files_compress(sources: Vec<String>, dest: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if sources.is_empty() {
            return Err("nothing to compress".to_string());
        }
        let name = Path::new(&dest)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| "destination has no name".to_string())?
            .to_string();
        if !archive::is_extractable(&name) {
            return Err(format!("unsupported archive format: {name}"));
        }
        let r = root()?;
        let rels: Vec<String> = sources.iter().map(|s| rel(s)).collect();
        let out = r.create(rel(&dest)).map_err(|e| e.to_string())?;
        if archive::is_zip(&name) {
            archive::zip_compress(&r, &rels, out)
        } else {
            archive::compress(&r, &rels, out, archive::is_gzip_tar(&name))
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// List the entries of a tar-family archive without extracting it (FM-R12
/// browse-into-archive). Read-only: the archive is read through the root
/// capability and only entry metadata is returned, so the UI can navigate into a
/// `.tar`/`.tar.gz`/`.tgz` as a read-scoped folder.
#[tauri::command]
async fn files_archive_list(archive: String) -> Result<Vec<archive::ArchiveEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let name = Path::new(&archive)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| "archive has no name".to_string())?
            .to_string();
        if !archive::is_extractable(&name) {
            return Err(format!("unsupported archive format: {name}"));
        }
        let r = root()?;
        let file = r.open(rel(&archive)).map_err(|e| e.to_string())?;
        if archive::is_zip(&name) {
            archive::zip_list(file)
        } else {
            archive::list_named(&name, file)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Open a path with the default handler.
#[tauri::command]
fn files_open(path: String) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(abs(&path))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// One app the Open-With picker offers (a serializable view of the core
/// `DesktopApp`; `exec` is the verbatim `.desktop` Exec passed back to
/// `files_open_with`).
#[derive(Serialize)]
struct AppInfo {
    name: String,
    exec: String,
    terminal: bool,
}

/// The freedesktop application directories, user first: `$XDG_DATA_HOME/
/// applications` (or `~/.local/share/applications`) then each `$XDG_DATA_DIRS/
/// applications` (default `/usr/local/share` + `/usr/share`). User-first so a
/// user `.desktop` overrides a system one of the same id.
fn application_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = dirs::data_local_dir() {
        out.push(home.join("applications"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    for d in data_dirs.split(':').filter(|s| !s.is_empty()) {
        out.push(Path::new(d).join("applications"));
    }
    out
}

/// The applications that declare they handle `path`'s MIME type, for the
/// Open-With picker. The file's type comes from `xdg-mime`; `.desktop` entries
/// are read from the standard application dirs (a user entry overriding a system
/// one of the same id) and matched + sorted by the core. An unresolved MIME or
/// an unreadable dir yields fewer entries, never an error.
#[tauri::command]
fn files_apps_for(path: String) -> Vec<AppInfo> {
    let abs = abs(&path);
    let mime = match std::process::Command::new("xdg-mime")
        .args(["query", "filetype", &abs])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => return Vec::new(),
    };
    if mime.is_empty() {
        return Vec::new();
    }
    let mut apps = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    for dir in application_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            // The desktop-file id (basename) decides override order: the first
            // occurrence (user dir, scanned first) wins; skip later same-id files.
            let Some(id) = p.file_name().and_then(|f| f.to_str()) else {
                continue;
            };
            if !seen_ids.insert(id.to_string()) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&p) {
                if let Some(app) = arlen_file_browser_core::openwith::parse_desktop_app(id, &text) {
                    apps.push(app);
                }
            }
        }
    }
    // The user's default handler for this type leads the picker (freedesktop
    // expectation), read from `mimeapps.list`; the rest stay alphabetical.
    let default_id = default_handler_for(&mime);
    arlen_file_browser_core::openwith::apps_for_mime(&apps, &mime, default_id.as_deref())
        .into_iter()
        .map(|a| AppInfo {
            name: a.name,
            exec: a.exec,
            terminal: a.terminal,
        })
        .collect()
}

/// The user's default application (a desktop-file id) for `mime`, from
/// `mimeapps.list` in the user config dir ($XDG_CONFIG_HOME, else ~/.config).
/// An unreadable or absent file means no default; the picker stays alphabetical.
fn default_handler_for(mime: &str) -> Option<String> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    let text = std::fs::read_to_string(base.join("mimeapps.list")).ok()?;
    arlen_file_browser_core::openwith::default_app_for(&text, mime)
}

/// Open `path` with the app whose `.desktop` `Exec=` is `exec` (from
/// `files_apps_for`). The core expands the Exec to an argv (field codes -> the
/// file path) and we spawn it WITHOUT a shell, so a path with spaces or shell
/// metacharacters is one inert argument. A `Terminal=true` app launches as-is
/// for now (no terminal wrapper); most Open-With targets are GUI apps.
#[tauri::command]
fn files_open_with(path: String, exec: String) -> Result<(), String> {
    let argv = arlen_file_browser_core::openwith::expand_exec(&exec, &abs(&path));
    let Some((program, args)) = argv.split_first() else {
        return Err("empty exec".to_string());
    };
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The mounted, non-system volumes for the Devices sidebar (removable drives +
/// extra data mounts), from `lsblk --json`. An absent/failing lsblk yields an
/// empty list rather than an error - the sidebar just shows no devices.
#[tauri::command]
fn files_devices() -> Vec<devices::MountedDevice> {
    match std::process::Command::new("lsblk")
        .args(["--json", "-o", "NAME,LABEL,MOUNTPOINT,RM,TYPE,FSTYPE"])
        .output()
    {
        Ok(o) if o.status.success() => {
            devices::mounted_volumes(&String::from_utf8_lossy(&o.stdout))
        }
        _ => Vec::new(),
    }
}

/// Run a `udisksctl` verb on a block device for the Devices sidebar. Spawned
/// without a shell (the device path is one inert argv element), and the device
/// must be a `/dev/` node, so the caller cannot smuggle a command. udisks2's
/// own polkit decides authorisation. Returns the trimmed stderr on failure.
fn udisksctl(verb: &str, device: &str) -> Result<(), String> {
    if !device.starts_with("/dev/") {
        return Err("not a block device".to_string());
    }
    let out = std::process::Command::new("udisksctl")
        .args([verb, "-b", device])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Mount a block device (`udisksctl mount`). For a device the Devices sidebar
/// lists as unmounted.
#[tauri::command]
fn files_mount(device: String) -> Result<(), String> {
    udisksctl("mount", &device)
}

/// Unmount a block device (`udisksctl unmount`).
#[tauri::command]
fn files_unmount(device: String) -> Result<(), String> {
    udisksctl("unmount", &device)
}

/// Safely remove the drive of a block device (`udisksctl power-off`; udisks
/// resolves the partition to its drive and unmounts first). The "eject" action.
#[tauri::command]
fn files_eject(device: String) -> Result<(), String> {
    udisksctl("power-off", &device)
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

/// The Projekte sidebar section (the live `FILE_PART_OF` projects from the KG).
#[derive(Serialize)]
struct Project {
    id: String,
    name: String,
    path: String,
}

/// Map project rows (`{ id, name, path }`) into sidebar entries, skipping any
/// row missing the id or name. Pure, so the shaping is unit-tested without a
/// daemon.
fn projects_from_rows(rows: &[std::collections::HashMap<String, serde_json::Value>]) -> Vec<Project> {
    rows.iter()
        .filter_map(|r| {
            let id = r.get("id").and_then(|v| v.as_str())?;
            let name = r.get("name").and_then(|v| v.as_str())?;
            let path = r.get("path").and_then(|v| v.as_str()).unwrap_or("");
            Some(Project {
                id: id.to_string(),
                name: name.to_string(),
                path: path.to_string(),
            })
        })
        .collect()
}

/// The live projects for the sidebar's Projects section. Best-effort: an absent
/// daemon or an out-of-scope read yields no entries (the rest of the sidebar
/// still renders). Only live projects are shown (`expired_at IS NULL`); archived
/// ones are omitted. The query is static (no interpolation), so no escaping.
#[tauri::command]
async fn files_projects() -> Vec<Project> {
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());
    let cypher = "MATCH (p:Project) WHERE p.expired_at IS NULL \
                  RETURN p.id AS id, p.name AS name, p.root_path AS path LIMIT 64";
    match client.query_rows(cypher).await {
        Ok(rows) => projects_from_rows(&rows),
        Err(_) => Vec::new(),
    }
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

/// The Zuletzt sidebar section: the most-recently-accessed files, surfaced from
/// the KG (not re-derived by re-scanning the filesystem). The File node id is
/// the absolute path; `accessed` is its `last_accessed` time.
#[derive(Serialize)]
struct RecentFile {
    /// Absolute file path (the File node id).
    path: String,
    /// The basename, for the row label.
    name: String,
    /// Last-accessed time, microseconds since the Unix epoch (0 if absent).
    accessed: i64,
}

/// Map recent-file rows (`{ path, accessed }`) into sidebar entries, deriving
/// the basename label and skipping any row missing the path. Pure, so the
/// shaping is unit-tested without a daemon.
fn recent_from_rows(rows: &[std::collections::HashMap<String, serde_json::Value>]) -> Vec<RecentFile> {
    rows.iter()
        .filter_map(|r| {
            let path = r.get("path").and_then(|v| v.as_str())?;
            let accessed = r.get("accessed").and_then(|v| v.as_i64()).unwrap_or(0);
            let name = std::path::Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path)
                .to_string();
            Some(RecentFile {
                path: path.to_string(),
                name,
                accessed,
            })
        })
        .collect()
}

/// The recently-accessed files for the sidebar's Recent section, newest first.
/// Best-effort: an absent daemon, an out-of-scope read, or a caller without
/// `system.File` read scope yields no entries (the rest of the sidebar still
/// renders). The query is static (no interpolation), so no escaping.
#[tauri::command]
async fn files_recent() -> Vec<RecentFile> {
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    let client = os_sdk::graph::UnixGraphClient::new(socket.to_string_lossy().into_owned());
    let cypher = "MATCH (f:File) WHERE f.last_accessed IS NOT NULL \
                  RETURN f.path AS path, f.last_accessed AS accessed \
                  ORDER BY f.last_accessed DESC LIMIT 32";
    match client.query_rows(cypher).await {
        Ok(rows) => recent_from_rows(&rows),
        Err(_) => Vec::new(),
    }
}

/// Map a recent-file row into a [`FileEntry`] for the Recent navigation LOCATION
/// (item 12: Recent/Trash become navigable locations the same browser view renders,
/// not bespoke overlays). `full_path` carries each file's own absolute path (Recent
/// files are scattered, not under one dir), and `modified_unix` carries the
/// last-accessed time (micros → secs) so the location's "Last accessed" column reads
/// it. Pure, so the shaping is unit-tested without a daemon.
fn recent_to_entry(rf: &RecentFile) -> FileEntry {
    FileEntry {
        is_hidden: rf.name.starts_with('.'),
        name: rf.name.clone(),
        kind: EntryKind::File,
        size: None,
        // micros → secs; a negative/absent (0) accessed time becomes None.
        modified_unix: (rf.accessed > 0).then(|| (rf.accessed / 1_000_000) as u64),
        readonly: false,
        symlink_target: None,
        full_path: Some(rf.path.clone()),
    }
}

/// Resolve a virtual navigation location to a file listing (item 12). `"recent"`
/// returns the recently-accessed files as [`FileEntry`]s (each carrying its own
/// `full_path`), so the browser controller can navigate to `recent` like a folder.
/// `"trash"` is the next slice (it additionally needs a per-entry restore token +
/// the freedesktop deletion-date parse); an unknown location is refused. The
/// column/action presentation on these entries is arlen-ui's (the `full_path` field
/// is the seam it reads).
#[tauri::command]
async fn files_list_location(location: String) -> Result<Vec<FileEntry>, String> {
    match location.as_str() {
        "recent" => Ok(files_recent().await.iter().map(recent_to_entry).collect()),
        "trash" => Err("trash location listing is the next slice".to_string()),
        other => Err(format!("unknown virtual location: {other}")),
    }
}

/// Tauri application entry point invoked from `main.rs`.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Publish the file-manager's global menu into the topbar via the
/// `shell.menu` surface. The menu travels over the Event Bus (cross
/// The bundle identifier, used both as the GTK program name (so the Wayland
/// toplevel app_id is this, see [`run`]) and as the menu's publish key, so the
/// shell's focused-window app_id matches the published menu (#2).
const APP_ID: &str = "dev.arlen.files";

/// process) to the desktop-shell's GlobalMenuBar, keyed by the app id, so
/// it appears whenever a Files window is focused. The id must match the
/// focused window's compositor `xdg_toplevel.app_id`, which `run` pins to
/// [`APP_ID`] via the GTK program name (GTK3 ignores `enableGTKAppId` for the
/// Wayland app_id). `ARLEN_APP_ID` stays an override for a launcher that pins
/// a different id.
async fn publish_app_menu() {
    use os_sdk::menu::{Menu, MenuGroup, MenuItem};

    let app_id = std::env::var("ARLEN_APP_ID").unwrap_or_else(|_| APP_ID.to_string());
    let socket = os_sdk::runtime::socket_path("ARLEN_PRODUCER_SOCKET", "event-bus-producer.sock");
    let emitter = os_sdk::event::UnixEventEmitter::new(socket.to_string_lossy().into_owned());
    let menu = Menu::new(emitter, app_id);

    let groups = vec![
        MenuGroup::new(
            "File",
            vec![
                MenuItem::item("New Folder", "file.new_folder"),
                MenuItem::item("New Window", "file.new_window"),
                MenuItem::separator(),
                MenuItem::item("Properties", "file.properties"),
                MenuItem::separator(),
                MenuItem::item("Close Window", "file.close"),
            ],
        ),
        MenuGroup::new(
            "Edit",
            vec![
                MenuItem::item("Undo", "edit.undo"),
                MenuItem::separator(),
                MenuItem::item("Cut", "edit.cut"),
                MenuItem::item("Copy", "edit.copy"),
                MenuItem::item("Paste", "edit.paste"),
                MenuItem::separator(),
                MenuItem::item("Rename", "edit.rename"),
                MenuItem::item("Move to Trash", "edit.trash"),
                MenuItem::item("Select All", "edit.select_all"),
            ],
        ),
        MenuGroup::new(
            "View",
            vec![
                MenuItem::item("Refresh", "view.refresh"),
                MenuItem::item("Show Hidden Files", "view.toggle_hidden"),
                MenuItem::separator(),
                MenuItem::submenu(
                    "Sort By",
                    vec![
                        MenuItem::item("Name", "view.sort.name"),
                        MenuItem::item("Size", "view.sort.size"),
                        MenuItem::item("Type", "view.sort.type"),
                        MenuItem::item("Modified", "view.sort.modified"),
                    ],
                ),
            ],
        ),
        MenuGroup::new(
            "Go",
            vec![
                MenuItem::item("Home", "go.home"),
                MenuItem::item("Recent", "go.recent"),
                MenuItem::item("Trash", "go.trash"),
                MenuItem::separator(),
                MenuItem::item("Parent Folder", "go.up"),
            ],
        ),
        MenuGroup::new("Help", vec![MenuItem::item("About Files", "help.about")]),
    ];

    if let Err(e) = menu.register(groups).await {
        log::warn!("failed to publish the files app menu: {e}");
    }
}

/// Forwarded to the webview as `arlen://menu-action` when the user
/// clicks a topbar menu item. The frontend maps `action` to the
/// matching file-manager operation.
#[derive(Clone, Serialize)]
struct MenuActionEvent {
    action: String,
}

/// Receive topbar-menu clicks and forward them into this app's webview.
///
/// The menu is published into the topbar over the Event Bus
/// ([`publish_app_menu`]); the shell publishes the clicked action back
/// onto the bus as `app.menu.action_invoked`. We subscribe to that
/// back-channel (filtered to our own app_id by the SDK) and re-emit
/// each action as a Tauri event the frontend handles (#2b). Best-effort:
/// if the bus is unreachable the menu simply stays inert, like the
/// publish side.
async fn run_menu_action_listener(app: tauri::AppHandle) {
    let app_id = std::env::var("ARLEN_APP_ID").unwrap_or_else(|_| APP_ID.to_string());
    let socket =
        os_sdk::runtime::socket_path("ARLEN_CONSUMER_SOCKET", "event-bus-consumer.sock");
    let consumer = os_sdk::event_consumer::UnixEventConsumer::new(
        socket.to_string_lossy().into_owned(),
    );
    let mut actions = match os_sdk::menu::subscribe_menu_actions(&consumer, app_id).await {
        Ok(rx) => rx,
        Err(e) => {
            log::warn!("menu-action channel unavailable: {e}");
            return;
        }
    };
    while let Some(action) = actions.recv().await {
        if let Err(e) = app.emit("arlen://menu-action", MenuActionEvent { action }) {
            log::warn!("forwarding a menu action to the webview failed: {e}");
        }
    }
}

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // Pin the GTK program name to the bundle id BEFORE GTK initialises. On
    // webkit2gtk-4.1 (GTK3 - what the Tauri apps link) the Wayland
    // `xdg_toplevel.app_id` is taken from `g_get_prgname()` (the binary name,
    // `arlen-files`), NOT the GApplication id that `enableGTKAppId` sets - so
    // without this the focused window presents `arlen-files` while the shell
    // looks up the menu under the bundle id `dev.arlen.files`, and the topbar
    // menu never matches/renders (#2). Setting prgname here makes the toplevel
    // app_id the bundle id, which `enableGTKAppId` intended but cannot achieve
    // on GTK3. `gtk_init` keeps an already-set prgname, so this wins.
    glib::set_prgname(Some(APP_ID));

    tauri::Builder::default()
        .setup(|app| {
            // Publish the global menu once the app is up; it routes over the
            // Event Bus to the shell topbar (cross-process).
            tauri::async_runtime::spawn(publish_app_menu());
            // Receive topbar-menu clicks back from the shell and forward them
            // into the webview so the frontend runs the operation (#2b).
            tauri::async_runtime::spawn(run_menu_action_listener(app.handle().clone()));
            Ok(())
        })
        .plugin(tauri_plugin_arlen_shell::init())
        .manage(thumbnail::ThumbnailLimiter::new())
        .manage(Mutex::new(UndoStack::new()))
        .invoke_handler(tauri::generate_handler![
            shell_present,
            frontend_log,
            files_list,
            files_list_location,
            files_breadcrumb,
            files_places,
            files_info,
            files_search,
            files_op,
            files_set_permissions,
            files_set_exif_tags,
            files_get_exif_tags,
            files_undo,
            files_bookmarks,
            files_bookmark_add,
            files_bookmark_remove,
            files_open,
            files_apps_for,
            files_open_with,
            files_devices,
            files_mount,
            files_unmount,
            files_eject,
            files_extract,
            files_compress,
            files_archive_list,
            files_templates,
            files_projects,
            files_saved_searches,
            files_recent,
            files_trash_list,
            files_trash_empty,
            files_trash_restore,
            files_symlink,
            thumbnail::files_thumbnail,
            capability::ai_capability
        ])
        .run(tauri::generate_context!())
        .expect("error while running arlen-files");
}

#[cfg(test)]
mod tests {
    use super::{
        abs, escape_cypher_literal, projects_from_rows, provenance_to_woher, recent_from_rows,
        recent_to_entry, verwandt_from_rows, EntryKind, RecentFile,
    };
    use std::collections::HashMap;

    #[test]
    fn recent_to_entry_carries_the_full_path_and_accessed_time() {
        // Item 12: a recent file maps to a FileEntry for the Recent navigation
        // location - its own absolute path in `full_path`, last-accessed (micros ->
        // secs) in `modified_unix`, classified as a File.
        let rf = RecentFile {
            path: "/home/u/proj/notes.md".to_string(),
            name: "notes.md".to_string(),
            accessed: 5_000_000, // 5s in micros
        };
        let e = recent_to_entry(&rf);
        assert_eq!(e.full_path.as_deref(), Some("/home/u/proj/notes.md"));
        assert_eq!(e.name, "notes.md");
        assert_eq!(e.kind, EntryKind::File);
        assert_eq!(e.modified_unix, Some(5));
        assert!(!e.is_hidden);
        // An absent (0) accessed time is None, not epoch-zero.
        let rf0 = RecentFile { accessed: 0, ..rf };
        assert_eq!(recent_to_entry(&rf0).modified_unix, None);
    }

    #[test]
    fn projects_map_rows_and_skip_incomplete_ones() {
        let mut full = HashMap::new();
        full.insert("id".to_string(), serde_json::json!("proj-1"));
        full.insert("name".to_string(), serde_json::json!("Arlen"));
        full.insert("path".to_string(), serde_json::json!("/home/tim/arlen"));
        // A row missing the name is skipped; a missing path defaults to empty.
        let mut no_name = HashMap::new();
        no_name.insert("id".to_string(), serde_json::json!("proj-2"));
        let mut no_path = HashMap::new();
        no_path.insert("id".to_string(), serde_json::json!("proj-3"));
        no_path.insert("name".to_string(), serde_json::json!("Loose"));

        let projects = projects_from_rows(&[full, no_name, no_path]);
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].name, "Arlen");
        assert_eq!(projects[0].path, "/home/tim/arlen");
        assert_eq!(projects[1].name, "Loose");
        assert_eq!(projects[1].path, "");
    }

    #[test]
    fn recent_derives_basename_and_skips_pathless_rows() {
        let mut full = HashMap::new();
        full.insert("path".to_string(), serde_json::json!("/home/tim/notes.md"));
        full.insert("accessed".to_string(), serde_json::json!(1700000000000000_i64));
        // A row missing the path is skipped; a missing accessed defaults to 0.
        let no_path: HashMap<String, serde_json::Value> = HashMap::new();
        let mut no_time = HashMap::new();
        no_time.insert("path".to_string(), serde_json::json!("/etc/hosts"));

        let recent = recent_from_rows(&[full, no_path, no_time]);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].name, "notes.md", "the basename labels the row");
        assert_eq!(recent[0].path, "/home/tim/notes.md");
        assert_eq!(recent[0].accessed, 1700000000000000);
        assert_eq!(recent[1].name, "hosts");
        assert_eq!(recent[1].accessed, 0);
    }

    #[test]
    fn verwandt_maps_project_rows_to_relations() {
        let mut row = HashMap::new();
        row.insert("name".to_string(), serde_json::json!("Arlen"));
        let rels = verwandt_from_rows(&[row]);
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].label, "Part of project");
        assert_eq!(rels[0].target, "Arlen");
    }

    #[test]
    fn verwandt_skips_rows_without_a_name() {
        let empty: HashMap<String, serde_json::Value> = HashMap::new();
        let mut non_string = HashMap::new();
        non_string.insert("name".to_string(), serde_json::json!(42));
        assert!(verwandt_from_rows(&[empty, non_string]).is_empty());
    }

    #[test]
    fn cypher_literal_escapes_quote_and_backslash() {
        // Backslash first, so an escaped quote is not double-escaped.
        assert_eq!(escape_cypher_literal("a'b"), "a\\'b");
        assert_eq!(escape_cypher_literal("a\\b"), "a\\\\b");
        assert_eq!(escape_cypher_literal("/home/tim/it's a\\dir"), "/home/tim/it\\'s a\\\\dir");
        assert_eq!(escape_cypher_literal("/plain/path"), "/plain/path");
    }

    #[test]
    fn woher_names_only_own_actors_and_summarises_others() {
        let view = os_sdk::graph::ProvenanceView {
            actors: vec!["com.acme.editor".to_string()],
            accessed_by_others: true,
        };
        let lines = provenance_to_woher(&view);
        // The caller's own actor is named; the foreign actor is only summarised.
        assert!(lines.iter().any(|l| l.detail == "com.acme.editor"));
        assert!(lines.iter().any(|l| l.detail == "another app"));
        assert!(!lines.iter().any(|l| l.detail.contains("co-tenant")));
    }

    #[test]
    fn woher_is_empty_with_no_actors_and_no_others() {
        let view = os_sdk::graph::ProvenanceView {
            actors: vec![],
            accessed_by_others: false,
        };
        assert!(provenance_to_woher(&view).is_empty());
    }

    #[test]
    fn abs_makes_a_root_relative_path_absolute() {
        assert_eq!(abs("home/tim/notes.txt"), "/home/tim/notes.txt");
    }

    #[test]
    fn abs_is_idempotent_for_an_already_absolute_path() {
        assert_eq!(abs("/home/tim/notes.txt"), "/home/tim/notes.txt");
    }

    #[test]
    fn abs_maps_root_and_empty_to_slash() {
        assert_eq!(abs("."), "/");
        assert_eq!(abs(""), "/");
        assert_eq!(abs("/"), "/");
    }
}
