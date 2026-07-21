//! Mouse + touchpad input-config types the settings input commands read/write.
//! The serde structs + their libinput-neutral defaults; pure of Tauri and
//! unit-tested in CI (the get/set commands + the compositor.toml read/write stay
//! in the host).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseConfig {
    pub acceleration: f64,
    pub natural_scroll: bool,
    pub left_handed: bool,
    /// Linear multiplier on wheel scroll deltas. 1.0 = libinput
    /// default; clamped to 0.1..3.0 on the compositor side.
    pub scroll_speed: f64,
}

impl Default for MouseConfig {
    fn default() -> Self {
        Self {
            acceleration: 0.0,
            natural_scroll: false,
            left_handed: false,
            scroll_speed: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TouchpadConfig {
    pub tap_to_click: bool,
    pub natural_scroll: bool,
    pub two_finger_scroll: bool,
    pub disable_while_typing: bool,
    pub acceleration: f64,
    /// `"clickfinger"` (default) or `"areas"`. The compositor rejects
    /// unknown strings with a warning — the UI picker only offers
    /// the two documented values so this is a belt-and-braces check.
    pub click_method: String,
    /// Tap-and-hold to drag a window/selection. Requires
    /// `tap_to_click`.
    pub tap_drag: bool,
}

impl Default for TouchpadConfig {
    fn default() -> Self {
        Self {
            tap_to_click: true,
            natural_scroll: true,
            two_finger_scroll: true,
            disable_while_typing: true,
            acceleration: 0.0,
            click_method: "clickfinger".into(),
            tap_drag: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn mouse_config_defaults_are_zero() {
        let c = MouseConfig::default();
        assert_eq!(c.acceleration, 0.0);
        assert!(!c.natural_scroll);
        assert!(!c.left_handed);
        // scroll_speed 1.0 is libinput's neutral factor — changing this
        // would silently multiply every existing user's scroll.
        assert_eq!(c.scroll_speed, 1.0);
    }

    #[test]
    fn touchpad_defaults_match_spec() {
        let c = TouchpadConfig::default();
        assert!(c.tap_to_click);
        assert!(c.natural_scroll);
        assert!(c.two_finger_scroll);
        assert!(c.disable_while_typing);
        assert_eq!(c.acceleration, 0.0);
        assert_eq!(c.click_method, "clickfinger");
        assert!(c.tap_drag);
    }
}
