//! The power-daemon config: the critical-battery auto-action (system-services-plan.md PWR-R6).
//!
//! At a configured critical floor the daemon can suspend the system to disk (or
//! power off) so a dying battery does not cause a hard, dirty power loss. This is
//! a **high-impact auto-action**, so it is **off by default** (`action = "none"`):
//! the daemon never suspends or powers off the machine on its own until the user
//! opts in via `~/.config/arlen/power.toml`. Whether to ship it on by default,
//! and the floor/action, is a deliberate decision verified on real hardware
//! (like the other logind actions, PWR-R2/R4); this builds the gated mechanism.
//!
//! ```toml
//! [critical_action]
//! action = "none"   # none | hibernate | power-off
//! floor  = 2        # act at or below this battery percent, while on battery
//! ```
//!
//! The decision ([`PowerConfig::critical_action`]) is a pure function so it is
//! unit-tested without hardware; the actual logind call is in `main`.

use serde::Deserialize;

use crate::logind::PowerAction;

/// The action to take at the critical floor. `None` (the default) disables the
/// auto-action entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CriticalActionKind {
    /// No auto-action (the safe default): the daemon only notifies, never acts.
    #[default]
    None,
    /// Suspend to disk (preserves session state).
    Hibernate,
    /// Clean power-off (preferable to a dirty battery death when hibernate is
    /// not available).
    PowerOff,
}

/// The `[critical_action]` config block.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CriticalActionConfig {
    /// What to do at the floor; `None` by default (off).
    pub action: CriticalActionKind,
    /// The battery percent at or below which the action fires (while on battery).
    pub floor: u8,
}

impl Default for CriticalActionConfig {
    fn default() -> Self {
        Self {
            action: CriticalActionKind::None,
            floor: 2,
        }
    }
}

/// The power daemon's config (`~/.config/arlen/power.toml`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PowerConfig {
    /// The critical-battery auto-action.
    pub critical_action: CriticalActionConfig,
}

impl PowerConfig {
    /// Load `~/.config/arlen/power.toml`, falling back to the safe default
    /// (auto-action off) when the file is absent or unparseable - a malformed
    /// config must never accidentally enable an auto power-off.
    pub fn load() -> Self {
        match std::fs::read_to_string(os_sdk::config::config_path("power")) {
            Ok(text) => toml::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// The logind action to perform at this reading, or `None`. Fires only when
    /// the action is enabled, the machine is on battery, the charge is at or
    /// below the floor, and it has not already fired this descent
    /// (`already_acted`). Pure.
    pub fn critical_action(
        &self,
        percentage: u8,
        on_battery: bool,
        already_acted: bool,
    ) -> Option<PowerAction> {
        if already_acted || !on_battery || percentage > self.critical_action.floor {
            return None;
        }
        match self.critical_action.action {
            CriticalActionKind::None => None,
            CriticalActionKind::Hibernate => Some(PowerAction::Hibernate),
            CriticalActionKind::PowerOff => Some(PowerAction::PowerOff),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(action: CriticalActionKind, floor: u8) -> PowerConfig {
        PowerConfig {
            critical_action: CriticalActionConfig { action, floor },
        }
    }

    #[test]
    fn default_is_off() {
        let c = PowerConfig::default();
        assert_eq!(c.critical_action.action, CriticalActionKind::None);
        // Even at 0% the default never acts.
        assert_eq!(c.critical_action(0, true, false), None);
    }

    #[test]
    fn fires_once_below_the_floor_on_battery() {
        let c = cfg(CriticalActionKind::Hibernate, 2);
        // Above the floor: no action.
        assert_eq!(c.critical_action(5, true, false), None);
        // At/below the floor, on battery, not yet acted: fire.
        assert_eq!(c.critical_action(2, true, false), Some(PowerAction::Hibernate));
        assert_eq!(c.critical_action(1, true, false), Some(PowerAction::Hibernate));
        // Already acted this descent: do not re-fire.
        assert_eq!(c.critical_action(1, true, true), None);
    }

    #[test]
    fn never_acts_on_ac() {
        let c = cfg(CriticalActionKind::PowerOff, 2);
        assert_eq!(c.critical_action(1, false, false), None);
    }

    #[test]
    fn power_off_kind_maps_to_power_off() {
        let c = cfg(CriticalActionKind::PowerOff, 3);
        assert_eq!(c.critical_action(3, true, false), Some(PowerAction::PowerOff));
    }
}
