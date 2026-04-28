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
    /// Toast renderer settings (visual only — the daemon rules live in
    /// `notifications.toml`). Settings app writes here to control where
    /// and how the shell draws toasts; WHEN to show them is the daemon's
    /// job.
    #[serde(default)]
    pub toast: ToastConfig,
}

/// Toast renderer configuration (consumed by the frontend Toaster).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToastConfig {
    /// Anchor for the toast stack. One of:
    /// `"top-right" | "top-left" | "top-center" | "bottom-right" | "bottom-left"`.
    #[serde(default = "default_toast_position")]
    pub position: String,
    /// Toast width in pixels.
    #[serde(default = "default_toast_width")]
    pub width: u32,
    /// Animation flavour for entry/exit. One of `"slide" | "fade" | "none"`.
    #[serde(default = "default_toast_animation")]
    pub animation: String,
}

fn default_toast_position() -> String {
    "top-right".into()
}
fn default_toast_width() -> u32 {
    380
}
fn default_toast_animation() -> String {
    "slide".into()
}

impl Default for ToastConfig {
    fn default() -> Self {
        Self {
            position: default_toast_position(),
            width: default_toast_width(),
            animation: default_toast_animation(),
        }
    }
}

/// Night light configuration. The schedule + location fields are
/// optional with sensible defaults so existing shell.toml files
/// stay readable after the D2 night-light backend lands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NightLightConfig {
    /// User's intent for the manual toggle. The compositor's actual
    /// "is the warm tint on right now" state is derived from this
    /// + the schedule, so we never have to store a transient.
    #[serde(default)]
    pub enabled: bool,
    /// Color temperature in Kelvin (lower = warmer). Compositor
    /// clamps to its supported warm-tint range (1000–6500 K).
    #[serde(default = "default_temperature")]
    pub temperature: u16,
    /// When the warm tint should activate.
    #[serde(default)]
    pub schedule: NightLightSchedule,
    /// Custom-mode start, minutes since midnight (default 22:00).
    #[serde(default = "default_custom_start")]
    pub custom_start: u32,
    /// Custom-mode end, minutes since midnight (default 07:00).
    #[serde(default = "default_custom_end")]
    pub custom_end: u32,
    /// Latitude for sunset/sunrise mode. `0.0` means unset; in that
    /// case sunset mode falls back to the manual flag.
    #[serde(default)]
    pub latitude: f64,
    /// Longitude for sunset/sunrise mode.
    #[serde(default)]
    pub longitude: f64,
}

/// Schedule mode mirrored from the compositor's
/// `lunaris-shell-overlay::night_light_schedule` enum. Kept as a
/// string in TOML so the file is readable; the dispatcher converts
/// to the protocol uint at the boundary.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NightLightSchedule {
    Manual,
    SunsetSunrise,
    Custom,
}

impl Default for NightLightSchedule {
    fn default() -> Self {
        NightLightSchedule::Manual
    }
}

impl NightLightSchedule {
    /// Encode to the wire-protocol uint. Mirrors the
    /// `night_light_schedule` enum in `lunaris-shell-overlay.xml`.
    pub fn to_protocol(self) -> u32 {
        match self {
            NightLightSchedule::Manual => 0,
            NightLightSchedule::SunsetSunrise => 1,
            NightLightSchedule::Custom => 2,
        }
    }
}

fn default_temperature() -> u16 {
    3400
}

fn default_custom_start() -> u32 {
    22 * 60
}

fn default_custom_end() -> u32 {
    7 * 60
}

impl Default for NightLightConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            temperature: default_temperature(),
            schedule: NightLightSchedule::default(),
            custom_start: default_custom_start(),
            custom_end: default_custom_end(),
            latitude: 0.0,
            longitude: 0.0,
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

/// Watch `~/.config/lunaris/shell.toml` for external writes (e.g. from
/// the Settings app) and emit a `lunaris://shell-config-changed` Tauri
/// event so the frontend can re-read its config and re-render.
///
/// Same notify+debounce pattern as `theme::commands::start_appearance_watcher`.
pub fn start_shell_config_watcher(app: tauri::AppHandle) {
    use notify::{EventKind, RecursiveMode, Watcher};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use tauri::{Emitter, Manager};

    let target = config_path();
    let watch_dir = match target.parent() {
        Some(p) => p.to_path_buf(),
        None => return,
    };
    let _ = fs::create_dir_all(&watch_dir);

    std::thread::spawn(move || {
        let app_clone = app.clone();
        let target_clone = target.clone();
        let last_fire = Mutex::new(Instant::now() - Duration::from_secs(1));

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
                            .map(|n| n == "shell.toml")
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
                let _ = app_clone.emit("lunaris://shell-config-changed", ());
                // Cross-app night-light flow: app-settings writes
                // shell.toml's [night_light] section directly (it
                // can't reach desktop-shell's Tauri commands across
                // processes). The watcher relays the new state to
                // the compositor here so the gamma engine reflects
                // the change without requiring desktop-shell IPC.
                if let Some(sender) = app_clone
                    .try_state::<std::sync::Arc<crate::shell_overlay_client::ShellOverlaySender>>()
                {
                    crate::night_light::replay_persisted_state(std::sync::Arc::clone(&sender));
                }
            },
        ) {
            Ok(w) => w,
            Err(e) => {
                log::warn!("shell_config: watcher init failed: {e}");
                return;
            }
        };

        if let Err(e) = watcher.watch(&watch_dir, RecursiveMode::NonRecursive) {
            log::warn!("shell_config: watch failed: {e}");
            return;
        }
        // Hold the watcher alive for the process lifetime.
        std::mem::forget(watcher);
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}
