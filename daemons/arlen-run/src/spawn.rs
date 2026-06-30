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
/// Wayland and PipeWire sockets and the session D-Bus, bound read-write (they
/// are sockets). Only sockets that actually exist are bound, because `bwrap`
/// fails the launch on a bind whose source is missing; the `exists` predicate
/// is injected so the mapping is pure and testable without a real session.
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
    env: BTreeMap<String, String>,
    net: NetworkPolicy,
    plumbing: Vec<Bind>,
) -> Result<Confinement, ConfinerError> {
    let dir_refs: Vec<&Path> = app_dirs.iter().map(PathBuf::as_path).collect();
    let masked_refs: Vec<&Path> = masked.iter().map(PathBuf::as_path).collect();
    let skeleton = app_runtime_profile(usr, &dir_refs, &masked_refs, env, net)?;
    Ok(skeleton.complete(plumbing, Vec::new()))
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
/// inherited fd above stderr, start a new process group, and apply Landlock
/// over `writable` (so bwrap and the app both inherit the filesystem
/// confinement). The order matters: Landlock opens path fds, so it must precede
/// the seccomp filter (a later commit) that removes `openat`.
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
) -> std::io::Result<u8> {
    use std::os::unix::process::{CommandExt, ExitStatusExt};

    let writable: Vec<PathBuf> = writable.to_vec();

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
    full_argv.extend_from_slice(argv);

    let mut cmd = Command::new("bwrap");
    cmd.args(&full_argv);
    // SAFETY: the closure runs in the child after fork, before exec. The
    // launcher is single-threaded so the post-fork child is too, making the
    // ruleset allocations safe; the syscalls (close_range, fcntl, setpgid, the
    // Landlock setup) only narrow the child's own capabilities.
    unsafe {
        cmd.pre_exec(move || {
            // Mark every fd above stderr close-on-exec so no launcher fd (daemon
            // sockets, the profile file) leaks into the confined app, while
            // leaving them open through pre_exec and execve. Using
            // CLOSE_RANGE_CLOEXEC instead of an immediate close is deliberate:
            // std::process reports a pre_exec OR execve failure to the parent
            // over a CLOEXEC pipe (an fd >= 3), so closing fds outright would
            // make a setpgid / cgroup-join / Landlock / exec failure
            // unreportable (the child still _exits before exec, so it is never
            // fail-open, but the parent would misread the cause). Marking
            // CLOEXEC closes every launcher fd atomically on a successful exec
            // and keeps the error pipe alive until then. Needs kernel >= 5.11,
            // below the Landlock >= 5.13 floor this launcher already requires.
            let rc =
                libc::close_range(3, libc::c_uint::MAX, libc::CLOSE_RANGE_CLOEXEC as libc::c_int);
            if rc != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The seccomp memfd must reach bwrap (it reads `--seccomp <fd>`), so
            // re-clear the CLOEXEC bit close_range just set on it. Done right
            // after close_range so nothing re-marks it; the error pipe stays
            // CLOEXEC, only this one fd is kept open across the exec.
            if let Some(fd) = seccomp_fd {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            // Put the child in its own process group so a stray signal to the
            // launcher's group does not race the cgroup-based reaping.
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // Join the per-launch cgroup BEFORE Landlock: a read-only `/`
            // ruleset would deny the write to cgroup.procs.
            if let Some(procs) = &cgroup_procs {
                crate::cgroup::join_current(procs)?;
            }
            // Filesystem confinement, inherited by bwrap and the app. The app
            // seccomp filter (which bwrap installs after this, on the app only)
            // may drop path-open, so Landlock's path opens must happen first.
            crate::landlock_apply::apply_landlock(&writable)?;
            Ok(())
        });
    }

    let spawned = cmd.spawn();
    // The child inherited the memfd at fork; the parent's copy is no longer
    // needed and is closed regardless of how the spawn went.
    if let Some(fd) = seccomp_fd {
        unsafe { libc::close(fd) };
    }
    let status = spawned?.wait()?;
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
            BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]),
            NetworkPolicy::None,
            Vec::new(),
        )
        .unwrap();
        let argv = bwrap_argv(&conf, &["/usr/bin/echo".into(), "hi".into()]);
        let code = spawn_and_wait(&argv, &[], None, None).expect("bwrap spawns");
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
            BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]),
            NetworkPolicy::None,
            Vec::new(),
        )
        .unwrap();
        let argv = bwrap_argv(&conf, &["/usr/bin/echo".into(), "hi".into()]);
        let bpf = crate::seccomp::app_filter_bytes().expect("filter compiles");
        let code = spawn_and_wait(&argv, &[], None, Some(bpf)).expect("bwrap spawns");
        assert_eq!(code, 0, "the allowlist must permit a basic confined exec");
    }
}
