//! `arlen-powerd` — the Arlen power daemon (system-services-plan.md PWR-R1).
//!
//! Reads UPower on the system bus, aggregates a coarse [`PowerState`], and
//! both publishes `power.state` on the event bus (push) and serves the latest
//! snapshot over `org.arlen.Power1` on the session bus (pull) whenever it
//! changes. A poll loop is sufficient for the coarse snapshot (UPower only
//! changes state every few seconds); the signal-driven refresh is a later
//! refinement, and suspend/idle/profile management (PWR-R2..R7) builds on top.

use std::sync::Arc;
use std::time::Duration;

use arlen_powerd::battery::{self, BatteryLevel};
use arlen_powerd::config::PowerConfig;
use arlen_powerd::dbus::{PowerInterface, SharedState};
use arlen_powerd::logind;
use arlen_powerd::power::PowerState;
use os_sdk::event::{EventEmitter, UnixEventEmitter};
use prost::Message as _;
use tokio::sync::RwLock;
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

    // The shared snapshot the org.arlen.Power1 interface serves. The poll loop
    // writes the latest reading; pull consumers (shell, apps, SDK) read it
    // without forking UPower. Served on the SESSION bus (this is a per-user
    // daemon); UPower reads stay on the system bus above.
    let shared: SharedState = Arc::new(RwLock::new(PowerState::default()));
    // Held for the daemon lifetime to keep the org.arlen.Power1 name owned, and
    // reused to send the low/critical battery notifications (PWR-R6) over the
    // same session bus.
    let session_bus = match serve_dbus(shared.clone()).await {
        Some(conn) => Some(conn),
        None => {
            // A missing session bus must not stop the event-bus publish path:
            // the push channel still works, only the pull surface (and the
            // low-battery notification) is absent.
            warn!("org.arlen.Power1 unavailable; continuing with event-bus publish only");
            None
        }
    };

    // SST-R3: the socket-pull surface. os-sdk is socket-based and does not speak
    // D-Bus, so the org.arlen.Power1 read properties leave SDK consumers without a
    // query path; this serves the same snapshot over a Unix socket. Best-effort:
    // a failure to bind leaves the D-Bus + event-push surfaces working.
    let query_path = os_sdk::power::socket_path();
    match arlen_powerd::query_socket::bind(&query_path) {
        Ok(listener) => {
            info!(socket = %query_path.display(), "power query socket listening");
            tokio::spawn(arlen_powerd::query_socket::serve(listener, shared.clone()));
        }
        Err(e) => warn!("power query socket unavailable: {e}"),
    }

    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]);

    // The critical-battery auto-action config (off by default; PWR-R6).
    let power_config = PowerConfig::load();
    info!(
        critical_action = ?power_config.critical_action.action,
        floor = power_config.critical_action.floor,
        "power config loaded"
    );

    let mut last: Option<PowerState> = None;
    // The hysteretic battery level depends on the previous level, so it is
    // tracked across ticks separately from the raw snapshot.
    let mut level = BatteryLevel::Normal;
    // Latch so the critical auto-action fires once per descent below the floor,
    // reset when the machine charges or rises back above it.
    let mut critical_acted = false;
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
                            // Update the pull snapshot first so a consumer that
                            // reacts to the push event reads the fresh value.
                            *shared.write().await = state.clone();
                            let bytes = state.to_payload().encode_to_vec();
                            match emitter.emit("power.state", bytes).await {
                                Ok(()) => debug!(?state, "published power.state"),
                                Err(e) => warn!("power.state emit failed: {e}"),
                            }

                            // Coarse battery-level transition (PWR-R6): publish
                            // power.low / power.critical / power.recovered once
                            // per crossing, not on every percentage tick.
                            let next = battery::next_level(level, state.percentage, state.on_battery);
                            if let Some(evt) = battery::transition_event(level, next) {
                                emit_transition(&emitter, evt, String::new()).await;
                                // PWR-R6: alert the user once on entering low /
                                // critical (a no-op for the recovered transition).
                                if let Some(conn) = session_bus.as_ref() {
                                    arlen_powerd::notify::send_battery_notification(
                                        conn,
                                        next,
                                        state.percentage,
                                    )
                                    .await;
                                }
                            }
                            level = next;

                            // Coarse profile change (PWR-R6): publish
                            // power.profile_changed when the active profile
                            // actually changes to a known value.
                            if let Some(prev) = last.as_ref() {
                                if prev.profile != state.profile
                                    && state.profile != arlen_powerd::profiles::PROFILE_UNKNOWN
                                {
                                    emit_transition(
                                        &emitter,
                                        "power.profile_changed",
                                        state.profile.clone(),
                                    )
                                    .await;
                                }
                            }

                            // PWR-R6 critical-battery auto-action (off by
                            // default). Reset the latch once off the floor (on AC
                            // or risen above it), then fire the configured action
                            // once on the descent below it.
                            if !state.on_battery
                                || state.percentage > power_config.critical_action.floor
                            {
                                critical_acted = false;
                            }
                            if let Some(action) = power_config.critical_action(
                                state.percentage,
                                state.on_battery,
                                critical_acted,
                            ) {
                                warn!(
                                    percentage = state.percentage,
                                    action = action.as_str(),
                                    "critical battery: performing the configured auto-action"
                                );
                                match logind::perform(&conn, action).await {
                                    Ok(()) => critical_acted = true,
                                    Err(e) => warn!(
                                        "critical battery auto-action {} failed: {e}",
                                        action.as_str()
                                    ),
                                }
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

/// Publish a coarse `power.*` transition with an optional detail string. These
/// are the events safe to graph-promote (a crossing, a profile change), unlike
/// the percentage-churning `power.state`. Best-effort: a publish failure is
/// logged, never fatal.
async fn emit_transition(emitter: &UnixEventEmitter, event_type: &str, detail: String) {
    let bytes = os_sdk::proto::PowerTransitionPayload { detail }.encode_to_vec();
    match emitter.emit(event_type, bytes).await {
        Ok(()) => debug!(event_type, "published power transition"),
        Err(e) => warn!("{event_type} emit failed: {e}"),
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

/// Path under which the power interface is served.
const POWER_OBJECT_PATH: &str = "/org/arlen/Power1";
/// The well-known name the power interface owns on the session bus.
const POWER_BUS_NAME: &str = "org.arlen.Power1";

/// Claim `org.arlen.Power1` on the session bus and serve the shared snapshot.
///
/// Returns the owning connection (it must be held for the lifetime of the
/// daemon to keep the name) or `None` if the session bus is unavailable, so
/// the event-bus publish path keeps working without the pull surface.
///
/// The interface also drives logind / power-profiles-daemon for the gated
/// actions; it does so on its own dedicated **system-bus** connection (the poll
/// loop's read connection reconnects independently), so a service restart on
/// the system bus does not disturb the read path and vice versa. If the system
/// bus is unavailable at startup the actions fail closed.
async fn serve_dbus(shared: SharedState) -> Option<zbus::Connection> {
    let action_bus = connect_system_bus().await;
    let iface = PowerInterface::new(shared, action_bus);
    match zbus::connection::Builder::session()
        .and_then(|b| b.name(POWER_BUS_NAME))
        .and_then(|b| b.serve_at(POWER_OBJECT_PATH, iface))
        .map(|b| b.build())
    {
        Ok(fut) => match fut.await {
            Ok(conn) => {
                info!(name = POWER_BUS_NAME, path = POWER_OBJECT_PATH, "serving power interface");
                Some(conn)
            }
            Err(e) => {
                warn!("failed to serve {POWER_BUS_NAME}: {e}");
                None
            }
        },
        Err(e) => {
            warn!("failed to build session connection for {POWER_BUS_NAME}: {e}");
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

    // The active power profile is read best-effort from power-profiles-daemon
    // (also on the system bus); its absence leaves the field at "unknown"
    // rather than failing the whole snapshot.
    let profile = arlen_powerd::profiles::read_active_profile(conn).await;

    Some(PowerState::from_upower(
        on_battery,
        percentage,
        state,
        time_to_empty,
        time_to_full,
        lid_present,
        lid_closed,
        profile,
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
