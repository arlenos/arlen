//! Connection-scoped peer authentication for IPC brokers.
//!
//! The shape every broker hosted by desktop-shell instantiates
//! per accepted Unix-socket connection. Resolves the peer's
//! `app_id` from `SO_PEERCRED + /proc`, loads the user's
//! permission profile, and projects scopes the broker can
//! match against per request.
//!
//! See `docs/architecture/peer-auth-system.md`.

use std::os::unix::io::AsRawFd;

use thiserror::Error;

use crate::identity::{
    app_id_from_pid, pid_start_time, IdentityError,
};
use crate::{load_profile, PermissionError, PermissionProfile};

/// Errors from connection-time auth setup.
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("peer credentials unavailable: {0}")]
    PeerCred(std::io::Error),
    #[error("cross-uid IPC not supported: caller_uid={caller}, peer_uid={peer}")]
    CrossUid { caller: u32, peer: u32 },
    #[error("identity resolution: {0}")]
    Identity(#[from] IdentityError),
    #[error("permission profile: {0}")]
    Profile(PermissionError),
    #[error("peer process exited or PID recycled")]
    PeerNotAlive,
}

impl From<PermissionError> for AuthError {
    fn from(e: PermissionError) -> Self {
        // NotFound = no profile = no scopes; treated as a valid
        // "default deny" state so the broker can still gate per
        // request. Other errors propagate as profile failures.
        match e {
            PermissionError::NotFound { .. } => AuthError::Profile(e),
            _ => AuthError::Profile(e),
        }
    }
}

/// Identity + scope state held by a broker per accepted
/// connection. Cheap to clone the resolved fields if the
/// broker hands the auth off to a request task; the inner
/// state itself is `!Clone` because of the OwnedFd-style
/// invariant on the start_time tuple.
#[derive(Debug)]
pub struct ConnectionAuth {
    pid: u32,
    uid: u32,
    start_time: u64,
    app_id: String,
    profile: PermissionProfile,
    /// The pinned peer pidfd, retained ONLY on the enforce path so `verify_alive`
    /// is race-free (`PeerPidfd::is_alive`) for the life of the connection. `None`
    /// on the shadow/legacy path, which falls back to the `/proc` start_time
    /// recheck.
    pidfd: Option<crate::peer_pidfd::PeerPidfd>,
}

impl ConnectionAuth {
    /// Extract identity + permissions from a freshly-accepted
    /// Unix socket fd. Generic over anything that exposes the
    /// raw fd so it works for both `std::os::unix::net::UnixStream`
    /// and `tokio::net::UnixStream` (no `peer_cred()` requirement,
    /// which is unstable as of Rust 1.90).
    ///
    /// `caller_uid` is whoever the broker runs as (typically
    /// `getuid()`); cross-uid IPC is rejected.
    pub fn extract_from<F: AsRawFd>(
        stream: &F,
        caller_uid: u32,
    ) -> Result<Self, AuthError> {
        Self::extract_from_inner(stream, caller_uid, false)
    }

    /// Like [`extract_from`](Self::extract_from) but ALSO accepts a peer
    /// running as root (uid 0), a principal strictly more-privileged than
    /// `caller_uid`. Used by the user auditd's ingest socket so a trusted
    /// ROOT system daemon (the config-broker, which must run as a separate
    /// more-privileged uid so the user's own uid cannot write the AI
    /// master-switch store) can record to the owner's single ledger.
    ///
    /// Sound because root grants itself no new capability by connecting
    /// here: it can already read/tamper the owner's ledger file directly.
    /// The admitted-writer allowlist + the attested identity resolution
    /// still gate the caller, and a same-uid non-root peer is unaffected
    /// (still rejected unless its uid matches). NB this is shadow-mode
    /// correct: the legacy `/proc/exe` resolver ignores uid, so a root
    /// peer resolves normally; the stamped resolver (`PeerPidfd`) still
    /// rejects cross-uid, so a future ENFORCE cutover of this socket must
    /// add a matching root-accepting stamped variant.
    pub fn extract_from_trusting_root<F: AsRawFd>(
        stream: &F,
        caller_uid: u32,
    ) -> Result<Self, AuthError> {
        Self::extract_from_inner(stream, caller_uid, true)
    }

