//! `arlen-journald-parser` - the systemd-journal Tier-2 ingestion daemon
//! (foundation-gap-audit "journald per-service parser").
//!
//! Follows the system journal for three services (NetworkManager, bluetoothd,
//! systemd-logind) via `journalctl --follow --output=json` and emits a coarse
//! `system.service` event per recognised transition onto the event bus. The
//! classification is the pure [`arlen_journald_parser::classify`] core; this
//! host owns only the subprocess lifecycle, line reading and the emit.
//!
//! Per-user daemon (it emits onto this user's per-uid event-bus producer
//! socket, mirroring the power daemon's "per-user daemon over a system source"
//! shape). It reads the SYSTEM journal, which needs journal-read access; the
//! shipped unit grants it through `SupplementaryGroups=systemd-journal`. If the
//! journal is unreadable, `journalctl` simply yields nothing and the daemon
//! idles (no crash); that is a deployment-permission state, not a code fault.
//!
//! Privacy: the daemon emits only what [`classify`] constructs - coarse
//! transitions with non-sensitive detail (an interface name, a session id),
//! never an SSID or a credential (the movement-profile line system-services
//! draws). Raw-tier ingestion only; nothing here promotes to a graph node.

use std::time::Duration;

use arlen_journald_parser::classify::{self, ServiceEvent};
use os_sdk::event::{EventEmitter, UnixEventEmitter};
use prost::Message as _;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{debug, info, warn};

/// The journal units this tier follows. Bounding the follow to these three
/// keeps the read volume small and matches the gap-audit's named set.
const FOLLOWED_UNITS: &[&str] = &[
    "NetworkManager.service",
    "bluetooth.service",
    "systemd-logind.service",
];

/// The event type every classified transition is published under.
const EVENT_TYPE: &str = "system.service";

/// Backoff before respawning `journalctl` after it exits or fails to start, so
/// a transient journal/permission hiccup retries without a hot loop.
const RESPAWN_BACKOFF: Duration = Duration::from_secs(5);

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let producer = os_sdk::runtime::socket_path("ARLEN_PRODUCER_SOCKET", "event-bus-producer.sock");
    info!(socket = %producer.display(), "journald-parser starting");
    let emitter = UnixEventEmitter::new(producer.to_string_lossy().into_owned());

    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]);

    tokio::select! {
        _ = follow_journal(&emitter) => {}
        _ = shutdown_signal() => info!("journald-parser shutting down"),
    }
}

/// Follow the journal indefinitely, respawning `journalctl` with backoff if it
/// exits (a journal rotation, a transient error). Returns only on a fatal spawn
/// path that cannot recover, which in practice does not happen (the loop always
/// retries), so the caller treats this as "runs until the shutdown signal".
async fn follow_journal(emitter: &UnixEventEmitter) {
    loop {
        match run_once(emitter).await {
            Ok(()) => warn!("journalctl exited; respawning after backoff"),
            Err(e) => warn!("journalctl follow failed: {e}; respawning after backoff"),
        }
        tokio::time::sleep(RESPAWN_BACKOFF).await;
    }
}

/// Spawn one `journalctl --follow` and pump its lines until it exits.
async fn run_once(emitter: &UnixEventEmitter) -> std::io::Result<()> {
    let mut cmd = Command::new("journalctl");
    cmd.arg("--follow")
        .arg("--output=json")
        // Only new entries: this is a live ingestion tier, not a backfill.
        .arg("--since=now")
        // Never page or wait for a pager; stream straight to stdout.
        .arg("--no-pager")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null());
    for unit in FOLLOWED_UNITS {
        cmd.arg("-u").arg(unit);
    }
    // Reap the journalctl child if this daemon is killed.
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("journalctl stdout missing"))?;
    let mut lines = BufReader::new(stdout).lines();

    while let Some(line) = lines.next_line().await? {
        if let Some(parsed) = classify::parse_line(&line) {
            if let Some(event) = classify::classify(&parsed) {
                emit_event(emitter, &event).await;
            }
        }
    }

    // Stream ended: journalctl exited. Reap it and let the caller respawn.
    let _ = child.wait().await;
    Ok(())
}

/// Encode and publish a classified transition. Best-effort: an emit failure
/// (event bus down) is logged, never fatal - the next event reconnects.
async fn emit_event(emitter: &UnixEventEmitter, event: &ServiceEvent) {
    let payload = os_sdk::proto::ServiceEventPayload {
        service: event.service.clone(),
        kind: event.kind.clone(),
        detail: event.detail.clone(),
    }
    .encode_to_vec();
    match emitter.emit(EVENT_TYPE, payload).await {
        Ok(()) => debug!(service = %event.service, kind = %event.kind, "published system.service"),
        Err(e) => warn!("system.service emit failed: {e}"),
    }
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
