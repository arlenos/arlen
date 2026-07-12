//! Recycle-proof connection-identity resolution (the stamped-identity strand).
//!
//! [`crate::connection_auth::ConnectionAuth`] today resolves the peer app_id from a
//! racy `SO_PEERCRED` pid plus a `/proc/{pid}/stat` start-time re-check: the pid can
//! be recycled between the `getsockopt` and the `/proc` read, and the start-time
//! guard only *detects* the recycle after the fact. [`app_id_from_connection`]
//! replaces that with [`crate::peer_pidfd::PeerPidfd`]: `SO_PEERPIDFD` hands back a
//! pidfd that PINS the peer process, reserving its pid for as long as the pidfd is
//! held. The `/proc/{pid}` reads below still resolve BY PID NUMBER, but under that
//! pin the pid names exactly the pinned process and cannot be recycled to a
//! different one (recycle-proof while held, not a fully pidfd-relative open — the
//! kernel exposes no `readlinkat`-via-pidfd, so path resolution stays by-number, and
//! the inode attestation below is done via `fstatat` on the pinned `/proc/{pid}` fd
//! rather than a re-stat of the exe path string). The resolved app_id is the same
//! one [`crate::identity::path_to_app_id`] produces (including its F3 rule-4 inode
//! gate), carrying an explicit [`IdentitySource`] label for the shadow-mode
//! divergence audit that wires this in.
//!
//! Fail-closed is the whole contract (the polkit CVE-2021-3560 anti-pattern: a
//! mid-check disconnect must never authorise). Every pidfd/registry/exe error maps to
//! an [`AuthError`] the caller MUST treat as DENY; there is no path that returns a
//! usable identity from a failed lookup. Cross-uid is rejected inside `PeerPidfd`
//! before any `/proc` read. No broker yet: this slice produces the Tier-2 inode
//! registry attestation ([`IdentitySource::InodeRegistry`]) and the path-trust
//! fallback ([`IdentitySource::LegacyProc`]); the launcher-stamped Tier-1
//! ([`IdentitySource::Stamped`]) lands with the broker.

use crate::connection_auth::AuthError;
use crate::identity::{exe_ino_dev, exe_path_openat, path_to_app_id};
use crate::identity_registry::IdentityRegistry;
use crate::peer_pidfd::{PeerPidfd, PidfdError};

/// How the app_id in a [`StampedIdentity`] was resolved, strongest to weakest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentitySource {
    /// Launcher-stamped and broker-verified (Tier 1, unforgeable). NOT produced by
    /// this slice (no broker yet); reserved so consumers can match on it now.
    Stamped,
    /// Resolved from the pinned pid's binary AND attested by the root-owned inode
    /// registry: the resolved app is enrolled and its binary's `(ino, dev)` matches
    /// the record installd wrote. A same-uid copy to the app's path has a new inode
    /// and is rejected; a hardlink shares it and passes. The unforgeable same-uid
    /// gate for enrolled apps.
    InodeRegistry,
    /// Resolved from the pinned pid's `/proc/{pid}/exe` canonical path with NO
    /// inode-registry record: a root-owned system-daemon path (rules 1-3,
    /// path-trusted because root owns the directory) or an app not yet enrolled (the
    /// documented pre-enrolment residual, still path-spoofable). Race-free (the pidfd
    /// pins the process) but not inode-attested.
    LegacyProc,
}

/// A kernel-attested identity for the peer of an accepted Unix socket: the resolved
/// `app_id`, the peer uid, how the app_id was attested ([`IdentitySource`]), and the
/// pinned [`PeerPidfd`] held for this value's lifetime so the peer cannot be recycled
/// while a consumer acts on the identity. Liveness is [`Self::is_alive`] on the
/// pinned pidfd (race-free), not a start-time comparison.
#[derive(Debug)]
pub struct StampedIdentity {
    app_id: String,
    peer: PeerPidfd,
    source: IdentitySource,
}

impl StampedIdentity {
    /// The resolved app id (the value the allowlists key on).
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// The peer pid, read race-free from the pinned pidfd.
    pub fn pid(&self) -> u32 {
        self.peer.pid()
    }

    /// The peer uid (always equals the `caller_uid` passed to
    /// [`app_id_from_connection`]; cross-uid was rejected).
    pub fn uid(&self) -> u32 {
        self.peer.uid()
    }

    /// How the app_id was attested.
    pub fn source(&self) -> IdentitySource {
        self.source
    }

