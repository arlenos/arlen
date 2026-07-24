//! The identity broker's register/lookup ops (pure dispatch).
//!
//! The stamped-identity Tier-1 broker (`stamped-identity-plan.md`) lives
//! as an op on this separate-uid daemon so its records are not
//! same-uid-writable. The trusted launcher (`arlen-run`), holding an
//! `--app-id` it resolved from the root `IdentityRegistry` before the
//! child ran, `Register`s the child's pidfd against that app_id; a daemon
//! at `accept()` `Lookup`s its peer's pidfd. Both carry the pidfd over
//! `SCM_RIGHTS` (`arlen_permissions::fd_passing`); the socket layer
//! extracts it and supplies it here, so this dispatch is pure over
//! `(store, caller_app_id, request, received_fd)` and unit-testable
//! without a socket - the same shape as [`crate::protocol::handle_request`].

use std::os::fd::OwnedFd;
use std::sync::Mutex;

use arlen_permissions::identity_store::{IdentityStore, IdentityStoreError};
// The wire contract lives in the low-level permissions crate so a daemon
// can `lookup` without depending on this broker daemon crate; re-exported
// here for the dispatch + the socket handler.
pub use arlen_permissions::identity_wire::{IdentityRequest, IdentityResponse};

/// The apps allowed to REGISTER an identity: only the trusted launcher
/// `arlen-run`, which alone holds an authenticated `--app-id` (resolved
/// from the root `IdentityRegistry` before the child ran). Every other
/// same-uid caller may `Lookup` (each admission daemon does, for a pidfd
/// it already holds) but never `Register` - otherwise any process could
/// stamp an arbitrary app_id onto a pidfd it controls, which is exactly
/// the spoof this design closes.
///
/// The resolved id `arlen-run` requires the identity resolver to map the
/// launcher's canonical binary to it; that resolver entry lands with the
/// socket-wiring slice (this allowlist is the auth structure it plugs
/// into). Until then this is exercised by the pure dispatch tests.
const IDENTITY_REGISTRARS: &[&str] = &["arlen-run"];

/// True iff `app_id` may register an identity. In a debug build the
/// `dev.arlen-run` cargo-run id also passes (the resolver yields
/// `dev.<bin>` for an unpackaged binary), matching the master-switch
/// writer-admit convention; in release only the canonical id.
pub fn is_admitted_registrar(app_id: &str) -> bool {
    IDENTITY_REGISTRARS.contains(&app_id)
        || (cfg!(debug_assertions) && app_id == "dev.arlen-run")
}

