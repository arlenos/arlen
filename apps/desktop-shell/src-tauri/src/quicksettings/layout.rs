/// Quick Settings layout state at `~/.config/lunaris/quicksettings.toml`.
///
/// The layout is *user intent* — which tiles are visible, in which order,
/// at what size. The shell composes the rendered grid by merging this
/// file with the manifest-declared catalogue (`defaults.rs` for system-
/// tier tiles, `lunaris-modules` manifests for module tiles).
///
/// Same concurrency story as `shell_config`: every write goes through
/// `WRITE_LOCK` and the `update(|file| …)` helper. Atomic tmp+rename so
/// a crash mid-write can never tear the TOML.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use lunaris_modules::TileSize;
use serde::{Deserialize, Serialize};

static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Top-level `quicksettings.toml` shape.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LayoutFile {
    /// Ordered list of tile entries. The render order matches the
    /// vector order; visibility is per-entry.
    #[serde(default, rename = "tile")]
    pub tiles: Vec<TileEntry>,
}

/// One tile's user-side state. The renderer joins this with the
/// manifest-declared tile catalogue: catalogue gives icon/label/
/// detail-component/click-behaviour, this file gives placement+
/// visibility+effective size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileEntry {
    /// Fully qualified tile id. System-tier tiles use `system.<name>`;
    /// module tiles use `<module-id>:<tile.id>`.
    pub id: String,
    /// `true` to render the tile, `false` to keep it in the layout but
    /// hide it from the grid (so adding it back later restores the
    /// original position).
    #[serde(default = "default_visible")]
    pub visible: bool,
    /// User-picked size. Must be in the catalogue tile's `allowed_sizes`
    /// at render time; otherwise the renderer falls back to the
    /// catalogue's `default_size`.
    pub size: TileSize,
}

fn default_visible() -> bool {
    true
}

/// Resolve the layout file path; create the parent directory.
fn config_path() -> PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lunaris");
    let _ = fs::create_dir_all(&dir);
    dir.join("quicksettings.toml")
}

/// Load the on-disk layout. Missing file → empty layout (the renderer
/// then falls back to `defaults::bundled_layout()`). Parse errors
/// bubble up as a string error so the frontend can surface them in a
/// toast.
#[tauri::command]
pub fn qs_layout_get() -> Result<LayoutFile, String> {
    let path = config_path();
    if !path.exists() {
        return Ok(LayoutFile::default());
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("read: {e}"))?;
    toml::from_str(&content).map_err(|e| format!("parse: {e}"))
}

/// Replace the entire layout. Frontend uses this for the
/// drag-reorder save in the customisation UI.
#[tauri::command]
pub fn qs_layout_set(layout: LayoutFile) -> Result<(), String> {
    let _g = WRITE_LOCK.lock().map_err(|_| "WRITE_LOCK poisoned".to_string())?;
    write_atomic(&layout)
}

/// Reset to bundled defaults — deletes `quicksettings.toml`. Renderer
/// then falls back to `defaults::bundled_layout()`.
#[tauri::command]
pub fn qs_layout_reset() -> Result<(), String> {
    let _g = WRITE_LOCK.lock().map_err(|_| "WRITE_LOCK poisoned".to_string())?;
    let path = config_path();
    if path.exists() {
        fs::remove_file(&path).map_err(|e| format!("remove: {e}"))?;
    }
    Ok(())
}

/// Move a tile to a new index (0-based). Out-of-range indices clamp to
/// `[0, len)`. Unknown tile ids return Ok without touching the file so
/// idempotent resets in the UI are safe.
#[tauri::command]
pub fn qs_layout_move_tile(id: String, to_index: usize) -> Result<LayoutFile, String> {
    update(|file| {
        let from = match file.tiles.iter().position(|t| t.id == id) {
            Some(i) => i,
            None => return,
        };
        let entry = file.tiles.remove(from);
        let target = to_index.min(file.tiles.len());
        file.tiles.insert(target, entry);
    })
}

/// Set a tile's visibility. Unknown ids are inserted with
/// `default_size = OneByOne` so the user can hide a tile that was
/// catalogue-only (i.e. never placed yet).
#[tauri::command]
pub fn qs_layout_set_visibility(id: String, visible: bool) -> Result<LayoutFile, String> {
    update(|file| {
        if let Some(entry) = file.tiles.iter_mut().find(|t| t.id == id) {
            entry.visible = visible;
        } else {
            file.tiles.push(TileEntry {
                id,
                visible,
                size: TileSize::OneByOne,
            });
        }
    })
}

