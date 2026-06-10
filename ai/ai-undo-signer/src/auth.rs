//! Peer authentication for the signer's submit/lookup socket
//! (reversible-receipts-and-the-effect-model.md §6).
//!
//! Only the AI agent may submit undo-log entries or look them up. The peer's
//! identity is the kernel-attested `SO_PEERCRED` pid resolved to an `app_id`
//! (never a request field), checked against the [`ADMITTED`] allowlist. This
//! reuses `arlen_permissions::ConnectionAuth`, the same gate the audit-daemon
//! ingest socket uses, with the same PID-reuse guard (`verify_alive`).
//!
//! **F3 residual (the same one the audit-daemon and knowledge daemon carry).**
//! `ConnectionAuth` requires the peer to share the signer's uid, and a same-uid
//! `app_id` is spoofable by a determined same-uid attacker (the binary path and
//! `~/.config/permissions` are user-writable). §6 wants the signer to be a
//! *different* uid so a same-uid compromised agent can neither forge nor read the
//! log; that separation is provisioned at deploy time (a dedicated signer uid +
//! admitting the agent's uid here) and closed fully only by installd's
//! inode-keyed identity registry. This module is the mechanism; the same-uid
//! build is honest defense-in-depth (process isolation + tamper-evidence against
//! a non-agent same-uid process and accidental corruption), not integrity against
//! a fully-compromised agent.

use std::os::unix::io::AsRawFd;

use arlen_permissions::connection_auth::ConnectionAuth;

use crate::error::{Result, SignerError};

/// The app ids permitted to reach the signer socket. Only the agent submits
/// undo entries and looks them up; nothing else has any business here.
pub const ADMITTED: &[&str] = &["ai-agent"];

/// The agent's exact cargo-run `dev.*` id, admitted only in debug
/// builds. An exact match, not a broad `dev.` prefix: any locally-built
/// crate resolves to some `dev.<bin>`, so a prefix would let any
/// cargo-run binary reach the signer socket.
#[cfg(debug_assertions)]
const DEV_ADMITTED: &str = "dev.arlen-ai-agent";

/// Whether a resolved peer `app_id` may use the signer socket. In debug
/// builds the agent's exact `dev.*` id is also admitted so a locally-run
/// agent (from a `target/` tree) can be exercised, matching the
/// audit-daemon's convention.
pub fn caller_is_admitted(app_id: &str) -> bool {
    if ADMITTED.contains(&app_id) {
        return true;
    }
    #[cfg(debug_assertions)]
    {
        app_id == DEV_ADMITTED
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

/// The signer process's own uid, for `ConnectionAuth` peer extraction.
#[allow(unsafe_code)]
pub fn current_uid() -> u32 {
    // SAFETY: getuid() has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

/// Authenticate a freshly-accepted connection: resolve the peer's kernel-attested
/// identity, then admit it only if its `app_id` is on [`ADMITTED`]. Returns the
/// `ConnectionAuth` (whose `verify_alive` the serve loop re-checks per request) or
/// a [`SignerError::Unauthorized`] the caller logs and drops the connection on.
pub fn authenticate<F: AsRawFd>(stream: &F, caller_uid: u32) -> Result<ConnectionAuth> {
    let auth = ConnectionAuth::extract_from(stream, caller_uid)
        .map_err(|e| SignerError::Unauthorized(format!("peer identity: {e}")))?;
    if !caller_is_admitted(auth.app_id()) {
        return Err(SignerError::Unauthorized(format!(
            "app id {:?} is not admitted to the undo-log signer",
            auth.app_id()
        )));
    }
    Ok(auth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_is_admitted_and_others_are_not() {
        assert!(caller_is_admitted("ai-agent"));
        for other in ["ai-daemon", "ai-proxy", "knowledge", "com.attacker", ""] {
            assert!(!caller_is_admitted(other), "{other} must not be admitted");
        }
    }

    #[test]
    fn dev_prefixed_ids_are_admitted_only_in_debug() {
        // Only the agent's exact cargo-run id is admitted in debug, and
        // never in release. An arbitrary `dev.*` crate is always refused.
        assert_eq!(
            caller_is_admitted("dev.arlen-ai-agent"),
            cfg!(debug_assertions)
        );
        assert!(!caller_is_admitted("dev.evil"));
        assert!(!caller_is_admitted("dev.arlen-knowledge"));
    }

    #[test]
    fn current_uid_matches_the_process_uid() {
        // Sanity: it returns this process's real uid (non-panicking, stable).
        assert_eq!(current_uid(), current_uid());
    }
}
