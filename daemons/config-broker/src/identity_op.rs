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
/// The one registrar still admitted by NAME, and the reason it has not moved to
/// provenance yet.
///
/// `arlen-run` is spawned by the shell per app launch, not by the session root, so
/// nothing registers it and the derived rule below would refuse it - taking every
/// confined app launch with it. The delegation the store now carries is what makes
/// removing this possible: the session grants the shell the right to pass the right
/// on, so the shell can stamp the launcher it spawns. That call site is the last
/// step; until it exists, deleting this entry would be a correct-looking change
/// that breaks launching.
const IDENTITY_REGISTRARS: &[&str] = &["arlen-run"];

/// True iff `app_id` may register an identity. In a debug build the
/// `dev.arlen-run` cargo-run id also passes (the resolver yields
/// `dev.<bin>` for an unpackaged binary), matching the master-switch
/// writer-admit convention; in release only the canonical id.
pub fn is_admitted_registrar(app_id: &str) -> bool {
    IDENTITY_REGISTRARS.contains(&app_id)
        || (cfg!(debug_assertions) && app_id == "dev.arlen-run")
}

/// Whether this caller may register an identity, by PROVENANCE first.
///
/// Three ways in, and only the first two are the design:
///
///   1. the caller IS the session root, which nothing registers because it is
///      what does the registering - the bootstrap, named in
///      [`arlen_permissions::identity_store::SESSION_ROOT`] rather than hidden as
///      one entry in a list of names;
///   2. the session root registered the caller, which is the derivation: the
///      registrar set is exactly what the root started, and it changes when the
///      root does rather than when someone edits a constant;
///   3. the caller is in [`IDENTITY_REGISTRARS`], which today is `arlen-run`
///      alone and is transitional - see that constant for why it cannot go yet.
///
/// A binary claiming to be a registrar with none of those is refused. The pidfd is
/// the caller's own peer handle, so (2) cannot be answered by a name a caller
/// supplies.
pub fn caller_may_register(
    store: &std::sync::Mutex<arlen_permissions::identity_store::IdentityStore>,
    caller_app_id: &str,
    caller_pidfd: std::os::fd::BorrowedFd<'_>,
) -> bool {
    if caller_app_id == arlen_permissions::identity_store::SESSION_ROOT {
        return true;
    }
    if cfg!(debug_assertions) && caller_app_id == "dev.arlen-session" {
        return true;
    }
    let admitted_by_provenance = store
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .may_register(caller_pidfd);
    admitted_by_provenance || is_admitted_registrar(caller_app_id)
}

