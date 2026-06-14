//! `arlen-powerd` — the Arlen power daemon (system-services-plan.md PWR-R1).
//!
//! Reads UPower on the system bus, aggregates a coarse [`PowerState`], and
//! publishes `power.state` on the event bus whenever it changes. A poll loop
//! is sufficient for the coarse snapshot (UPower only changes state every few
//! seconds); the signal-driven refresh and the `org.arlen.Power1` D-Bus + query
//! socket are the next PWR-R1 increment, and suspend/idle/profile management
//! (PWR-R2..R7) build on top.

use std::time::Duration;

use arlen_powerd::power::PowerState;
use os_sdk::event::{EventEmitter, UnixEventEmitter};
use prost::Message as _;
use tracing::{debug, info, warn};

/// UPower well-known name + the aggregate display-device path.
const UPOWER_BUS: &str = "org.freedesktop.UPower";
const UPOWER_ROOT_PATH: &str = "/org/freedesktop/UPower";
const UPOWER_DISPLAY_DEVICE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
const UPOWER_DEVICE_IFACE: &str = "org.freedesktop.UPower.Device";
const UPOWER_ROOT_IFACE: &str = "org.freedesktop.UPower";

/// How often to re-read UPower. Coarse: the snapshot only carries
/// state/percentage/time/lid, all slow-moving; emit happens only on change.
const POLL_INTERVAL: Duration = Duration::from_secs(10);

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let producer = os_sdk::runtime::socket_path("ARLEN_PRODUCER_SOCKET", "event-bus-producer.sock");
    info!(socket = %producer.display(), "power-daemon starting");
    let emitter = UnixEventEmitter::new(producer.to_string_lossy().into_owned());

    // System bus for UPower. If it is unavailable at startup we still run and
    // retry on each poll, so a late dbus/UPower start recovers without a crash.
    let mut sysbus = connect_system_bus().await;

    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]);

    let mut last: Option<PowerState> = None;
    let mut ticker = tokio::time::interval(POLL_INTERVAL);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let conn = match sysbus.as_ref() {
                    Some(c) => c.clone(),
                    None => {
                        sysbus = connect_system_bus().await;
                        match sysbus.as_ref() {
                            Some(c) => c.clone(),
                            None => continue,
                        }
                    }
                };
                match read_power_state(&conn).await {
                    Some(state) => {
                        if last.as_ref() != Some(&state) {
                            let bytes = state.to_payload().encode_to_vec();
                            match emitter.emit("power.state", bytes).await {
                                Ok(()) => debug!(?state, "published power.state"),
                                Err(e) => warn!("power.state emit failed: {e}"),
                            }
                            last = Some(state);
                        }
                    }
                    None => {
                        // A read failure after a good connection usually means
                        // UPower/dbus went away; drop the cached bus so the next
                        // tick reconnects.
                        sysbus = None;
                    }
                }
            }
            _ = shutdown_signal() => {
                info!("power-daemon shutting down");
                break;
            }
        }
    }
}

/// Connect to the system bus, logging (not failing) on error.
async fn connect_system_bus() -> Option<zbus::Connection> {
    match zbus::Connection::system().await {
        Ok(c) => Some(c),
        Err(e) => {
            warn!("system bus unavailable: {e}");
            None
        }
    }
}

/// Read the current power state from UPower, or `None` if UPower is
/// unreachable or reports no battery (a desktop with `State`=Unknown and 0%).
async fn read_power_state(conn: &zbus::Connection) -> Option<PowerState> {
    let device = zbus::Proxy::new(conn, UPOWER_BUS, UPOWER_DISPLAY_DEVICE, UPOWER_DEVICE_IFACE)
        .await
        .ok()?;
    let root = zbus::Proxy::new(conn, UPOWER_BUS, UPOWER_ROOT_PATH, UPOWER_ROOT_IFACE)
        .await
        .ok()?;

    let percentage: f64 = device.get_property("Percentage").await.ok()?;
    let state: u32 = device.get_property("State").await.ok()?;
    let time_to_empty: i64 = device.get_property("TimeToEmpty").await.unwrap_or(0);
    let time_to_full: i64 = device.get_property("TimeToFull").await.unwrap_or(0);

    let on_battery: bool = root.get_property("OnBattery").await.unwrap_or(false);
    let lid_present: bool = root.get_property("LidIsPresent").await.unwrap_or(false);
    let lid_closed: bool = root.get_property("LidIsClosed").await.unwrap_or(false);

    // No real battery: nothing useful to publish.
    if percentage == 0.0 && state == 0 {
        return None;
    }

    Some(PowerState::from_upower(
        on_battery,
        percentage,
        state,
        time_to_empty,
        time_to_full,
        lid_present,
        lid_closed,
        None,
    ))
}

/// Resolve when the process receives SIGINT or SIGTERM.
async fn shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
