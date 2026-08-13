//! The identity broker's in-memory pidfd-keyed record store.
//!
//! This is the security kernel of the stamped-identity Tier-1 broker
//! (`stamped-identity-plan.md`). The trusted launcher (`arlen-run`),
//! holding an `--app-id` it resolved from the root `IdentityRegistry`
//! before the child ran, registers the child's **pidfd** (passed to the
//! broker over `SCM_RIGHTS`) against that app_id. A daemon, at
//! `accept()`, presents its peer's pidfd and asks the broker "who is
//! this?" - identity as a QUERY, never a `readlink(/proc/exe)`.
//!
//! # Why hold the pidfd (the pid-reuse defense)
//!
//! A bare pid is not an identity: a process exits and its pid is reused
//! by an unrelated one. The broker instead **keeps each registered
//! pidfd open**. A pidfd pins its process: while the broker holds it,
//! reading the pid back from `/proc/self/fdinfo` is race-free, and the
//! process's death is observable (the pid reads back `-1`). So a match
//! requires BOTH the held pidfd AND the presented pidfd to be alive with
//! the same pid. Two live processes cannot share a pid, so same-pid +
//! both-alive means the same process. If the registered process died and
//! its pid was recycled, the held pidfd is dead - it never matches - so
//! the recycled process can never inherit the dead one's app_id. This is
//! the launcher-stamped, root-anchored, unforgeable [Tier
//! 1](crate::stamped_identity::IdentitySource::Stamped) identity.
//!
//! The store is a pure model over owned fds: the SCM_RIGHTS wire
//! plumbing (the launcher `RegisterIdentity` op, the daemon
//! `LookupIdentity` op on the separate-uid config/secrets daemon) layers
//! on top of it, so the reuse-defense logic here can be unit-tested in
//! isolation against real pidfds.

use std::os::fd::{AsRawFd, OwnedFd};

use crate::peer_pidfd::pidfd_pid;

/// The one identity trusted by CONSTRUCTION rather than by a registration.
///
/// `arlen-session` is what the login path starts, exactly once per login, and it
/// is what mints the session id. Nothing registers it, because it is what does the
/// registering - so the chain has to start somewhere and this is that place, said
/// out loud.
///
/// It is a binary for this reason. A shell script's running process is the
/// interpreter (`/proc/<pid>/exe` reads `/bin/sh`), so every script on the machine
/// presents one identity and none can be told apart; and whoever could write the
/// script file would become the root of trust for the whole session. Measured
/// 13 Aug, which is what moved `/usr/bin/arlen-session` from a script to an ELF
/// binary whose inode the identity registry can attest.
pub const SESSION_ROOT: &str = "session";

/// A single launcher-stamped identity: the held pidfd (kept open to pin
/// the process and observe its death) plus the app_id the launcher
/// attested and the pid captured at registration.
#[derive(Debug)]
struct Record {
    /// Held open for the record's life so the pinned process cannot be
    /// recycled out from under a later lookup.
    pidfd: OwnedFd,
    /// The pid the held pidfd referred to at registration. Liveness is
    /// `pidfd_pid(pidfd) == Some(pid)`; a dead process reads back a
    /// different value (or fails), so the record stops matching.
    pid: u32,
    /// The launcher-attested app id keyed to this process.
    app_id: String,
    /// The app id of the caller that made this registration.
    ///
    /// What turns the registrar set from a list into a derivation. A registrar is
    /// trusted because the SESSION ROOT started it from a shipped path, never
    /// because its name appears somewhere - and a hand-kept list of names is a
    /// permission, which drifts. Recording the registrar makes "may this process
    /// register?" a question about provenance that the store can answer.
    registered_by: String,
}

impl Record {
    /// True iff the held pidfd still refers to the original live
    /// process (race-free: the held fd cannot be recycled). A dead
    /// process reads back `Pid: -1`/an error, so this returns false and
    /// the record no longer matches any lookup.
    fn is_live(&self) -> bool {
        pidfd_pid(self.pidfd.as_raw_fd()) == Some(self.pid)
    }
}

/// The broker's live set of pidfd-keyed identity records.
///
/// Not `Clone` (each record owns a unique pidfd) and not internally
/// synchronised - the owning daemon wraps it in its own lock.
#[derive(Debug, Default)]
pub struct IdentityStore {
    records: Vec<Record>,
}