    /// Whether the pinned peer process is still alive. Race-free: the held pidfd
    /// refers to exactly the process that connected, so a recycled pid cannot
    /// masquerade as alive. Consumers call this before honoring each request.
    pub fn is_alive(&self) -> bool {
        self.peer.is_alive()
    }
}

/// Resolve the identity of a freshly-accepted Unix socket's peer, race-free and
/// fail-closed. `caller_uid` is whoever the daemon runs as (typically `getuid()`);
/// a cross-uid peer is rejected. Every failure is an [`AuthError`] the caller MUST
/// treat as DENY.
pub fn app_id_from_connection<F: std::os::fd::AsRawFd>(
    stream: &F,
    caller_uid: u32,
) -> Result<StampedIdentity, AuthError> {
    // 1. Pin the peer via SO_PEERPIDFD: a race-free pid and the cross-uid rejection.
    //    Any pidfd failure (kernel < 6.5, peer already exited, cross-uid) is a DENY.
    let peer = PeerPidfd::from_socket(stream, caller_uid).map_err(auth_error_from_pidfd)?;

    // 2. Read the pinned pid's exe ONCE. The pidfd holds the process open, so the pid
    //    cannot be recycled and this exe (used for both the resolve and the source
    //    label below) is the connecting process's real binary, consistently.
    let exe = exe_path_openat(peer.pid())?;

    // 3. Resolve the app_id. path_to_app_id applies the F3 rule-4 inode gate
    //    internally: an enrolled app whose binary inode does not match its record is
    //    rejected as UnknownBinary, never mis-resolved to the enrolled identity.
    let app_id = path_to_app_id(&exe)?;

    // 4. Label the attestation source. The inode comes from the pinned /proc/{pid}
    //    fd (following the exe magic-link), never a re-stat of the exe path string,
    //    so the InodeRegistry label cannot be forged by a same-uid path swap.
    let source = classify_source(caller_uid, &app_id, peer.pid());

    Ok(StampedIdentity {
        app_id,
        peer,
        source,
    })
}

/// [`IdentitySource::InodeRegistry`] iff the resolved app is enrolled in the
/// root-owned registry for `uid` AND its binary's inode matches the record; otherwise
/// [`IdentitySource::LegacyProc`]. A registry that cannot be loaded (absent or
/// corrupt) yields `LegacyProc`: the app_id itself already resolved through
/// `path_to_app_id`, whose rule-4 treats a corrupt registry as root-caused and
/// cooperative, so the honest label here is the weakest (not inode-attested), never a
/// hard error that would deny an otherwise-valid connection.
fn classify_source(uid: u32, app_id: &str, pid: u32) -> IdentitySource {
    // The pinned exe inode (following the exe magic-link under /proc/{pid}); None on
    // any stat failure -> not inode-attested.
    let ino_dev = exe_ino_dev(pid);
    // A corrupt (not merely absent) registry is the trust root failing to load. It
    // is root-owned, so corruption is root-caused, not a same-uid attack; the app
    // still resolves cooperatively and is labeled LegacyProc. But surface it as a
    // distinct audit event so the shadow rollout can tell a corrupt trust-root apart
    // from a genuinely-unenrolled app (both otherwise read as LegacyProc).
    let registry = match IdentityRegistry::load(uid) {
        Ok(r) => Some(r),
        Err(_) => {
            tracing::warn!(
                target: "audit",
                event = "identity.registry_unloadable",
                uid,
                app_id,
                "inode registry could not be loaded; treating as not inode-attested"
            );
            None
        }
    };
    classify_source_with(registry.as_ref(), app_id, ino_dev)
}

/// Pure core of [`classify_source`]: [`IdentitySource::InodeRegistry`] iff the app is
/// enrolled AND the pinned exe's `(ino, dev)` matches the record; else
/// [`IdentitySource::LegacyProc`]. Takes the inode as data (from the pinned fd, not a
/// path re-stat) so the labeling is unit-testable without the filesystem.
fn classify_source_with(
    registry: Option<&IdentityRegistry>,
    app_id: &str,
    ino_dev: Option<(u64, u64)>,
) -> IdentitySource {
    match (registry.and_then(|r| r.lookup(app_id)), ino_dev) {
        (Some(record), Some((ino, dev))) if record.ino == ino && record.dev == dev => {
            IdentitySource::InodeRegistry
        }
        _ => IdentitySource::LegacyProc,
    }
}

