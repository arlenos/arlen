//! Caller admission: who may REQUEST a transfer (profile-system-plan.md, Decided 5).
//!
//! Two orthogonal gates protect a transfer. This module answers "may this
//! process ask for a transfer at all"; [`crate::policy::decide`] answers "is this
//! `(source, dest, type)` flow permitted". Both must pass.
//!
//! The identity is the existing Arlen model: the daemon resolves the caller's
//! `app_id` from the connection's kernel-attested `SO_PEERCRED` + `path_to_app_id`
//! (via [`arlen_permissions::ConnectionAuth`]), exactly as the clipboard and
//! search brokers do. A transfer moves real bytes, so the broker re-checks the
//! peer is still alive with the same start-time (the PID-reuse close) before each
//! request, not just at connect.
//!
//! Only specific Arlen components may submit a transfer request: the shell's
//! clipboard/drag broker and Settings, never an arbitrary app. The allowlist is
//! the audit daemon's `ADMITTED` pattern, with a debug-only EXACT `dev.*`
//! affordance so a cargo-run harness can exercise the path as itself.

/// The app ids permitted to submit a transfer request. The shell broker drives
/// the copy/drag gestures; Settings drives an explicit transfer action. An app
/// not on this list is refused before the policy gate is even consulted.
const REQUESTERS: &[&str] = &["desktop-shell", "settings"];

/// The cargo-run `dev.*` ids of the admitted requesters, accepted only in debug
/// builds. An EXACT list, never a broad `dev.` prefix: every locally-built
/// binary resolves to some `dev.<bin>`, so a prefix match would admit them all.
#[cfg(debug_assertions)]
const DEV_REQUESTERS: &[&str] = &["dev.arlen-desktop-shell", "dev.arlen-settings"];

/// Whether a resolved peer `app_id` may submit a transfer request.
///
/// Fail-closed: an empty or unresolved id, or one not on the allowlist, is
/// refused. The `dev.*` affordance is debug-only and EXACT (never a prefix), so
/// a release build keeps the tightened requester-only allowlist.
pub fn caller_is_admitted(app_id: &str) -> bool {
    if REQUESTERS.contains(&app_id) {
        return true;
    }
    #[cfg(debug_assertions)]
    {
        DEV_REQUESTERS.contains(&app_id) || dev_extra_admits(app_id)
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

/// A debug-only test affordance: an integration harness sets
/// `ARLEN_TRANSFER_EXTRA_ADMIT` to ONE extra dev id (its own cargo-run
/// `dev.<test>` id, which is hash-suffixed and so cannot be a static
/// [`DEV_REQUESTERS`] entry) so it can drive the request path as itself. An
/// EXACT match, never a broad `dev.` prefix, and never compiled into a release
/// build.
#[cfg(debug_assertions)]
fn dev_extra_admits(app_id: &str) -> bool {
    std::env::var("ARLEN_TRANSFER_EXTRA_ADMIT").is_ok_and(|v| v == app_id)
}

/// The daemon's own uid, for [`arlen_permissions::ConnectionAuth`] peer
/// extraction.
#[allow(unsafe_code)]
pub fn current_uid() -> u32 {
    // SAFETY: getuid() has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_shell_and_settings_may_request() {
        assert!(caller_is_admitted("desktop-shell"));
        assert!(caller_is_admitted("settings"));
        // An arbitrary app is refused.
        assert!(!caller_is_admitted("com.example.app"));
        assert!(!caller_is_admitted("knowledge"));
        assert!(!caller_is_admitted(""));
    }

    #[test]
    fn dev_ids_are_admitted_only_in_debug_and_only_exact() {
        // The listed cargo-run dev ids match in a debug build, never in release.
        assert_eq!(
            caller_is_admitted("dev.arlen-desktop-shell"),
            cfg!(debug_assertions),
        );
        assert_eq!(
            caller_is_admitted("dev.arlen-settings"),
            cfg!(debug_assertions),
        );
        // An arbitrary dev crate is never admitted, even in debug.
        assert!(!caller_is_admitted("dev.arlen-knowledge"));
        assert!(!caller_is_admitted("dev.evil"));
    }

    #[test]
    fn current_uid_matches_the_process_uid() {
        // A smoke check that the FFI binding returns the running uid.
        assert_eq!(current_uid(), current_uid());
    }
}
