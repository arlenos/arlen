/// Tauri commands for the theme system.
///
/// Provides `ThemeState` (managed Tauri state) and commands for reading,
/// switching, and customizing themes. All commands that change the active
/// appearance emit a `lunaris://theme-v2-changed` event with the resolved
/// `CssVariables` so the frontend can update in real time.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use super::css::{to_css_string, to_css_variables, CssVariables};
use super::loader::{resolve_theme, ThemeError, ThemeLoader};
use super::schema::{AppearanceConfig, ThemeInfo};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Managed Tauri state for the theme system.
pub struct ThemeState {
    loader: ThemeLoader,
    config: Mutex<AppearanceConfig>,
    config_path: PathBuf,
}

impl ThemeState {
    /// Initialize the theme state from the given config and data directories.
    ///
    /// Reads `config_dir/appearance.toml` if it exists, otherwise uses
    /// defaults. The `data_dir/themes/` directory is scanned for user themes.
    pub fn new(config_dir: PathBuf, data_dir: PathBuf) -> Result<Self, ThemeError> {
        let user_themes_dir = data_dir.join("themes");
        let _ = std::fs::create_dir_all(&user_themes_dir);

        let loader = ThemeLoader::new_with_user_dir(user_themes_dir)?;

        let config_path = config_dir.join("appearance.toml");
        let config = if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            toml::from_str(&content).map_err(|e| ThemeError::Parse {
                path: config_path.display().to_string(),
                source: e,
            })?
        } else {
            AppearanceConfig::default()
        };

        Ok(Self {
            loader,
            config: Mutex::new(config),
            config_path,
        })
    }

    /// Persist the current config to disk.
    fn save_config(&self, config: &AppearanceConfig) -> Result<(), ThemeError> {
        if let Some(parent) = self.config_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = toml::to_string_pretty(config)
            .map_err(|e| ThemeError::Serialize(e.to_string()))?;
        std::fs::write(&self.config_path, content)?;
        Ok(())
    }

    /// Resolve the current theme into CSS variables.
    fn resolve(&self) -> Result<CssVariables, ThemeError> {
        let config = self.config.lock().unwrap();
        let tokens = resolve_theme(&self.loader, &config)?;
        Ok(to_css_variables(&tokens, &config.overrides))
    }

    /// Resolve and emit the theme-changed event.
    fn resolve_and_emit(&self, app: &AppHandle) -> Result<CssVariables, ThemeError> {
        let css = self.resolve()?;
        let _ = app.emit("lunaris://theme-v2-changed", &css);
        Ok(css)
    }

    /// Re-read `appearance.toml` from disk, replacing the in-memory config.
    /// Silent no-op if the file was removed (keeps current config).
    pub fn reload_from_disk(&self) -> Result<(), ThemeError> {
        if !self.config_path.exists() {
            return Ok(());
        }
        let content = std::fs::read_to_string(&self.config_path)?;
        let new_config: AppearanceConfig = toml::from_str(&content).map_err(|e| {
            ThemeError::Parse {
                path: self.config_path.display().to_string(),
                source: e,
            }
        })?;
        *self.config.lock().unwrap() = new_config;
        Ok(())
    }

    /// Path watched by `start_appearance_watcher`.
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }
}

// ---------------------------------------------------------------------------
// Live-reload watcher
// ---------------------------------------------------------------------------

/// Watch `~/.config/lunaris/appearance.toml` for external writes (e.g. from
/// the Settings app) and re-emit `theme-v2-changed` so the shell UI picks
/// up the new accent / radius / fonts without needing a restart.
///
/// Editors typically write atomically (tmp + rename), so we watch the
/// parent directory and filter on the target filename. A short debounce
/// collapses the rename/create/modify burst into one reload.
pub fn start_appearance_watcher(app: AppHandle) {
    let state = app.state::<ThemeState>();
    let target = state.config_path().to_path_buf();
    let watch_dir = match target.parent() {
        Some(p) => p.to_path_buf(),
        None => {
            log::warn!("theme: appearance.toml has no parent dir");
            return;
        }
    };
    let _ = std::fs::create_dir_all(&watch_dir);

    std::thread::spawn(move || {
        let app_clone = app.clone();
        let target_clone = target.clone();
        let last_fire = Mutex::new(Instant::now() - Duration::from_secs(1));

        let mut watcher = match notify::recommended_watcher(
            move |event: Result<Event, _>| {
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
                            .map(|n| n == "appearance.toml")
                            .unwrap_or(false)
                });
                if !touches_target {
                    return;
                }

                // Debounce: collapse bursts from atomic renames.
                {
                    let mut lf = last_fire.lock().unwrap();
                    if lf.elapsed() < Duration::from_millis(100) {
                        return;
                    }
                    *lf = Instant::now();
                }

                // Small sleep to let the rename settle before we read.
                std::thread::sleep(Duration::from_millis(30));

                let state = app_clone.state::<ThemeState>();
                if let Err(e) = state.reload_from_disk() {
                    log::warn!("theme: reload_from_disk failed: {e}");
                    return;
                }
                if let Err(e) = state.resolve_and_emit(&app_clone) {
                    log::warn!("theme: resolve_and_emit failed: {e}");
                }
            },
        ) {
            Ok(w) => w,
            Err(e) => {
                log::warn!("theme: failed to create appearance watcher: {e}");
                return;
            }
        };

        if let Err(e) = watcher.watch(&watch_dir, RecursiveMode::NonRecursive) {
            log::warn!("theme: failed to watch {}: {e}", watch_dir.display());
            return;
        }

        // Keep the watcher alive.
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}

