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

/// The app ids permitted to reach the signer socket.
///
/// The agent submits undo entries. `undod`, the session undo service, READS them
/// for the recent-actions panel - and it was missing here, so on a shipped image
/// every read it made would have been refused while a developer build worked,
/// because the `dev.*` fallback resolved it. Caught by the shipped-unit identity
/// test, which noticed the binary had no app id at all.
pub const ADMITTED: &[&str] = &["ai-agent", "undod"];

/// The app ids that may hand the signer a receipt and do nothing else.
///
/// Files and `trash-rm` journal a person's own delete so it can be put back.
/// Both submitted to this socket from the day they were written and neither was
/// on [`ADMITTED`], which a developer build hid: nothing here admitted them in
/// debug either, but their submit is best-effort and reports a refusal on
/// stderr, so the delete worked and the durable record was silently missing.
///
/// Submit only, because that is all either does. They have no reason to read
/// what the agent did or to move another producer's entry to a new state, and
/// the socket should not offer it to them.
///
/// The ids are the ones the install paths resolve to: Files under
/// `/usr/lib/arlen/apps/dev.arlen.files/`, `trash-rm` as `/usr/bin/arlen-trash-rm`.
pub const SUBMITTERS: &[&str] = &["dev.arlen.files", "trash-rm"];

/// The submitters' exact cargo-run ids, admitted only in debug builds.
#[cfg(debug_assertions)]
const DEV_SUBMITTERS: &[&str] = &["dev.arlen-files", "dev.trash-rm"];

/// Whether `app_id` is one of the submit-only producers.
fn is_submitter(app_id: &str) -> bool {
    if SUBMITTERS.contains(&app_id) {
        return true;
    }
    #[cfg(debug_assertions)]
    {
        DEV_SUBMITTERS.contains(&app_id)
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

/// Whether a caller that reached the socket may send this request. The agent
/// and the undo service may send any; a submitter may only seal a new entry.
pub fn may_send(app_id: &str, request: &arlen_ai_undo_proto::Request) -> bool {
    if is_submitter(app_id) {
        return matches!(request, arlen_ai_undo_proto::Request::SubmitCreated(_));
    }
    true
}

/// The agent's exact cargo-run `dev.*` id, admitted only in debug
/// builds. An exact match, not a broad `dev.` prefix: any locally-built
/// crate resolves to some `dev.<bin>`, so a prefix would let any
/// cargo-run binary reach the signer socket.
#[cfg(debug_assertions)]
const DEV_ADMITTED: &[&str] = &["dev.arlen-ai-agent", "dev.arlen-undod"];

/// Whether a resolved peer `app_id` may use the signer socket. In debug
/// builds the agent's exact `dev.*` id is also admitted so a locally-run
/// agent (from a `target/` tree) can be exercised, matching the
/// audit-daemon's convention.
pub fn caller_is_admitted(app_id: &str) -> bool {
    if ADMITTED.contains(&app_id) || is_submitter(app_id) {
        return true;
    }
    #[cfg(debug_assertions)]
    {
        DEV_ADMITTED.contains(&app_id)
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

    /// Nothing here is behind `debug_assertions`, so this holds in the profile
    /// the image is built with: the ids come from the install paths, and those
    /// ids are admitted.
    #[test]
    fn a_persons_own_delete_is_journaled_from_the_installed_binaries() {
        use arlen_permissions::identity::path_to_app_id;
        use std::path::Path;
        for path in [
            "/usr/lib/arlen/apps/dev.arlen.files/bin/arlen-files",
            "/usr/bin/arlen-trash-rm",
        ] {
            let id = path_to_app_id(Path::new(path)).unwrap();
            assert!(SUBMITTERS.contains(&id.as_str()), "{path} resolves to {id}");
            assert!(caller_is_admitted(&id), "{id} must reach the signer");
        }
    }

    #[test]
    fn a_submitter_seals_an_entry_and_does_nothing_else() {
        use arlen_ai_undo_proto::Request;
        for id in SUBMITTERS {
            assert!(!may_send(id, &Request::LiveEntries));
            assert!(!may_send(id, &Request::ListRecent { limit: 5 }));
            assert!(!may_send(id, &Request::LookupEntry { op_id: "x".into() }));
        }
        assert!(may_send("ai-agent", &Request::LiveEntries));
        assert!(may_send("undod", &Request::ListRecent { limit: 5 }));
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
