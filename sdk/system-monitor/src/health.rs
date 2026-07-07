//! The daemon-health verdict (`system-monitor-plan.md`): "is the system OK, and if
//! not, the one broken thing" - a liveness probe over the core Arlen daemons'
//! sockets reduced to one plain verdict. Not a metrics dashboard; the sovereign
//! question "are my own services intact".

use std::path::PathBuf;

/// A core Arlen daemon whose liveness the health verdict probes.
pub struct DaemonSpec {
    /// The human-facing daemon name.
    pub name: &'static str,
    /// Its socket file name under the Arlen runtime dir.
    pub socket: &'static str,
}

/// The core daemons the health verdict covers - the ones whose absence means the
/// desktop's own services are down. Extensible; each is probed independently.
pub const CORE_DAEMONS: &[DaemonSpec] = &[
    DaemonSpec { name: "event bus", socket: "event-bus-consumer.sock" },
    DaemonSpec { name: "knowledge graph", socket: "knowledge.sock" },
    DaemonSpec { name: "audit ledger", socket: "audit-ingest.sock" },
    DaemonSpec { name: "notifications", socket: "notification.sock" },
    DaemonSpec { name: "module runtime", socket: "modulesd.sock" },
];

/// One daemon's probed liveness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonStatus {
    /// The human-facing daemon name.
    pub name: String,
    /// Whether the daemon's socket accepted a connection.
    pub healthy: bool,
}

/// The overall daemon-health verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthVerdict {
    /// Every probed daemon answered. `count` is how many.
    AllHealthy {
        /// The number of healthy daemons.
        count: usize,
    },
    /// One or more daemons did not answer, named in `down`.
    Degraded {
        /// The daemons that did not answer (the "one broken thing" to surface).
        down: Vec<String>,
        /// How many of the rest are up.
        healthy: usize,
    },
}

/// Reduce probed daemon statuses to the health verdict (pure). Any down daemon
/// degrades the verdict and is named, so the surface can show "the one broken
/// thing" rather than a wall of green.
pub fn health_verdict(statuses: &[DaemonStatus]) -> HealthVerdict {
    let down: Vec<String> =
        statuses.iter().filter(|s| !s.healthy).map(|s| s.name.clone()).collect();
    let healthy = statuses.len() - down.len();
    if down.is_empty() {
        HealthVerdict::AllHealthy { count: healthy }
    } else {
        HealthVerdict::Degraded { down, healthy }
    }
}

/// Resolve the canonical Arlen runtime socket path for `file_name`
/// (`$XDG_RUNTIME_DIR/arlen/<name>` else `/run/arlen/<name>`), matching the SDK
/// runtime-path convention.
fn runtime_socket_path(file_name: &str) -> PathBuf {
    match std::env::var("XDG_RUNTIME_DIR") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir).join("arlen").join(file_name),
        _ => PathBuf::from("/run/arlen").join(file_name),
    }
}

/// Probe one daemon's liveness by CONNECTING its socket. Connect (not mere file
/// existence) so a stale socket left by a crashed daemon reads as down; a local
/// connect resolves without a round-trip, so no timeout is needed.
pub async fn probe_daemon(socket: &str) -> bool {
    tokio::net::UnixStream::connect(runtime_socket_path(socket)).await.is_ok()
}

/// Probe every core daemon and reduce to the health verdict.
pub async fn daemon_health() -> HealthVerdict {
    let mut statuses = Vec::with_capacity(CORE_DAEMONS.len());
    for spec in CORE_DAEMONS {
        statuses.push(DaemonStatus {
            name: spec.name.to_string(),
            healthy: probe_daemon(spec.socket).await,
        });
    }
    health_verdict(&statuses)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(name: &str, healthy: bool) -> DaemonStatus {
        DaemonStatus { name: name.to_string(), healthy }
    }

    #[test]
    fn all_up_is_all_healthy() {
        let v = health_verdict(&[status("a", true), status("b", true)]);
        assert_eq!(v, HealthVerdict::AllHealthy { count: 2 });
    }

    #[test]
    fn a_down_daemon_degrades_and_is_named() {
        let v = health_verdict(&[status("event bus", true), status("knowledge graph", false)]);
        assert_eq!(
            v,
            HealthVerdict::Degraded { down: vec!["knowledge graph".to_string()], healthy: 1 }
        );
    }

    #[test]
    fn an_empty_probe_is_trivially_healthy() {
        assert_eq!(health_verdict(&[]), HealthVerdict::AllHealthy { count: 0 });
    }

    #[tokio::test]
    async fn a_missing_socket_probes_down() {
        // No daemon bound this name in the test env, so connect fails -> down.
        assert!(!probe_daemon("definitely-not-bound-xyz.sock").await);
    }
}
