//! PWR-R3 blank executor: power the outputs off on deep idle.
//!
//! When an idle stage's action is [`Blank`](crate::idle::IdleAction::Blank)
//! the daemon asks the compositor to power the screens off (DPMS-equivalent)
//! via `wlr-output-power-management-v1`, and back on at resume. Unlike the
//! idle-notify client this is a one-shot action, so each call opens a short
//! Wayland connection, sets every output's power mode, flushes, and closes -
//! there is no long-lived event loop to keep.
//!
//! The compositor (cosmic-comp) implements the protocol; a compositor
//! without it, or no display, makes this a logged no-op (the screen simply
//! stays as it is).

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_output::WlOutput, wl_registry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::output_power_management::v1::client::zwlr_output_power_manager_v1::ZwlrOutputPowerManagerV1;
use wayland_protocols_wlr::output_power_management::v1::client::zwlr_output_power_v1::{
    Mode, ZwlrOutputPowerV1,
};

/// Whether to power the outputs on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerMode {
    /// Screens on (resume).
    On,
    /// Screens off (blank on idle).
    Off,
}

impl From<PowerMode> for Mode {
    fn from(m: PowerMode) -> Self {
        match m {
            PowerMode::On => Mode::On,
            PowerMode::Off => Mode::Off,
        }
    }
}

/// A failure powering the outputs. Non-fatal: the screen just stays as it is.
#[derive(Debug, thiserror::Error)]
pub enum BlankError {
    /// No Wayland display to connect to.
    #[error("no wayland display: {0}")]
    Connect(#[from] wayland_client::ConnectError),
    /// The registry could not be enumerated.
    #[error("wayland globals: {0}")]
    Globals(#[from] wayland_client::globals::GlobalError),
    /// The compositor does not implement wlr-output-power-management-v1.
    #[error("compositor has no wlr-output-power-management-v1")]
    NoManager,
    /// The round trip that flushes the mode changes failed.
    #[error("wayland dispatch: {0}")]
    Dispatch(#[from] wayland_client::DispatchError),
}

struct State;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlOutput, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlOutput,
        _: <WlOutput as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrOutputPowerManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputPowerManagerV1,
        _: <ZwlrOutputPowerManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrOutputPowerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputPowerV1,
        _: <ZwlrOutputPowerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Set every output's power to `mode` (blocking). Opens a short Wayland
/// connection, binds the output-power manager, creates a power object per
/// `wl_output`, sets its mode, and round-trips to flush. Returns the number
/// of outputs acted on. Call off the async runtime (e.g. `spawn_blocking`).
pub fn set_all_outputs(mode: PowerMode) -> Result<usize, BlankError> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
    let qh = queue.handle();

    let manager: ZwlrOutputPowerManagerV1 =
        globals.bind(&qh, 1..=1, ()).map_err(|_| BlankError::NoManager)?;

    // One power object per output; set its mode. Held until the round trip
    // flushes the requests to the compositor.
    let mut powers: Vec<ZwlrOutputPowerV1> = Vec::new();
    globals.contents().with_list(|list| {
        for g in list {
            if g.interface == WlOutput::interface().name {
                let output: WlOutput =
                    globals
                        .registry()
                        .bind(g.name, g.version.min(4), &qh, ());
                let power = manager.get_output_power(&output, &qh, ());
                power.set_mode(mode.into());
                powers.push(power);
            }
        }
    });

    let count = powers.len();
    queue.roundtrip(&mut State)?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_mode_maps_to_the_protocol_mode() {
        assert_eq!(Mode::from(PowerMode::On), Mode::On);
        assert_eq!(Mode::from(PowerMode::Off), Mode::Off);
    }

    /// Non-intrusive runtime verify against a live compositor that implements
    /// wlr-output-power-management-v1 (the dev/CI host does): power the outputs
    /// ON (a no-op when they are already on, so nothing the user sees changes)
    /// and confirm the connect -> bind manager -> per-output get_output_power ->
    /// set_mode -> round-trip path works against at least one output.
    /// `#[ignore]d`: needs `$WAYLAND_DISPLAY` + the protocol.
    #[test]
    #[ignore = "needs a live wlr-output-power-management-v1 compositor"]
    fn set_outputs_on_is_a_no_op_that_exercises_the_path() {
        let count = set_all_outputs(PowerMode::On).expect("set outputs on");
        assert!(count >= 1, "expected at least one output");
    }
}