/// Whether this caller may grant the registrar right to what it registers.
///
/// The session root by construction, and what the root started directly - the
/// second level of [`caller_may_register`], and the one that bounds the chain. A
/// registrar that was itself delegated (`arlen-run`) is deliberately NOT here.
pub fn caller_may_grant_registrar(
    store: &std::sync::Mutex<arlen_permissions::identity_store::IdentityStore>,
    caller_app_id: &str,
    caller_pidfd: std::os::fd::BorrowedFd<'_>,
) -> bool {
    if caller_app_id == arlen_permissions::identity_store::SESSION_ROOT {
        return true;
    }
    if cfg!(debug_assertions) && caller_app_id == "dev.arlen-session" {
        return true;
    }
    store
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .may_grant_registrar(caller_pidfd)
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
    caller_pidfd: Option<std::os::fd::BorrowedFd<'_>>,
) -> IdentityResponse {
    match request {
        IdentityRequest::Register { app_id, registrar } => {
            // Provenance first: the session root, or something the session root
            // registered. A caller whose own pidfd we do not have cannot be
            // answered by provenance at all, so it is refused rather than falling
            // back to its name.
            let Some(caller_pidfd) = caller_pidfd else {
                return IdentityResponse::Refused(
                    "no caller handle, so registrar provenance cannot be established".into(),
                );
            };
            if !caller_may_register(store, caller_app_id, caller_pidfd) {
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
            // The right to register may only be PASSED ON by what the session
            // root started itself. Two levels, deliberately: the root grants it to
            // the shell, the shell grants it to the launcher it spawns per app,
            // and the launcher stamps apps with the bit clear. Without this stop a
            // launcher could stamp an app AS a launcher, and every process
            // downstream of one launch could then claim any identity - which is
            // the hole the broker exists to close.
            if registrar && !caller_may_grant_registrar(store, caller_app_id, caller_pidfd) {
                return IdentityResponse::Refused(format!(
                    "caller '{caller_app_id}' may register but may not pass that right on"
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
            match store.register(fd, app_id, caller_app_id.to_string(), registrar) {
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
    use std::os::fd::AsFd;
    use std::os::fd::FromRawFd;

    /// A pidfd for the CALLER in the register tests.
    ///
    /// The tests exercise the transitional `arlen-run`-by-name path, so the
    /// handle's provenance does not matter to them - what matters is that the
    /// dispatch now REQUIRES one, since a caller we cannot identify cannot be
    /// admitted by provenance and must not fall back to its name.
    fn caller_handle() -> std::os::fd::OwnedFd {
        self_pidfd()
    }

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
                registrar: false,
            },
            Some(self_pidfd()),
            Some(caller_handle().as_fd()),
        );
        assert_eq!(reg, IdentityResponse::Registered);

        let got = handle_identity(&s, "any-daemon", IdentityRequest::Lookup, Some(self_pidfd()), None);
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
            "dev.arlen.settings",
            IdentityRequest::Register {
                app_id: "com.evil.squat".into(),
                registrar: false,
            },
            Some(self_pidfd()),
            Some(caller_handle().as_fd()),
        );
        assert!(matches!(reg, IdentityResponse::Refused(_)));
        // Nothing was stamped - a later lookup finds no record.
        let got = handle_identity(&s, "any-daemon", IdentityRequest::Lookup, Some(self_pidfd()), None);
        assert_eq!(got, IdentityResponse::NotFound);
    }

    /// Even the admitted registrar may not stamp a RESERVED/privileged id
    /// (the LD_PRELOAD-registrar residual must not mint `settings` /
    /// `system.*` / an AI principal onto a controlled pidfd).
    #[test]
    fn a_registrar_may_not_stamp_a_reserved_id() {
        for reserved in ["dev.arlen.settings", "system", "system.knowledge", "ai-agent", "org.arlen.x"] {
            let s = store();
            let reg = handle_identity(
                &s,
                "arlen-run",
                IdentityRequest::Register {
                    app_id: reserved.into(),
                    registrar: false,
                },
                Some(self_pidfd()),
                Some(caller_handle().as_fd()),
            );
            assert!(
                matches!(reg, IdentityResponse::Refused(_)),
                "reserved id {reserved} must be refused"
            );
            // Nothing was stamped.
            let got = handle_identity(&s, "d", IdentityRequest::Lookup, Some(self_pidfd()), None);
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
                IdentityRequest::Register { app_id: bad.into(), registrar: false },
                Some(self_pidfd()),
                Some(caller_handle().as_fd()),
            );
            assert!(
                matches!(reg, IdentityResponse::Refused(_)),
                "malformed id {bad:?} must be refused"
            );
            let got = handle_identity(&s, "d", IdentityRequest::Lookup, Some(self_pidfd()), None);
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
                    app_id: "x".into(),
                    registrar: false,
                },
                None,
                Some(caller_handle().as_fd()),
            ),
            IdentityResponse::Error(_)
        ));
        assert!(matches!(
            handle_identity(&s, "any-daemon", IdentityRequest::Lookup, None, None),
            IdentityResponse::Error(_)
        ));
    }

    #[test]
    fn a_registrar_is_admitted_by_provenance_and_refused_without_it() {
        let s = store();
        let me = caller_handle();

        // Nothing has registered this process, and it is not the root, so it may
        // not register - whatever it calls itself. This is the case the derived
        // rule exists for: a binary CLAIMING to be a registrar has no provenance.
        assert!(!caller_may_register(&s, "some-daemon", me.as_fd()));

        // The session root needs no registration, because it is what registers.
        // The bootstrap, and the only identity trusted by construction.
        assert!(caller_may_register(
            &s,
            arlen_permissions::identity_store::SESSION_ROOT,
            me.as_fd()
        ));

        // Registered BY the root: admitted, and the name played no part.
        s.lock()
            .unwrap()
            .register(
                self_pidfd(),
                "session-supervisor".into(),
                arlen_permissions::identity_store::SESSION_ROOT.into(),
                true,
            )
            .unwrap();
        assert!(caller_may_register(&s, "some-daemon", me.as_fd()));
    }

    #[test]
    fn a_delegated_registrar_may_not_stamp_another_registrar() {
        // The bound on the chain, at the dispatch level. The shell may pass the
        // right to the launcher it spawns; the launcher may not pass it to an app.
        // Refused rather than silently downgraded: a caller that asked for the
        // right and got a record without it would believe it holds one.
        let s = store();
        let me = caller_handle();
        s.lock()
            .unwrap()
            .register(self_pidfd(), "arlen-run".into(), "arlen-desktop-shell".into(), true)
            .unwrap();

        assert!(caller_may_register(&s, "arlen-run", me.as_fd()));
        assert!(!caller_may_grant_registrar(&s, "arlen-run", me.as_fd()));
        assert!(matches!(
            handle_identity(
                &s,
                "arlen-run",
                IdentityRequest::Register {
                    app_id: "com.example.app".into(),
                    registrar: true,
                },
                Some(self_pidfd()),
                Some(me.as_fd()),
            ),
            IdentityResponse::Refused(_)
        ));

        // The same call WITHOUT the request for the right goes through: an app
        // launch is exactly this, and it must not be collateral damage.
        assert!(matches!(
            handle_identity(
                &s,
                "arlen-run",
                IdentityRequest::Register {
                    app_id: "com.example.app".into(),
                    registrar: false,
                },
                Some(self_pidfd()),
                Some(me.as_fd()),
            ),
            IdentityResponse::Registered
        ));
    }

    #[test]
    fn a_registration_by_anyone_else_does_not_confer_the_power_to_register() {
        // The property that makes this a derivation rather than a transitive
        // free-for-all: `arlen-run` registers apps all day, and none of those apps
        // becomes a registrar. Only what the ROOT started does.
        let s = store();
        s.lock()
            .unwrap()
            .register(self_pidfd(), "com.example.app".into(), "arlen-run".into(), false)
            .unwrap();
        assert!(!caller_may_register(&s, "com.example.app", caller_handle().as_fd()));
    }

    /// An unregistered process resolves to `NotFound`, not a fabricated id.
    #[test]
    fn an_unregistered_process_is_not_found() {
        let s = store();
        let got = handle_identity(&s, "any-daemon", IdentityRequest::Lookup, Some(self_pidfd()), None);
        assert_eq!(got, IdentityResponse::NotFound);
    }

    /// The request/response types round-trip through the JSON frame the
    /// socket layer will use.
    #[test]
    fn requests_and_responses_round_trip_json() {
        let req = IdentityRequest::Register {
            app_id: "com.example.app".into(),
            registrar: false,
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
