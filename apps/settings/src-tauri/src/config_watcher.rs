//! File watcher for `~/.config/lunaris/*.toml`.
//!
//! Emits one event per file:
//!   * `config:appearance:changed`
//!   * `config:notifications:changed`
//!   * `config:shell:changed`
//!   * `config:compositor:changed`
//!
//! Editors often write atomically (write to tempfile, then rename),
//! so we watch the parent directory and filter on the filename.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter};

const DEBOUNCE: Duration = Duration::from_millis(100);

/// (filename, event-name) pairs we care about.
const WATCHED: &[(&str, &str)] = &[
    ("appearance.toml", "config:appearance:changed"),
    ("notifications.toml", "config:notifications:changed"),
    ("shell.toml", "config:shell:changed"),
    ("compositor.toml", "config:compositor:changed"),
];

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lunaris")
}

/// Start the multi-file Lunaris config watcher in a background thread.
/// The old name is kept for backwards-compat with `lib.rs::setup`.
pub fn start_appearance_watcher(app: AppHandle) {
    std::thread::Builder::new()
        .name("settings-config-watcher".into())
        .spawn(move || {
            if let Err(e) = run(app) {
                log::warn!("config watcher exited: {e}");
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
    log::info!("watching {} for Lunaris config changes", dir.display());

    let mut last_fired: HashMap<&'static str, Instant> = HashMap::new();

    while let Ok(res) = rx.recv() {
        let Ok(event) = res else { continue };
        if !matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        ) {
            continue;
        }
        for (filename, event_name) in WATCHED {
            let touches = event
                .paths
                .iter()
                .any(|p| p.file_name().and_then(|n| n.to_str()) == Some(*filename));
            if !touches {
                continue;
            }
            // Per-file debounce: rapid bursts on a single save collapse into one.
            let now = Instant::now();
            let too_soon = last_fired
                .get(event_name)
                .map(|t| now.duration_since(*t) < DEBOUNCE)
                .unwrap_or(false);
            if too_soon {
                continue;
            }
            last_fired.insert(*event_name, now);
            if let Err(e) = app.emit(*event_name, ()) {
                log::warn!("emit {event_name} failed: {e}");
            }
        }
    }

    Ok(())
}
