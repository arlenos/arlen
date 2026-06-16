//! Read-only theme consumer for non-shell Arlen apps (GAP-20).
//!
//! The desktop shell is the theme authority: it resolves `appearance.toml`
//! and broadcasts the resolved [`CssVariables`] to a per-user runtime file
//! (`$XDG_RUNTIME_DIR/arlen/theme.json`) on every theme change. This module
//! is the consumer every other Arlen app embeds via the plugin: a
//! [`theme_get`] command that reads the current broadcast (falling back to
//! the bundled default if the shell has not written yet) plus a watcher that
//! re-emits `arlen://theme-v2-changed` when the broadcast changes, so a theme
//! switch live-reskins the whole desktop, not just the shell.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use arlen_theme::css::CssVariables;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter, Runtime};

/// The runtime broadcast file the shell writes the resolved theme to. Same
/// precedence as the shell writer: `ARLEN_THEME_BROADCAST` override, else
/// `$XDG_RUNTIME_DIR/arlen/theme.json`, else `/run/arlen/theme.json`.
fn broadcast_path() -> PathBuf {
    os_sdk::runtime::socket_path("ARLEN_THEME_BROADCAST", "theme.json")
}

/// The bundled-default CSS variables, used until the shell has broadcast a
/// resolved theme (e.g. an app launched before the shell, or in a bare dev
/// session). Dark is the default variant.
fn fallback_css() -> CssVariables {
    match arlen_theme::ArlenTheme::from_bundled(arlen_theme::DARK_TOML) {
        Ok(theme) => arlen_theme::css::to_css_variables(&theme, None),
        Err(_) => CssVariables {
            variables: std::collections::BTreeMap::new(),
            font_scale: 1.0,
            variant: "dark".into(),
        },
    }
}

/// Read the current broadcast theme, or the bundled default if the broadcast
/// file is absent or unparseable.
pub fn current_css() -> CssVariables {
    let path = broadcast_path();
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(css) = serde_json::from_slice::<CssVariables>(&bytes) {
            return css;
        }
    }
    fallback_css()
}

/// Return the current resolved CSS variables for this app to inject. The
/// frontend `initArlenTheme()` kit primitive invokes this once on mount.
#[tauri::command]
pub fn theme_get() -> CssVariables {
    current_css()
}

/// Watch the broadcast file's directory and, on a `theme.json` change,
/// re-read it and emit `arlen://theme-v2-changed` so the app's frontend
/// re-injects the new variables without a restart. Mirrors the shell's own
/// appearance watcher (atomic writes → watch the parent dir, debounce the
/// rename burst).
pub fn spawn_theme_watcher<R: Runtime>(app: AppHandle<R>) {
    let target = broadcast_path();
    let watch_dir = match target.parent() {
        Some(p) => p.to_path_buf(),
        None => {
            log::warn!("theme consumer: broadcast path has no parent dir");
            return;
        }
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
            let touches_target = event.paths.iter().any(|p| {
                p.file_name()
                    .map(|n| n == "theme.json")
                    .unwrap_or(false)
            });
            if !touches_target {
                return;
            }

            // Debounce: collapse the atomic-rename burst.
            {
                let mut lf = last_fire.lock().unwrap();
                if lf.elapsed() < Duration::from_millis(100) {
                    return;
                }
                *lf = Instant::now();
            }
            // Let the rename settle before reading.
            std::thread::sleep(Duration::from_millis(30));

            let css = current_css();
            if let Err(e) = app_clone.emit("arlen://theme-v2-changed", &css) {
                log::warn!("theme consumer: emit failed: {e}");
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                log::warn!("theme consumer: failed to create watcher: {e}");
                return;
            }
        };

        if let Err(e) = watcher.watch(&watch_dir, RecursiveMode::NonRecursive) {
            log::warn!("theme consumer: failed to watch {}: {e}", watch_dir.display());
            return;
        }

        // Keep the watcher alive for the life of the app.
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}
