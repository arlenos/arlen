/// Configuration loading and hot-reload.

pub mod types;
pub mod watcher;

pub use types::{
    AppOverride, Config, DndConfig, DndMode, DndSchedule, GroupingConfig, HistoryConfig,
    ScheduleMode,
};

use std::path::{Path, PathBuf};

/// Default config file path.
pub fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("arlen/notifications.toml")
}

/// The theme-emitted per-event sound-name map (`sdk/theme` writes it on apply,
/// `sound-system-plan.md` SO-R1). The daemon merges it under the user's own
/// `notifications.toml` overrides.
pub fn theme_sounds_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("arlen/sounds.toml")
}

/// Load config from a TOML file, then fold in the active theme's `[sounds]` map
/// (the theme is the lower-precedence source: a name the user set in
/// `notifications.toml` wins). Returns defaults if the file is missing.
pub fn load_config(path: &Path) -> Config {
    let mut config = match std::fs::read_to_string(path) {
        Ok(contents) => match toml::from_str::<Config>(&contents) {
            Ok(c) => {
                tracing::info!("loaded config from {}", path.display());
                c
            }
            Err(e) => {
                tracing::warn!("failed to parse {}: {e}, using defaults", path.display());
                Config::default()
            }
        },
        Err(_) => {
            tracing::info!("no config at {}, using defaults", path.display());
            Config::default()
        }
    };
    merge_theme_sounds(&mut config, &theme_sounds_path());
    config
}

/// The theme's per-event sound names, as written by `sdk/theme`'s emitter (keyed
/// by the Arlen event, not the freedesktop default name the daemon keys overrides
/// by, so the merge translates).
#[derive(serde::Deserialize, Default)]
struct ThemeSoundsFile {
    #[serde(default)]
    sounds: ThemeSoundsTable,
}

/// The four theme-overridable Arlen events.
#[derive(serde::Deserialize, Default)]
struct ThemeSoundsTable {
    notification: Option<String>,
    error: Option<String>,
    warning: Option<String>,
    action: Option<String>,
}

/// Fold the theme's `[sounds]` map (if present) into the daemon's per-event sound
/// overrides. The override map is keyed by each event's freedesktop default name
/// (so `cue_name_for_event` finds it), so each theme entry is translated through
/// [`SoundEvent::sound_name`]. The user's `notifications.toml` overrides take
/// precedence: an already-present key is left untouched. A missing or unparseable
/// file is a no-op (the daemon keeps the standard names).
fn merge_theme_sounds(config: &mut Config, theme_path: &Path) {
    use crate::sound::SoundEvent;
    let Ok(text) = std::fs::read_to_string(theme_path) else {
        return;
    };
    let parsed: ThemeSoundsFile = match toml::from_str(&text) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("ignoring unparseable {}: {e}", theme_path.display());
            return;
        }
    };
    let t = parsed.sounds;
    for (value, event) in [
        (t.notification, SoundEvent::NotificationArrived),
        (t.error, SoundEvent::Error),
        (t.warning, SoundEvent::Warning),
        (t.action, SoundEvent::ActionCompleted),
    ] {
        if let Some(name) = value {
            // The user's own override (already in the map) wins.
            config
                .sound
                .overrides
                .entry(event.sound_name().to_string())
                .or_insert(name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_load_missing_file() {
        let c = load_config(Path::new("/nonexistent/path.toml"));
        assert_eq!(c.dnd.mode, DndMode::Off);
    }

    #[test]
    fn test_load_valid_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "[dnd]\nmode = \"priority\"\n").unwrap();

        let c = load_config(&path);
        assert_eq!(c.dnd.mode, DndMode::Priority);
    }

    #[test]
    fn test_load_legacy_mode_on() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("legacy.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "[dnd]\nmode = \"on\"\n").unwrap();
        let c = load_config(&path);
        assert_eq!(c.dnd.mode, DndMode::Priority);
    }

    #[test]
    fn test_load_invalid_toml() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "{{{{invalid").unwrap();

        let c = load_config(&path);
        assert_eq!(c.dnd.mode, DndMode::Off); // defaults
    }

    #[test]
    fn theme_sounds_fold_into_the_override_map_keyed_by_default_name() {
        let dir = tempfile::TempDir::new().unwrap();
        let theme = dir.path().join("sounds.toml");
        std::fs::write(&theme, "[sounds]\nnotification = \"bell\"\nerror = \"dialog-error\"\n").unwrap();
        let mut c = Config::default();
        merge_theme_sounds(&mut c, &theme);
        // Keyed by the freedesktop default name the daemon looks overrides up by.
        assert_eq!(c.sound.overrides.get("message-new-instant").map(String::as_str), Some("bell"));
        assert_eq!(c.sound.overrides.get("dialog-error").map(String::as_str), Some("dialog-error"));
    }

    #[test]
    fn a_user_override_wins_over_the_theme() {
        let dir = tempfile::TempDir::new().unwrap();
        let theme = dir.path().join("sounds.toml");
        std::fs::write(&theme, "[sounds]\nnotification = \"bell\"\n").unwrap();
        let mut c = Config::default();
        c.sound.overrides.insert("message-new-instant".to_string(), "user-choice".to_string());
        merge_theme_sounds(&mut c, &theme);
        assert_eq!(
            c.sound.overrides.get("message-new-instant").map(String::as_str),
            Some("user-choice"),
            "the user's notifications.toml override is not displaced by the theme"
        );
    }

    #[test]
    fn a_missing_theme_sounds_file_is_a_no_op() {
        let mut c = Config::default();
        merge_theme_sounds(&mut c, Path::new("/nonexistent/sounds.toml"));
        assert!(c.sound.overrides.is_empty());
    }
}
