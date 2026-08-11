//! Assembling and spawning the `bwrap` command for a confined launch.
//!
//! Three pieces, the first two pure and unit-tested, the third needing a real
//! kernel (so its test is `#[ignore]`d):
//!
//! - [`plumbing_binds`] determines the universal plumbing a GUI app needs that
//!   is not on the security axis (the Wayland/PipeWire/D-Bus sockets), filtered
//!   to what actually exists via an injected predicate so it is testable.
//! - [`build_confinement`] turns the profile-derived inputs into a runnable
//!   [`Confinement`] (skeleton + `complete`), and [`bwrap_argv`] assembles the
//!   final argument vector (`<bwrap flags> -- <program> <args>`).
//! - [`spawn_and_wait`] runs `bwrap`, closing inherited fds and starting a new
//!   process group in the child, then waits and maps the exit status.
//!
//! Beyond what `bwrap` itself sets (the namespaces, `no_new_privs`, the pruned
//! mount view, `--clearenv`), the `pre_exec` chain joins the per-command cgroup
//! and applies Landlock over the writable set, the parent installs the egress
//! seam, and the app seccomp allowlist is handed to `bwrap` via `--seccomp <fd>`
//! (so `bwrap` installs it on the app after its own setup). The real egress
//! enforcer (a netns + forwarding proxy) is the remaining confinement layer.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use arlen_confiner::{app_runtime_profile, Bind, Confinement, ConfinerError, NetworkPolicy};

/// The universal plumbing a GUI app needs that is not on the security axis: the
/// Wayland and PipeWire sockets, the session D-Bus, and the Arlen runtime
/// directory, all bound read-write (they are sockets). Only paths that actually
/// exist are bound, because `bwrap` fails the launch on a bind whose source is
/// missing; the `exists` predicate is injected so the mapping is pure and
/// testable without a real session.
///
/// **`$XDG_RUNTIME_DIR/arlen` is where every Arlen daemon socket lives** -
/// knowledge, clipboard, intents, launch, search, terminal-read - and leaving it
/// out was measured, on a real profile, to cut a confined app off from all of
/// them: `ls: cannot access '/run/user/1000/arlen'`. That is not confinement, it
/// is blinding. A profile granting nine `system.File` read scopes cannot have
/// them checked on a socket the app cannot open, so the profile and the sandbox
/// would contradict each other.
///
/// Bound because **reachability is not authority**: each of those sockets
/// authorises its caller, so binding grants nothing and restores the ability to
/// be told no - and a refusal is auditable where an absence is not. Decided
/// 11 Aug, `app-enrollment-plan.md`. A socket whose mere reachability is
/// sensitive may still be withheld individually; none of these is.
///
/// **Read-write rather than read-only, and that is measured too.** A read-only
/// bind still permits `connect()` (checked under bwrap: a client connected to a
/// socket inside a `--ro-bind` directory), which would have been the tighter
/// choice for a pure client - but the terminal BINDS `terminal-read.sock` here
/// for `terminal-run-mcp` to read, and creating a socket needs a writable
/// directory. Read-only would break it. This grants nothing a same-uid process
/// does not already have outside the sandbox.
///
/// `wayland_display` is `$WAYLAND_DISPLAY`: an absolute path is taken verbatim,
/// a bare name is resolved under `runtime_dir`.
pub fn plumbing_binds(
    runtime_dir: &Path,
    wayland_display: Option<&str>,
    exists: impl Fn(&Path) -> bool,
) -> Vec<Bind> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(wl) = wayland_display {
        let p = if Path::new(wl).is_absolute() {
            PathBuf::from(wl)
        } else {
            runtime_dir.join(wl)
        };
        candidates.push(p);
    }
    candidates.push(runtime_dir.join("pipewire-0"));
    candidates.push(runtime_dir.join("bus"));
    candidates.push(runtime_dir.join("arlen"));

    let mut binds = Vec::new();
    for p in candidates {
        if !exists(&p) {
            continue;
        }
        if let Some(s) = p.to_str() {
            binds.push(Bind::ReadWrite(s.to_string(), s.to_string()));
        }
    }
    binds
}

/// Build the runnable confinement from the profile-derived writable set and
/// network policy plus the universal plumbing: construct the app-runtime
/// skeleton (`/usr` read-only, the app dirs writable, the network policy) and
/// `complete` it with the plumbing binds. The launcher applies Landlock and the
/// network host-filter on top before the child execs.
pub fn build_confinement(
    usr: &Path,
    app_dirs: &[PathBuf],
    masked: &[PathBuf],
    read_only: &[PathBuf],
    env: BTreeMap<String, String>,
    net: NetworkPolicy,
    plumbing: Vec<Bind>,
) -> Result<Confinement, ConfinerError> {
    let dir_refs: Vec<&Path> = app_dirs.iter().map(PathBuf::as_path).collect();
    let masked_refs: Vec<&Path> = masked.iter().map(PathBuf::as_path).collect();
    let skeleton = app_runtime_profile(usr, &dir_refs, &masked_refs, env, net)?;
    // Binding only `/usr` leaves a merged-`/usr` system's root-level compat paths
    // (`/lib64` etc.) absent, so a dynamically-linked app cannot find its ELF
    // interpreter and `execvp` fails ENOENT ("no such file"). Add them read-only.
    let mut plumbing = plumbing;
    plumbing.extend(merged_usr_compat_binds());
    // The read-only subtrees ride in with the plumbing, which is the one place a
    // read-only bind belongs: `app_runtime_profile` takes writable dirs, and
    // handing it these would be the writable grant the format exists to avoid.
    plumbing.extend(
        read_only
            .iter()
            .filter(|p| p.exists())
            .map(|p| Bind::ReadOnly(p.display().to_string(), p.display().to_string())),
    );
    Ok(skeleton.complete(plumbing, Vec::new()))
}

