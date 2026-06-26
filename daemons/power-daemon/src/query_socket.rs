//! The power-daemon query socket (SST-R3, the socket half of the read API).
//!
//! os-sdk apps pull the current power state in the SDK's native socket style
//! (see [`os_sdk::power`]): on each connection the daemon writes the latest
//! snapshot as JSON and closes. The D-Bus `org.arlen.Power1` properties and the
//! `power.state` event payload carry the same shape, so a caller gets identical
//! data by D-Bus, event-push or this socket-pull. This exists because os-sdk is
//! socket-based and does not speak D-Bus, so the D-Bus read surface alone leaves
//! SDK consumers without a pull path.

use std::os::unix::fs::FileTypeExt;
use std::path::Path;

use os_sdk::power::PowerSnapshot;
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, warn};

use crate::dbus::SharedState;
use crate::power::PowerState;

/// Map the daemon's internal snapshot to the wire shape os-sdk decodes (the enum
/// fields become their canonical strings, the same ones the D-Bus properties
/// serve). Pure, so the mapping is unit-tested directly.
pub fn snapshot_wire(state: &PowerState) -> PowerSnapshot {
    PowerSnapshot {
        on_battery: state.on_battery,
        percentage: state.percentage,
        charge_state: state.charge.as_str().to_string(),
        time_to_empty_seconds: state.time_to_empty_seconds,
        time_to_full_seconds: state.time_to_full_seconds,
        lid_state: state.lid.as_str().to_string(),
        profile: state.profile.clone(),
    }
}

/// Bind the query socket at `path`, replacing a stale socket left by a prior run.
/// Mode 0600: the snapshot is the user's own power state, owner-only like the
/// other per-user sockets. Only an existing *socket* is removed first, never a
/// regular file or symlink a launcher may have placed (fail-safe).
pub fn bind(path: &Path) -> std::io::Result<UnixListener> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_socket() {
            let _ = std::fs::remove_file(path);
        }
    }
    let listener = UnixListener::bind(path)?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    Ok(listener)
}

/// Serve one connection: write the snapshot as JSON, then drop (the close is the
/// reader's EOF). A read-only single write; no request is parsed.
pub async fn serve_connection(mut stream: UnixStream, snapshot: &PowerSnapshot) {
    match serde_json::to_vec(snapshot) {
        Ok(bytes) => {
            if let Err(e) = stream.write_all(&bytes).await {
                debug!("power query: write failed: {e}");
                return;
            }
            let _ = stream.flush().await;
        }
        Err(e) => warn!("power query: serialise failed: {e}"),
    }
}

/// Accept loop: snapshot the shared state per connection and serve it. Runs for
/// the daemon lifetime. A transient accept error backs off briefly rather than
/// spinning.
pub async fn serve(listener: UnixListener, shared: SharedState) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let snapshot = snapshot_wire(&shared.read().await.clone());
                serve_connection(stream, &snapshot).await;
            }
            Err(e) => {
                warn!("power query: accept failed: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::PowerState;
    use tokio::io::AsyncReadExt;

    #[test]
    fn snapshot_wire_carries_the_canonical_strings() {
        let state = PowerState::from_upower(false, 41.0, 1, 9999, 1800, true, false, Some("balanced".into()));
        let wire = snapshot_wire(&state);
        assert!(!wire.on_battery);
        assert_eq!(wire.percentage, 41);
        assert_eq!(wire.charge_state, "charging");
        assert_eq!(wire.profile, "balanced");
        assert_eq!(wire.lid_state, "open");
        // Charging: time-to-full is meaningful, time-to-empty gated to 0.
        assert_eq!(wire.time_to_full_seconds, 1800);
        assert_eq!(wire.time_to_empty_seconds, 0);
    }

    #[tokio::test]
    async fn serve_connection_writes_the_decodable_snapshot() {
        let (server, mut client) = UnixStream::pair().unwrap();
        let snapshot = snapshot_wire(&PowerState::from_upower(
            true, 73.6, 2, 4200, 0, true, false, None,
        ));
        let want = snapshot.clone();
        tokio::spawn(async move {
            serve_connection(server, &snapshot).await;
            // `server` drops here -> EOF for the client read below.
        });
        let mut buf = Vec::new();
        client.read_to_end(&mut buf).await.unwrap();
        let got: PowerSnapshot = serde_json::from_slice(&buf).unwrap();
        assert_eq!(got, want);
    }
}