/// Dispatch one identity request against the shared store for an
/// authenticated caller. `received` is the pidfd the caller passed over
/// `SCM_RIGHTS`: for `Register` it is the CHILD's (moved into the store
/// and held), for `Lookup` it is the daemon's PEER's (borrowed for the
/// match, then dropped). A missing fd is an error - both ops require one,
/// and the no-fd path never fabricates an identity.
pub fn handle_identity(
    store: &Mutex<IdentityStore>,
    caller_app_id: &str,
    request: IdentityRequest,
    received: Option<OwnedFd>,
) -> IdentityResponse {
    match request {
        IdentityRequest::Register { app_id } => {
            if !is_admitted_registrar(caller_app_id) {
                return IdentityResponse::Refused(format!(
                    "caller '{caller_app_id}' may not register an identity"
                ));
            }
            // Defense in depth against a registrar-gate bypass (the documented
            // LD_PRELOAD-on-the-real-arlen-run residual): even an admitted
            // registrar may not stamp a RESERVED/privileged id (system, system.*,
            // org.arlen.*, ai-daemon, ai-agent, settings) onto a pidfd it
            // controls. The launcher only ever stamps real user app ids
            // (reverse-DNS), so a reserved id from here is a bypass attempt. This
            // keeps the stamped Tier-1 path no weaker than the rule-4 user path,
            // which already refuses these via the same guard.
            if arlen_permissions::identity::is_reserved_app_id(&app_id) {
                return IdentityResponse::Refused(format!(
                    "'{app_id}' is a reserved app id and may not be stamped"
                ));
            }
            // Validate the id's charset here too, symmetric with the resolver's
            // lookup-side guard: a malformed (e.g. path-traversal-shaped) id must
            // never be STORED, so no future lookup consumer that trusts a raw stamp
            // can inherit one - defense in depth beyond the resolver already
            // refusing an invalid stamp on read.
            if !arlen_permissions::is_valid_app_id(&app_id) {
                return IdentityResponse::Refused(format!(
                    "'{app_id}' is not a valid app id and may not be stamped"
                ));
            }
            let Some(fd) = received else {
                return IdentityResponse::Error("register requires a pidfd".into());
            };
            // A poisoned lock is recovered rather than propagated: the
            // store is a plain record set, an earlier panic leaves it
            // consistent, and refusing every future op would be a worse
            // failure mode than continuing.
            let mut store = store.lock().unwrap_or_else(|e| e.into_inner());
            match store.register(fd, app_id) {
                Ok(()) => IdentityResponse::Registered,
                Err(IdentityStoreError::DeadPidfd) => {
                    IdentityResponse::Error("pidfd does not refer to a live process".into())
                }
            }
        }
        IdentityRequest::Lookup => {
            let Some(fd) = received else {
                return IdentityResponse::Error("lookup requires a pidfd".into());
            };
            let store = store.lock().unwrap_or_else(|e| e.into_inner());
            match store.lookup(&fd) {
                Some(app_id) => IdentityResponse::Resolved {
                    app_id: app_id.to_string(),
                },
                None => IdentityResponse::NotFound,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::FromRawFd;

    /// A pidfd to this very process (register/lookup "self").
    fn self_pidfd() -> OwnedFd {
        // SAFETY: pidfd_open(getpid(), 0) returns a fresh owned fd for a
        // live process (self is always alive here).
        let raw = unsafe {
            libc::syscall(libc::SYS_pidfd_open, libc::getpid() as libc::pid_t, 0)
        };
        assert!(raw >= 0, "pidfd_open(self)");
        // SAFETY: the kernel handed us a fresh owned fd.
        unsafe { OwnedFd::from_raw_fd(raw as libc::c_int) }
    }

    fn store() -> Mutex<IdentityStore> {
        Mutex::new(IdentityStore::new())
    }

    /// The launcher registers, then any caller resolves the same process
    /// via an independent pidfd.
    #[test]
    fn register_then_lookup_resolves_the_stamped_id() {
        let s = store();
        let reg = handle_identity(
            &s,
            "arlen-run",
            IdentityRequest::Register {
                app_id: "com.example.app".into(),
            },
            Some(self_pidfd()),
        );
        assert_eq!(reg, IdentityResponse::Registered);

        let got = handle_identity(&s, "any-daemon", IdentityRequest::Lookup, Some(self_pidfd()));
        assert_eq!(
            got,
            IdentityResponse::Resolved {
                app_id: "com.example.app".into()
            }
        );
    }

    /// A non-launcher caller cannot register: refused, nothing stamped.
    #[test]
    fn a_non_launcher_may_not_register() {
        let s = store();
        let reg = handle_identity(
            &s,
            "settings",
            IdentityRequest::Register {
                app_id: "com.evil.squat".into(),
            },
            Some(self_pidfd()),
        );
        assert!(matches!(reg, IdentityResponse::Refused(_)));
        // Nothing was stamped - a later lookup finds no record.
        let got = handle_identity(&s, "any-daemon", IdentityRequest::Lookup, Some(self_pidfd()));
        assert_eq!(got, IdentityResponse::NotFound);
    }

    /// Even the admitted registrar may not stamp a RESERVED/privileged id
    /// (the LD_PRELOAD-registrar residual must not mint `settings` /
    /// `system.*` / an AI principal onto a controlled pidfd).
    #[test]
    fn a_registrar_may_not_stamp_a_reserved_id() {
        for reserved in ["settings", "system", "system.knowledge", "ai-agent", "org.arlen.x"] {
            let s = store();
            let reg = handle_identity(
                &s,
                "arlen-run",
                IdentityRequest::Register {
                    app_id: reserved.into(),
                },
                Some(self_pidfd()),
            );
            assert!(
                matches!(reg, IdentityResponse::Refused(_)),
                "reserved id {reserved} must be refused"
            );
            // Nothing was stamped.
            let got = handle_identity(&s, "d", IdentityRequest::Lookup, Some(self_pidfd()));
            assert_eq!(got, IdentityResponse::NotFound);
        }
    }

    /// A malformed (path-traversal-shaped or empty) id is refused at REGISTER, so
    /// it can never be stored - symmetric with the resolver's lookup-side guard.
    #[test]
    fn a_registrar_may_not_stamp_a_malformed_id() {
        for bad in ["../../etc/passwd", "", "Has Caps", "a/b"] {
            let s = store();
            let reg = handle_identity(
                &s,
                "arlen-run",
                IdentityRequest::Register { app_id: bad.into() },
                Some(self_pidfd()),
            );
            assert!(
                matches!(reg, IdentityResponse::Refused(_)),
                "malformed id {bad:?} must be refused"
            );
            let got = handle_identity(&s, "d", IdentityRequest::Lookup, Some(self_pidfd()));
            assert_eq!(got, IdentityResponse::NotFound);
        }
    }

    /// Both ops require a pidfd; the no-fd path errors, never fabricates.
    #[test]
    fn a_missing_pidfd_is_an_error() {
        let s = store();
        assert!(matches!(
            handle_identity(
                &s,
                "arlen-run",
                IdentityRequest::Register {
                    app_id: "x".into()
                },
                None
            ),
            IdentityResponse::Error(_)
        ));
        assert!(matches!(
            handle_identity(&s, "any-daemon", IdentityRequest::Lookup, None),
            IdentityResponse::Error(_)
        ));
    }

    /// An unregistered process resolves to `NotFound`, not a fabricated id.
    #[test]
    fn an_unregistered_process_is_not_found() {
        let s = store();
        let got = handle_identity(&s, "any-daemon", IdentityRequest::Lookup, Some(self_pidfd()));
        assert_eq!(got, IdentityResponse::NotFound);
    }

    /// The request/response types round-trip through the JSON frame the
    /// socket layer will use.
    #[test]
    fn requests_and_responses_round_trip_json() {
        let req = IdentityRequest::Register {
            app_id: "com.example.app".into(),
        };
        let bytes = serde_json::to_vec(&req).unwrap();
        assert_eq!(serde_json::from_slice::<IdentityRequest>(&bytes).unwrap(), req);

        let resp = IdentityResponse::Resolved {
            app_id: "com.example.app".into(),
        };
        let bytes = serde_json::to_vec(&resp).unwrap();
        assert_eq!(
            serde_json::from_slice::<IdentityResponse>(&bytes).unwrap(),
            resp
        );
    }
}