    fn extract_from_inner<F: AsRawFd>(
        stream: &F,
        caller_uid: u32,
        allow_root: bool,
    ) -> Result<Self, AuthError> {
        let (peer_pid, peer_uid) =
            so_peercred(stream.as_raw_fd()).map_err(AuthError::PeerCred)?;

        if peer_uid != caller_uid && !(allow_root && peer_uid == 0) {
            return Err(AuthError::CrossUid {
                caller: caller_uid,
                peer: peer_uid,
            });
        }

        // Legacy resolution: the racy SO_PEERCRED pid -> /proc/exe app_id. Note the
        // missing `?`. It is authoritative in shadow mode and nothing but an
        // observation in enforce mode, so its failure may only fail the connection
        // on the path that actually uses it. See `resolve_identity`.
        let legacy = app_id_from_pid(peer_pid);

        // Stamped-identity resolver (pidfd-pinned, race-free). In shadow mode it is
        // observed for divergence only; in enforce mode it is authoritative and
        // fail-closed. Default is shadow (any ARLEN_STAMPED_IDENTITY != "enforce").
        // NB: this is the same-uid extract_from path; the knowledge daemon's
        // cross-uid AI-daemon resolver is separate and is NOT touched here.
        let mode = stamped_mode();
        let stamped = crate::stamped_identity::app_id_from_connection(stream, caller_uid);
        observe_stamped_divergence(peer_pid, legacy.as_deref(), &stamped, mode);

        let (pid, app_id, start_time, pidfd) =
            resolve_identity(mode, peer_pid, legacy, stamped)?;
        let profile = match load_profile(&app_id) {
            Ok(p) => p,
            Err(PermissionError::NotFound { .. }) => {
                // No profile = default-deny. Construct an empty
                // profile so per-scope checks return false.
                // This is foundation §7.3 semantics: explicit
                // grants only.
                empty_profile(&app_id)
            }
            Err(other) => return Err(AuthError::Profile(other)),
        };

        Ok(Self {
            pid,
            uid: peer_uid,
            start_time,
            app_id,
            profile,
            pidfd,
        })
    }

    /// Re-verify that the original peer process is still alive
    /// AND has the same start_time (catches PID recycling).
    /// Brokers call this before honoring each request; on
    /// failure the connection should be dropped.
    pub fn verify_alive(&self) -> Result<(), AuthError> {
        // Enforce path: the retained pidfd gives race-free liveness. The pin refers
        // to exactly the process that connected, so a recycled pid cannot
        // masquerade as alive (no start_time comparison to win).
        if let Some(pidfd) = &self.pidfd {
            return if pidfd.is_alive() {
                Ok(())
            } else {
                Err(AuthError::PeerNotAlive)
            };
        }
        // Legacy/shadow path: the /proc start_time recheck (detects recycling
        // after the fact, the pre-pidfd guard).
        let now_start = match pid_start_time(self.pid) {
            Ok(t) => t,
            Err(IdentityError::ProcessNotFound(_)) => {
                return Err(AuthError::PeerNotAlive)
            }
            Err(other) => return Err(AuthError::Identity(other)),
        };
        if now_start != self.start_time {
            return Err(AuthError::PeerNotAlive);
        }
        Ok(())
    }

    /// Resolved app id (from `/proc/{pid}/exe` mapping). Stable
    /// for the lifetime of this auth — does not change even on
    /// permission.changed events.
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// Peer pid. Use only for logging/audit; per-request gating
    /// must go through `verify_alive` + scope checks.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Peer uid. Equals `caller_uid` on the [`extract_from`](Self::extract_from)
    /// path (cross-uid rejected); on [`extract_from_trusting_root`](Self::extract_from_trusting_root)
    /// it is `caller_uid` or 0 (a more-privileged root producer).
    pub fn uid(&self) -> u32 {
        self.uid
    }

    /// Current cached permission profile. Re-load via
    /// `refresh_profile` after a `permission.changed` event
    /// for this app_id.
    pub fn profile(&self) -> &PermissionProfile {
        &self.profile
    }

