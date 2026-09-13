//! Power-state aggregation.
//!
//! Reads UPower (battery charge, charging state, time-to-empty/full, AC line,
//! lid) into a coarse [`PowerState`] snapshot and converts it to the
//! `power.state` event payload (system-services-plan.md PWR-R1). The daemon
//! publishes a fresh snapshot on change so the shell's battery indicator (and
//! the AI layer) consume one shared, bus-delivered value instead of each
//! forking UPower. Battery-percentage churn stays a live read, never
//! graph-promoted; only the coarse transitions are local provenance.
//!
//! The raw-UPower → snapshot mapping is a pure function ([`PowerState::from_upower`])
//! so it is unit-tested without a live system bus; the live read
//! ([`UPowerReader`]) wraps it around zbus property calls.

use serde::Serialize;

/// The battery charge state, normalised from UPower's `State` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChargeState {
    /// State could not be determined.
    #[default]
    Unknown,
    /// Charging (incl. UPower's pending-charge).
    Charging,
    /// Discharging (incl. UPower's pending-discharge).
    Discharging,
    /// Fully discharged.
    Empty,
    /// Fully charged.
    Full,
}

impl ChargeState {
    /// Map UPower's `org.freedesktop.UPower.Device.State` enum:
    /// 0=Unknown, 1=Charging, 2=Discharging, 3=Empty, 4=FullyCharged,
    /// 5=PendingCharge, 6=PendingDischarge.
    pub fn from_upower(state: u32) -> Self {
        match state {
            1 | 5 => ChargeState::Charging,
            2 | 6 => ChargeState::Discharging,
            3 => ChargeState::Empty,
            4 => ChargeState::Full,
            _ => ChargeState::Unknown,
        }
    }

    /// Whether the battery is gaining charge (charging or pending-charge).
    pub fn is_charging(self) -> bool {
        matches!(self, ChargeState::Charging)
    }

    /// The wire string for the `power.state` payload.
    pub fn as_str(self) -> &'static str {
        match self {
            ChargeState::Unknown => "unknown",
            ChargeState::Charging => "charging",
            ChargeState::Discharging => "discharging",
            ChargeState::Empty => "empty",
            ChargeState::Full => "full",
        }
    }
}

/// The lid state. `None` means the machine has no lid (a desktop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LidState {
    /// No lid present (desktop).
    #[default]
    None,
    /// Lid open.
    Open,
    /// Lid closed.
    Closed,
}

impl LidState {
    /// Derive from UPower's `LidIsPresent` / `LidIsClosed` root properties.
    pub fn from_upower(present: bool, closed: bool) -> Self {
        match (present, closed) {
            (false, _) => LidState::None,
            (true, true) => LidState::Closed,
            (true, false) => LidState::Open,
        }
    }

    /// The wire string for the `power.state` payload.
    pub fn as_str(self) -> &'static str {
        match self {
            LidState::None => "none",
            LidState::Open => "open",
            LidState::Closed => "closed",
        }
    }
}

/// A coarse power-state snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct PowerState {
    /// True on battery, false on AC.
    pub on_battery: bool,
    /// Battery charge, 0-100.
    pub percentage: u8,
    /// Charge state.
    pub charge: ChargeState,
    /// Seconds to empty (0 when unknown or charging).
    pub time_to_empty_seconds: i64,
    /// Seconds to full (0 when unknown or discharging).
    pub time_to_full_seconds: i64,
    /// Lid state.
    pub lid: LidState,
    /// Active power profile ("performance"|"balanced"|"power-saver"|"unknown").
    pub profile: String,
}

impl PowerState {
    /// Whether this is a DIFFERENT power state, for deciding what to publish.
    ///
    /// Not `==`, and the difference is the whole point. `PowerState` carries two
    /// running estimates, `time_to_empty_seconds` and `time_to_full_seconds`, and
    /// UPower re-computes those continuously - so a plain equality guard on the
    /// whole struct never holds and every poll looks like a change.
    ///
    /// Measured on a plugged-in laptop at 100%, nothing touched: four
    /// `power.state` events in thirty-five seconds, one about every nine seconds,
    /// with percentage, charge state, lid and profile identical across all four
    /// and only the seconds-to-empty drifting. That is a power-state change
    /// recorded in the graph, forever, on a machine whose power state has not
    /// changed since it was plugged in.
    ///
    /// The estimates still travel IN the payload and stay live on the pull
    /// socket, which is where a consumer that wants the number should read it.
    /// They just do not make an event.
    pub fn is_new_state(&self, other: &Self) -> bool {
        self.on_battery != other.on_battery
            || self.percentage != other.percentage
            || self.charge != other.charge
            || self.lid != other.lid
            || self.profile != other.profile
    }

    /// Build a snapshot from raw UPower values. Pure: no I/O, so the
    /// normalisation (state mapping, percentage clamp, time-field gating, lid
    /// derivation) is unit-tested directly. `profile` comes from
    /// power-profiles-daemon (PWR-R5); pass `None` until that lands.
    #[allow(clippy::too_many_arguments)]
    pub fn from_upower(
        on_battery: bool,
        percentage: f64,
        upower_state: u32,
        time_to_empty: i64,
        time_to_full: i64,
        lid_present: bool,
        lid_closed: bool,
        profile: Option<String>,
    ) -> Self {
        let charge = ChargeState::from_upower(upower_state);
        // Only the relevant direction's estimate is meaningful, and only when
        // positive; UPower reports 0 (and sometimes negatives) when unknown.
        let time_to_full_seconds = if charge.is_charging() {
            time_to_full.max(0)
        } else {
            0
        };
        let time_to_empty_seconds = if charge.is_charging() {
            0
        } else {
            time_to_empty.max(0)
        };
        PowerState {
            on_battery,
            percentage: percentage.round().clamp(0.0, 100.0) as u8,
            charge,
            time_to_empty_seconds,
            time_to_full_seconds,
            lid: LidState::from_upower(lid_present, lid_closed),
            profile: profile.unwrap_or_else(|| "unknown".to_string()),
        }
    }