// ---------------------------------------------------------------------------
// Error wrapper (Tauri needs Serialize)
// ---------------------------------------------------------------------------

/// Serializable error wrapper for Tauri command returns.
#[derive(Debug, Serialize)]
pub struct ThemeCommandError {
    message: String,
}

impl From<ThemeError> for ThemeCommandError {
    fn from(e: ThemeError) -> Self {
        Self {
            message: e.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Get the resolved CSS variables for the current theme + overrides.
#[tauri::command]
pub fn get_theme(state: tauri::State<'_, ThemeState>) -> Result<CssVariables, ThemeCommandError> {
    Ok(state.resolve()?)
}

/// Get the resolved CSS as an injectable string.
#[tauri::command]
pub fn get_theme_css(state: tauri::State<'_, ThemeState>) -> Result<String, ThemeCommandError> {
    let css = state.resolve()?;
    Ok(to_css_string(&css))
}

/// Switch to a different theme by ID.
#[tauri::command]
pub fn set_theme(
    id: String,
    state: tauri::State<'_, ThemeState>,
    app: AppHandle,
) -> Result<CssVariables, ThemeCommandError> {
    // Verify the theme exists before switching.
    state.loader.load(&id)?;

    let mut config = state.config.lock().unwrap();
    config.theme.active = id;
    state.save_config(&config)?;
    drop(config);

    Ok(state.resolve_and_emit(&app)?)
}

/// List all available themes (built-in + user).
#[tauri::command]
pub fn get_available_themes(state: tauri::State<'_, ThemeState>) -> Vec<ThemeInfo> {
    state.loader.list_themes()
}

/// Get the currently active theme ID.
#[tauri::command]
pub fn get_active_theme_id(
    state: tauri::State<'_, ThemeState>,
) -> Result<String, ThemeCommandError> {
    let config = state.config.lock().unwrap();
    Ok(config.theme.active.clone())
}

/// Set a custom accent color override.
#[tauri::command]
pub fn set_accent_color(
    color: String,
    state: tauri::State<'_, ThemeState>,
    app: AppHandle,
) -> Result<CssVariables, ThemeCommandError> {
    let mut config = state.config.lock().unwrap();
    config.overrides.accent = if color.is_empty() {
        None
    } else {
        Some(color)
    };
    state.save_config(&config)?;
    drop(config);

    Ok(state.resolve_and_emit(&app)?)
}

/// Set the font scale multiplier (clamped to 0.5 - 2.0).
#[tauri::command]
pub fn set_font_scale(
    scale: f32,
    state: tauri::State<'_, ThemeState>,
    app: AppHandle,
) -> Result<CssVariables, ThemeCommandError> {
    let clamped = scale.clamp(0.5, 2.0);

    let mut config = state.config.lock().unwrap();
    config.overrides.font_scale = if (clamped - 1.0).abs() < 0.001 {
        None
    } else {
        Some(clamped)
    };
    state.save_config(&config)?;
    drop(config);

    Ok(state.resolve_and_emit(&app)?)
}

/// Toggle reduce-motion accessibility setting.
#[tauri::command]
pub fn set_reduce_motion(
    enabled: bool,
    state: tauri::State<'_, ThemeState>,
    app: AppHandle,
) -> Result<CssVariables, ThemeCommandError> {
    let mut config = state.config.lock().unwrap();
    config.accessibility.reduce_motion = enabled;
    state.save_config(&config)?;
    drop(config);

    Ok(state.resolve_and_emit(&app)?)
}

/// Get the full appearance config.
#[tauri::command]
pub fn get_appearance_config(
    state: tauri::State<'_, ThemeState>,
) -> Result<AppearanceConfig, ThemeCommandError> {
    let config = state.config.lock().unwrap();
    Ok(config.clone())
}

/// Reset all theme settings to defaults.
#[tauri::command]
pub fn reset_theme(
    state: tauri::State<'_, ThemeState>,
    app: AppHandle,
) -> Result<CssVariables, ThemeCommandError> {
    let default_config = AppearanceConfig::default();
    let mut config = state.config.lock().unwrap();
    *config = default_config;
    state.save_config(&config)?;
    drop(config);

    Ok(state.resolve_and_emit(&app)?)
}
