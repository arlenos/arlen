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

/// A lid or power-key action, as a config keyword. `None` disables that action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LidActionKind {
    /// Do nothing (leave the session running).
    None,
    /// Suspend to RAM.
    Suspend,
    /// Suspend to disk.
    Hibernate,
    /// Power the machine off.
    PowerOff,
}

impl LidActionKind {
    /// The logind action, or `None` for [`LidActionKind::None`].
    fn to_action(self) -> Option<PowerAction> {
        match self {
            LidActionKind::None => Option::None,
            LidActionKind::Suspend => Some(PowerAction::Suspend),
            LidActionKind::Hibernate => Some(PowerAction::Hibernate),
            LidActionKind::PowerOff => Some(PowerAction::PowerOff),
        }
    }
}

/// The `[lid]` config block (PWR-R4). The defaults match the built-in behaviour -
/// suspend on lid-close but not while docked, power-off on the power key - so a
/// machine with no `[lid]` block behaves exactly as before; a user sets `on-close`
/// to `none` to keep a closed laptop awake, or to `hibernate`, etc.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct LidConfigToml {
    /// The action when the lid closes.
    pub on_close: LidActionKind,
    /// Skip the lid-close action while docked (an external display attached).
    pub ignore_when_docked: bool,
    /// The action when the power button is pressed.
    pub on_power_key: LidActionKind,
}

impl Default for LidConfigToml {
    fn default() -> Self {
        Self {
            on_close: LidActionKind::Suspend,
            ignore_when_docked: true,
            on_power_key: LidActionKind::PowerOff,
        }
    }
}

impl LidConfigToml {
    /// The resolved lid policy the daemon acts on.
    pub fn resolve(&self) -> crate::lid::LidConfig {
        crate::lid::LidConfig {
            on_lid_close: self.on_close.to_action(),
            ignore_lid_when_docked: self.ignore_when_docked,
            on_power_key: self.on_power_key.to_action(),
        }
    }
}

/// The power daemon's config (`~/.config/arlen/power.toml`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PowerConfig {
    /// The critical-battery auto-action.
    pub critical_action: CriticalActionConfig,
    /// The lid / power-key policy (PWR-R4).
    pub lid: LidConfigToml,
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
            ..PowerConfig::default()
        }
    }

    #[test]
    fn the_lid_default_matches_the_builtin_behaviour() {
        // A machine with no [lid] block behaves as before: suspend on lid-close,
        // not while docked, power-off on the power key.
        let resolved = LidConfigToml::default().resolve();
        assert_eq!(resolved.on_lid_close, Some(PowerAction::Suspend));
        assert!(resolved.ignore_lid_when_docked);
        assert_eq!(resolved.on_power_key, Some(PowerAction::PowerOff));
    }

    #[test]
    fn a_lid_block_can_disable_or_change_the_action() {
        let toml = "[lid]\non-close = \"none\"\nignore-when-docked = false\non-power-key = \"hibernate\"\n";
        let cfg: PowerConfig = toml::from_str(toml).unwrap();
        let resolved = cfg.lid.resolve();
        // `none` keeps a closed laptop awake; the dock skip is off; the power key
        // hibernates.
        assert_eq!(resolved.on_lid_close, None);
        assert!(!resolved.ignore_lid_when_docked);
        assert_eq!(resolved.on_power_key, Some(PowerAction::Hibernate));
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