/// The merged-`/usr` compatibility binds - `/lib64`, `/lib`, `/bin`, `/sbin`, the
/// ones present - bound read-only so a dynamically-linked app finds its ELF
/// interpreter (`/lib64/ld-linux-*`) and the tools on the default PATH. On a
/// merged system each is a symlink into `/usr`, so binding it exposes the `/usr`
/// content at the root path the interpreter reference uses; a non-merged system's
/// real dirs bind directly. Absent paths are skipped (bwrap rejects a missing
/// bind source), so this is safe on any layout.
fn merged_usr_compat_binds() -> Vec<Bind> {
    // The path list lives in the confiner, which is where the other two
    // confinement constructors reach for it. Keeping a third copy here is how
    // `command_profile` came to be missing it entirely and `run_command` could not
    // execute a dynamically linked program.
    arlen_confiner::merged_usr_compat_roots()
        .into_iter()
        .map(|p| Bind::ReadOnly(p.clone(), p))
        .collect()
}

/// Assemble the full `bwrap` argument vector: the confinement's flags followed
/// by the `--` separator and the program with its arguments. The caller spawns
/// `bwrap` with this vector. Pure and deterministic for a given confinement.
pub fn bwrap_argv(confinement: &Confinement, program: &[String]) -> Vec<String> {
    let mut argv = confinement.bwrap_args();
    argv.push("--".into());
    argv.extend(program.iter().cloned());
    argv
}

/// Spawn `bwrap` with the assembled argument vector, then wait and return the
/// propagated exit code. In the child, before exec, in order: close every
/// inherited fd above stderr, start a new process group, and join the per-launch
/// cgroup. Landlock is deliberately NOT applied to bwrap (it would break bwrap's
/// own user-namespace + newroot setup, see `child_pre_exec`); the filesystem
/// confinement is bwrap's `--ro-bind` mount namespace, and the app seccomp filter
/// is installed by bwrap itself via `--seccomp <fd>` after its setup, just before
/// exec.
///
/// `bwrap` propagates the app's own exit code, so the returned `u8` is the
/// app's exit status (or `128 + signal` if the app was killed by a signal). A
/// failure to spawn `bwrap` at all is an `Err`, which the caller maps to the
/// `SPAWN` exit code; the launcher never falls back to an unconfined run.
///
/// The launcher is single-threaded at spawn time, so the post-fork child is
/// single-threaded and the `pre_exec` allocations (the Landlock ruleset) are
/// safe; do not introduce threads before this call.
#[cfg(target_os = "linux")]
pub fn spawn_and_wait(
    argv: &[String],
    writable: &[PathBuf],
    cgroup_procs: Option<PathBuf>,
    seccomp_bpf: Option<Vec<u8>>,
    app_id: &str,
) -> std::io::Result<u8> {
    use std::os::unix::process::{CommandExt, ExitStatusExt};

    // Landlock is NOT applied here (see `child_pre_exec`): a ruleset installed on
    // the launcher's child before `execve("bwrap")` also confines bwrap's OWN
    // setup, which writes `/proc/self/{setgroups,uid_map,gid_map}` for the user
    // namespace and its private newroot tmpfs - none of which any app-writable set
    // covers - so a read-only-`/` ruleset makes bwrap fail before the app ever
    // runs. bwrap's `--ro-bind` mount namespace IS the filesystem confinement (the
    // filtered path relies on exactly this), and a proper Landlock layer belongs
    // INSIDE the sandbox, applied to the app after bwrap's setup, not to bwrap.
    let _ = writable;

    // The app seccomp allowlist is delivered to bwrap as `--seccomp <fd>`: the
    // compiled cBPF lives in a memfd the child inherits, and bwrap installs it on
    // the app after its own namespace/mount setup, just before exec. The fd must
    // survive the exec into bwrap, so the pre_exec close_range (which marks every
    // fd CLOEXEC) clears CLOEXEC on this one fd again below. The memfd is created
    // here, in the parent, so its number is stable across the fork.
    let mut full_argv: Vec<String> = Vec::with_capacity(argv.len() + 2);
    let seccomp_fd: Option<libc::c_int> = match &seccomp_bpf {
        Some(bpf) => {
            let fd = make_seccomp_memfd(bpf)?;
            full_argv.push("--seccomp".into());
            full_argv.push(fd.to_string());
            Some(fd)
        }
        None => None,
    };
    // The Tier-1 identity stamp: bwrap writes the sandboxed child's HOST pid to a
    // json-status pipe (the app runs under --unshare-pid, so its host pid differs
    // from bwrap's) and blocks on a second pipe until the launcher registers it at
    // the identity broker. Skipped when `app_id` is empty (unit tests) or if the
    // pipes cannot be made - BEST-EFFORT: a failed stamp launches unstamped and the
    // app then resolves via /proc, never a broken launch. The handshake's args go
    // among the bwrap flags (before the `--` in `argv`).
    let stamp = if app_id.is_empty() {
        None
    } else {
        match crate::stamp::StampHandshake::new() {
            Ok(h) => {
                full_argv.extend(h.bwrap_args());
                Some(h)
            }
            Err(e) => {
                eprintln!(
                    "arlen-run: warning: identity stamp pipes unavailable, launching unstamped: {e}"
                );
                None
            }
        }
    };
    full_argv.extend_from_slice(argv);

    let mut cmd = Command::new("bwrap");
    cmd.args(&full_argv);
    // The fds bwrap must inherit past the child's close_range: the seccomp memfd
    // plus the stamp pipes' child-side ends (json-status write + block read).
    // Copied out so `seccomp_fd` stays usable for the parent-side close below.
    let mut keep_fds: Vec<libc::c_int> = seccomp_fd.iter().copied().collect();
    if let Some(h) = &stamp {
        keep_fds.extend_from_slice(&h.child_keep_fds());
    }
    // SAFETY: the closure runs in the child after fork, before exec. The
    // launcher is single-threaded so the post-fork child is too, making the
    // ruleset allocations safe; the syscalls (close_range, fcntl, setpgid, the
    // Landlock setup) only narrow the child's own capabilities.
    // Captured in the PARENT: inside the closure this would be the child's own pid.
    let launcher_pid = std::process::id() as libc::pid_t;
    unsafe {
        cmd.pre_exec(move || child_pre_exec(None, &cgroup_procs, &keep_fds, launcher_pid));
    }

    let spawned = cmd.spawn();
    // The child inherited the memfd at fork; the parent's copy is no longer
    // needed and is closed regardless of how the spawn went.
    if let Some(fd) = seccomp_fd {
        unsafe { libc::close(fd) };
    }
    // Run the stamp handshake now that bwrap is forked: read the child pid, register
    // it, and unblock bwrap so it execs the app. Only when the spawn succeeded - a
    // failed spawn has no bwrap waiting on the block pipe. `complete` always writes
    // the unblock byte (even on stamp failure), so a live bwrap is never wedged.
    if let Some(h) = stamp {
        if spawned.is_ok() {
            h.complete(
                app_id,
                &arlen_permissions::identity_wire::identity_broker_connect_path(),
            );
        }
    }
    let status = spawned?.wait()?;
    Ok(exit_code(status.code(), status.signal()))
}