/// A registration failed. The only failure is a dead/unreadable pidfd:
/// the launcher handed a handle to a process that is already gone (or an
/// fd that is not a pidfd), so there is nothing to stamp. Fail-closed -
/// the caller must not proceed as if the registration succeeded.
#[derive(Debug, thiserror::Error)]
pub enum IdentityStoreError {
    /// The pidfd did not refer to a live process at registration time.
    #[error("pidfd does not refer to a live process")]
    DeadPidfd,
}

impl IdentityStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `app_id` for the process the launcher-supplied `pidfd`
    /// pins. Reads the pid from the pidfd (race-free); refuses a pidfd
    /// whose process is already dead ([`IdentityStoreError::DeadPidfd`]).
    ///
    /// Any prior record for the SAME live process (a re-register) or a
    /// dead record whose pid this new process recycled is dropped first,
    /// so a live pid maps to exactly one identity. The `pidfd` is held
    /// open for the record's life.
    pub fn register(
        &mut self,
        pidfd: OwnedFd,
        app_id: String,
        registered_by: String,
    ) -> Result<(), IdentityStoreError> {
        let pid = pidfd_pid(pidfd.as_raw_fd()).ok_or(IdentityStoreError::DeadPidfd)?;
        // Keep only live records for a DIFFERENT pid: this drops every
        // dead record and the (at most one) live record sharing this pid.
        // A live same-pid record can only be the same process (two live
        // processes cannot share a pid), so replacing it is correct.
        self.records.retain(|r| r.is_live() && r.pid != pid);
        self.records.push(Record {
            pidfd,
            pid,
            app_id,
            registered_by,
        });
        Ok(())
    }

    /// Look up the app_id for the process the `presented` pidfd pins
    /// (a daemon's `SO_PEERPIDFD` peer handle). Returns the launcher-
    /// stamped app_id, or `None` when no live record matches.
    ///
    /// A match requires the presented process to be alive AND a held
    /// record to be alive with the same pid - so a dead registrant (its
    /// pid perhaps recycled by the presenter) never matches, and an
    /// unregistered process resolves to `None` (the caller falls to a
    /// weaker tier or denies, per its policy).
    pub fn lookup(&self, presented: &OwnedFd) -> Option<&str> {
        let want = pidfd_pid(presented.as_raw_fd())?;
        self.records
            .iter()
            .find(|r| r.pid == want && r.is_live())
            .map(|r| r.app_id.as_str())
    }

    /// Whether the process `presented` pins may itself register identities.
    ///
    /// Derived, never listed. A process may register iff the SESSION ROOT
    /// registered it - so the set of registrars is exactly what the root started,
    /// and it changes when the root's behaviour changes rather than when someone
    /// remembers to edit a constant. A hand-kept list of names IS a permission, and
    /// hand-kept permissions drift; this one cannot, because nothing writes it.
    ///
    /// The root itself is not in the store - nothing registered it - so
    /// [`SESSION_ROOT`] is the one identity trusted by construction, and the
    /// caller checks for it separately. That is the bootstrap, named rather than
    /// hidden inside a list where it would look like just another entry.
    pub fn may_register(&self, presented: std::os::fd::BorrowedFd<'_>) -> bool {
        let Some(want) = pidfd_pid(presented.as_raw_fd()) else {
            return false;
        };
        self.records
            .iter()
            .any(|r| r.pid == want && r.is_live() && r.registered_by == SESSION_ROOT)
    }

    /// Drop every record whose pinned process has exited. Housekeeping
    /// the owning daemon runs periodically; correctness does not depend
    /// on it (lookup already skips dead records), it only frees the held
    /// fds.
    pub fn prune(&mut self) {
        self.records.retain(Record::is_live);
    }

    /// The number of records currently held (live or not-yet-pruned).
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the store holds no records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsFd;
    use super::*;
    use crate::peer_pidfd::pidfd_open;
    use std::process::{Command, Stdio};

    /// A pidfd to this very process, for registering/looking up "self".
    fn self_pidfd() -> OwnedFd {
        pidfd_open(std::process::id()).expect("pidfd_open(self)")
    }

    /// Register a process, then look it up with a SECOND, independent
    /// pidfd to the same process: same live pid on both sides matches.
    #[test]
    fn only_what_the_session_root_registered_may_itself_register() {
        // The derivation, at the store level. `arlen-run` registers apps all day
        // and none of them becomes a registrar; only the root's own children do.
        // That is what keeps this a provenance rule rather than a transitive
        // free-for-all where any registration confers the power to register.
        let mut store = IdentityStore::new();
        let me = self_pidfd();

        store
            .register(self_pidfd(), "com.example.app".into(), "arlen-run".into())
            .unwrap();
        assert!(!store.may_register(me.as_fd()));

        store
            .register(self_pidfd(), "session-supervisor".into(), SESSION_ROOT.into())
            .unwrap();
        assert!(store.may_register(me.as_fd()));
    }

    #[test]
    fn a_registered_process_resolves_via_an_independent_pidfd() {
        let mut store = IdentityStore::new();
        store.register(self_pidfd(), "com.example.app".into(), "arlen-run".into()).unwrap();

        // A distinct pidfd to the same (live) process must still resolve.
        let presented = self_pidfd();
        assert_eq!(store.lookup(&presented), Some("com.example.app"));
    }

    /// An unregistered process resolves to `None`, not a fabricated id.
    #[test]
    fn an_unregistered_process_is_none() {
        let store = IdentityStore::new();
        assert_eq!(store.lookup(&self_pidfd()), None);
    }

    /// A registered app_id belongs to ITS process only: a different live
    /// process (a spawned child) does not inherit it.
    #[test]
    fn a_different_process_does_not_inherit_an_id() {
        let mut store = IdentityStore::new();
        store.register(self_pidfd(), "com.example.self".into(), "arlen-run".into()).unwrap();

        let mut child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sleep");
        let child_pidfd = pidfd_open(child.id()).expect("pidfd_open(child)");
        assert_eq!(store.lookup(&child_pidfd), None, "child must not inherit self's id");

        let _ = child.kill();
        let _ = child.wait();
    }

    /// The pid-reuse defense: once the registered process dies, its held
    /// pidfd is dead, so the record never matches again - even for a
    /// lookup presenting that exact old pid. A recycled process can never
    /// inherit the dead one's identity.
    #[test]
    fn a_dead_registrant_never_matches_even_on_its_old_pid() {
        let mut store = IdentityStore::new();

        let mut child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn sleep");
        let child_pid = child.id();
        // A pidfd captured while the child is alive, used to present the
        // child's pid to lookup even after it dies (stands in for a
        // recycled process that happens to reuse the pid).
        let held_for_lookup = pidfd_open(child_pid).expect("pidfd_open(child)");
        store.register(pidfd_open(child_pid).unwrap(), "com.example.child".into(), "arlen-run".into()).unwrap();
        assert_eq!(store.lookup(&held_for_lookup), Some("com.example.child"));

        // Kill + reap: the registered pidfd is now dead.
        child.kill().unwrap();
        child.wait().unwrap();

        // The held record no longer matches - the reuse-proof core. (The
        // presented pidfd is also dead here, which alone yields None, but
        // the record being dead is the property that closes reuse when a
        // NEW live process later presents the recycled pid.)
        assert_eq!(store.lookup(&held_for_lookup), None);
        // Any lookup for the current live process still yields None (self
        // was never registered), proving no stale record leaked an id.
        assert_eq!(store.lookup(&self_pidfd()), None);

        store.prune();
        assert!(store.is_empty(), "prune drops the dead record");
    }

    /// Re-registering the same live process replaces its id rather than
    /// accumulating two records for one pid.
    #[test]
    fn re_registering_replaces_the_id() {
        let mut store = IdentityStore::new();
        store.register(self_pidfd(), "com.example.old".into(), "arlen-run".into()).unwrap();
        store.register(self_pidfd(), "com.example.new".into(), "arlen-run".into()).unwrap();
        assert_eq!(store.len(), 1);
        assert_eq!(store.lookup(&self_pidfd()), Some("com.example.new"));
    }

    /// A dead pidfd cannot be registered (the launcher handed a handle to
    /// a process that already exited).
    #[test]
    fn registering_a_dead_pidfd_fails_closed() {
        let mut store = IdentityStore::new();
        let mut child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn");
        let pidfd = pidfd_open(child.id()).expect("pidfd_open");
        child.kill().unwrap();
        child.wait().unwrap();
        match store.register(pidfd, "com.example.dead".into(), "arlen-run".into()) {
            Err(IdentityStoreError::DeadPidfd) => {}
            other => panic!("expected DeadPidfd, got {other:?}"),
        }
        assert!(store.is_empty());
    }
}