    /// Test-only constructor: build a `ConnectionAuth` with a
    /// pre-supplied app_id + profile, bypassing SO_PEERCRED
    /// extraction. The pid + start_time are the calling test
    /// process's own values so `verify_alive` succeeds.
    ///
    /// Use only in tests — production code MUST go through
    /// [`Self::extract_from`] so identity is kernel-attested.
    #[doc(hidden)]
    pub fn for_test(app_id: impl Into<String>, profile: PermissionProfile) -> Self {
        let pid = std::process::id();
        let start_time = pid_start_time(pid).unwrap_or(0);
        // SAFETY: getuid() never fails.
        let uid = unsafe { libc::getuid() };
        Self {
            pid,
            uid,
            start_time,
            app_id: app_id.into(),
            profile,
            // Tests use the start_time recheck path (no pinned pidfd).
            pidfd: None,
        }
    }

    /// Re-load the permission profile from disk. Called when a
    /// `permission.changed` event arrives for this app_id.
    /// Identity (pid + start_time + app_id) stays unchanged.
    pub fn refresh_profile(&mut self) -> Result<(), AuthError> {
        match load_profile(&self.app_id) {
            Ok(p) => self.profile = p,
            Err(PermissionError::NotFound { .. }) => {
                self.profile = empty_profile(&self.app_id);
            }
            Err(other) => return Err(AuthError::Profile(other)),
        }
        Ok(())
    }
}

/// Whether the pidfd-stamped identity resolver runs observe-only (shadow, the
/// default) or is authoritative + fail-closed (enforce). Selected per process by
/// `ARLEN_STAMPED_IDENTITY`; any value other than `enforce` (including unset) is
/// shadow, the no-behavior-change default, so a typo never silently enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StampedMode {
    Shadow,
    Enforce,
}

/// Pick the connection's identity from the two resolvers, and decide which of them
/// is allowed to fail the connection.
///
/// Shadow keeps the legacy `/proc/exe` app_id, so a legacy failure is fatal there:
/// there is nothing else to be. Enforce takes the app_id + pid from under the pidfd
/// pin and retains the pin, so `verify_alive` is race-free for the life of the
/// connection rather than only at accept; the legacy value is consumed by nothing
/// but `observe_stamped_divergence`, and so it is dropped here, error and all.
///
/// That asymmetry is the point of this function. `app_id_from_pid` reads
/// `/proc/<pid>/exe`, which is ptrace-gated: a daemon hardened with a mount-namespace
/// directive (`PrivateTmp`, `ProtectHome`, `ReadWritePaths`, any of them) still
/// publishes a readable `/proc/<pid>/stat` but denies the exe link. Measured
/// 11 Aug 2026 against the shipped units. So on a hardened peer the legacy resolver
/// fails while the pidfd resolver does not, and letting the observation propagate
/// would refuse exactly the peers the stamped path exists to serve.
fn resolve_identity(
    mode: StampedMode,
    peer_pid: u32,
    legacy: Result<String, IdentityError>,
    stamped: Result<crate::stamped_identity::StampedIdentity, AuthError>,
) -> Result<(u32, String, u64, Option<crate::peer_pidfd::PeerPidfd>), AuthError> {
    match mode {
        StampedMode::Shadow => Ok((peer_pid, legacy?, pid_start_time(peer_pid)?, None)),
        StampedMode::Enforce => {
            let s = stamped?;
            let app_id = s.app_id().to_string();
            let peer = s.into_peer();
            let spid = peer.pid();
            let start_time = pid_start_time(spid)?;
            Ok((spid, app_id, start_time, Some(peer)))
        }
    }
}

/// Pure parse of the mode env value, so the default-safe semantics are testable
/// without touching the process environment.
fn parse_stamped_mode(value: Option<&str>) -> StampedMode {
    match value {
        Some("enforce") => StampedMode::Enforce,
        _ => StampedMode::Shadow,
    }
}

fn stamped_mode() -> StampedMode {
    parse_stamped_mode(std::env::var("ARLEN_STAMPED_IDENTITY").ok().as_deref())
}

