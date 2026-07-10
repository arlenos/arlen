//! `SO_PEERPIDFD` peer authentication for security-load-bearing IPC.
//!
//! The kernel-6.5+ upgrade of [`crate::connection_auth`]'s
//! `SO_PEERCRED` + `/proc/{pid}/stat` start-time re-check. A bare
//! `SO_PEERCRED` pid can be recycled between the `getsockopt` and a
//! later `/proc` read; the start-time guard only *detects* the
//! recycle after the fact. `SO_PEERPIDFD` instead hands back a
//! **pidfd that pins the peer process**: while the broker holds it
//! the pid cannot be reused for a different process, so the pid read
//! from the pidfd is race-free and liveness is the pidfd's own state
//! (no stat comparison to win).
//!
//! Every credential-lookup failure maps to an error the caller MUST
//! treat as DENY (the polkit CVE-2021-3560 anti-pattern: a mid-check
//! disconnect must never grant). This is the auth shape the
//! separate-uid config/secrets daemon that owns the AI master
//! switches uses (`same-uid-isolation-plan.md` Tier-A #1); it
//! generalises the knowledge daemon's `SO_PEERCRED`'d typed-write
//! socket. `SO_PEERCRED` still supplies the uid (authentic at
//! connect); only the racy pid from that same call is discarded in
//! favour of the pidfd's.

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use thiserror::Error;

/// A failure during pidfd-based peer authentication. EVERY variant
/// means the caller must refuse: a credential lookup that did not
/// cleanly succeed never authorises.
#[derive(Debug, Error)]
pub enum PidfdError {
    /// `getsockopt(SO_PEERPIDFD)` failed. On a kernel < 6.5 this is
    /// `ENOPROTOOPT`/`EINVAL`; if the peer already exited it is
    /// `ENODATA`. Either way: deny.
    #[error("SO_PEERPIDFD unavailable: {0}")]
    Getsockopt(std::io::Error),
    /// The peer uid could not be read via `SO_PEERCRED`.
    #[error("peer uid unavailable: {0}")]
    PeerUid(std::io::Error),
    /// The pidfd's `/proc/self/fdinfo` entry could not be read or
    /// parsed, or reported a dead process (`Pid: -1`).
    #[error("pidfd info unavailable")]
    PidfdInfo,
    /// The authenticated peer uid is not the uid the broker runs as;
    /// cross-uid IPC is refused.
    #[error("cross-uid IPC refused: caller={caller}, peer={peer}")]
    CrossUid { caller: u32, peer: u32 },
}

/// A kernel-attested, recycle-proof handle to the peer of an
/// accepted Unix-socket connection. Holds the pidfd open for its
/// lifetime so the pinned process cannot be recycled out from under
/// the broker.
#[derive(Debug)]
pub struct PeerPidfd {
    /// Kept open to pin the peer process (the recycle guarantee);
    /// read for liveness via `/proc/self/fdinfo`.
    pidfd: OwnedFd,
    pid: u32,
    uid: u32,
}

impl PeerPidfd {
    /// Authenticate the peer of a freshly-accepted Unix socket via
    /// `SO_PEERPIDFD` (+ `SO_PEERCRED` for the uid), rejecting a
    /// cross-uid peer. Fails closed on every lookup error.
    ///
    /// `caller_uid` is whoever the broker runs as (typically
    /// `getuid()`). Generic over anything exposing a raw fd so it
    /// works for both `std` and `tokio` `UnixStream`.
    pub fn from_socket<F: AsRawFd>(
        stream: &F,
        caller_uid: u32,
    ) -> Result<Self, PidfdError> {
        let fd = stream.as_raw_fd();
        let pidfd = peer_pidfd(fd).map_err(PidfdError::Getsockopt)?;
        let uid = peer_uid(fd).map_err(PidfdError::PeerUid)?;
        if uid != caller_uid {
            return Err(PidfdError::CrossUid {
                caller: caller_uid,
                peer: uid,
            });
        }
        let pid = pidfd_pid(pidfd.as_raw_fd()).ok_or(PidfdError::PidfdInfo)?;
        Ok(Self { pidfd, pid, uid })
    }

