//! The transfer request surface (profile-system-plan.md, Decided 5).
//!
//! TRANSPORT DECISION. The online-accounts daemon serves on the session bus,
//! where the bus authoritatively stamps the sender and answers
//! `GetConnectionUnixProcessID`, so a caller cannot forge another connection's
//! identity. That soundness argument does NOT transfer to the Transfer Daemon:
//! it is a SYSTEM dual-uid broker mediating two user sessions, not a per-session
//! bus service. So the request surface is a raw per-uid
//! [`tokio::net::UnixListener`] (one per profile, under each profile's per-uid
//! runtime dir), authenticated with [`arlen_permissions::ConnectionAuth`] +
//! `verify_alive` per request - the kernel-attested `SO_PEERCRED` path the
//! clipboard and search brokers use, with no bus-PID indirection.
//!
//! SOURCE IS THE SOCKET, NOT A REQUEST FIELD. Because each per-uid listener's
//! `caller_uid` is one profile's uid, a request that arrives on the source
//! profile's socket is provably from a process in that profile (cross-uid is
//! rejected by `extract_from`). The listener's profile therefore ESTABLISHES the
//! request's `source`; a caller cannot forge it. The daemon binds the resolved
//! source profile from the socket and rejects a request whose body `source`
//! disagrees, so the policy gate keys on an attested source.
//!
//! This module owns the path resolution and the per-request flow shape
//! (auth -> gate -> deliver); the live `UnixListener` accept loop and the live
//! cross-uid delivery are the daemon-side wiring (`main`) plus the deferred
//! broker. The interface name `org.arlen.Transfer1` and the D-Bus activation
//! file in `dist/` are kept for discoverability and a future bus-side control
//! surface, but the data path is the raw per-uid socket described above.

use std::path::PathBuf;

use crate::request::ProfileId;

/// The well-known interface name (kept for discoverability and the dist D-Bus
/// activation file; the data path is the raw per-uid socket).
pub const INTERFACE_NAME: &str = "org.arlen.Transfer1";

/// The transfer request socket for one profile uid:
/// `$XDG_RUNTIME_DIR/arlen/transfer.sock` when `XDG_RUNTIME_DIR` is set, else
/// `/run/user/{uid}/arlen/transfer.sock`.
///
/// The Transfer Daemon binds one such socket per profile it brokers; a request
/// arriving on a profile's socket is attested (by `caller_uid` == that profile's
/// uid) to come from that profile, which is how the request's `source` is
/// established structurally rather than trusted from the request body.
pub fn request_socket_path(uid: u32) -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{uid}")));
    base.join("arlen").join("transfer.sock")
}

/// Whether a request body's claimed `source` matches the profile that the
/// listening socket attests (the socket's bound profile). A mismatch is refused:
/// the source is the socket, never a caller-chosen field.
pub fn source_matches_socket(claimed: &ProfileId, socket_profile: &ProfileId) -> bool {
    claimed == socket_profile
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_socket_lives_under_the_arlen_runtime_dir() {
        // Whichever base resolves (XDG_RUNTIME_DIR or the per-uid fallback), the
        // socket is `<base>/arlen/transfer.sock` - per-user, never system-wide.
        // Env is read, not mutated, so this is parallel-safe.
        let path = request_socket_path(1234);
        assert!(
            path.ends_with("arlen/transfer.sock"),
            "got {}",
            path.display(),
        );
    }

    #[test]
    fn the_source_must_match_the_socket_profile() {
        let work = ProfileId::new("work").unwrap();
        let personal = ProfileId::new("personal").unwrap();
        // A request claiming the socket's own profile is accepted as the source.
        assert!(source_matches_socket(&work, &work));
        // A request claiming a different source than the socket attests is
        // refused: the source is the socket, not a request field.
        assert!(!source_matches_socket(&personal, &work));
    }
}
