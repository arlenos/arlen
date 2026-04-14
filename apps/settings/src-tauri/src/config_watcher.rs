//! File watcher for `~/.config/lunaris/appearance.toml`.
//!
//! Emits `config:appearance:changed` Tauri events on disk changes.
//! The frontend reloads the theme store in response.
//!
//! Editors often write atomically (write to tempfile, then rename),
//! so we watch the parent directory and filter for the filename.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter};

const DEBOUNCE: Duration = Duration::from_millis(100);

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lunaris")
}

/// Start the appearance.toml watcher in a background thread.
pub fn start_appearance_watcher(app: AppHandle) {
    std::thread::Builder::new()
        .name("settings-config-watcher".into())
        .spawn(move || {
            if let Err(e) = run(app) {
                log::warn!("appearance watcher exited: {e}");
            }
        })
        .expect("failed to spawn config watcher thread");
}

fn run(app: AppHandle) -> notify::Result<()> {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);

    let (tx, rx) = mpsc::channel::<Result<Event, notify::Error>>();
    let mut watcher: RecommendedWatcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        notify::Config::default(),
    )?;
    watcher.watch(&dir, RecursiveMode::NonRecursive)?;
    log::info!("watching {} for appearance.toml changes", dir.display());

    let mut last_fired = Instant::now() - DEBOUNCE;

    while let Ok(res) = rx.recv() {
        let Ok(event) = res else { continue };
        if !matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        ) {
            continue;
        }
        let matches = event
            .paths
            .iter()
            .any(|p| p.file_name().and_then(|n| n.to_str()) == Some("appearance.toml"));
        if !matches {
            continue;
        }
        // Debounce bursts of events from a single save.
        if last_fired.elapsed() < DEBOUNCE {
            continue;
        }
        last_fired = Instant::now();
        if let Err(e) = app.emit("config:appearance:changed", ()) {
            log::warn!("emit config:appearance:changed failed: {e}");
        }
    }

    Ok(())
}