/// Set a tile's size. Unknown ids are inserted; size is clamped to the
/// catalogue's `allowed_sizes` at render time, not here.
#[tauri::command]
pub fn qs_layout_set_size(id: String, size: TileSize) -> Result<LayoutFile, String> {
    update(|file| {
        if let Some(entry) = file.tiles.iter_mut().find(|t| t.id == id) {
            entry.size = size;
        } else {
            file.tiles.push(TileEntry {
                id,
                visible: true,
                size,
            });
        }
    })
}

/// Read the freshest layout under `WRITE_LOCK`, hand it to the patcher,
/// write the result atomically. Returns the post-write state so the
/// frontend can update its store without a second round-trip.
fn update<F>(patch: F) -> Result<LayoutFile, String>
where
    F: FnOnce(&mut LayoutFile),
{
    let _g = WRITE_LOCK.lock().map_err(|_| "WRITE_LOCK poisoned".to_string())?;
    let path = config_path();
    let mut file = if path.exists() {
        let content = fs::read_to_string(&path).map_err(|e| format!("read: {e}"))?;
        toml::from_str::<LayoutFile>(&content).map_err(|e| format!("parse: {e}"))?
    } else {
        LayoutFile::default()
    };
    patch(&mut file);
    write_atomic(&file)?;
    Ok(file)
}

