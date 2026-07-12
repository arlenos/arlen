//! Race-free connection-identity resolution (the stamped-identity strand, slice 1).
//!
//! [`crate::connection_auth::ConnectionAuth`] today resolves the peer app_id from a
//! racy `SO_PEERCRED` pid plus a `/proc/{pid}/stat` start-time re-check: the pid can
//! be recycled between the `getsockopt` and the `/proc` read, and the start-time
//! guard only *detects* the recycle after the fact. [`app_id_from_connection`]
//! replaces that with [`crate::peer_pidfd::PeerPidfd`]: `SO_PEERPIDFD` hands back a
//! pidfd that PINS the peer process, so the pid cannot be recycled and every
//! `/proc/{pid}/exe` read below refers to exactly the process that connected. The
//! resolved app_id is the same one [`crate::identity::path_to_app_id`] produces
//! (including its F3 rule-4 inode gate), now obtained race-free and carrying an
//! explicit [`IdentitySource`] label for the shadow-mode divergence audit that wires
//! this in next.
//!
//! Fail-closed is the whole contract (the polkit CVE-2021-3560 anti-pattern: a
//! mid-check disconnect must never authorise). Every pidfd/registry/exe error maps to
//! an [`AuthError`] the caller MUST treat as DENY; there is no path that returns a
//! usable identity from a failed lookup. Cross-uid is rejected inside `PeerPidfd`
//! before any `/proc` read. No broker yet: this slice produces the Tier-2 inode
//! registry attestation ([`IdentitySource::InodeRegistry`]) and the path-trust
//! fallback ([`IdentitySource::LegacyProc`]); the launcher-stamped Tier-1
//! ([`IdentitySource::Stamped`]) lands with the broker.

use std::path::Path;

use crate::connection_auth::AuthError;
use crate::identity::{exe_path_openat, path_to_app_id};
use crate::identity_registry::{verify_binary, IdentityRegistry};
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

    // 4. Label the attestation source from the SAME exe, so the label is self-evident
    //    rather than inferred from step 3's internal behavior.
    let source = classify_source(caller_uid, &app_id, &exe);

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
fn classify_source(uid: u32, app_id: &str, exe: &Path) -> IdentitySource {
    let registry = IdentityRegistry::load(uid).ok();
    classify_source_with(registry.as_ref(), app_id, exe)
}

/// Pure core of [`classify_source`] over an already-loaded registry, so the labeling
/// is unit-testable without the on-disk file or an env override.
fn classify_source_with(
    registry: Option<&IdentityRegistry>,
    app_id: &str,
    exe: &Path,
) -> IdentitySource {
    match registry.and_then(|r| r.lookup(app_id)) {
        Some(record) if verify_binary(record, exe) => IdentitySource::InodeRegistry,
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
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    fn write_bin(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(b"binary").unwrap();
        p
    }

    /// A socketpair's peer is this very process: it resolves race-free to our own
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

    /// An enrolled app whose binary inode matches its record is InodeRegistry-attested.
    #[test]
    fn labels_a_matching_enrolled_binary_as_inode_registry() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = write_bin(tmp.path(), "app");
        let mut reg = IdentityRegistry::default();
        reg.record("com.example.app".into(), IdentityRecord::for_path(&bin).unwrap());
        assert_eq!(
            classify_source_with(Some(&reg), "com.example.app", &bin),
            IdentitySource::InodeRegistry
        );
    }

    /// A copy of an enrolled binary at a different path has a new inode: the spoof is
    /// labeled LegacyProc, not attested (belt-and-braces; path_to_app_id would have
    /// already rejected it as UnknownBinary).
    #[test]
    fn labels_a_spoof_copy_as_legacy_proc() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = write_bin(tmp.path(), "app");
        let mut reg = IdentityRegistry::default();
        reg.record("com.example.app".into(), IdentityRecord::for_path(&bin).unwrap());
        let copy = tmp.path().join("evil-copy");
        std::fs::copy(&bin, &copy).unwrap();
        assert_eq!(
            classify_source_with(Some(&reg), "com.example.app", &copy),
            IdentitySource::LegacyProc
        );
    }

    /// An unenrolled app, and an absent/unloadable registry, both label LegacyProc.
    #[test]
    fn labels_unenrolled_and_no_registry_as_legacy_proc() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = write_bin(tmp.path(), "app");
        let reg = IdentityRegistry::default();
        assert_eq!(
            classify_source_with(Some(&reg), "not.enrolled", &bin),
            IdentitySource::LegacyProc
        );
        assert_eq!(
            classify_source_with(None, "com.example.app", &bin),
            IdentitySource::LegacyProc
        );
    }
}
