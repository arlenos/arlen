//! Landlock filesystem confinement for the launched app.
//!
//! [`apply_landlock`] installs a Landlock ruleset that permits **read**
//! everywhere the bwrap mount view still exposes and **write** only under the
//! app's declared writable set (its own state dirs plus the `[filesystem]`
//! grants). It is applied in the child's `pre_exec`, after the inherited fds
//! are closed and before `execve(bwrap)`, so bwrap and the app both inherit it.
//!
//! Ordering inside the child is load-bearing: Landlock opens a `PathFd` for
//! each rule (a path-open), so it must run **before** the seccomp filter that
//! later removes `openat`. It also runs after the cgroup join (a later commit),
//! for the same reason — once the syscall surface is narrowed, neither the
//! cgroup write nor the path opens are possible.
//!
//! Landlock can only ever *reduce* access, so a writable path that cannot be
//! opened (it does not exist, or is unreadable) is skipped rather than
//! aborting the launch: the result is the app getting *less* access, never
//! more. The launcher creates the app's own state dirs before spawning so
//! their write grant is always expressible. A kernel that does not enforce the
//! ruleset at all is fatal (fail-closed: never run believing we are confined
//! when we are not).

#![cfg(target_os = "linux")]

use std::io;
use std::path::Path;

use landlock::{
    Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr,
    RulesetStatus, ABI,
};

/// Map a landlock crate error into an `io::Error` so it threads through the
/// `pre_exec` closure (which returns `io::Result<()>`).
fn ll_err(e: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::other(e)
}

/// Apply a Landlock ruleset granting read everywhere reachable and write only
/// under each path in `writable`. Everything outside the writable set is
/// read-only (and everything outside the bwrap mount view is absent entirely).
///
/// Returns `Err` only on a ruleset failure or a kernel that did not enforce the
/// ruleset; a single unopenable writable path is skipped (fail-safe), because a
/// dropped write rule reduces access rather than widening it.
pub fn apply_landlock(writable: &[impl AsRef<Path>]) -> io::Result<()> {
    // ABI v5 covers every filesystem right through IoctlDev; the crate is
    // best-effort by default, so an older kernel drops unsupported rights
    // rather than failing the call (the NotEnforced check below is the floor).
    let abi = ABI::V5;
    let mut ruleset = Ruleset::default()
        .handle_access(AccessFs::from_all(abi))
        .map_err(ll_err)?
        .create()
        .map_err(ll_err)?;

    // Read (and execute) everywhere the bwrap mounts left visible: the
    // read-only `/usr`, the plumbing sockets. Granting read on `/` is sound
    // because bwrap already pruned the mount namespace to the allowed binds, so
    // there is nothing outside them to read.
    ruleset = ruleset
        .add_rule(PathBeneath::new(PathFd::new("/").map_err(ll_err)?, AccessFs::from_read(abi)))
        .map_err(ll_err)?;

    // Full access under each writable dir. A path that cannot be opened is
    // skipped (fail-safe): the grant simply is not expressed, the app gets less
    // access, never more.
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
    if status.ruleset == RulesetStatus::NotEnforced {
        return Err(io::Error::other("landlock not enforced by kernel"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real Landlock confinement: needs Linux >= 5.13 with Landlock enabled.
    /// Forks so the irreversible `restrict_self` only affects the child, then
    /// probes that a write inside the grant succeeds and a write outside the
    /// grant is denied. The child uses raw libc fs calls to avoid allocating
    /// after fork. Run with `--ignored --test-threads=1`.
    #[test]
    #[ignore = "needs Linux >=5.13 with Landlock enabled"]
    fn denies_writes_outside_the_grant() {
        use std::os::unix::ffi::OsStrExt;

        let dir = tempfile::tempdir().unwrap();
        let inside = dir.path().join("ok");
        let mut inside_c = inside.as_os_str().as_bytes().to_vec();
        inside_c.push(0);
        let outside_c = b"/etc/arlen-run-landlock-selftest\0";

        // SAFETY: fork in a deliberately-run, single-threaded (`--test-threads=1`)
        // ignored test. The child only issues syscalls below before _exit.
        let pid = unsafe { libc::fork() };
        if pid == 0 {
            if apply_landlock(&[dir.path()]).is_err() {
                unsafe { libc::_exit(10) };
            }
            // open(O_WRONLY|O_CREAT) inside the grant must succeed,
            // and the same outside the grant must fail.
            let in_fd = unsafe {
                libc::open(inside_c.as_ptr() as *const libc::c_char, libc::O_WRONLY | libc::O_CREAT, 0o600)
            };
            let in_ok = in_fd >= 0;
            if in_fd >= 0 {
                unsafe { libc::close(in_fd) };
            }
            let out_fd = unsafe {
                libc::open(outside_c.as_ptr() as *const libc::c_char, libc::O_WRONLY | libc::O_CREAT, 0o600)
            };
            let out_denied = out_fd < 0;
            if out_fd >= 0 {
                unsafe { libc::close(out_fd) };
            }
            let code = if in_ok && out_denied { 0 } else { 20 };
            unsafe { libc::_exit(code) };
        }

        let mut status = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
        let code = if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status)
        } else {
            -1
        };
        assert_eq!(code, 0, "landlock self-test: write inside ok, write outside denied");
    }
}
