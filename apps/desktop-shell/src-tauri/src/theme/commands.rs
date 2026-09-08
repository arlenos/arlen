//! Tauri commands for the theme system.
//!
//! Provides `ThemeState` (managed Tauri state) and commands for reading,
//! switching, and customizing themes. All commands that change the active
//! appearance emit a `arlen://theme-v2-changed` event with the resolved
//! `CssVariables` so the frontend can update in real time.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use arlen_theme::ArlenTheme;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use arlen_desktop_shell_core::theme::css::{to_css_string, to_css_variables, CssVariables};
use arlen_desktop_shell_core::theme::loader::{resolve_theme, ThemeError, ThemeLoader};
use arlen_desktop_shell_core::theme::schema::{AppearanceConfig, ThemeInfo};

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

    /// Resolve the current theme into the resolved [`ArlenTheme`] plus its
    /// CSS variables (the in-app form). The theme drives the foreign-toolkit
    /// apply on a change; the variables drive the Svelte UI.
    fn resolve_full(&self) -> Result<(ArlenTheme, CssVariables), ThemeError> {
        let config = self.config.lock().unwrap();
        let theme = resolve_theme(&self.loader, &config)?;
        let css = to_css_variables(&theme, &config.overrides);
        Ok((theme, css))
    }

    /// Resolve the current theme into CSS variables.
    fn resolve(&self) -> Result<CssVariables, ThemeError> {
        Ok(self.resolve_full()?.1)
    }

    /// The XDG config root (`~/.config`), the parent of Arlen's own config
    /// dir (`~/.config/arlen`), under which the foreign-toolkit files
    /// (`gtk-3.0/gtk.css`, `qt6ct/`, `kitty/`, …) live.
    fn xdg_config_dir(&self) -> Option<PathBuf> {
        self.config_path
            .parent()
            .and_then(|arlen_dir| arlen_dir.parent())
            .map(Path::to_path_buf)
    }

    /// Resolve and emit the theme-changed event, then regenerate the
    /// foreign-toolkit (GTK/Qt/terminal) config files so a theme switch
    /// reskins non-Svelte apps too (best-effort: a write failure is logged,
    /// never fatal to the in-app update).
    fn resolve_and_emit(&self, app: &AppHandle) -> Result<CssVariables, ThemeError> {
        let (theme, css) = self.resolve_full()?;
        self.write_toolkit_files(&theme);
        // Broadcast the resolved variables to the per-user runtime file so
        // every other Arlen app's theme consumer live-reskins too, not just
        // the shell's own webviews (GAP-20). The shell is the theme authority;
        // this is its one-way broadcast. Best-effort: a write failure never
        // blocks the in-app update.
        write_theme_broadcast(&css);
        let _ = app.emit("arlen://theme-v2-changed", &css);
        Ok(css)
    }

    /// Write the foreign-toolkit (GTK/Qt/terminal) config files for a resolved
    /// theme, so a theme reaches non-Svelte apps too.
    ///
    /// Best-effort by design: a write failure is logged and never blocks the
    /// in-app update, and a file the user wrote themselves is kept rather than
    /// clobbered - `write_foreign_toolkit_configs` reports both.
    fn write_toolkit_files(&self, theme: &ArlenTheme) {
        let Some(config_dir) = self.xdg_config_dir() else {
            return;
        };
        let report = arlen_theme::apply::write_foreign_toolkit_configs(theme, &config_dir);
        for (path, err) in &report.errors {
            log::warn!("theme apply: failed to write {}: {err}", path.display());
        }
        for path in &report.skipped_foreign {
            log::info!(
                "theme apply: kept a non-Arlen file at {} (not overwritten)",
                path.display()
            );
        }
        if let Some(selection) = &report.selection {
            select_interface(selection);
        }
    }

    /// Reconcile the runtime theme broadcast to the current selection.
    ///
    /// The shell is the theme authority; on startup it rewrites the per-user
    /// broadcast file so a stale broadcast left in `$XDG_RUNTIME_DIR` by a prior
    /// session (or a bare dev run) cannot steer apps that read it before the shell
    /// has answered a theme change. Best-effort: a resolve or write failure is
    /// logged, never fatal to startup.
    pub fn broadcast_current(&self) {
        match self.resolve_full() {
            Ok((theme, css)) => {
                write_theme_broadcast(&css);
                // The same reconcile, for the toolkits. Until this was here the
                // GTK, Qt and terminal files were written ONLY on a theme
                // change, so a machine where nobody had touched the appearance
                // page had none of them - and a GTK3 app rendered stock Adwaita
                // on a system that ships its own theme, because the settings.ini
                // that selects it had never been written. Same argument as the
                // broadcast above: a stale file steers apps, and an absent one
                // steers them just as wrongly.
                self.write_toolkit_files(&theme);
            }
            Err(e) => log::warn!("theme: startup broadcast reconcile failed: {e}"),
        }
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

/// The GSettings schema GTK reads its interface choices from.
const INTERFACE_SCHEMA: &str = "org.gnome.desktop.interface";

/// Name the theme in `org.gnome.desktop.interface`, because the file does not.
///
/// **This is the step that makes the GTK3 widget theme reach a GTK3 app**, and it
/// was missing. Measured on 8 September: with our `gtk-3.0/settings.ini` in
/// place and correct, a GTK3 probe resolves `gtk-theme-name='Adwaita'`,
/// `gtk-font-name='Adwaita Sans 11'` - not one value from the file arrives.
/// GTK3 prefers this schema whenever it is installed, and with no value set it
/// uses the schema's own default, which silently beats the file. Forcing
/// `GTK_THEME=Arlen` gave the correct sheet, so the theme was always right;
/// nothing was selecting it. Setting `gtk-theme` here made the same probe answer
/// `Arlen` and paint our `#f5f5f7`.
///
/// The schemas are on our image (37 of them compiled, this one among them), so
/// this is not a developer-host quirk. `settings.ini` is still written: it is
/// what a system WITHOUT these schemas reads, and the two now render the same
/// `InterfaceSelection` rather than deciding separately.
///
/// Absent schema, absent key and a read-only backend are all normal outcomes
/// rather than faults - a machine that does not have this vocabulary is served
/// by the file - so each is logged and skipped rather than raised.
fn select_interface(selection: &arlen_theme::gtk::InterfaceSelection) {
    use gtk::gio;
    use gtk::prelude::SettingsExt;

    let Some(source) = gio::SettingsSchemaSource::default() else {
        log::info!("theme apply: no GSettings schema source, so settings.ini is the only reader");
        return;
    };
    let Some(schema) = source.lookup(INTERFACE_SCHEMA, true) else {
        log::info!("theme apply: {INTERFACE_SCHEMA} is not installed, so settings.ini is the only reader");
        return;
    };
    let settings = gio::Settings::new(INTERFACE_SCHEMA);

    // Only the keys this schema version actually has. `color-scheme` arrived in
    // gsettings-desktop-schemas 42 and `font-name` has been there forever, so
    // asking rather than assuming keeps an older system from a G_SETTINGS abort:
    // writing a key a schema does not declare is fatal in GIO, not an error.
    let mut set_string = |key: &str, value: &str| {
        if !schema.has_key(key) {
            log::info!("theme apply: {INTERFACE_SCHEMA} has no {key}, left alone");
            return;
        }
        if let Err(e) = settings.set_string(key, value) {
            log::warn!("theme apply: could not set {key}: {e}");
        }
    };

    if let Some(name) = &selection.gtk_theme {
        set_string("gtk-theme", name);
    }
    if let Some(icons) = &selection.icon_theme {
        set_string("icon-theme", icons);
    }
    set_string("cursor-theme", &selection.cursor_theme);
    set_string("font-name", &selection.font);
    // The schema's own vocabulary for the same bit `settings.ini` spells
    // `gtk-application-prefer-dark-theme`.
    set_string(
        "color-scheme",
        if selection.dark { "prefer-dark" } else { "prefer-light" },
    );
    if schema.has_key("cursor-size") {
        if let Err(e) = settings.set_int("cursor-size", selection.cursor_size as i32) {
            log::warn!("theme apply: could not set cursor-size: {e}");
        }
    }
    // Push the writes through the backend before returning: dconf batches them,
    // and the shell may be about to hand over to apps that read the schema.
    gio::Settings::sync();
}


/// The per-user runtime path the shell broadcasts the resolved theme to, so
/// every other Arlen app's theme consumer can read the live theme without
/// re-resolving `appearance.toml` (GAP-20). Precedence matches
/// `os_sdk::runtime` (the consumer side): the `ARLEN_THEME_BROADCAST`
/// override, else `$XDG_RUNTIME_DIR/arlen/theme.json`, else
/// `/run/arlen/theme.json`.
fn theme_broadcast_path() -> PathBuf {
    if let Ok(p) = std::env::var("ARLEN_THEME_BROADCAST") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("arlen").join("theme.json");
        }
    }
    PathBuf::from("/run/arlen/theme.json")
}