    /// The peer pid, read from the pinned pidfd (race-free, unlike a
    /// bare `SO_PEERCRED` pid).
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// The peer uid (always equals `caller_uid` from `from_socket` —
    /// cross-uid is rejected).
    pub fn uid(&self) -> u32 {
        self.uid
    }

    /// True iff the pinned peer process is still alive. Race-free:
    /// the held pidfd refers to exactly the original process, so a
    /// recycled pid cannot masquerade as alive. A dead process reads
    /// back `Pid: -1` (or the fdinfo read fails) and returns false.
    /// Brokers call this before honoring each request; on `false`
    /// the connection must be dropped.
    pub fn is_alive(&self) -> bool {
        pidfd_pid(self.pidfd.as_raw_fd()) == Some(self.pid)
    }
}

/// `getsockopt(SO_PEERPIDFD)` → an owned pidfd pinning the peer.
fn peer_pidfd(fd: libc::c_int) -> std::io::Result<OwnedFd> {
    let mut raw: libc::c_int = -1;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: raw + len are valid pointers for the call; fd is a
    // borrowed valid socket.
    let r = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERPIDFD,
            &mut raw as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if r != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if raw < 0 {
        return Err(std::io::Error::other(
            "SO_PEERPIDFD returned an invalid fd",
        ));
    }
    // SAFETY: the kernel just handed us a fresh owned fd we are now
    // responsible for closing.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

/// `getsockopt(SO_PEERCRED)` → peer uid. The pid from the same call
/// is the *racy* one the caller deliberately ignores in favour of
/// the pidfd's; only the uid (authentic at connect) is taken here.
fn peer_uid(fd: libc::c_int) -> std::io::Result<u32> {
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: cred + len are valid for the call; fd is a borrowed
    // valid socket.
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
    Ok(cred.uid)
}

/// Read the peer pid from a pidfd's `/proc/self/fdinfo` entry. The
/// pidfd pins the process, so this pid is race-free; a dead process
/// reports `Pid: -1` and a missing/garbled entry both yield `None`
/// (fail-closed).
fn pidfd_pid(pidfd: libc::c_int) -> Option<u32> {
    let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{pidfd}")).ok()?;
    for line in info.lines() {
        if let Some(rest) = line.strip_prefix("Pid:") {
            let pid: i64 = rest.trim().parse().ok()?;
            return if pid > 0 { Some(pid as u32) } else { None };
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    /// A socketpair's peer is this very process: `SO_PEERPIDFD`
    /// resolves to our own pid/uid and reports alive.
    #[test]
    fn authenticates_own_process_over_a_socket_pair() {
        let (a, _b) = UnixStream::pair().expect("socketpair");
        // SAFETY: getuid never fails.
        let uid = unsafe { libc::getuid() };
        let peer = PeerPidfd::from_socket(&a, uid).expect("auth");
        assert_eq!(peer.pid(), std::process::id());
        assert_eq!(peer.uid(), uid);
        assert!(peer.is_alive());
    }

    /// A caller uid that differs from the peer's is refused (the
    /// cross-uid guard), even though the pidfd lookup itself works.
    #[test]
    fn rejects_a_cross_uid_caller() {
        let (a, _b) = UnixStream::pair().expect("socketpair");
        // SAFETY: getuid never fails.
        let wrong = unsafe { libc::getuid() }.wrapping_add(1);
        match PeerPidfd::from_socket(&a, wrong) {
            Err(PidfdError::CrossUid { .. }) => {}
            other => panic!("expected CrossUid, got {other:?}"),
        }
    }

    /// A non-pidfd / absent fdinfo entry yields `None` rather than a
    /// fabricated pid — the fail-closed parse path.
    #[test]
    fn pidfd_pid_fails_closed_on_a_bad_fd() {
        assert_eq!(pidfd_pid(libc::c_int::MAX), None);
    }
}
