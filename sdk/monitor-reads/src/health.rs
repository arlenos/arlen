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
    /// The environment variable a launcher sets to pin this socket, where one
    /// exists. Probing the default paths while a launcher has pinned the socket
    /// somewhere else reports a live daemon as down, which is the failure this
    /// whole probe is meant to detect rather than produce.
    pub env: Option<&'static str>,
}

/// The core daemons the health verdict covers - the ones whose absence means the
/// desktop's own services are down. Extensible; each is probed independently.
pub const CORE_DAEMONS: &[DaemonSpec] = &[
    DaemonSpec {
        name: "event bus",
        socket: "event-bus-consumer.sock",
        env: Some("ARLEN_CONSUMER_SOCKET"),
    },
    DaemonSpec {
        name: "knowledge graph",
        socket: "knowledge.sock",
        env: Some("ARLEN_KNOWLEDGE_SOCKET"),
    },
    // No override variable for these three today. `None` says so rather than
    // guessing a name: an invented variable nobody sets would read as support
    // for pinning that does not exist.
    DaemonSpec { name: "audit ledger", socket: "audit-ingest.sock", env: None },
    DaemonSpec { name: "notifications", socket: "notification.sock", env: None },
    DaemonSpec {
        name: "module runtime",
        socket: "modulesd.sock",
        env: Some("ARLEN_MODULESD_SOCKET"),
    },
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

/// Where `file_name` might be, in the order to try.
///
/// Both places, not one or the other. The previous version took
/// `$XDG_RUNTIME_DIR/arlen/<name>` whenever XDG was set and only reached
/// `/run/arlen/<name>` when it was not - two alternatives rather than two
/// candidates. In a dev session that is right; on a booted image, where the
/// daemons are system services binding under `/run/arlen` and the probe runs in a
/// session that does have XDG set, it checks a directory nothing binds into and
/// **every core daemon reads as down** - event bus, knowledge, audit,
/// notifications, module runtime. A health panel calling a healthy system dead is
/// worse than no panel, because it sends someone debugging the wrong thing.
///
/// The session path stays first: a per-user daemon should win over a system one
/// of the same name.
fn runtime_socket_candidates(env: Option<&str>, file_name: &str) -> Vec<PathBuf> {
    let pinned = env.and_then(|name| std::env::var(name).ok());
    candidates_from(
        pinned.as_deref(),
        std::env::var("XDG_RUNTIME_DIR").ok().as_deref(),
        file_name,
    )
}

/// The candidate list with the environment passed in, so the order can be tested
/// without setting a process-wide variable - which is flaky under a parallel run
/// and changes what a neighbouring test resolves. `os-sdk`'s resolver is split the
/// same way, for the same reason.
fn candidates_from(pinned: Option<&str>, xdg: Option<&str>, file_name: &str) -> Vec<PathBuf> {
    // A pinned path is an instruction, not a hint: if a launcher named it, that is
    // where the daemon is, and probing anywhere else would answer a different
    // question.
    if let Some(p) = pinned.filter(|p| !p.is_empty()) {
        return vec![PathBuf::from(p)];
    }
    let mut candidates = Vec::with_capacity(2);
    if let Some(dir) = xdg.filter(|d| !d.is_empty()) {
        candidates.push(PathBuf::from(dir).join("arlen").join(file_name));
    }
    candidates.push(PathBuf::from("/run/arlen").join(file_name));
    candidates
}

/// Probe one daemon's liveness by CONNECTING its socket. Connect (not mere file
/// existence) so a stale socket left by a crashed daemon reads as down; a local
/// connect resolves without a round-trip, so no timeout is needed.
///
/// Healthy if EITHER candidate connects. Connecting is what makes trying both
/// safe: a path that is absent, or holds a stale socket, simply fails and the
/// next one is tried, so this cannot report a dead daemon as alive.
pub async fn probe_daemon(spec: &DaemonSpec) -> bool {
    for candidate in runtime_socket_candidates(spec.env, spec.socket) {
        if tokio::net::UnixStream::connect(candidate).await.is_ok() {
            return true;
        }
    }
    false
}

/// Probe every core daemon and reduce to the health verdict.
pub async fn daemon_health() -> HealthVerdict {
    let mut statuses = Vec::with_capacity(CORE_DAEMONS.len());
    for spec in CORE_DAEMONS {
        statuses.push(DaemonStatus {
            name: spec.name.to_string(),
            healthy: probe_daemon(spec).await,
        });
    }
    health_verdict(&statuses)
}

#[cfg(test)]
mod tests {

    /// The system path is always a candidate, even in a session that has XDG.
    /// It was not, and that is why a booted image reported five live daemons as
    /// down: they bind under `/run/arlen`, the probe runs in a session, and the
    /// session path was taken as an alternative rather than as a first try.
    #[test]
    fn the_system_path_is_always_a_candidate() {
        assert_eq!(
            candidates_from(None, Some("/run/user/1000"), "knowledge.sock"),
            vec![
                PathBuf::from("/run/user/1000/arlen/knowledge.sock"),
                PathBuf::from("/run/arlen/knowledge.sock"),
            ],
            "the session path first, the system path still tried"
        );
    }

    /// No XDG at all, and an empty XDG, both mean the same thing: one candidate,
    /// the system path. An empty string is how an environment says "unset".
    #[test]
    fn without_a_session_runtime_dir_only_the_system_path_remains() {
        let expected = vec![PathBuf::from("/run/arlen/audit-ingest.sock")];
        assert_eq!(candidates_from(None, None, "audit-ingest.sock"), expected);
        assert_eq!(candidates_from(None, Some(""), "audit-ingest.sock"), expected);
    }

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

    /// A pinned path is the only candidate, so a launcher that moves a socket is
    /// believed rather than second-guessed.
    #[test]
    fn a_pinned_path_is_the_only_candidate() {
        assert_eq!(
            candidates_from(Some("/tmp/pinned.sock"), Some("/run/user/1000"), "knowledge.sock"),
            vec![PathBuf::from("/tmp/pinned.sock")]
        );
    }

    #[tokio::test]
    async fn a_missing_socket_probes_down() {
        // No daemon bound this name in the test env, so connect fails -> down.
        let spec = DaemonSpec {
            name: "nothing",
            socket: "definitely-not-bound-xyz.sock",
            env: None,
        };
        assert!(!probe_daemon(&spec).await);
    }
}