    /// Convert to the `power.state` wire payload.
    pub fn to_payload(&self) -> os_sdk::proto::PowerStatePayload {
        os_sdk::proto::PowerStatePayload {
            on_battery: self.on_battery,
            percentage: self.percentage as u32,
            state: self.charge.as_str().to_string(),
            time_to_empty_seconds: self.time_to_empty_seconds,
            time_to_full_seconds: self.time_to_full_seconds,
            lid_state: self.lid.as_str().to_string(),
            profile: self.profile.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot with a given estimate, everything else fixed.
    fn plugged_in(seconds_to_empty: i64) -> PowerState {
        PowerState {
            on_battery: false,
            percentage: 100,
            charge: ChargeState::Discharging,
            time_to_empty_seconds: seconds_to_empty,
            time_to_full_seconds: 0,
            lid: LidState::Open,
            profile: "balanced".to_string(),
        }
    }

    #[test]
    fn a_drifting_estimate_is_not_a_new_state() {
        // The measurement this came from: a plugged-in laptop at 100%, nothing
        // touched, publishing `power.state` about every nine seconds because
        // UPower re-computes seconds-to-empty continuously. Everything a person
        // would call the power state was identical across all of them.
        let before = plugged_in(2_934_900);
        let after = plugged_in(2_934_586);
        assert_ne!(before, after, "the structs do differ, which is why == failed");
        assert!(
            !after.is_new_state(&before),
            "but the power state did not change"
        );
    }

    #[test]
    fn every_field_a_person_would_call_the_state_still_counts() {
        let base = plugged_in(2_934_900);
        let changed = [
            PowerState { on_battery: true, ..base.clone() },
            PowerState { percentage: 99, ..base.clone() },
            PowerState { charge: ChargeState::Charging, ..base.clone() },
            PowerState { lid: LidState::Closed, ..base.clone() },
            PowerState { profile: "power-saver".to_string(), ..base.clone() },
        ];
        for other in changed {
            assert!(
                other.is_new_state(&base),
                "this is a real transition and must publish: {other:?}"
            );
        }
    }

    #[test]
    fn charge_state_maps_upower_enum_including_pending() {
        assert_eq!(ChargeState::from_upower(0), ChargeState::Unknown);
        assert_eq!(ChargeState::from_upower(1), ChargeState::Charging);
        assert_eq!(ChargeState::from_upower(2), ChargeState::Discharging);
        assert_eq!(ChargeState::from_upower(3), ChargeState::Empty);
        assert_eq!(ChargeState::from_upower(4), ChargeState::Full);
        assert_eq!(ChargeState::from_upower(5), ChargeState::Charging);
        assert_eq!(ChargeState::from_upower(6), ChargeState::Discharging);
        assert_eq!(ChargeState::from_upower(99), ChargeState::Unknown);
    }

    #[test]
    fn lid_state_none_when_absent_regardless_of_closed() {
        assert_eq!(LidState::from_upower(false, true), LidState::None);
        assert_eq!(LidState::from_upower(false, false), LidState::None);
        assert_eq!(LidState::from_upower(true, true), LidState::Closed);
        assert_eq!(LidState::from_upower(true, false), LidState::Open);
    }

    #[test]
    fn discharging_keeps_time_to_empty_zeroes_time_to_full() {
        let s = PowerState::from_upower(true, 73.6, 2, 4200, 0, true, false, None);
        assert!(s.on_battery);
        assert_eq!(s.percentage, 74);
        assert_eq!(s.charge, ChargeState::Discharging);
        assert_eq!(s.time_to_empty_seconds, 4200);
        assert_eq!(s.time_to_full_seconds, 0);
        assert_eq!(s.lid, LidState::Open);
        assert_eq!(s.profile, "unknown");
    }

    #[test]
    fn charging_keeps_time_to_full_zeroes_time_to_empty() {
        let s = PowerState::from_upower(false, 41.0, 1, 9999, 1800, true, false, Some("balanced".into()));
        assert!(!s.on_battery);
        assert_eq!(s.charge, ChargeState::Charging);
        assert_eq!(s.time_to_full_seconds, 1800);
        assert_eq!(s.time_to_empty_seconds, 0);
        assert_eq!(s.profile, "balanced");
    }

    #[test]
    fn percentage_clamps_and_negative_times_floor_to_zero() {
        let s = PowerState::from_upower(true, 150.0, 2, -1, 0, false, false, None);
        assert_eq!(s.percentage, 100);
        assert_eq!(s.time_to_empty_seconds, 0);
        assert_eq!(s.lid, LidState::None);
    }

    #[test]
    fn payload_round_trips_the_fields() {
        let s = PowerState::from_upower(true, 50.0, 2, 3600, 0, true, true, None);
        let p = s.to_payload();
        assert!(p.on_battery);
        assert_eq!(p.percentage, 50);
        assert_eq!(p.state, "discharging");
        assert_eq!(p.time_to_empty_seconds, 3600);
        assert_eq!(p.lid_state, "closed");
        assert_eq!(p.profile, "unknown");
    }
}
