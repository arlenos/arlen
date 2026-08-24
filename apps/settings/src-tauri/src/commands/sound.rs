//! The notification sounds page: read and write the daemon's `[sound]` table.
//!
//! THROUGH THE SAME FILE THE DAEMON READS, `~/.config/arlen/notifications.toml`,
//! and through the same format-preserving writer every other Settings page uses.
//! A second route to one config is how the two disagree about what is in it.
//!
//! The panel and the daemon name the mute the other way round on purpose: a page
//! offers "sounds are on", a daemon records "muted". The flip lives here, once, so
//! neither side has to think about the other's polarity.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::config::{config_get, config_set, ConfigFile};

/// The sound settings as the page renders them.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SoundSettings {
    /// Whether system sounds play at all. The daemon stores `muted`.
    pub enabled: bool,
    /// The freedesktop sound-theme name.
    pub theme: String,
    /// Master volume, `0.0..=1.0`.
    pub volume: f64,
    /// Per-event cue overrides, keyed by the daemon's event name. The value
    /// `disabled` silences that event; the page clears an entry by sending an
    /// empty string.
    pub overrides: BTreeMap<String, String>,
}

/// What the page may change. Every field optional: a patch says what it touched,
/// so a page that only moved the volume cannot overwrite a theme somebody set in
/// another window between the read and the write.
#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SoundPatch {
    pub enabled: Option<bool>,
    pub theme: Option<String>,
    pub volume: Option<f64>,
    pub overrides: Option<BTreeMap<String, String>>,
}

/// Read one key out of the notifications config, or `None` when it is not set.
fn get(key: &str) -> Option<serde_json::Value> {
    match config_get(ConfigFile::Notifications, Some(key.to_string())) {
        Ok(serde_json::Value::Null) => None,
        Ok(v) => Some(v),
        Err(_) => None,
    }
}

/// The current sound settings.
///
/// An unset field falls to the daemon's own default rather than to something
/// invented here: `sound.*` is absent from a fresh config, and the page must show
/// what the daemon would do, not what this file guesses.
#[tauri::command]
pub fn sound_settings() -> Result<SoundSettings, String> {
    let defaults = arlen_notification_daemon::config::types::SoundConfig::default();
    let muted = get("sound.muted")
        .and_then(|v| v.as_bool())
        .unwrap_or(defaults.muted);
    Ok(SoundSettings {
        enabled: !muted,
        theme: get("sound.theme")
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or(defaults.theme),
        volume: get("sound.volume")
            .and_then(|v| v.as_f64())
            .unwrap_or(f64::from(defaults.volume)),
        overrides: get("sound.overrides")
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default(),
    })
}

/// Change some of them.
///
/// One key per touched field, through the format-preserving writer, so the
/// comments and the rest of the file survive - and so a failure to write one field
/// is reported rather than leaving the caller to assume the whole patch landed.
#[tauri::command]
pub async fn sound_set(patch: SoundPatch) -> Result<(), String> {
    if let Some(enabled) = patch.enabled {
        // The polarity flip, in the one place that knows about both names.
        config_set(
            ConfigFile::Notifications,
            "sound.muted".into(),
            serde_json::Value::Bool(!enabled),
        )
        .await?;
    }
    if let Some(theme) = patch.theme {
        config_set(
            ConfigFile::Notifications,
            "sound.theme".into(),
            serde_json::Value::String(theme),
        )
        .await?;
    }
    if let Some(volume) = patch.volume {
        // Clamped here rather than trusted: the daemon's scale is 0..=1 and a
        // value outside it would be written to a file the daemon then reads.
        let clamped = volume.clamp(0.0, 1.0);
        config_set(
            ConfigFile::Notifications,
            "sound.volume".into(),
            serde_json::json!(clamped),
        )
        .await?;
    }
    if let Some(overrides) = patch.overrides {
        // An empty string is how the page clears one, so it never reaches the
        // file: an override to "" would silence nothing and name no cue.
        let kept: BTreeMap<String, String> = overrides
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .collect();
        config_set(
            ConfigFile::Notifications,
            "sound.overrides".into(),
            serde_json::to_value(kept).map_err(|e| e.to_string())?,
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// The mute value this page writes is the one the daemon acts on.
    ///
    /// Two halves in two crates: the page writes a string into `sound.overrides`,
    /// and the notification daemon's resolver decides what it means. They agreed
    /// only by both being spelled by hand, which is how the value came to be
    /// documented here for days while the resolver still treated it as a cue name
    /// nobody's theme ships, i.e. as a missing sound rather than a chosen silence.
    #[test]
    fn the_documented_mute_value_is_the_one_the_resolver_honours() {
        assert_eq!(
            arlen_notification_daemon::sound::DISABLED_OVERRIDE,
            "disabled"
        );
    }
}
