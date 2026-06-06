/// Quick-Settings tile status channels.
///
/// Tiles render an `active` indicator + a `status_text` subtitle. The
/// underlying truth lives in the indicator backends (`network`,
/// `bluetooth`, `audio`, `system_toggles`, `night_light`, …). Instead
/// of plumbing one bespoke event per tile, the QS panel listens on a
/// single namespaced Tauri event-bus channel and the backends publish
/// updates onto it as they happen.
///
/// Wire format: Tauri event named `arlen://qs/status/<channel>`
/// carrying `StatusUpdate { active, status_text, since_ms? }`. The
/// `<channel>` segment matches the manifest's
/// `quicksettings.tile.status_channel` field — system-tier tile names
/// use a `system.<id>` convention.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

/// Single update broadcast on a status channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusUpdate {
    /// `true` when the underlying state is "on" / "connected" /
    /// "active" — whatever the tile uses to colour itself accent.
    pub active: bool,
    /// Subtitle string; tiles wrap it under the label. Backends may
    /// pass an empty string when there is nothing useful to show
    /// (e.g. WiFi off → tile shows "Off" via its own copy table).
    #[serde(default)]
    pub status_text: String,
    /// Optional millisecond timestamp at which the active state
    /// began. Tiles use this to render live duration counters
    /// (recording badge, focus session). Zero / absent disables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_ms: Option<u64>,
}

impl StatusUpdate {
    /// Convenience for the common toggle case: on/off + label.
    pub fn toggle(active: bool, status_text: impl Into<String>) -> Self {
        Self {
            active,
            status_text: status_text.into(),
            since_ms: None,
        }
    }
}

/// Build the full event name for a channel.
pub fn event_name(channel: &str) -> String {
    format!("arlen://qs/status/{channel}")
}

/// Publish `update` on `channel`. All windows/webviews receive it
/// (Tauri broadcast semantics). Channel id should match the manifest's
/// `status_channel` field — e.g. `"system.network"`, `"system.bluetooth"`,
/// `"system.dnd"`.
pub fn publish(app: &AppHandle, channel: &str, update: StatusUpdate) {
    let _ = app.emit(&event_name(channel), update);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_name_uses_qs_namespace() {
        assert_eq!(event_name("system.network"), "arlen://qs/status/system.network");
    }

    #[test]
    fn toggle_constructor_zeroes_since_ms() {
        let u = StatusUpdate::toggle(true, "Connected");
        assert!(u.active);
        assert_eq!(u.status_text, "Connected");
        assert!(u.since_ms.is_none());
    }

    #[test]
    fn status_update_round_trips_via_serde() {
        let u = StatusUpdate {
            active: true,
            status_text: "Connected".into(),
            since_ms: Some(1_700_000_000_000),
        };
        let s = serde_json::to_string(&u).unwrap();
        let back: StatusUpdate = serde_json::from_str(&s).unwrap();
        assert_eq!(back.active, true);
        assert_eq!(back.status_text, "Connected");
        assert_eq!(back.since_ms, Some(1_700_000_000_000));
    }

    #[test]
    fn since_ms_omitted_when_none() {
        let u = StatusUpdate::toggle(false, "");
        let s = serde_json::to_string(&u).unwrap();
        assert!(!s.contains("since_ms"), "since_ms should be skipped when None: {s}");
    }
}
