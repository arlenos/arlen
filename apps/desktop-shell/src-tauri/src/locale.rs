//! The UI language, for the shell itself.
//!
//! Every other app gets this from `tauri-plugin-arlen-shell`; the shell embeds no
//! plugin, being the shell, so it reads the same file through the same reader in
//! `arlen_i18n::chosen`. Settings writes the choice, everyone reads it - the shell
//! is the theme authority but it is only a reader of the language.

use arlen_i18n::{chosen_locale, locale_config_path};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Runtime};

/// The current UI language tag.
#[tauri::command]
pub fn locale_get() -> String {
    chosen_locale()
}

/// Watch the choice and re-emit `arlen://locale-changed`, so a switch reaches a
/// shell that is already running.
pub fn spawn_locale_watcher<R: Runtime>(app: AppHandle<R>) {
    let target = locale_config_path();
    let Some(watch_dir) = target.parent().map(|p| p.to_path_buf()) else {
        log::warn!("locale: config path has no parent dir");
        return;
    };
    let _ = std::fs::create_dir_all(&watch_dir);

    std::thread::spawn(move || {
        let app_clone = app.clone();
        let last_fire = std::sync::Mutex::new(Instant::now() - Duration::from_secs(1));

        let mut watcher = match notify::recommended_watcher(move |event: Result<Event, _>| {
            let Ok(event) = event else { return };
            if !matches!(
                event.kind,
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
            ) {
                return;
            }
            if !event
                .paths
                .iter()
                .any(|p| p.file_name().map(|n| n == "locale.toml").unwrap_or(false))
            {
                return;
            }
            {
                let mut lf = last_fire.lock().unwrap();
                if lf.elapsed() < Duration::from_millis(100) {
                    return;
                }
                *lf = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(30));
            if let Err(e) = app_clone.emit("arlen://locale-changed", &chosen_locale()) {
                log::warn!("locale: emit failed: {e}");
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                log::warn!("locale: failed to create watcher: {e}");
                return;
            }
        };

        if let Err(e) = watcher.watch(&watch_dir, RecursiveMode::NonRecursive) {
            log::warn!("locale: failed to watch {}: {e}", watch_dir.display());
            return;
        }
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}
