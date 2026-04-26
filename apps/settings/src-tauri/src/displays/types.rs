/// Frontend-facing display types.
///
/// Mirrors the structure of the wlr-output-management head/mode
/// data, but in a JSON-friendly camelCase form that the Tauri
/// frontend can consume without knowing about Wayland.

use serde::{Deserialize, Serialize};

/// One connected output (= one physical monitor or virtual display).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Monitor {
    /// Connector name as reported by DRM (`eDP-1`, `DP-1`, `HDMI-A-2`).
    pub connector: String,
    /// EDID make / model / serial (best-effort, may be empty for
    /// virtual outputs).
    pub make: String,
    pub model: String,
    pub serial: String,
    /// `width × height` in millimetres, 0 if unknown.
    pub physical_size_mm: (i32, i32),
    /// Available modes for this output.
    pub modes: Vec<MonitorMode>,
    /// Index of the currently-active mode in `modes`. `None` if
    /// the output is disabled or the mode is unknown.
    pub current_mode: Option<usize>,
    /// Index of the preferred mode in `modes`, if the compositor
    /// reported one.
    pub preferred_mode: Option<usize>,
    /// Logical position in the global compositor coordinate space.
    pub position: Position,
    /// HiDPI scale factor (1.0, 1.5, 2.0, …).
    pub scale: f64,
    /// Output rotation / mirroring.
    pub transform: Transform,
    /// Whether the output is currently driven (vs. disabled).
    pub enabled: bool,
    /// Whether the output is mirroring another connector. The value
    /// is the target connector. Mutually exclusive with `enabled`'s
    /// "true active output" reading on the wlr side; on the Lunaris
    /// `OutputConfig` side this is encoded as
    /// `OutputState::Mirroring`.
    pub mirroring: Option<String>,
    /// VRR / adaptive sync state.
    pub vrr: VrrState,
    /// Whether this is the xwayland-primary output.
    pub primary: bool,
    /// Max bits per channel, 0 if not set.
    pub max_bpc: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorMode {
    pub width: i32,
    pub height: i32,
    /// Refresh rate in milli-Hertz (60_000 = 60 Hz).
    pub refresh_mhz: u32,
    pub preferred: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Transform {
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VrrState {
    Enabled,
    Disabled,
    Force,
}

/// Configuration the frontend wants to apply for a single monitor.
/// Mirrors the fields the user can control in the side panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorConfig {
    pub connector: String,
    /// Index into the monitor's `modes` array. Only valid when
    /// `enabled` is `Active`.
    pub mode_index: Option<usize>,
    pub position: Position,
    pub scale: f64,
    pub transform: Transform,
    pub enabled: EnabledKind,
    pub vrr: VrrState,
    pub primary: bool,
    /// 0 means "auto / default".
    pub max_bpc: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "type", content = "target")]
pub enum EnabledKind {
    Active,
    Disabled,
    Mirror(String),
}

impl Default for Position {
    fn default() -> Self {
        Self { x: 0, y: 0 }
    }
}

impl Transform {
    /// Convert from the wlr-output wire transform value to the
    /// Lunaris enum. `None` for unrecognised numbers (forward-compat
    /// for protocols adding new transforms).
    pub fn from_wlr(t: u32) -> Option<Self> {
        Some(match t {
            0 => Transform::Normal,
            1 => Transform::Rotate90,
            2 => Transform::Rotate180,
            3 => Transform::Rotate270,
            4 => Transform::Flipped,
            5 => Transform::Flipped90,
            6 => Transform::Flipped180,
            7 => Transform::Flipped270,
            _ => return None,
        })
    }

    pub fn to_wlr(self) -> u32 {
        match self {
            Transform::Normal => 0,
            Transform::Rotate90 => 1,
            Transform::Rotate180 => 2,
            Transform::Rotate270 => 3,
            Transform::Flipped => 4,
            Transform::Flipped90 => 5,
            Transform::Flipped180 => 6,
            Transform::Flipped270 => 7,
        }
    }
}

impl VrrState {
    /// `zwlr_output_head_v1::AdaptiveSyncState` is enabled/disabled.
    /// We keep our own `Force` variant for parity with the
    /// `cosmic-comp-config::AdaptiveSync::Force` value, which the
    /// compositor honours by always running the output at the
    /// highest variable refresh rate.
    pub fn from_wlr_bool(adaptive: bool) -> Self {
        if adaptive {
            VrrState::Enabled
        } else {
            VrrState::Disabled
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_round_trip() {
        for raw in 0u32..8 {
            let t = Transform::from_wlr(raw).unwrap();
            assert_eq!(t.to_wlr(), raw);
        }
    }

    #[test]
    fn transform_unknown_returns_none() {
        assert!(Transform::from_wlr(99).is_none());
    }

    #[test]
    fn enabled_kind_serialises_with_tag() {
        let ek = EnabledKind::Mirror("DP-1".into());
        let json = serde_json::to_string(&ek).unwrap();
        assert!(json.contains("\"type\":\"mirror\""));
        assert!(json.contains("\"target\":\"DP-1\""));
    }
}
