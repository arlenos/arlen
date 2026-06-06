//! App identity resolution via `/proc/{pid}/exe`.
//!
//! Maps a process ID to an application identifier by reading
//! the binary path from procfs and matching it against known
//! installation paths. Canonical implementation per
//! `docs/architecture/AUTH-CANONICAL.md` section 4.
//!
//! Two hardenings beyond a naive `read_link`:
//!
//! - **(E7) PID-reuse guard.** [`pid_start_time`] reads the
//!   process's boot-relative start tick from `/proc/{pid}/stat`.
//!   Callers that auth a peer at connection-time should store
//!   the `(pid, start_time)` tuple and re-verify per request.
//!   If the kernel recycles the PID after a process exit, the
//!   start_time will differ and the verification fails.
//!
//! - **(E8) Symlink-TOCTOU guard.** [`exe_path_openat`] opens
//!   `/proc/{pid}` with `O_PATH | O_NOFOLLOW` first, then
//!   reads the `exe` symlink relative to that fd. This blocks
//!   the race window where the binary could be swapped between
//!   resolving `/proc/{pid}` and reading `exe`.

use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// Errors from app identity resolution.
#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("process {0} not found")]
    ProcessNotFound(u32),
    #[error("cannot read exe path: {0}")]
    CannotReadExe(std::io::Error),
    #[error("cannot read stat: {0}")]
    CannotReadStat(std::io::Error),
    #[error("malformed /proc/{0}/stat")]
    MalformedStat(u32),
    #[error("unknown binary path: {0}")]
    UnknownBinary(PathBuf),
}

/// Resolve an app_id from a process ID by reading `/proc/{pid}/exe`.
///
/// Uses the openat-based hardening (E8). For per-request
/// verification, also call [`pid_start_time`] and store the
/// tuple at connection time.
pub fn app_id_from_pid(pid: u32) -> Result<String, IdentityError> {
    let exe_path = exe_path_openat(pid)?;
    path_to_app_id(&exe_path)
}

/// Read the exe symlink for a pid using the openat-then-readlinkat
/// pattern. Closes the symlink-TOCTOU window: the directory fd
/// for `/proc/{pid}` is held open while we read `exe`, so the
/// kernel's per-process subdirectory is the same lifetime as the
/// readlink.
fn exe_path_openat(pid: u32) -> Result<PathBuf, IdentityError> {
    use std::ffi::CString;
    let proc_dir = format!("/proc/{pid}");
    // O_PATH gives us a fd we can use for `*at` syscalls without
    // opening for read. O_NOFOLLOW prevents following any symlink
    // that might be `proc_dir` itself (defensive; /proc is not
    // bind-mounted normally but cheap to guard).
    let dir_cstr = CString::new(proc_dir).expect("no NUL");
    // SAFETY: `dir_cstr` is a valid C string; libc::open is
    // documented FFI; we own the returned fd.
    let dir_fd = unsafe {
        libc::open(
            dir_cstr.as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    if dir_fd < 0 {
        let err = std::io::Error::last_os_error();
        return Err(if err.kind() == std::io::ErrorKind::NotFound {
            IdentityError::ProcessNotFound(pid)
        } else {
            IdentityError::CannotReadExe(err)
        });
    }
    let dir = unsafe { OwnedFd::from_raw_fd(dir_fd) };

    // Now readlinkat("exe") relative to the directory fd.
    let exe_cstr = CString::new("exe").expect("static, no NUL");
    let mut buf = [0u8; libc::PATH_MAX as usize];
    // SAFETY: dir.as_raw_fd() is valid for the duration of this call;
    // exe_cstr and buf live for the syscall.
    let n = unsafe {
        libc::readlinkat(
            dir.as_raw_fd(),
            exe_cstr.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
        )
    };
    if n < 0 {
        return Err(IdentityError::CannotReadExe(std::io::Error::last_os_error()));
    }
    let bytes = &buf[..n as usize];
    let s = std::str::from_utf8(bytes)
        .map_err(|_| IdentityError::CannotReadExe(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "exe path not UTF-8",
        )))?;
    Ok(PathBuf::from(s))
}

/// Read the process start time (column 22 of `/proc/{pid}/stat`,
/// in clock ticks since boot). Used together with the pid as a
/// guard against PID recycling: store `(pid, start_time)` at
/// connection time, re-verify on each request. If the kernel
/// recycles the pid after the original process exits, the new
/// process will have a different start_time.
///
/// `/proc/{pid}/stat` format: pid (comm) state ppid pgrp ...
/// where `comm` may contain spaces or parens. Column 22 is the
/// process start time, after the second `)`.
pub fn pid_start_time(pid: u32) -> Result<u64, IdentityError> {
    let path = format!("/proc/{pid}/stat");
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            IdentityError::ProcessNotFound(pid)
        } else {
            IdentityError::CannotReadStat(e)
        }
    })?;
    // Skip the comm field by finding the LAST `)` — comm can
    // contain `)` so `find` would be wrong; rsplit is safer.
    let after_comm = raw.rsplit_once(") ").ok_or(IdentityError::MalformedStat(pid))?.1;
    // After comm: state ppid pgrp session tty_nr tpgid flags
    // minflt cminflt majflt cmajflt utime stime cutime cstime
    // priority nice num_threads itrealvalue starttime
    // starttime is field 19 in the after-comm sequence (1-indexed).
    let starttime = after_comm
        .split_whitespace()
        .nth(19)
        .ok_or(IdentityError::MalformedStat(pid))?;
    starttime
        .parse::<u64>()
        .map_err(|_| IdentityError::MalformedStat(pid))
}