/// Map a pidfd authentication failure onto the shared [`AuthError`]. EVERY variant is
/// a DENY; there is no lenient mapping.
fn auth_error_from_pidfd(e: PidfdError) -> AuthError {
    match e {
        PidfdError::CrossUid { caller, peer } => AuthError::CrossUid { caller, peer },
        // A dead or garbled pidfd means the peer is gone or unattestable: treat as
        // not-alive, the same DENY the start-time recycle check surfaces.
        PidfdError::PidfdInfo => AuthError::PeerNotAlive,
        // SO_PEERPIDFD / SO_PEERCRED getsockopt failures (incl. kernel < 6.5) are a
        // credential-unavailable DENY.
        PidfdError::Getsockopt(e) | PidfdError::PeerUid(e) => AuthError::PeerCred(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity_registry::{IdentityRecord, IdentityRegistry};
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    fn enrolled(app_id: &str, ino: u64, dev: u64) -> IdentityRegistry {
        let mut reg = IdentityRegistry::default();
        reg.record(
            app_id.into(),
            IdentityRecord {
                install_path: PathBuf::from("/usr/lib/arlen/apps/x/bin"),
                ino,
                dev,
            },
        );
        reg
    }

    /// A socketpair's peer is this very process: it resolves recycle-proof to our own
    /// pid/uid, reports alive, and (the cargo-test binary is unenrolled) is labeled
    /// LegacyProc.
    #[test]
    fn resolves_own_process_over_a_socket_pair() {
        let (a, _b) = UnixStream::pair().expect("socketpair");
        // SAFETY: getuid never fails.
        let uid = unsafe { libc::getuid() };
        let ident = app_id_from_connection(&a, uid).expect("resolve");
        assert_eq!(ident.pid(), std::process::id());
        assert_eq!(ident.uid(), uid);
        assert!(ident.is_alive());
        assert!(!ident.app_id().is_empty());
        // The test binary is not installd-enrolled, so it is never InodeRegistry.
        assert_eq!(ident.source(), IdentitySource::LegacyProc);
    }

    /// A caller uid that differs from the peer's is refused before any /proc read.
    #[test]
    fn rejects_a_cross_uid_caller() {
        let (a, _b) = UnixStream::pair().expect("socketpair");
        // SAFETY: getuid never fails.
        let wrong = unsafe { libc::getuid() }.wrapping_add(1);
        match app_id_from_connection(&a, wrong) {
            Err(AuthError::CrossUid { .. }) => {}
            other => panic!("expected CrossUid, got {other:?}"),
        }
    }

    /// An enrolled app whose pinned exe inode matches its record is InodeRegistry.
    #[test]
    fn labels_a_matching_enrolled_inode_as_inode_registry() {
        let reg = enrolled("com.example.app", 42, 7);
        assert_eq!(
            classify_source_with(Some(&reg), "com.example.app", Some((42, 7))),
            IdentitySource::InodeRegistry
        );
    }

    /// A mismatching inode (a copy/spoof: different ino or dev) is LegacyProc, never
    /// attested - the inode comes from the pinned fd, so a path swap cannot forge it.
    #[test]
    fn labels_a_mismatching_inode_as_legacy_proc() {
        let reg = enrolled("com.example.app", 42, 7);
        // Different inode (a copy).
        assert_eq!(
            classify_source_with(Some(&reg), "com.example.app", Some((99, 7))),
            IdentitySource::LegacyProc
        );
        // Different device (same ino number on another fs).
        assert_eq!(
            classify_source_with(Some(&reg), "com.example.app", Some((42, 8))),
            IdentitySource::LegacyProc
        );
    }

    /// Unenrolled app, absent registry, and a failed inode stat all label LegacyProc.
    #[test]
    fn labels_unenrolled_absent_and_no_inode_as_legacy_proc() {
        let reg = enrolled("com.example.app", 42, 7);
        // Enrolled registry but a different app id.
        assert_eq!(
            classify_source_with(Some(&reg), "not.enrolled", Some((42, 7))),
            IdentitySource::LegacyProc
        );
        // No registry at all.
        assert_eq!(
            classify_source_with(None, "com.example.app", Some((42, 7))),
            IdentitySource::LegacyProc
        );
        // Registry + app match, but the pinned inode stat failed (None) -> fail-safe.
        assert_eq!(
            classify_source_with(Some(&reg), "com.example.app", None),
            IdentitySource::LegacyProc
        );
    }
}
