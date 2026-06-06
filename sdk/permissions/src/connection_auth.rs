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
        let (peer_pid, peer_uid) =
            so_peercred(stream.as_raw_fd()).map_err(AuthError::PeerCred)?;

        if peer_uid != caller_uid {
            return Err(AuthError::CrossUid {
                caller: caller_uid,
                peer: peer_uid,
            });
        }

        let pid = peer_pid;
        let app_id = app_id_from_pid(pid)?;
        let start_time = pid_start_time(pid)?;
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
        })
    }

    /// Re-verify that the original peer process is still alive
    /// AND has the same start_time (catches PID recycling).
    /// Brokers call this before honoring each request; on
    /// failure the connection should be dropped.
    pub fn verify_alive(&self) -> Result<(), AuthError> {
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

    /// Peer uid (always equals `caller_uid` from extract_from
    /// — cross-uid is rejected).
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
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
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
}
