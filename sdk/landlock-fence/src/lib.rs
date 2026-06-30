//! Shared Landlock write-fence for self-confining Arlen daemons
//! (`same-uid-isolation-plan.md` Tier-A #2: Landlock in every Arlen
//! process).
//!
//! A daemon that holds security-load-bearing state (the AI master
//! switches, the credential vault) is a high-value RCE target. If such a
//! process were ever compromised through a parser bug, an unfenced process
//! could rewrite any file the service uid can reach. [`fence_writes`]
//! installs a Landlock ruleset that permits **read** everywhere (shared
//! libs, `/proc` for caller-pid resolution, the D-Bus/socket paths a daemon
//! connects to) and **write** only under an explicit allowlist of dirs -
//! the daemon's own legitimate filesystem footprint. A compromised daemon
//! can then still serve a corrupted reply over its own channel, but it
//! cannot persist anything outside the dirs it owns.
//!
//! ## Thread model (load-bearing)
//!
//! A Landlock domain is inherited only by threads created *after*
//! `restrict_self`, never by threads that already exist. A tokio
//! multi-thread runtime spawns its workers when the runtime is built, so
//! the fence MUST be applied on the main thread BEFORE the runtime is
//! built - otherwise the workers that actually field connections run
//! unconfined and the fence is theater. The canonical caller shape is:
//!
//! ```ignore
//! fn main() {
//!     // ... synchronous setup, create the writable dirs ...
//!     fence_writes(&[store_dir, socket_dir]).ok(); // best-effort, see below
//!     let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
//!     rt.block_on(async { /* serve */ });
//! }
//! ```
//!
//! A `writable` dir that cannot be opened is skipped fail-safe (the grant
//! is simply not expressed, so the process gets less access, never more),
//! so the caller must create those dirs before calling.
//!
//! ## Child processes inherit the fence (so a daemon that spawns helpers is
//! usually the WRONG target)
//!
//! A `fork+exec`'d child inherits the parent's Landlock domain, and a domain
//! can only ever stack TIGHTER - a downstream re-confiner (e.g. `arlen-run`)
//! can intersect-tighten it but never re-grant a path the parent forbade. So a
//! daemon fenced to its own private dirs that then spawns a helper needing
//! broader access (a transfer daemon spawning `rclone` to write user-chosen
//! destinations, an installer writing arbitrary install paths, a notification
//! daemon spawning `aplay`/`paplay` which need `/dev/snd` or `~/.config/pulse`)
//! will silently break that helper on a Landlock-capable kernel. This fence is
//! for daemons that write ONLY their own private state and spawn no
//! access-needing children. A daemon that moves data to arbitrary destinations
//! or shells out to device-touching helpers needs PER-OPERATION confinement on
//! a separately launched worker that does not inherit the daemon's domain, not
//! a daemon-startup write-fence.
//!
//! ## Failure model (the caller decides)
//!
//! This primitive only reports the outcome; it never exits. The fence is
//! defense-in-depth: on a kernel that cannot enforce it (Linux < 5.13, or
//! Landlock disabled) the daemon is exactly as safe as with no fence at
//! all, so a typical caller logs a [`FenceOutcome::NotEnforced`] and
//! continues - refusing to run over a missing hardening add-on would be the
//! worse outcome. A hardened deployment that wants the confinement
//! *guaranteed* can treat `NotEnforced`/`Err` as fatal (the per-daemon
//! `*_REQUIRE_FENCE` env convention), so "the daemon is running" implies
//! "the daemon is write-confined".
//!
//! ## Seccomp interaction
//!
//! A systemd unit with `SystemCallFilter=@system-service` must also allow
//! `landlock_create_ruleset`, `landlock_add_rule`, `landlock_restrict_self`
//! (an older `@system-service` allowlist predates Landlock), or these calls
//! `EPERM`, `fence_writes` fails, and a best-effort fence silently stays off
//! even on a capable kernel.

use std::io;
use std::path::Path;

use landlock::{
    Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr,
    RulesetStatus, ABI,
};

/// Whether the kernel actually enforced the fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenceOutcome {
    /// The kernel installed and enforced the ruleset; the process is now
    /// write-confined to the granted set.
    Enforced,
    /// The kernel did not enforce (too old / Landlock unavailable). The
    /// process runs unconfined - no worse than no fence - and the
    /// defense-in-depth is simply absent. The caller logs this.
    NotEnforced,
}

/// Map a landlock crate error into an `io::Error` so the caller threads it
/// through normal error handling.
fn ll_err(e: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::other(e)
}