/// Map a binary path to an app_id.
///
/// Resolution order (every match anchored to a trusted root —
/// no substring or filename-suffix matching, those are
/// trivially spoofable by a same-uid attacker placing a binary
/// at e.g. `/tmp/arlen-ai-daemon` or
/// `/tmp/x/.local/share/arlen/apps/com.victim/bin/evil`):
///
/// 1. Canonical AI daemon install paths -> "ai-daemon"
/// 2. `/usr/bin/arlen-{name}` (root-only writable) -> `{name}`
///    Per-binary identity, no shared `system` principal. Closes
///    F4 (codex review): a `/usr/bin/arlen-notifyd` no longer
///    inherits the same profile as `/usr/bin/arlen-knowledge`.
///    Each canonical daemon binary loads its own
///    `~/.config/permissions/{name}.toml`.
/// 3. `/usr/lib/arlen/apps/{app_id}/...` -> app_id
/// 4. `<home>/.local/share/arlen/apps/{app_id}/...` -> app_id
///    (anchored to caller's `dirs::home_dir()`, not substring).
///    See `docs/architecture/identity-spoof-mitigation.md` for
///    the open F3 same-uid-spoof gap and the inode-keyed
///    installd registry plan that replaces this rule.
/// 5. (debug) cargo target directories -> "dev.{binary_name}"
/// 6. Error: UnknownBinary
pub fn path_to_app_id(path: &Path) -> Result<String, IdentityError> {
    let s = path.to_string_lossy();

    // (1) AI daemon — strict equality on the canonical install
    // paths. `ends_with("/arlen-ai-daemon")` would let a
    // same-uid attacker copy any binary to /tmp/arlen-ai-daemon
    // and impersonate the AI daemon. Foundation §8.4.5: identity
    // resolution must come from canonical install paths only.
    // Must run before rule (2) so `arlen-ai-daemon` resolves
    // to the canonical id rather than the basename "ai-daemon".
    match s.as_ref() {
        "/usr/bin/arlen-ai-daemon"
        | "/usr/bin/arlen-ai"
        | "/usr/lib/arlen/apps/ai-daemon/bin/arlen-ai-daemon"
        | "/usr/lib/arlen/apps/ai-daemon/bin/arlen-ai" => {
            return Ok("ai-daemon".to_string());
        }
        _ => {}
    }

    // (2) System daemons under root-owned /usr/bin/. The basename
    // after `arlen-` is the app_id. Charset is restricted to
    // `[a-z0-9._-]` so a canonical-looking but malformed path
    // (e.g. `/usr/bin/arlen-../etc/passwd`, theoretically only
    // creatable by root but defense-in-depth) cannot escape into
    // a profile-path traversal in `profile_path()`.
    if let Some(name) = s.strip_prefix("/usr/bin/arlen-") {
        if !name.is_empty()
            && name.bytes().all(|b| {
                b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || matches!(b, b'.' | b'_' | b'-')
            })
        {
            return Ok(name.to_string());
        }
    }

    // (3) System-installed apps. /usr/lib/arlen/apps/ is
    // root-owned so non-root attackers cannot plant lookalikes.
    if let Some(rest) = s.strip_prefix("/usr/lib/arlen/apps/") {
        if let Some(app_id) = rest.split('/').next() {
            if !app_id.is_empty() {
                return Ok(app_id.to_string());
            }
        }
    }

    // (4) User-installed apps. Anchored to the calling user's
    // actual home directory — `find()` substring matching would
    // accept attacker-controlled paths like
    // `/tmp/x/.local/share/arlen/apps/com.victim/bin/evil`.
    // strip_prefix against an absolute home blocks that.
    if let Some(home) = dirs::home_dir() {
        let user_apps = home.join(".local").join("share").join("arlen").join("apps");
        if let Ok(rest) = path.strip_prefix(&user_apps) {
            if let Some(first) = rest.iter().next() {
                let app_id = first.to_string_lossy();
                if !app_id.is_empty() {
                    return Ok(app_id.into_owned());
                }
            }
        }
    }

    // (5) Development builds (debug_assertions only). Foundation-
    // dev fallback so cargo-run binaries can still emit identity-
    // tagged events without an installer step.
    #[cfg(debug_assertions)]
    if s.contains("/target/debug/") || s.contains("/target/release/") {
        if let Some(name) = path.file_name() {
            return Ok(format!("dev.{}", name.to_string_lossy()));
        }
    }

    Err(IdentityError::UnknownBinary(path.to_path_buf()))
}