/// Write the resolved CSS variables to the runtime broadcast file (atomic
/// tmp + rename). Best-effort: any failure is logged and ignored, never
/// fatal to the in-app theme update.
fn write_theme_broadcast(css: &CssVariables) {
    let path = theme_broadcast_path();
    let json = match serde_json::to_vec(css) {
        Ok(j) => j,
        Err(e) => {
            log::warn!("theme broadcast: serialize failed: {e}");
            return;
        }
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Per INSTANCE, not per app (`app-instance-model.md`): two windows sharing a
    // temp name cross over rather than tear.
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    if let Err(e) = std::fs::write(&tmp, &json) {
        log::warn!("theme broadcast: write {} failed: {e}", tmp.display());
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        log::warn!("theme broadcast: rename to {} failed: {e}", path.display());
        let _ = std::fs::remove_file(&tmp);
    }
}

// ---------------------------------------------------------------------------
// Live-reload watcher
// ---------------------------------------------------------------------------

/// Watch `~/.config/arlen/appearance.toml` for external writes (e.g. from
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
                // `theme.toml` lives in this same directory, so its events arrive
                // here and are discarded by this filter. That is correct only
                // while nothing writes it: `ConfigFile::Customization` maps to it
                // but no Settings page selects that variant yet, and
                // `reload_from_disk` below reads `appearance.toml` alone. When the
                // Appearance suite starts writing the customization layer, THIS
                // FILTER AND `reload_from_disk` HAVE TO CHANGE WITH IT - otherwise
                // the layer is written and silently ignored, which looks like a
                // setting that does nothing rather than like a missing wire.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The key GTK3 actually reads gets our theme's name in it.
    ///
    /// `#[ignore]`d because it writes real GSettings, so it needs a private
    /// config home and its own session bus or it would edit the developer's
    /// desktop. Run it, because the bug this closes was invisible for exactly as
    /// long as nobody did:
    ///
    /// ```text
    /// XDG_CONFIG_HOME=$(mktemp -d) dbus-run-session -- \
    ///   cargo test -p arlen-desktop-shell --lib select_interface -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "writes GSettings; needs a private XDG_CONFIG_HOME and dbus-run-session"]
    fn select_interface_names_the_theme_in_the_schema_gtk_reads() {
        use gtk::gio;
        use gtk::prelude::SettingsExt;

        let selection = arlen_theme::gtk::InterfaceSelection {
            gtk_theme: Some("Arlen".into()),
            icon_theme: None,
            cursor_theme: "default".into(),
            cursor_size: 24,
            font: "Inter Variable 14px".into(),
            dark: true,
        };
        select_interface(&selection);

        let settings = gio::Settings::new(INTERFACE_SCHEMA);
        assert_eq!(settings.string("gtk-theme"), "Arlen");
        assert_eq!(settings.string("color-scheme"), "prefer-dark");
        assert_eq!(settings.string("font-name"), "Inter Variable 14px");
        assert_eq!(settings.int("cursor-size"), 24);
        // The icon theme was None, so the key keeps whatever it had: naming a set
        // this machine does not have would make it LESS iconned, which is the
        // same rule the settings file follows.
        assert_ne!(settings.string("icon-theme"), "");
    }

    /// The startup reconcile has to actually produce the files, and the one that
    /// matters most is the settings file: without it a GTK3 app never finds the
    /// theme the image ships, and the failure is invisible - the app just looks
    /// like stock Adwaita.
    ///
    /// Driven through `write_toolkit_files` rather than `broadcast_current`
    /// because the latter also writes the runtime broadcast, which lives in
    /// `$XDG_RUNTIME_DIR` and is the developer's own session; a test has no
    /// business writing there.
    #[test]
    fn the_toolkit_files_land_under_the_xdg_config_root() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let config_dir = tmp.path().join("arlen");
        std::fs::create_dir_all(&config_dir).expect("config dir");
        let state = ThemeState::new(config_dir, tmp.path().join("data")).expect("state");

        let theme = ArlenTheme::from_bundled(arlen_theme::DARK_TOML).expect("resolve");
        state.write_toolkit_files(&theme);

        // The sheet and the settings file, which are the two halves of the GTK3
        // spoke: colour and selection.
        let ini = tmp.path().join("gtk-3.0/settings.ini");
        assert!(ini.is_file(), "no settings.ini at {}", ini.display());
        let text = std::fs::read_to_string(&ini).expect("read");
        // NOT the icon theme: the bundled theme names `default`, which is a
        // cursor redirect rather than an icon theme, so `installed_icon_theme`
        // correctly leaves the key out on most machines. This asserted it until
        // that gate landed on 8 September, and the assertion was demanding the
        // bug back. The cursor and the font are the two that are always there.
        assert!(text.contains("gtk-cursor-theme-name="), "{text}");
        assert!(text.contains("gtk-font-name="), "{text}");
        assert!(tmp.path().join("gtk-3.0/gtk.css").is_file());
        assert!(tmp.path().join("gtk-4.0/gtk.css").is_file());
    }
}
