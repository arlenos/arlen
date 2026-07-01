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

/// The persistent theme selection: `appearance.toml [theme] active`, or the
/// bundled default `"dark"` when the file or key is absent (matching the shell's
/// schema default). The shell resolves the broadcast from this selection, so a
/// broadcast whose variant contradicts it is stale (a leftover from a prior
/// session, e.g. a `just dev` light run left in `$XDG_RUNTIME_DIR`).
fn selected_theme() -> String {
    os_sdk::config::Config::load("appearance")
        .ok()
        .and_then(|c| c.get::<String>("theme.active"))
        .unwrap_or_else(|| "dark".into())
}

/// The dark/light class a built-in selection must resolve to, or `None` for a
/// custom theme name the consumer cannot classify without the shell (the authority
/// for custom themes, which re-broadcasts on start).
fn expected_variant(selection: &str) -> Option<&'static str> {
    match selection {
        "dark" => Some("dark"),
        "light" => Some("light"),
        _ => None,
    }
}

/// Whether a broadcast with `broadcast_variant` is fresh for the current
/// `selection`. A built-in selection requires the broadcast's variant to match;
/// a custom selection is trusted (only the shell resolves it). The dark/light
/// mismatch (Tim's stale-broadcast bug) is exactly the false case here.
fn broadcast_is_fresh(broadcast_variant: &str, selection: &str) -> bool {
    match expected_variant(selection) {
        Some(expected) => broadcast_variant == expected,
        None => true,
    }
}

/// The bundled CSS variables for a selection: light if the selection is the
/// built-in light theme, else dark (the schema default and the safe fallback).
/// Used when no broadcast exists yet (an app launched before the shell, or a bare
/// dev session) or when the broadcast is stale.
fn bundled_css(selection: &str) -> CssVariables {
    let (toml, variant) = if selection == "light" {
        (arlen_theme::LIGHT_TOML, "light")
    } else {
        (arlen_theme::DARK_TOML, "dark")
    };
    match arlen_theme::ArlenTheme::from_bundled(toml) {
        Ok(theme) => arlen_theme::css::to_css_variables(&theme, None),
        Err(_) => CssVariables {
            variables: std::collections::BTreeMap::new(),
            font_scale: 1.0,
            variant: variant.into(),
        },
    }
}

/// Read the current broadcast theme, or the bundled default when the broadcast is
/// absent, unparseable, or stale.
///
/// A stale broadcast is one whose variant contradicts the persistent selection in
/// `appearance.toml` (e.g. a leftover `light` broadcast from an earlier session
/// while the selection is the default `dark`). The shell is the theme authority and
/// resolves the broadcast from the selection, so a mismatch means the broadcast
/// predates the current selection and must not override it; the consumer falls back
/// to the bundled theme for the selected variant instead. A broadcast for a custom
/// selected theme is trusted (only the shell can resolve custom themes, and it
/// re-broadcasts on start).
pub fn current_css() -> CssVariables {
    let selection = selected_theme();
    let path = broadcast_path();
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(css) = serde_json::from_slice::<CssVariables>(&bytes) {
            if broadcast_is_fresh(&css.variant, &selection) {
                return css;
            }
        }
    }
    bundled_css(&selection)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_light_broadcast_is_rejected_for_the_default_dark_selection() {
        // Tim's bug: a leftover `light` broadcast from a prior session must not
        // override the default (dark) selection.
        assert!(!broadcast_is_fresh("light", "dark"));
        // The matching cases are fresh.
        assert!(broadcast_is_fresh("dark", "dark"));
        assert!(broadcast_is_fresh("light", "light"));
        // The reverse stale case: a dark broadcast while light is selected.
        assert!(!broadcast_is_fresh("dark", "light"));
    }

    #[test]
    fn a_custom_selection_trusts_the_broadcast() {
        // The consumer cannot classify a custom theme; only the shell resolves it,
        // so any broadcast variant is trusted for a custom selection.
        assert!(broadcast_is_fresh("dark", "solarized"));
        assert!(broadcast_is_fresh("light", "solarized"));
        assert_eq!(expected_variant("solarized"), None);
    }

    #[test]
    fn bundled_fallback_tracks_the_selected_variant() {
        // No broadcast: the fallback respects the selected built-in variant, so a
        // light selection with no shell still comes up light (not forced dark).
        assert_eq!(bundled_css("light").variant, "light");
        assert_eq!(bundled_css("dark").variant, "dark");
        // A custom selection falls back to the safe dark default.
        assert_eq!(bundled_css("solarized").variant, "dark");
    }
}
