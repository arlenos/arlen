/// Shell state persistence via `~/.config/lunaris/shell.toml`.
///
/// Stores Quick Settings state (night light, brightness, layout mode) so they
/// survive across reboots.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level shell configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShellConfig {
    /// Night light (color temperature) settings.
    #[serde(default)]
    pub night_light: NightLightConfig,
    /// Display settings (brightness).
    #[serde(default)]
    pub display: DisplayConfig,
    /// Window layout mode.
    #[serde(default)]
    pub layout: LayoutConfig,
}

/// Night light configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NightLightConfig {
    /// Whether night light is currently active.
    #[serde(default)]
    pub enabled: bool,
    /// Color temperature in Kelvin (lower = warmer).
    #[serde(default = "default_temperature")]
    pub temperature: u16,
}

fn default_temperature() -> u16 {
    3400
}

impl Default for NightLightConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            temperature: default_temperature(),
        }
    }
}

/// Display configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayConfig {
    /// Brightness as a fraction 0.0 to 1.0.
    #[serde(default = "default_brightness")]
    pub brightness: f32,
}

fn default_brightness() -> f32 {
    1.0
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            brightness: default_brightness(),
        }
    }
}

/// Layout configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutConfig {
    /// Layout mode: "float" or "tile".
    #[serde(default = "default_layout_mode")]
    pub mode: String,
}

fn default_layout_mode() -> String {
    "float".to_string()
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            mode: default_layout_mode(),
        }
    }
}

/// Resolves the config file path, creating the parent directory if needed.
fn config_path() -> PathBuf {
    let config_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("lunaris");
    let _ = fs::create_dir_all(&config_dir);
    config_dir.join("shell.toml")
}

/// Reads the shell config from disk, returning defaults if the file is missing
/// or unparseable.
#[tauri::command]
pub fn get_shell_config() -> Result<ShellConfig, String> {
    let path = config_path();
    if !path.exists() {
        return Ok(ShellConfig::default());
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("read: {e}"))?;
    toml::from_str(&content).map_err(|e| format!("parse: {e}"))
}

/// Writes the shell config to disk.
#[tauri::command]
pub fn save_shell_config(config: ShellConfig) -> Result<(), String> {
    let path = config_path();
    let content = toml::to_string_pretty(&config).map_err(|e| format!("serialize: {e}"))?;
    fs::write(&path, content).map_err(|e| format!("write: {e}"))
}