/// Install the write-confinement: read (and execute) everywhere, write only
/// under each path in `writable`.
///
/// Granting read on `/` is sound for a self-confining daemon: it
/// legitimately reads shared libs and `/proc/<pid>/exe` (caller-identity
/// resolution can land on a binary anywhere it is legitimately installed),
/// and read access leaks nothing a corrupted reply could not already
/// convey - the property the fence adds is that *writes* are confined to
/// the daemon's own dirs. A `writable` path that cannot be opened is skipped
/// fail-safe, so the caller must create those dirs first.
///
/// The irreversible `restrict_self` applies to the calling thread and every
/// thread it later creates, so call this on the main thread before the async
/// runtime starts (see the module docs).
pub fn fence_writes(writable: &[impl AsRef<Path>]) -> io::Result<FenceOutcome> {
    // ABI v5 covers every filesystem right through IoctlDev; the crate is
    // best-effort by default, so an older kernel drops unsupported rights
    // rather than failing the call (the NotEnforced check below is the floor).
    let abi = ABI::V5;
    let mut ruleset = Ruleset::default()
        .handle_access(AccessFs::from_all(abi))
        .map_err(ll_err)?
        .create()
        .map_err(ll_err)?;

    // Read (+ execute) everywhere. Nothing a self-confining daemon does
    // writes outside the granted set below, so a blanket read grant is the
    // whole non-write surface.
    ruleset = ruleset
        .add_rule(PathBeneath::new(
            PathFd::new("/").map_err(ll_err)?,
            AccessFs::from_read(abi),
        ))
        .map_err(ll_err)?;

    // Full access under each writable dir. `from_all` (not `from_write`) is
    // required so socket creation (`MakeSock`) and file replace work; a dir
    // that cannot be opened is skipped fail-safe.
    for dir in writable {
        let dir = dir.as_ref();
        let fd = match PathFd::new(dir) {
            Ok(fd) => fd,
            Err(_) => continue,
        };
        ruleset = ruleset
            .add_rule(PathBeneath::new(fd, AccessFs::from_all(abi)))
            .map_err(ll_err)?;
    }

    let status = ruleset.restrict_self().map_err(ll_err)?;
    Ok(if status.ruleset == RulesetStatus::NotEnforced {
        FenceOutcome::NotEnforced
    } else {
        FenceOutcome::Enforced
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real Landlock confinement: needs Linux >= 5.13 with Landlock
    /// enabled. Forks so the irreversible `restrict_self` only affects the
    /// child, then probes the fence's two halves: a write inside the granted
    /// dir succeeds, a write outside it is denied, and a read of an existing
    /// path outside the grant still succeeds (the read-everywhere half that
    /// distinguishes this write-fence from a no-filesystem sandbox). The
    /// child probes with raw libc fs calls. NB the child's first call,
    /// `fence_writes`, allocates inside the landlock crate (Ruleset/Vec) -
    /// only the open/close probes after it are alloc-free. A post-fork alloc
    /// is async-signal-unsafe in general, but this is a deliberately-run,
    /// single-threaded (`--test-threads=1`) ignored test, so the allocator
    /// lock cannot be held by another thread at fork. Run with
    /// `--ignored --test-threads=1`.
    #[test]
    #[ignore = "needs Linux >=5.13 with Landlock enabled"]
    fn confines_writes_but_keeps_reads() {
        use std::os::unix::ffi::OsStrExt;

        let dir = tempfile::tempdir().unwrap();
        let inside = dir.path().join("ok");
        let mut inside_c = inside.as_os_str().as_bytes().to_vec();
        inside_c.push(0);
        let outside_write_c = b"/etc/arlen-landlock-fence-selftest\0";
        // An existing, world-readable path outside the write grant.
        let outside_read_c = b"/etc/hostname\0";

        // SAFETY: fork in a deliberately-run, single-threaded
        // (`--test-threads=1`) ignored test. The child only issues the
        // syscalls below before `_exit`.
        let pid = unsafe { libc::fork() };
        if pid == 0 {
            match fence_writes(&[dir.path()]) {
                Ok(FenceOutcome::Enforced) => {}
                // Not enforced (old kernel) - exit a distinct code the parent
                // treats as a skip.
                Ok(FenceOutcome::NotEnforced) => unsafe { libc::_exit(11) },
                Err(_) => unsafe { libc::_exit(10) },
            }
            // write inside the grant must succeed
            let in_fd = unsafe {
                libc::open(
                    inside_c.as_ptr() as *const libc::c_char,
                    libc::O_WRONLY | libc::O_CREAT,
                    0o600,
                )
            };
            let in_ok = in_fd >= 0;
            if in_fd >= 0 {
                unsafe { libc::close(in_fd) };
            }
            // write outside the grant must be denied
            let out_fd = unsafe {
                libc::open(
                    outside_write_c.as_ptr() as *const libc::c_char,
                    libc::O_WRONLY | libc::O_CREAT,
                    0o600,
                )
            };
            let out_denied = out_fd < 0;
            if out_fd >= 0 {
                unsafe { libc::close(out_fd) };
            }
            // read outside the grant must still succeed (read-everywhere)
            let read_fd =
                unsafe { libc::open(outside_read_c.as_ptr() as *const libc::c_char, libc::O_RDONLY) };
            let read_ok = read_fd >= 0;
            if read_fd >= 0 {
                unsafe { libc::close(read_fd) };
            }
            let code = if in_ok && out_denied && read_ok { 0 } else { 20 };
            unsafe { libc::_exit(code) };
        }

        let mut status = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        let code = if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status)
        } else {
            -1
        };
        if code == 11 {
            eprintln!("landlock not enforced on this kernel; skipping the confinement assertion");
            return;
        }
        assert_eq!(
            code, 0,
            "fence self-test: write inside ok, write outside denied, read outside ok"
        );
    }
}