/// The shared post-fork, pre-exec child confinement, run in order: mark every
/// inherited fd above stderr close-on-exec (so no launcher fd leaks into the
/// app, while std's error pipe survives to report a failure), keep `keep_fd`
/// (the seccomp memfd, direct-bwrap path only) open across the exec, start a new
/// process group, join the per-launch cgroup, then apply Landlock if a writable
/// set is given. Both production spawn paths - direct `bwrap` and the
/// `pasta`-wrapped filtered launch - pass `None`, so Landlock is NOT applied to
/// bwrap: a ruleset installed here confines bwrap's own user-namespace + newroot
/// setup (which writes `/proc/self/*` and a private tmpfs no app-writable set
/// covers), so it makes bwrap fail before the app runs. The `Some` arm is retained
/// for the follow-up that applies Landlock INSIDE the sandbox to the app after
/// bwrap's setup (an in-sandbox wrapper, like the seccomp/route wrappers), where
/// the writable set is the app's real filesystem confinement; `apply_landlock` is
/// the building block for it.
///
/// # Safety
/// Must run in the post-fork child before exec, single-threaded (the launcher is
/// single-threaded at spawn time), so the Landlock ruleset allocations are safe;
/// every syscall only narrows the child's own capabilities.
#[cfg(target_os = "linux")]
unsafe fn child_pre_exec(
    landlock_writable: Option<&[PathBuf]>,
    cgroup_procs: &Option<PathBuf>,
    keep_fds: &[libc::c_int],
    launcher_pid: libc::pid_t,
) -> std::io::Result<()> {
    // Tie the child's life to this launcher's: when the launcher dies the kernel
    // SIGKILLs the child. `bwrap --die-with-parent` already does this on the direct
    // path, but on the filtered path the process this execs into is `pasta`, which
    // has no equivalent flag - and `--die-with-parent` inside pasta's argv binds
    // bwrap to PASTA, not to the launcher. Without this, killing the launcher left
    // pasta reparented to init with the whole confined tree (namespace, proxy and
    // app) still running. Observed, not theorised: a killed test harness left a
    // `pasta -> bwrap -> bwrap -> rclone rcd` chain alive at ppid 1.
    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // PDEATHSIG only fires for a parent that dies AFTER it is set. If the launcher
    // exited inside the fork window the signal will never come, so check for it
    // rather than exec into a child that would outlive its purpose. Returning Err
    // here makes std report the failure and `_exit` the child before exec.
    if libc::getppid() != launcher_pid {
        return Err(std::io::Error::other("launcher exited during spawn"));
    }
    // CLOSE_RANGE_CLOEXEC (not an immediate close) so std's pre_exec/execve error
    // pipe (an fd >= 3) survives to report a failure, while every launcher fd is
    // closed atomically on a successful exec. Needs kernel >= 5.11, below the
    // Landlock >= 5.13 floor this launcher already requires.
    let rc = libc::close_range(3, libc::c_uint::MAX, libc::CLOSE_RANGE_CLOEXEC as libc::c_int);
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // Re-clear CLOEXEC on each fd bwrap must inherit across the exec: the seccomp
    // memfd (`--seccomp`) on the direct path, plus the identity-stamp pipes
    // (`--json-status-fd` write end + `--block-fd` read end) when stamping is on.
    // The filtered path passes an empty set and opens its filter from a file.
    for &fd in keep_fds {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    // A new process group so a stray signal to the launcher's group does not race
    // the cgroup-based reaping.
    if libc::setpgid(0, 0) != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // Join the per-launch cgroup BEFORE Landlock: a read-only `/` ruleset would
    // deny the write to cgroup.procs.
    if let Some(procs) = cgroup_procs {
        crate::cgroup::join_current(procs)?;
    }
    // Filesystem confinement, inherited by the whole tree. The app seccomp filter
    // (installed by bwrap after this, on the app only) may drop path-open, so
    // Landlock's path opens must happen first. Skipped (`None`) on the filtered
    // path: Landlock applied to the pasta parent is incompatible with pasta's tap
    // setup AND the nested bwrap userns it wraps (both need writes Landlock's
    // read-only `/` denies); that launch's fs confinement is bwrap's own mount
    // namespace instead. See `spawn_filtered_and_wait`.
    if let Some(writable) = landlock_writable {
        crate::landlock_apply::apply_landlock(writable)?;
    }
    Ok(())
}

/// Spawn a `FilteredHosts` launch: `bwrap` (the identical confinement) running
/// INSIDE a `pasta` route-absent network namespace whose only reachable peer is
/// the forwarding egress proxy (see [`crate::netns`]). The app's mandatory
/// seccomp filter is delivered to `bwrap` through a temp FILE the pasta wrapper
/// opens (a memfd would be dropped by `pasta`). The direct child is `pasta`,
/// which runs [`child_pre_exec`] (close_range + setpgid + cgroup-join) and hosts
/// the whole `bwrap` tree; `bwrap` does not `--unshare-net`, so it inherits
/// pasta's namespace. Returns the app's propagated exit code.
///
/// Landlock is NOT applied on this path: applied to the pasta parent it breaks
/// both pasta's tap setup and the nested `bwrap` userns (see `child_pre_exec`).
/// The app's filesystem confinement is `bwrap`'s own mount namespace (`--ro-bind`
/// plus the writable binds); the app still gets seccomp, the netns egress
/// boundary and the cgroup. Restoring Landlock's extra fs layer here is a
/// follow-up that must apply it to the app AFTER pasta (a re-exec confine step).
#[cfg(target_os = "linux")]
pub fn spawn_filtered_and_wait(
    bwrap_argv: &[String],
    writable: &[PathBuf],
    cgroup_procs: Option<PathBuf>,
    seccomp_bpf: Vec<u8>,
) -> std::io::Result<u8> {
    use std::io::Write;
    use std::os::unix::process::{CommandExt, ExitStatusExt};

    // The compiled seccomp on disk, where the pasta wrapper opens it inside the
    // namespace. Held until after the wait, then removed when it drops.
    let mut seccomp_file = tempfile::NamedTempFile::new()?;
    seccomp_file.write_all(&seccomp_bpf)?;
    seccomp_file.flush()?;
    let seccomp_path = seccomp_file
        .path()
        .to_str()
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "non-utf8 seccomp temp path")
        })?
        .to_string();

    // The confined app is `bwrap --seccomp <fd> <confinement> -- program`; wrap
    // the whole invocation in pasta's namespace, which opens the filter file.
    let mut app_argv = vec![
        "bwrap".to_string(),
        "--seccomp".to_string(),
        crate::netns::SECCOMP_WRAPPER_FD.to_string(),
    ];
    app_argv.extend_from_slice(bwrap_argv);
    let argv = crate::netns::pasta_argv(&app_argv, Some(&seccomp_path));

    let _ = writable; // Landlock is skipped on this path (see child_pre_exec).
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    // SAFETY: single-threaded post-fork child (see child_pre_exec). No memfd (the
    // wrapper opens the seccomp file) and no Landlock (`None`) on this path.
    // Captured in the PARENT (see the direct path).
    let launcher_pid = std::process::id() as libc::pid_t;
    unsafe {
        cmd.pre_exec(move || child_pre_exec(None, &cgroup_procs, &[], launcher_pid));
    }
    let status = cmd.spawn()?.wait()?;
    drop(seccomp_file); // remove the temp filter now the launch has ended
    Ok(exit_code(status.code(), status.signal()))
}