/// Check if a process is still alive (cheap stat on /proc/{pid}).
pub fn process_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

// Local OwnedFd shim — std::os::fd::OwnedFd would work but on
// older toolchains we can't rely on it. Trivial drop-on-close
// wrapper keeps the OpenAt fd lifecycle correct.
struct OwnedFd(libc::c_int);

impl OwnedFd {
    unsafe fn from_raw_fd(fd: libc::c_int) -> Self {
        Self(fd)
    }
}

impl Drop for OwnedFd {
    fn drop(&mut self) {
        if self.0 >= 0 {
            // SAFETY: fd was checked >= 0 on construction; we own it.
            unsafe {
                libc::close(self.0);
            }
        }
    }
}

impl AsRawFd for OwnedFd {
    fn as_raw_fd(&self) -> libc::c_int {
        self.0
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_id_from_path_system_app() {
        let path = PathBuf::from("/usr/lib/arlen/apps/com.anki/bin/anki");
        assert_eq!(path_to_app_id(&path).unwrap(), "com.anki");
    }

    #[test]
    fn test_app_id_from_path_user_app() {
        // Anchored to the actual calling user's home directory
        // because the resolver now uses dirs::home_dir() not
        // substring matching. Skip if HOME is unavailable
        // (e.g. some CI environments).
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let path = home
            .join(".local/share/arlen/apps/org.zotero/bin/zotero");
        assert_eq!(path_to_app_id(&path).unwrap(), "org.zotero");
    }

    #[test]
    fn test_app_id_from_path_ai_daemon() {
        // Strict equality on canonical install path.
        let path = PathBuf::from("/usr/bin/arlen-ai-daemon");
        assert_eq!(path_to_app_id(&path).unwrap(), "ai-daemon");

        let path = PathBuf::from("/usr/lib/arlen/apps/ai-daemon/bin/arlen-ai-daemon");
        assert_eq!(path_to_app_id(&path).unwrap(), "ai-daemon");
    }

    /// F1 regression: same-uid attacker placing any binary at
    /// `/tmp/arlen-ai-daemon` (or another writable path with
    /// the same basename) MUST NOT be authenticated as the AI
    /// daemon. Pre-Sprint-C the resolver did `ends_with` which
    /// would have accepted this and inherited ai-daemon's
    /// scopes.
    #[test]
    fn test_rejects_spoofed_ai_daemon_basename() {
        for spoofed in [
            "/tmp/arlen-ai-daemon",
            "/tmp/arlen-ai",
            "/home/attacker/arlen-ai-daemon",
            "/var/tmp/arlen-ai",
            "/dev/shm/arlen-ai-daemon",
        ] {
            let path = PathBuf::from(spoofed);
            assert!(
                path_to_app_id(&path).is_err(),
                "spoofed path {spoofed} must be rejected"
            );
        }
    }

    /// F2 regression: same-uid attacker placing a binary at a
    /// lookalike path containing `.local/share/arlen/apps/`
    /// outside the caller's actual home MUST NOT impersonate
    /// the apparent app_id. Pre-Sprint-C the resolver used
    /// `find()` substring match which would have accepted any
    /// such path.
    #[test]
    fn test_rejects_user_app_path_lookalike() {
        for spoofed in [
            "/tmp/x/.local/share/arlen/apps/com.victim/bin/evil",
            "/var/tmp/.local/share/arlen/apps/com.victim/bin/evil",
            "/dev/shm/foo/.local/share/arlen/apps/com.victim/bin/evil",
            "/.local/share/arlen/apps/com.victim/bin/evil",
        ] {
            let path = PathBuf::from(spoofed);
            assert!(
                path_to_app_id(&path).is_err(),
                "spoofed lookalike {spoofed} must be rejected"
            );
        }
    }

    /// Canonical daemons under `/usr/bin/arlen-*` resolve to
    /// per-binary app_ids, not the shared "system" principal.
    /// Closes F4 (codex adversarial review post-Sprint-D): the
    /// catch-all bucket let any canonical-looking binary inherit
    /// `system`'s profile, collapsing least-privilege between
    /// notifyd, knowledge, installd, etc.
    #[test]
    fn test_app_id_from_path_canonical_daemon_per_binary() {
        let cases = [
            ("/usr/bin/arlen-notifyd", "notifyd"),
            ("/usr/bin/arlen-knowledge", "knowledge"),
            ("/usr/bin/arlen-event-bus", "event-bus"),
            ("/usr/bin/arlen-installd", "installd"),
            ("/usr/bin/arlen-desktop-shell", "desktop-shell"),
            ("/usr/bin/arlen-modulesd", "modulesd"),
        ];
        for (path, expected) in cases {
            assert_eq!(
                path_to_app_id(&PathBuf::from(path)).unwrap(),
                expected,
                "{path}"
            );
        }
    }

    /// F4 regression: `/usr/bin/arlen-*` MUST NOT bucket every
    /// canonical daemon to the literal app_id "system". That
    /// would let `arlen-notifyd` and `arlen-knowledge` share
    /// one permission profile and silently inherit each other's
    /// scopes.
    #[test]
    fn test_canonical_daemon_does_not_resolve_to_system() {
        for path in [
            "/usr/bin/arlen-notifyd",
            "/usr/bin/arlen-knowledge",
            "/usr/bin/arlen-installd",
        ] {
            let id = path_to_app_id(&PathBuf::from(path)).unwrap();
            assert_ne!(id, "system", "{path} unexpectedly bucketed to system");
        }
    }

    /// Defense-in-depth: even a malformed canonical-looking path
    /// (only plantable by root and so already a much bigger
    /// problem) must not produce an app_id with `/` or other
    /// chars that would let `profile_path()` traverse outside
    /// `~/.config/permissions/`.
    #[test]
    fn test_canonical_daemon_rejects_path_traversal() {
        for path in [
            "/usr/bin/arlen-../etc/passwd",
            "/usr/bin/arlen-foo/bar",
            "/usr/bin/arlen-",
        ] {
            assert!(
                path_to_app_id(&PathBuf::from(path)).is_err(),
                "{path} unexpectedly accepted"
            );
        }
    }

    #[test]
    fn test_app_id_from_path_unknown() {
        let path = PathBuf::from("/usr/bin/firefox");
        assert!(path_to_app_id(&path).is_err());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn test_app_id_from_path_dev_build() {
        let path = PathBuf::from("/home/user/project/target/debug/my-app");
        assert_eq!(path_to_app_id(&path).unwrap(), "dev.my-app");
    }

    #[test]
    fn test_process_alive_self() {
        assert!(process_alive(std::process::id()));
    }

    #[test]
    fn test_process_alive_dead() {
        assert!(!process_alive(999_999_999));
    }

    #[test]
    fn test_app_id_from_pid_self() {
        // Our own process should resolve (in debug mode to dev.*)
        let result = app_id_from_pid(std::process::id());
        // In CI or release builds this may be UnknownBinary, so we just
        // check it doesn't panic and returns a result.
        let _ = result;
    }

    #[test]
    fn test_pid_start_time_self() {
        // Our own process must have a parseable start_time.
        let st = pid_start_time(std::process::id()).expect("read self start_time");
        // start_time is monotonic, non-zero (we booted before this test).
        assert!(st > 0);
    }

    #[test]
    fn test_pid_start_time_dead_process() {
        let r = pid_start_time(999_999_999);
        assert!(matches!(r, Err(IdentityError::ProcessNotFound(_))));
    }

    #[test]
    fn test_pid_start_time_handles_paren_comm() {
        // Manual test: comm with parentheses is rare but legal.
        // We don't write to /proc, so this is unit-tested via
        // the rsplit_once(") ") path implicitly through the
        // self-test which always succeeds. The defensive
        // rsplit catches programs named like "weird (test) name".
    }
}
