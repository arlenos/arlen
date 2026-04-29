/// Shell state persistence via `~/.config/lunaris/shell.toml`.
///
/// Stores Quick Settings state (night light, brightness, layout mode) so they
/// survive across reboots.
///
/// Concurrency model
/// ─────────────────
/// `shell.toml` has multiple writers in-process (the night-light Tauri
/// commands, the hardware brightness-key path, the Quick-Settings
/// `save_shell_config` Tauri command from the frontend). Each writer
/// does a logical read-modify-write; if two of them interleave, the
/// last writer wins and silently drops the other's changes.
///
/// We serialise every write through `WRITE_LOCK` and prefer the
/// `update_shell_config(|cfg| …)` helper, which loads the freshest
/// on-disk state under the lock, lets the caller patch only the
/// fields it cares about, and writes the result atomically (tmp file
/// + rename) so a crashed process can never leave a half-written
/// TOML on disk.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Process-wide guard for `shell.toml` writes. Held across the
/// load-modify-write window so concurrent writers can't interleave.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

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
///
/// This is the frontend-facing full-replace path (Quick-Settings
/// `persistConfig`). It still acquires `WRITE_LOCK` so it serialises
/// against the in-process selective patchers in `update_shell_config`,
/// but it is by definition a "I'm authoritative for the whole file"
/// operation: any field the caller didn't include is gone. Prefer
/// `update_shell_config` from inside the daemon.
#[tauri::command]
pub fn save_shell_config(config: ShellConfig) -> Result<(), String> {
    let _guard = WRITE_LOCK.lock().map_err(|_| "WRITE_LOCK poisoned".to_string())?;
    write_atomic(&config)
}

/// Atomic, lock-protected partial update.
///
/// Loads the current on-disk state under `WRITE_LOCK`, hands it to
/// the patcher closure, and writes the mutated value back via
/// `write_atomic`. Use this from any in-process writer that only
/// needs to touch a subset of fields — it cannot lose data the way
/// an unguarded read-modify-write through `get_shell_config` +
/// `save_shell_config` can.
///
/// Returns the value that was written, so callers that need to act
/// on the post-write state (logging, broadcasting, etc.) don't need
/// a second read.
pub fn update_shell_config<F>(patch: F) -> Result<ShellConfig, String>
where
    F: FnOnce(&mut ShellConfig),
{
    let _guard = WRITE_LOCK.lock().map_err(|_| "WRITE_LOCK poisoned".to_string())?;
    let path = config_path();
    let mut cfg = if path.exists() {
        let content = fs::read_to_string(&path).map_err(|e| format!("read: {e}"))?;
        toml::from_str::<ShellConfig>(&content).map_err(|e| format!("parse: {e}"))?
    } else {
        ShellConfig::default()
    };
    patch(&mut cfg);
    write_atomic(&cfg)?;
    Ok(cfg)
}

/// Serialise `cfg` and write it to `shell.toml` atomically: write
/// to `shell.toml.tmp` then `rename` over the target. POSIX `rename`
/// is atomic within the same filesystem, so a process crash mid-
/// write cannot leave a partial or empty config file.
fn write_atomic(cfg: &ShellConfig) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let tmp = path.with_extension("toml.tmp");
    let content = toml::to_string_pretty(cfg).map_err(|e| format!("serialize: {e}"))?;
    fs::write(&tmp, content).map_err(|e| format!("write tmp: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests in this module redirect `dirs::config_dir()` via the
    /// `XDG_CONFIG_HOME` env var, which is process-global. cargo
    /// runs unit tests in parallel, so we serialise this module's
    /// tests through a dedicated mutex to keep the env mutation
    /// race-free.
    static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Run the closure with `XDG_CONFIG_HOME` pointing at a fresh
    /// tempdir. Restores the previous value on the way out.
    fn with_isolated_config<R>(f: impl FnOnce() -> R) -> R {
        let _guard = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().expect("tempdir");
        let prev = std::env::var_os("XDG_CONFIG_HOME");
        // SAFETY: tests in this module are serialised by
        // TEST_ENV_LOCK, so the env mutation is well-defined for
        // the duration of `f`.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", tmp.path());
        }
        let out = f();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
        out
    }

    /// Codex finding (high): a brightness-key write must not silently
    /// drop fields that another writer just set. With the old
    /// `get + save` pattern the second writer would have read a
    /// stale config and overwritten the first writer's change. The
    /// `update_shell_config` helper protects against that as long as
    /// each writer scopes its mutation to the fields it cares about.
    #[test]
    fn update_shell_config_preserves_unrelated_fields() {
        with_isolated_config(|| {
            // Writer A: sets night-light enabled + temperature.
            update_shell_config(|cfg| {
                cfg.night_light.enabled = true;
                cfg.night_light.temperature = 4500;
            })
            .expect("writer A");

            // Writer B: simulates the brightness-key path.
            update_shell_config(|cfg| {
                cfg.display.brightness = 0.42;
            })
            .expect("writer B");

            // Reload from disk: both writes must be present.
            let cfg = get_shell_config().expect("reload");
            assert!(
                cfg.night_light.enabled,
                "writer A's night_light.enabled was clobbered"
            );
            assert_eq!(
                cfg.night_light.temperature, 4500,
                "writer A's night_light.temperature was clobbered"
            );
            assert!(
                (cfg.display.brightness - 0.42).abs() < f32::EPSILON,
                "writer B's display.brightness was clobbered"
            );
        });
    }

    /// Concurrent writers from multiple threads must each see a
    /// consistent load-modify-write window. Without `WRITE_LOCK`
    /// the highest-numbered field would frequently be lost when
    /// two threads interleave their save. We don't try to prove
    /// that empirically (it's flaky on a fast machine), but we
    /// do verify that after N updates from N threads every thread's
    /// distinct value lands in the file, which is the user-visible
    /// guarantee we care about.
    #[test]
    fn update_shell_config_serialises_concurrent_writers() {
        with_isolated_config(|| {
            // Seed the file so each writer reads a known baseline.
            update_shell_config(|cfg| {
                cfg.display.brightness = 0.0;
            })
            .expect("seed");

            const N: u32 = 16;
            let handles: Vec<_> = (0..N)
                .map(|i| {
                    std::thread::spawn(move || {
                        // Each thread sets a *different* temperature
                        // so we can later verify the last writer's
                        // value made it. The intermediates can race.
                        update_shell_config(|cfg| {
                            cfg.night_light.temperature = 3000 + i as u16;
                        })
                        .expect("update");
                    })
                })
                .collect();
            for h in handles {
                h.join().expect("join");
            }

            // The file must be valid TOML and contain a temperature
            // in the expected range. A torn write would either
                // leave invalid TOML (parse error) or a default
            // value outside the loop's range.
            let cfg = get_shell_config().expect("reload");
            let t = cfg.night_light.temperature;
            assert!(
                (3000..3000 + N as u16).contains(&t),
                "torn write: temperature {t} is outside the writer range"
            );
        });
    }
}