/// Emit an audit log line when the pidfd-stamped resolver disagrees with the legacy
/// `/proc` resolution, or when it could not run at all. Observation only (in both
/// modes): the shadow rollout confirms zero divergence over a dogfood session before
/// any socket is flipped to enforce. Routed to the `audit` tracing target so a
/// daemon's journald pipeline captures it.
fn observe_stamped_divergence(
    pid: u32,
    legacy: Result<&str, &IdentityError>,
    stamped: &Result<crate::stamped_identity::StampedIdentity, AuthError>,
    mode: StampedMode,
) {
    match (legacy, stamped) {
        (Ok(l), Ok(s)) if s.app_id() != l => tracing::warn!(
            target: "audit",
            event = "identity.divergence",
            pid,
            legacy = l,
            stamped = s.app_id(),
            source = ?s.source(),
            mode = ?mode,
            "stamped identity diverges from the legacy /proc resolution"
        ),
        (Ok(_), Ok(_)) => {}
        // The two resolvers cannot be compared, but the connection survives under
        // enforce. Logged rather than dropped because it is the shape a hardened
        // peer produces, and reading it as noise is how the cutover would be talked
        // out of hardening the daemons it is meant to allow.
        (Err(le), Ok(s)) => tracing::info!(
            target: "audit",
            event = "identity.legacy_unavailable",
            pid,
            stamped = s.app_id(),
            source = ?s.source(),
            mode = ?mode,
            error = %le,
            "legacy /proc resolution unavailable; stamped identity resolved"
        ),
        (_, Err(e)) => tracing::warn!(
            target: "audit",
            event = "identity.stamped_unavailable",
            pid,
            legacy = legacy.ok(),
            mode = ?mode,
            error = %e,
            "stamped identity resolver could not run"
        ),
    }
}

/// Read the peer's `(pid, uid)` from `SO_PEERCRED` WITHOUT resolving its binary
/// identity. For endpoints whose authentication is a presented credential (e.g. a
/// session token bound to the attested pid) rather than the peer's executable: a
/// peer running a generic interpreter (node, python) - which [`ConnectionAuth::
/// extract_from`]'s binary resolution rejects as `UnknownBinary` - is still
/// admissible under same-uid plus its own credential check. The CALLER must
/// enforce same-uid (`uid` == the daemon's) and verify the credential itself; this
/// only reads the kernel-attested pid/uid.
pub fn peer_credentials<F: AsRawFd>(stream: &F) -> std::io::Result<(u32, u32)> {
    so_peercred(stream.as_raw_fd())
}

/// `SO_PEERCRED` getsockopt wrapper. Returns `(pid, uid)`. We
/// use libc directly because `std::os::unix::net::UnixStream::
/// peer_cred()` is unstable as of Rust 1.90 (issue #42839).
fn so_peercred(fd: libc::c_int) -> std::io::Result<(u32, u32)> {
    // ucred layout (Linux): pid_t pid; uid_t uid; gid_t gid;
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len: libc::socklen_t = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: cred and len are valid pointers for the duration
    // of getsockopt; fd is a borrowed valid socket.
    let r = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if r != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if cred.pid <= 0 {
        return Err(std::io::Error::other(
            "SO_PEERCRED returned non-positive pid",
        ));
    }
    Ok((cred.pid as u32, cred.uid))
}

