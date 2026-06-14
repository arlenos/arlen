//! The `org.arlen.Power1` D-Bus interface.
//!
//! A read surface over the daemon's latest [`PowerState`] snapshot (PWR-R1):
//! the shell, apps and the SDK query power state on demand instead of forking
//! UPower. The poll loop in `main` updates the shared snapshot; this interface
//! serves it. Actions (`Suspend`/`SetProfile`/…) are added by PWR-R2/R5 and
//! gated by PWR-R7; this read interface is unprivileged.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::power::PowerState;

/// Shared, atomically-swappable latest power snapshot.
pub type SharedState = Arc<RwLock<PowerState>>;

/// The `org.arlen.Power1` object.
pub struct PowerInterface {
    state: SharedState,
}

impl PowerInterface {
    /// Wrap the shared snapshot the poll loop updates.
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }
}

#[zbus::interface(name = "org.arlen.Power1")]
impl PowerInterface {
    /// True on battery, false on AC.
    #[zbus(property)]
    async fn on_battery(&self) -> bool {
        self.state.read().await.on_battery
    }

    /// Battery charge, 0-100.
    #[zbus(property)]
    async fn percentage(&self) -> u8 {
        self.state.read().await.percentage
    }

    /// Charge state: "charging"|"discharging"|"full"|"empty"|"unknown".
    #[zbus(property)]
    async fn charge_state(&self) -> String {
        self.state.read().await.charge.as_str().to_string()
    }

    /// Seconds to empty (0 when unknown or charging).
    #[zbus(property)]
    async fn time_to_empty_seconds(&self) -> i64 {
        self.state.read().await.time_to_empty_seconds
    }

    /// Seconds to full (0 when unknown or discharging).
    #[zbus(property)]
    async fn time_to_full_seconds(&self) -> i64 {
        self.state.read().await.time_to_full_seconds
    }

    /// Lid state: "open"|"closed"|"none".
    #[zbus(property)]
    async fn lid_state(&self) -> String {
        self.state.read().await.lid.as_str().to_string()
    }

    /// Active power profile: "performance"|"balanced"|"power-saver"|"unknown".
    #[zbus(property)]
    async fn profile(&self) -> String {
        self.state.read().await.profile.clone()
    }
}
