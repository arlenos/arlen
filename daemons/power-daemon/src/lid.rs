//! Lid-switch and power-button policy (system-services-plan.md PWR-R4).
//!
//! Arlen owns the lid/power-button policy rather than leaving it to logind's
//! defaults, so it can be **dock-aware**: closing the lid while an external
//! display is attached must not suspend (the classic "laptop docked to a
//! monitor" case). The daemon sets logind's `HandleLidSwitch`/`HandlePowerKey`
//! to `ignore` and applies this policy itself, acting through the logind client
//! ([`crate::logind`]).
//!
//! This module is the **pure decision core**: given the configured action, the
//! lid state and whether the machine is docked, it returns the action to take
//! (or `None` to do nothing). Listening for the logind events and sourcing the
//! docked signal (an external-output count from the compositor) is the wiring
//! Tim verifies on metal; the policy itself is unit-tested here.

use crate::logind::PowerAction;

/// How the daemon reacts to the lid and the power button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LidConfig {
    /// The action when the lid closes (`None` = do nothing).
    pub on_lid_close: Option<PowerAction>,
    /// Skip the lid-close action while docked (an external display attached).
    pub ignore_lid_when_docked: bool,
    /// The action when the power button is pressed (`None` = do nothing).
    pub on_power_key: Option<PowerAction>,
}

impl Default for LidConfig {
    /// Suspend on lid-close (but not while docked); power-off on power-key.
    /// Conservative laptop defaults; a desktop has no lid so the lid action is
    /// simply never triggered.
    fn default() -> Self {
        LidConfig {
            on_lid_close: Some(PowerAction::Suspend),
            ignore_lid_when_docked: true,
            on_power_key: Some(PowerAction::PowerOff),
        }
    }
}

/// The action to take when the lid closes, honouring the dock-aware rule.
///
/// While docked (`docked == true`) with `ignore_lid_when_docked` set, returns
/// `None` so the session keeps running on the external display.
pub fn lid_close_action(cfg: &LidConfig, docked: bool) -> Option<PowerAction> {
    if docked && cfg.ignore_lid_when_docked {
        return None;
    }
    cfg.on_lid_close
}

/// The action to take when the power button is pressed.
pub fn power_key_action(cfg: &LidConfig) -> Option<PowerAction> {
    cfg.on_power_key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docked_lid_close_is_ignored_by_default() {
        let cfg = LidConfig::default();
        assert_eq!(lid_close_action(&cfg, true), None);
        assert_eq!(lid_close_action(&cfg, false), Some(PowerAction::Suspend));
    }

    #[test]
    fn docked_lid_close_acts_when_dock_awareness_disabled() {
        let cfg = LidConfig {
            ignore_lid_when_docked: false,
            ..LidConfig::default()
        };
        assert_eq!(lid_close_action(&cfg, true), Some(PowerAction::Suspend));
    }

    #[test]
    fn no_lid_action_when_unconfigured() {
        let cfg = LidConfig {
            on_lid_close: None,
            ..LidConfig::default()
        };
        assert_eq!(lid_close_action(&cfg, false), None);
    }

    #[test]
    fn power_key_uses_the_configured_action() {
        assert_eq!(power_key_action(&LidConfig::default()), Some(PowerAction::PowerOff));
        let cfg = LidConfig {
            on_power_key: None,
            ..LidConfig::default()
        };
        assert_eq!(power_key_action(&cfg), None);
    }
}