/// Build an empty profile (no permissions) for an app id with
/// no profile file. Used by extract_from + refresh_profile
/// when the file is absent — explicit-deny semantics.
fn empty_profile(app_id: &str) -> PermissionProfile {
    use crate::{AppTier, ProfileInfo};
    PermissionProfile {
        info: ProfileInfo {
            app_id: app_id.to_string(),
            tier: AppTier::ThirdParty,
        },
        graph: Default::default(),
        event_bus: Default::default(),
        filesystem: Default::default(),
        network: Default::default(),
        notifications: Default::default(),
        clipboard: Default::default(),
        system: Default::default(),
        input: Default::default(),
        search: Default::default(),
        intents: Default::default(),
        mcp: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanity: an empty-profile shell has no clipboard scopes.
    #[test]
    fn empty_profile_has_no_clipboard_scopes() {
        let p = empty_profile("com.unknown");
        assert!(!p.clipboard.read);
        assert!(!p.clipboard.write);
        assert!(!p.clipboard.read_sensitive);
        assert!(!p.clipboard.history);
    }

    // Live SO_PEERCRED tests can't run without a real socket
    // pair; leave full integration tests for the broker side
    // (clipboard_ipc tests in desktop-shell, which spin up a
    // UnixListener and connect-pair).

    /// The mode env parses default-safe: only the exact `enforce` enforces;
    /// unset, `shadow`, and any typo all stay shadow (no silent enforcement).
    #[test]
    fn stamped_mode_defaults_to_shadow() {
        assert_eq!(parse_stamped_mode(None), StampedMode::Shadow);
        assert_eq!(parse_stamped_mode(Some("shadow")), StampedMode::Shadow);
        assert_eq!(parse_stamped_mode(Some("Enforce")), StampedMode::Shadow);
        assert_eq!(parse_stamped_mode(Some("")), StampedMode::Shadow);
        assert_eq!(parse_stamped_mode(Some("enforce")), StampedMode::Enforce);
    }

    /// A daemon hardened with a mount-namespace directive denies `/proc/<pid>/exe`
    /// while `/proc/<pid>/stat` stays readable, so `app_id_from_pid` fails on a peer
    /// the pidfd resolver reads without trouble. Under enforce the legacy value is an
    /// observation and nothing else, so that failure must not reach the caller — the
    /// whole point of the cutover is to admit those peers.
    #[test]
    fn enforce_survives_a_legacy_resolution_that_failed() {
        use std::os::unix::net::UnixStream;
        let (a, _b) = UnixStream::pair().expect("socketpair");
        // SAFETY: getuid never fails.
        let uid = unsafe { libc::getuid() };
        let stamped = crate::stamped_identity::app_id_from_connection(&a, uid);
        assert!(stamped.is_ok(), "stamped resolver must work for self");
        let denied = IdentityError::CannotReadExe {
            pid: std::process::id(),
            source: std::io::Error::from_raw_os_error(libc::EACCES),
            why: " (stat readable)".to_string(),
        };

        let (pid, app_id, _start, pidfd) = resolve_identity(
            StampedMode::Enforce,
            std::process::id(),
            Err(denied),
            stamped,
        )
        .expect("enforce must not consult the legacy resolver");
        assert_eq!(pid, std::process::id());
        assert!(!app_id.is_empty());
        assert!(pidfd.is_some(), "enforce retains the pin for verify_alive");
    }

    /// The mirror: shadow has nothing but the legacy value, so there the same failure
    /// is fatal. Asserting both halves is what keeps the asymmetry deliberate.
    #[test]
    fn shadow_still_fails_when_the_legacy_resolution_failed() {
        use std::os::unix::net::UnixStream;
        let (a, _b) = UnixStream::pair().expect("socketpair");
        // SAFETY: getuid never fails.
        let uid = unsafe { libc::getuid() };
        let denied = IdentityError::UnknownBinary("/nowhere/arlen-probe".into());

        let r = resolve_identity(
            StampedMode::Shadow,
            std::process::id(),
            Err(denied),
            crate::stamped_identity::app_id_from_connection(&a, uid),
        );
        assert!(matches!(r, Err(AuthError::Identity(_))));
    }

    /// In shadow mode (the default) extract_from over a real socketpair still
    /// returns the legacy /proc resolution unchanged: the stamped resolver runs
    /// only for observation. The peer is this process, so both resolvers agree.
    #[test]
    fn shadow_extract_from_returns_the_legacy_identity() {
        use std::os::unix::net::UnixStream;
        let (a, _b) = UnixStream::pair().expect("socketpair");
        // SAFETY: getuid never fails.
        let uid = unsafe { libc::getuid() };
        let auth = ConnectionAuth::extract_from(&a, uid).expect("auth");
        assert_eq!(auth.uid(), uid);
        assert_eq!(auth.pid(), std::process::id());
        // Shadow keeps the legacy app_id; for this process it equals the stamped one.
        let legacy = app_id_from_pid(std::process::id()).expect("legacy app id");
        assert_eq!(auth.app_id(), legacy);
    }
}