/// Create an anonymous in-memory file holding the compiled seccomp cBPF and
/// return its fd, positioned at offset 0 so bwrap reads the whole program. The
/// fd is created without `MFD_CLOEXEC` (the child's pre_exec re-opens it across
/// the exec anyway); the parent closes its copy once the child has forked.
#[cfg(target_os = "linux")]
fn make_seccomp_memfd(bpf: &[u8]) -> std::io::Result<libc::c_int> {
    use std::ffi::CString;
    let name = CString::new("arlen-seccomp").expect("static name has no nul");
    // SAFETY: a plain memfd_create with a valid C string and no flags.
    let fd = unsafe { libc::memfd_create(name.as_ptr(), 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut written = 0usize;
    while written < bpf.len() {
        // SAFETY: writing `bpf[written..]` bytes to a fd we own.
        let n = unsafe {
            libc::write(
                fd,
                bpf[written..].as_ptr() as *const libc::c_void,
                bpf.len() - written,
            )
        };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e);
        }
        written += n as usize;
    }
    // SAFETY: rewind so bwrap reads from the start.
    if unsafe { libc::lseek(fd, 0, libc::SEEK_SET) } < 0 {
        let e = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(e);
    }
    Ok(fd)
}

/// Map a process exit status to a `u8` launcher exit code: a normal exit code
/// truncated to a byte, or `128 + signal` for a signal-terminated child (the
/// shell convention). A child with neither (should not happen) maps to `1`.
#[cfg(target_os = "linux")]
fn exit_code(code: Option<i32>, signal: Option<i32>) -> u8 {
    if let Some(c) = code {
        (c & 0xff) as u8
    } else if let Some(s) = signal {
        128u8.wrapping_add((s & 0x7f) as u8)
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plumbing_binds_only_what_exists() {
        let rt = Path::new("/run/user/1000");
        // Only the Wayland socket and the bus exist; pipewire is absent.
        let present = [
            PathBuf::from("/run/user/1000/wayland-0"),
            PathBuf::from("/run/user/1000/bus"),
        ];
        let binds = plumbing_binds(rt, Some("wayland-0"), |p| present.contains(&p.to_path_buf()));
        assert!(binds.contains(&Bind::ReadWrite(
            "/run/user/1000/wayland-0".into(),
            "/run/user/1000/wayland-0".into()
        )));
        assert!(binds.contains(&Bind::ReadWrite(
            "/run/user/1000/bus".into(),
            "/run/user/1000/bus".into()
        )));
        assert!(!binds.iter().any(|b| matches!(
            b,
            Bind::ReadWrite(s, _) if s.contains("pipewire")
        )));
    }

    /// The regression this exists for: without the Arlen runtime directory a
    /// confined app has no route to knowledge, clipboard, intents, launch,
    /// search or terminal-read, so its profile grants scopes that can never be
    /// checked. Measured on a real launch before it was fixed - the directory
    /// simply did not exist inside the sandbox.
    #[test]
    fn the_arlen_runtime_directory_is_bound() {
        let rt = Path::new("/run/user/1000");
        let present = [PathBuf::from("/run/user/1000/arlen")];
        let binds = plumbing_binds(rt, None, |p| present.contains(&p.to_path_buf()));
        assert_eq!(
            binds,
            vec![Bind::ReadWrite(
                "/run/user/1000/arlen".into(),
                "/run/user/1000/arlen".into()
            )],
            "the daemon sockets have to be reachable so the daemons can refuse"
        );
    }

    #[test]
    fn plumbing_binds_takes_an_absolute_wayland_display_verbatim() {
        let rt = Path::new("/run/user/1000");
        let binds = plumbing_binds(rt, Some("/tmp/wl.sock"), |_| true);
        assert!(binds.contains(&Bind::ReadWrite("/tmp/wl.sock".into(), "/tmp/wl.sock".into())));
    }

    #[test]
    fn plumbing_binds_empty_when_nothing_exists() {
        let binds = plumbing_binds(Path::new("/run/user/1000"), Some("wayland-1"), |_| false);
        assert!(binds.is_empty());
    }

    #[test]
    fn bwrap_argv_appends_the_program_after_a_separator() {
        let conf = build_confinement(
            Path::new("/usr"),
            &[PathBuf::from("/home/u/.config/arlen/apps/com.a.b")],
            &[],
            &[],
            BTreeMap::new(),
            NetworkPolicy::None,
            Vec::new(),
        )
        .unwrap();
        let argv = bwrap_argv(&conf, &["/usr/bin/echo".into(), "hi".into()]);
        let sep = argv.iter().position(|a| a == "--").expect("separator present");
        assert_eq!(&argv[sep + 1..], &["/usr/bin/echo".to_string(), "hi".to_string()]);
        // The flags before the separator are the confinement's own.
        assert!(argv[..sep].contains(&"--unshare-pid".to_string()));
        assert!(argv[..sep].contains(&"--unshare-net".to_string()));
    }

    #[test]
    fn build_confinement_keeps_the_network_up_for_unrestricted() {
        let conf = build_confinement(
            Path::new("/usr"),
            &[],
            &[],
            &[],
            BTreeMap::new(),
            NetworkPolicy::Unrestricted,
            Vec::new(),
        )
        .unwrap();
        assert!(!conf.bwrap_args().contains(&"--unshare-net".to_string()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn exit_code_maps_status_and_signal() {
        assert_eq!(exit_code(Some(0), None), 0);
        assert_eq!(exit_code(Some(42), None), 42);
        assert_eq!(exit_code(Some(256), None), 0); // truncated to a byte
        assert_eq!(exit_code(None, Some(9)), 137); // SIGKILL
        assert_eq!(exit_code(None, None), 1);
    }

    /// A real confined launch: needs `bwrap` and unprivileged user namespaces,
    /// so it is ignored by default and run explicitly on a capable kernel.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs bwrap and unprivileged userns on the host kernel"]
    fn echo_runs_confined_and_propagates_exit() {
        let conf = build_confinement(
            Path::new("/usr"),
            &[],
            &[],
            &[],
            BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]),
            NetworkPolicy::None,
            Vec::new(),
        )
        .unwrap();
        let argv = bwrap_argv(&conf, &["/usr/bin/echo".into(), "hi".into()]);
        // A basic confined exec must SUCCEED: bwrap sets up its user namespace and
        // mount view and runs echo. This regressed when Landlock was applied to
        // bwrap here (it denied bwrap's own `/proc/self/*` setup writes); the fix
        // was to leave the fs confinement to bwrap's mount namespace.
        let code = spawn_and_wait(&argv, &[], None, None, "").expect("bwrap spawns");
        assert_eq!(code, 0);
    }

    /// A real confined launch WITH the seccomp filter installed: the key check
    /// that the allowlist is not too tight to run an ordinary program. A denied
    /// syscall returns EPERM (not a kill), so a too-narrow allowlist surfaces as
    /// a non-zero exit here rather than a crash. Metal-only (bwrap + userns).
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs bwrap and unprivileged userns on the host kernel"]
    fn echo_runs_confined_under_the_seccomp_filter() {
        let conf = build_confinement(
            Path::new("/usr"),
            &[],
            &[],
            &[],
            BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]),
            NetworkPolicy::None,
            Vec::new(),
        )
        .unwrap();
        let argv = bwrap_argv(&conf, &["/usr/bin/echo".into(), "hi".into()]);
        let bpf = crate::seccomp::app_filter_bytes().expect("filter compiles");
        let code = spawn_and_wait(&argv, &[], None, Some(bpf), "").expect("bwrap spawns");
        assert_eq!(code, 0, "the allowlist must permit a basic confined exec");
    }

    /// The whole §0 filtered-launch composition, end to end: bind the real
    /// forwarding proxy with an allowlist, run a confined app through
    /// `spawn_filtered_and_wait` (bwrap inside the route-absent pasta namespace,
    /// seccomp via the wrapper file), and prove the app reaches ONLY the proxy,
    /// which refuses a non-allowlisted CONNECT (403). The probe reports the
    /// verdict as its exit code, propagated up through bwrap and pasta. The
    /// allowlisted-reachable half needs a real external host (the SSRF floor
    /// blocks loopback mocks), so this asserts the refusal, the security-relevant
    /// direction. Metal-only (pasta + bwrap + userns).
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs pasta + bwrap + unprivileged userns on the host kernel"]
    fn a_filtered_launch_reaches_the_proxy_which_refuses_a_non_allowlisted_host() {
        use crate::egress::{EgressEnforcer, ProxyEgressEnforcer};
        // The real forwarding proxy, allowlisting a host the probe will NOT ask
        // for. Held for the whole launch (its Drop stops the proxy).
        let guard = ProxyEgressEnforcer
            .install(&["allowed.invalid:443".to_string()])
            .expect("bind the egress proxy");
        let port = guard.proxy_port().expect("a filtered guard exposes its proxy port");

        // The confined probe: dial the proxy at the mapped gateway and CONNECT to
        // a NON-allowlisted host; the proxy refuses it (403) before dialing out.
        // Exit 0 on the refusal, distinct non-zeros otherwise, so the launch's
        // propagated exit code carries the verdict.
        let probe = format!(
            "exec 3<>/dev/tcp/{addr}/{port} 2>/dev/null || exit 20; \
             printf 'CONNECT blocked.invalid:443 HTTP/1.1\\r\\nHost: blocked.invalid:443\\r\\n\\r\\n' >&3; \
             resp=$(head -c 16 <&3 2>/dev/null); \
             case \"$resp\" in *403*) exit 0 ;; *) exit 30 ;; esac",
            addr = crate::netns::PROXY_NETNS_ADDR,
        );
        // A minimal bwrap binding all of `/` so bash's dynamic linker resolves;
        // this test exercises the netns + proxy + seccomp composition, not the
        // confinement's bind set. NB no `--unshare-net` - the app inherits pasta's
        // namespace.
        let argv: Vec<String> = [
            "--unshare-user",
            "--unshare-pid",
            "--ro-bind",
            "/",
            "/",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--",
            "/usr/bin/bash",
            "-c",
            &probe,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let bpf = crate::seccomp::app_filter_bytes().expect("filter compiles");
        let code = spawn_filtered_and_wait(&argv, &[], None, bpf).expect("the filtered launch spawns");
        assert_eq!(
            code, 0,
            "the proxy must refuse the non-allowlisted CONNECT (403) through the full \
             netns+proxy composition; exit {code} (20=proxy unreachable, 30=unexpected reply)"
        );
    }

    /// §0 acceptance leg 3: the raw-IP-BYPASS defense. The route-absence wrapper
    /// (`ip route del default`) leaves the netns with only its private /24, whose
    /// sole live peer is the proxy gateway, so a confined app that IGNORES the
    /// `*_proxy` env and dials a raw EXTERNAL IP directly reaches NOTHING (no
    /// route), not the host's real network. This is the boundary that makes the
    /// proxy the WHOLE egress rather than a cooperative hint. Metal-only.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs pasta + bwrap + unprivileged userns on the host kernel"]
    fn a_filtered_launch_cannot_reach_a_raw_external_ip_bypassing_the_proxy() {
        use crate::egress::{EgressEnforcer, ProxyEgressEnforcer};
        // A live proxy for the launch (the netns needs its gateway peer); the
        // allowlist is irrelevant here - the probe never dials the proxy.
        let guard = ProxyEgressEnforcer
            .install(&["allowed.invalid:443".to_string()])
            .expect("bind the egress proxy");
        let _port = guard.proxy_port().expect("a filtered guard exposes its proxy port");

        // The confined probe: dial a raw external IP DIRECTLY (not the proxy
        // gateway), bypassing the `*_proxy` env. With the default route deleted the
        // connect has no route and fails, so exit 0; a successful connect (a bypass)
        // exits 30. 1.1.1.1 is a real routable address, so this proves route-absence
        // blocks a would-otherwise-be-reachable host, not merely an unroutable one.
        let probe =
            "exec 3<>/dev/tcp/1.1.1.1/443 2>/dev/null || exit 0; exit 30".to_string();
        let argv: Vec<String> = [
            "--unshare-user",
            "--unshare-pid",
            "--ro-bind",
            "/",
            "/",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--",
            "/usr/bin/bash",
            "-c",
            &probe,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let bpf = crate::seccomp::app_filter_bytes().expect("filter compiles");
        let code = spawn_filtered_and_wait(&argv, &[], None, bpf).expect("the filtered launch spawns");
        assert_eq!(
            code, 0,
            "a raw external-IP dial must find no route (route-absence); \
             exit {code} (30=the raw IP was reachable = a bypass of the proxy)"
        );
    }

    /// Killing the launcher must take the confined tree with it.
    ///
    /// This is the regression guard for a real leak: on the filtered path the
    /// process the launcher execs is `pasta`, and the `--die-with-parent` inside
    /// pasta's argv binds *bwrap* to *pasta*, not the launch to the launcher. So a
    /// killed launcher left `pasta -> bwrap -> bwrap -> app` alive, reparented to
    /// init, with its network namespace and forwarding proxy intact. `PR_SET_PDEATHSIG`
    /// in `child_pre_exec` closes it.
    ///
    /// Shaped as a fork because "the launcher dies" needs a launcher that is not the
    /// test process. The child plays launcher and blocks in the launch; the parent
    /// waits for the confined payload to appear, kills the child, and requires the
    /// payload to be gone. Needs pasta + bwrap + unprivileged userns, and
    /// `--test-threads=1` like the other fork test in this crate.
    ///
    /// **This fails INTERMITTENTLY - roughly half of runs - and the failure is
    /// real.** A failing run takes the full twenty-second wait and then finds
    /// survivors; a passing run finishes in under a tenth of a second, which is
    /// fast enough to suspect it is not observing the same state rather than
    /// evidence the teardown worked. Observed on a failure, after a SIGKILLed
    /// launcher: two `bwrap` processes still running, the outer one reparented to
    /// pid 1, and NO pasta process left. So the teardown is not "PDEATHSIG never
    /// reached pasta" - pasta died. bwrap outlived it, because `--die-with-parent`
    /// is a `PR_SET_PDEATHSIG` against bwrap's own parent and PDEATHSIG is both
    /// per-process and racy: a parent dying between fork and the prctl delivers
    /// nothing. Chained two deep through pasta, it does not hold.
    ///
    /// The cgroup would be the backstop, but `main` reaps the leaf with `kill_all`
    /// AFTER `wait` returns, and a SIGKILLed launcher never reaches that line.
    ///
    /// Fixing it means a teardown that does not chain PDEATHSIG - plausibly
    /// spawning bwrap as the launcher's DIRECT child and attaching pasta to an
    /// already-created netns (`pasta --netns`), so the app is one level down rather
    /// than two. That is a restructure of the filtered launch path, not a flag.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs pasta + bwrap + unprivileged userns on the host kernel"]
    fn killing_the_launcher_kills_the_confined_tree() {
        // A marker unique to this run, carried in the payload argv. pasta and bwrap
        // both repeat the app argv, so scanning for it finds the whole chain.
        let sentinel = format!("ARLEN-PDEATH-{}", std::process::id());

        /// Pids whose cmdline carries `sentinel`, excluding ourselves.
        fn matching(sentinel: &str) -> Vec<libc::pid_t> {
            let me = std::process::id();
            let mut out = Vec::new();
            let Ok(entries) = std::fs::read_dir("/proc") else {
                return out;
            };
            for e in entries.flatten() {
                let name = e.file_name();
                let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else {
                    continue;
                };
                if pid == me {
                    continue;
                }
                if let Ok(raw) = std::fs::read(format!("/proc/{pid}/cmdline")) {
                    if String::from_utf8_lossy(&raw).contains(sentinel) {
                        out.push(pid as libc::pid_t);
                    }
                }
            }
            out
        }

        let payload = format!("sleep 300 # {sentinel}");
        let argv: Vec<String> = [
            "--unshare-user",
            "--unshare-pid",
            "--ro-bind",
            "/",
            "/",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--",
            "/usr/bin/bash",
            "-c",
            &payload,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        // SAFETY: a deliberate fork in an `--test-threads=1` test (the same pattern
        // and the same reasoning as the Landlock fork self-test). The child only
        // performs the launch and then `_exit`s, never returning to the harness.
        let launcher = unsafe { libc::fork() };
        assert!(launcher >= 0, "fork failed");
        if launcher == 0 {
            let bpf = crate::seccomp::app_filter_bytes().unwrap_or_default();
            // Blocks for the payload's lifetime; the parent kills us out of it.
            let _ = spawn_filtered_and_wait(&argv, &[], None, bpf);
            unsafe { libc::_exit(0) };
        }

        // Wait for the confined payload to actually exist before killing anything,
        // or the test would pass on a launch that never started.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while matching(&sentinel).is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let started = matching(&sentinel);
        if started.is_empty() {
            unsafe {
                libc::kill(launcher, libc::SIGKILL);
                libc::waitpid(launcher, std::ptr::null_mut(), 0);
            }
            panic!("the confined launch never started; nothing to observe");
        }

        // Kill the launcher and reap it, so the tree's only remaining anchor is gone.
        unsafe {
            libc::kill(launcher, libc::SIGKILL);
            libc::waitpid(launcher, std::ptr::null_mut(), 0);
        }

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !matching(&sentinel).is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let survivors = matching(&sentinel);

        // Clean up BEFORE asserting: a failure here means processes are still running,
        // and leaving a confined network daemon behind to prove a point is not on.
        for pid in &survivors {
            unsafe { libc::kill(*pid, libc::SIGKILL) };
        }

        assert!(
            survivors.is_empty(),
            "the confined tree outlived its launcher ({} process(es) left); \
             the PDEATHSIG chain through pasta does not hold - see this test's doc",
            survivors.len()
        );
    }

    /// The in-sandbox Landlock fence composes with the FILTERED path (pasta netns +
    /// the seccomp-wrapper file). To isolate the FENCE's contribution from bwrap's
    /// mount namespace, bwrap binds TWO dirs read-write but the fence grants only
    /// one: a write to the granted dir must succeed, a write to the ungranted-but-
    /// mount-writable dir must be DENIED by the fence (not the mount), and /dev/null
    /// must still be writable. Runs the real arlen-run binary as the `--landlock-exec`
    /// wrapper. Metal-only (pasta + bwrap + userns + Landlock).
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs pasta + bwrap + unprivileged userns + Landlock on the host kernel"]
    fn the_filtered_launch_fences_the_app_beyond_the_mount_namespace() {
        // The arlen-run binary sits beside the test binary's target/debug/deps dir.
        let exe = std::env::current_exe().unwrap();
        let arlen_run = exe.parent().unwrap().parent().unwrap().join("arlen-run");
        assert!(arlen_run.exists(), "arlen-run built at {}", arlen_run.display());

        // A host dir under /var/tmp (an existing path NOT in the fence's standard-
        // writable list, unlike /tmp), with granted/ + ungranted/ subdirs, bound
        // read-WRITE at its own path so both subdirs are mount-writable. The fence
        // grants only the granted subdir, so a denial of the ungranted one isolates
        // the fence's effect from the mount namespace.
        let base = tempfile::Builder::new()
            .prefix("arlen-flt-fence-")
            .tempdir_in("/var/tmp")
            .expect("temp dir under /var/tmp");
        let base_path = base.path().to_string_lossy().into_owned();
        std::fs::create_dir(base.path().join("granted")).unwrap();
        std::fs::create_dir(base.path().join("ungranted")).unwrap();
        let granted = format!("{base_path}/granted");
        let ungranted = format!("{base_path}/ungranted");
        // granted write ok, /dev/null ok, ungranted (mount-rw but fence-denied) must
        // fail -> exit 0 only if the fence is genuinely applied over the mount.
        let script = format!(
            "echo a > '{granted}/f' || exit 10; echo n > /dev/null || exit 11; \
             if echo b > '{ungranted}/f' 2>/dev/null; then exit 20; fi; exit 0"
        );
        // The fence grants ONLY the granted subdir; bwrap binds the base rw.
        let program = crate::landlock_exec::landlock_exec_program(
            &arlen_run.to_string_lossy(),
            &[std::path::PathBuf::from(&granted)],
            &["/bin/sh".to_string(), "-c".to_string(), script],
        );
        let mut argv: Vec<String> = [
            "--unshare-user", "--unshare-pid", "--ro-bind", "/", "/", "--proc", "/proc",
            "--dev", "/dev", "--bind", &base_path, &base_path, "--",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        argv.extend(program);
        let bpf = crate::seccomp::app_filter_bytes().expect("filter compiles");
        let code = spawn_filtered_and_wait(&argv, &[], None, bpf).expect("filtered launch spawns");
        assert_eq!(
            code, 0,
            "through pasta+seccomp+landlock the fence must permit the granted dir + \
             /dev/null and DENY the ungranted (mount-writable) dir; exit {code} \
             (10=granted failed, 11=/dev/null denied, 20=ungranted write ALLOWED = \
             fence not applied over the mount)"
        );
    }
}
