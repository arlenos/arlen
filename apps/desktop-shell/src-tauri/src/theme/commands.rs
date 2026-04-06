/// Tauri commands for the theme system.
///
/// Provides `ThemeState` (managed Tauri state) and commands for reading,
/// switching, and customizing themes. All commands that change the active
/// appearance emit a `lunaris://theme-v2-changed` event with the resolved
/// `CssVariables` so the frontend can update in real time.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

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