/// Atomic tmp+rename write.
fn write_atomic(file: &LayoutFile) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let tmp = path.with_extension("toml.tmp");
    let content = toml::to_string_pretty(file).map_err(|e| format!("serialize: {e}"))?;
    fs::write(&tmp, content).map_err(|e| format!("write tmp: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))
}

/// Watch `~/.config/lunaris/quicksettings.toml` for external writes
/// (the Settings app's QS-layout editor saves through `config_set`,
/// which lands here as a regular file change) and emit a
/// `lunaris://qs-layout-changed` Tauri event so the QuickSettings
/// panel reloads its tile layout without requiring a shell restart.
///
/// Same notify+debounce pattern as `shell_config::start_shell_config_watcher`:
/// editors do atomic tmp+rename writes (which fire as a `Create` on
/// the new path, not a `Modify`), so we listen for any kind of
/// event in the parent directory and filter by filename. The
/// 120ms debounce coalesces the rename's two events and the
/// 30ms post-event sleep gives the on-disk state a moment to
/// settle before the consumer re-reads.
pub fn start_qs_layout_watcher(app: tauri::AppHandle) {
    use notify::{EventKind, RecursiveMode, Watcher};
    use std::sync::Mutex as StdMutex;
    use std::time::{Duration, Instant};
    use tauri::Emitter;

    let target = config_path();
    let watch_dir = match target.parent() {
        Some(p) => p.to_path_buf(),
        None => return,
    };
    let _ = fs::create_dir_all(&watch_dir);

    std::thread::spawn(move || {
        let app_clone = app.clone();
        let target_clone = target.clone();
        let last_fire = StdMutex::new(Instant::now() - Duration::from_secs(1));

        let mut watcher = match notify::recommended_watcher(
            move |event: notify::Result<notify::Event>| {
                let Ok(event) = event else { return };
                if !matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                ) {
                    return;
                }
                let touches_target = event.paths.iter().any(|p| {
                    p == &target_clone
                        || p.file_name()
                            .map(|n| n == "quicksettings.toml")
                            .unwrap_or(false)
                });
                if !touches_target {
                    return;
                }
                {
                    let mut lf = last_fire.lock().unwrap();
                    if lf.elapsed() < Duration::from_millis(120) {
                        return;
                    }
                    *lf = Instant::now();
                }
                std::thread::sleep(Duration::from_millis(30));
                let _ = app_clone.emit("lunaris://qs-layout-changed", ());
            },
        ) {
            Ok(w) => w,
            Err(e) => {
                log::warn!("qs_layout: watcher init failed: {e}");
                return;
            }
        };

        if let Err(e) = watcher.watch(&watch_dir, RecursiveMode::NonRecursive) {
            log::warn!("qs_layout: watch failed: {e}");
            return;
        }
        std::mem::forget(watcher);
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_isolated_config<R>(f: impl FnOnce() -> R) -> R {
        let _g = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let prev = std::env::var_os("XDG_CONFIG_HOME");
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        }
        let out = f();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
        out
    }

    #[test]
    fn round_trip_empty_layout() {
        with_isolated_config(|| {
            assert!(qs_layout_get().unwrap().tiles.is_empty());
        });
    }

    #[test]
    fn set_then_get_preserves_order_and_size() {
        with_isolated_config(|| {
            let layout = LayoutFile {
                tiles: vec![
                    TileEntry { id: "system.network".into(), visible: true, size: TileSize::OneByOne },
                    TileEntry { id: "system.brightness".into(), visible: true, size: TileSize::TwoByOne },
                    TileEntry { id: "system.audio".into(), visible: false, size: TileSize::OneByOne },
                ],
            };
            qs_layout_set(layout.clone()).unwrap();
            let read = qs_layout_get().unwrap();
            assert_eq!(read.tiles.len(), 3);
            assert_eq!(read.tiles[0].id, "system.network");
            assert_eq!(read.tiles[1].size, TileSize::TwoByOne);
            assert!(!read.tiles[2].visible);
        });
    }

    #[test]
    fn move_tile_shifts_position() {
        with_isolated_config(|| {
            qs_layout_set(LayoutFile {
                tiles: vec![
                    TileEntry { id: "a".into(), visible: true, size: TileSize::OneByOne },
                    TileEntry { id: "b".into(), visible: true, size: TileSize::OneByOne },
                    TileEntry { id: "c".into(), visible: true, size: TileSize::OneByOne },
                ],
            })
            .unwrap();
            let after = qs_layout_move_tile("c".into(), 0).unwrap();
            assert_eq!(after.tiles[0].id, "c");
            assert_eq!(after.tiles[1].id, "a");
            assert_eq!(after.tiles[2].id, "b");
        });
    }

    #[test]
    fn move_tile_clamps_out_of_range_index() {
        with_isolated_config(|| {
            qs_layout_set(LayoutFile {
                tiles: vec![
                    TileEntry { id: "a".into(), visible: true, size: TileSize::OneByOne },
                    TileEntry { id: "b".into(), visible: true, size: TileSize::OneByOne },
                ],
            })
            .unwrap();
            let after = qs_layout_move_tile("a".into(), 999).unwrap();
            assert_eq!(after.tiles[0].id, "b");
            assert_eq!(after.tiles[1].id, "a");
        });
    }

    #[test]
    fn move_tile_unknown_id_is_noop() {
        with_isolated_config(|| {
            qs_layout_set(LayoutFile {
                tiles: vec![
                    TileEntry { id: "a".into(), visible: true, size: TileSize::OneByOne },
                ],
            })
            .unwrap();
            let after = qs_layout_move_tile("does-not-exist".into(), 0).unwrap();
            assert_eq!(after.tiles.len(), 1);
            assert_eq!(after.tiles[0].id, "a");
        });
    }

    #[test]
    fn set_visibility_existing_id() {
        with_isolated_config(|| {
            qs_layout_set(LayoutFile {
                tiles: vec![
                    TileEntry { id: "a".into(), visible: true, size: TileSize::OneByOne },
                ],
            })
            .unwrap();
            let after = qs_layout_set_visibility("a".into(), false).unwrap();
            assert!(!after.tiles[0].visible);
        });
    }

    #[test]
    fn set_visibility_inserts_unknown_id() {
        with_isolated_config(|| {
            let after = qs_layout_set_visibility("new.tile".into(), false).unwrap();
            assert_eq!(after.tiles.len(), 1);
            assert_eq!(after.tiles[0].id, "new.tile");
            assert!(!after.tiles[0].visible);
        });
    }

    #[test]
    fn set_size_inserts_unknown_id() {
        with_isolated_config(|| {
            let after = qs_layout_set_size("new.tile".into(), TileSize::TwoByTwo).unwrap();
            assert_eq!(after.tiles[0].size, TileSize::TwoByTwo);
            assert!(after.tiles[0].visible);
        });
    }

    #[test]
    fn reset_deletes_file() {
        with_isolated_config(|| {
            qs_layout_set(LayoutFile {
                tiles: vec![
                    TileEntry { id: "a".into(), visible: true, size: TileSize::OneByOne },
                ],
            })
            .unwrap();
            qs_layout_reset().unwrap();
            assert!(qs_layout_get().unwrap().tiles.is_empty());
        });
    }

    #[test]
    fn malformed_toml_returns_parse_error() {
        with_isolated_config(|| {
            let path = config_path();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "this is { not valid }").unwrap();
            assert!(qs_layout_get().unwrap_err().contains("parse"));
        });
    }
}
