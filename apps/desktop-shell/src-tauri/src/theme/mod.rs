/// Theme system for Lunaris.
///
/// `schema` defines the full token hierarchy. `loader` handles resolution
/// from built-in TOML files, user overrides, and accessibility settings.
///
/// The legacy `SurfaceTokens` / `load_tokens` / `start_watcher` API remains
/// here for backward compatibility with the existing frontend. New code should
/// use `schema::ThemeTokens` and `loader::ThemeLoader`.

pub mod commands;
pub mod css;
pub mod loader;
pub mod schema;

// ---------------------------------------------------------------------------
// Legacy API (reads ~/.config/lunaris/theme.toml, emits surface tokens)
// ---------------------------------------------------------------------------

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceTokens {
    pub bg_shell: String,
    pub bg_app: String,
    pub bg_card: String,
    pub bg_overlay: String,
    pub bg_input: String,
    pub fg_shell: String,
    pub fg_app: String,
    pub accent: String,
    pub border: String,
    pub radius: String,
}

impl SurfaceTokens {
    pub fn panda() -> Self {
        Self {
            bg_shell:   "#09090b".into(),
            bg_app:     "#ffffff".into(),
            bg_card:    "#f5f5f7".into(),
            bg_overlay: "#00000080".into(),
            bg_input:   "#f0f0f0".into(),
            fg_shell:   "#fafafa".into(),
            fg_app:     "#09090b".into(),
            accent:     "#09090b".into(),
            border:     "#e2e2e8".into(),
            radius:     "0.5rem".into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ThemeFile {
    color: Option<ColorSection>,
    radius: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ColorSection {
    bg: Option<BgSection>,
    fg: Option<FgSection>,
    accent: Option<String>,
    border: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BgSection {
    shell: Option<String>,
    app: Option<String>,
    card: Option<String>,
    overlay: Option<String>,
    input: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FgSection {
    shell: Option<String>,
    app: Option<String>,
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/etc"))
        .join("lunaris")
        .join("theme.toml")
}

pub fn load_tokens() -> SurfaceTokens {
    let path = config_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return SurfaceTokens::panda(),
    };

    let file: ThemeFile = match toml::from_str(&contents) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("lunaris: failed to parse theme.toml: {e}, using Panda defaults");
            return SurfaceTokens::panda();
        }
    };

    let panda = SurfaceTokens::panda();
    let bg = file.color.as_ref().and_then(|c| c.bg.as_ref());
    let fg = file.color.as_ref().and_then(|c| c.fg.as_ref());

    SurfaceTokens {
        bg_shell:   bg.and_then(|b| b.shell.clone()).unwrap_or(panda.bg_shell),
        bg_app:     bg.and_then(|b| b.app.clone()).unwrap_or(panda.bg_app),
        bg_card:    bg.and_then(|b| b.card.clone()).unwrap_or(panda.bg_card),
        bg_overlay: bg.and_then(|b| b.overlay.clone()).unwrap_or(panda.bg_overlay),
        bg_input:   bg.and_then(|b| b.input.clone()).unwrap_or(panda.bg_input),
        fg_shell:   fg.and_then(|f| f.shell.clone()).unwrap_or(panda.fg_shell),
        fg_app:     fg.and_then(|f| f.app.clone()).unwrap_or(panda.fg_app),
        accent:     file.color.as_ref().and_then(|c| c.accent.clone()).unwrap_or(panda.accent),
        border:     file.color.as_ref().and_then(|c| c.border.clone()).unwrap_or(panda.border),
        radius:     file.radius.unwrap_or(panda.radius),
    }
}

#[tauri::command]
pub fn get_surface_tokens() -> SurfaceTokens {
    load_tokens()
}

pub fn start_watcher(app: AppHandle) {
    let theme_path = config_path();
    let watch_dir = theme_path
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    std::thread::spawn(move || {
        let app_clone = app.clone();
        let theme_path_clone = theme_path.clone();

        let mut watcher = match notify::recommended_watcher(
            move |event: Result<Event, _>| {
                if let Ok(event) = event {
                    match event.kind {
                        EventKind::Modify(_) | EventKind::Create(_) => {
                            if event.paths.iter().any(|p| p.file_name().map(|n| n == "theme.toml").unwrap_or(false)) || event.paths.iter().any(|p| p == &theme_path_clone) {
                                let tokens = load_tokens();
                                if let Err(e) = app_clone.emit("lunaris://theme-changed", &tokens) {
                                    eprintln!("lunaris: failed to emit theme-changed: {e}");
                                }
                            }
                        }
                        _ => {}
                    }
                }
            },
        ) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("lunaris: failed to create theme watcher: {e}");
                return;
            }
        };

        if let Err(e) = watcher.watch(&watch_dir, RecursiveMode::NonRecursive) {
            eprintln!("lunaris: failed to watch theme dir: {e}");
            return;
        }

        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panda_tokens_are_valid_hex() {
        let t = SurfaceTokens::panda();
        for color in [&t.bg_shell, &t.bg_app, &t.bg_card, &t.fg_shell, &t.fg_app, &t.accent, &t.border] {
            assert!(color.starts_with('#'), "expected hex: {color}");
        }
    }

    #[test]
    fn load_tokens_returns_panda_when_no_file() {
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/lunaris-test-nonexistent-99999");
        let tokens = load_tokens();
        assert_eq!(tokens.bg_shell, SurfaceTokens::panda().bg_shell);
        std::env::remove_var("XDG_CONFIG_HOME");
    }
}
