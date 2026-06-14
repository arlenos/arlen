//! Battery-level transition tracking (system-services-plan.md PWR-R6).
//!
//! Watches the discharging battery and classifies it into [`BatteryLevel`]
//! (Normal/Low/Critical) so the daemon publishes a coarse `power.low` /
//! `power.critical` / `power.recovered` transition exactly once per crossing,
//! not on every percentage tick. These coarse transitions are the ones safe to
//! graph-promote; the raw percentage churn stays a live `power.state` read.
//!
//! The classifier carries **hysteresis**: a separate, higher exit threshold so
//! a battery wobbling around the entry point does not flap the warning. It is a
//! pure function of (previous level, percentage, on-battery) so it is
//! unit-tested without hardware. The critical *action* (suspend at the floor)
//! is a logind action gated by PWR-R7, deliberately not here.

/// Enter "low" at or below this charge (while discharging).
pub const LOW_ENTER: u8 = 20;
/// Enter "critical" at or below this charge (while discharging).
pub const CRITICAL_ENTER: u8 = 5;
/// Leave "critical" back up to "low" once charge climbs above this.
pub const CRITICAL_EXIT: u8 = 10;
/// Leave any warning back to "normal" once charge climbs to this.
pub const RECOVER_EXIT: u8 = 25;

/// A coarse battery level. Normal is the unwarned state (incl. on AC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BatteryLevel {
    /// No warning: charging, full, or comfortably above the low threshold.
    #[default]
    Normal,
    /// Below the low threshold while discharging.
    Low,
    /// Below the critical threshold while discharging.
    Critical,
}

/// Classify the next level from the previous one (hysteresis depends on it),
/// the current charge, and whether we are on battery.
///
/// On AC there is no battery warning, so the result is always `Normal`
/// (plugging in clears the warning immediately). While discharging, the entry
/// thresholds ([`LOW_ENTER`]/[`CRITICAL_ENTER`]) move the level down and the
/// higher exit thresholds ([`CRITICAL_EXIT`]/[`RECOVER_EXIT`]) move it back up,
/// so a charge wobbling at the boundary does not flap.
pub fn next_level(prev: BatteryLevel, percentage: u8, on_battery: bool) -> BatteryLevel {
    if !on_battery {
        return BatteryLevel::Normal;
    }
    match prev {
        BatteryLevel::Normal => {
            if percentage <= CRITICAL_ENTER {
                BatteryLevel::Critical
            } else if percentage <= LOW_ENTER {
                BatteryLevel::Low
            } else {
                BatteryLevel::Normal
            }
        }
        BatteryLevel::Low => {
            if percentage <= CRITICAL_ENTER {
                BatteryLevel::Critical
            } else if percentage >= RECOVER_EXIT {
                BatteryLevel::Normal
            } else {
                BatteryLevel::Low
            }
        }
        BatteryLevel::Critical => {
            if percentage >= RECOVER_EXIT {
                BatteryLevel::Normal
            } else if percentage > CRITICAL_EXIT {
                BatteryLevel::Low
            } else {
                BatteryLevel::Critical
            }
        }
    }
}

/// The `power.*` event type for a level change, or `None` if the level is
/// unchanged (no transition to publish).
pub fn transition_event(from: BatteryLevel, to: BatteryLevel) -> Option<&'static str> {
    if from == to {
        return None;
    }
    Some(match to {
        BatteryLevel::Critical => "power.critical",
        BatteryLevel::Low => "power.low",
        BatteryLevel::Normal => "power.recovered",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_ac_is_always_normal() {
        assert_eq!(next_level(BatteryLevel::Critical, 3, false), BatteryLevel::Normal);
        assert_eq!(next_level(BatteryLevel::Low, 15, false), BatteryLevel::Normal);
    }

    #[test]
    fn descends_through_thresholds_while_discharging() {
        assert_eq!(next_level(BatteryLevel::Normal, 50, true), BatteryLevel::Normal);
        assert_eq!(next_level(BatteryLevel::Normal, 20, true), BatteryLevel::Low);
        assert_eq!(next_level(BatteryLevel::Low, 5, true), BatteryLevel::Critical);
    }

    #[test]
    fn hysteresis_holds_the_level_in_the_band() {
        // A charge wobbling just above the low-enter point stays Low, not Normal.
        assert_eq!(next_level(BatteryLevel::Low, 21, true), BatteryLevel::Low);
        assert_eq!(next_level(BatteryLevel::Low, 24, true), BatteryLevel::Low);
        // Critical holds inside the critical-exit band (<= CRITICAL_EXIT), and
        // only steps up to Low once charge climbs past it.
        assert_eq!(next_level(BatteryLevel::Critical, 5, true), BatteryLevel::Critical);
        assert_eq!(next_level(BatteryLevel::Critical, 9, true), BatteryLevel::Critical);
        assert_eq!(next_level(BatteryLevel::Critical, 10, true), BatteryLevel::Critical);
        assert_eq!(next_level(BatteryLevel::Critical, 11, true), BatteryLevel::Low);
    }

    #[test]
    fn recovers_only_above_the_exit_threshold() {
        assert_eq!(next_level(BatteryLevel::Low, 24, true), BatteryLevel::Low);
        assert_eq!(next_level(BatteryLevel::Low, 25, true), BatteryLevel::Normal);
        assert_eq!(next_level(BatteryLevel::Critical, 25, true), BatteryLevel::Normal);
    }

    #[test]
    fn transition_event_only_fires_on_change() {
        assert_eq!(transition_event(BatteryLevel::Normal, BatteryLevel::Normal), None);
        assert_eq!(
            transition_event(BatteryLevel::Normal, BatteryLevel::Low),
            Some("power.low")
        );
        assert_eq!(
            transition_event(BatteryLevel::Low, BatteryLevel::Critical),
            Some("power.critical")
        );
        assert_eq!(
            transition_event(BatteryLevel::Critical, BatteryLevel::Normal),
            Some("power.recovered")
        );
    }
}
