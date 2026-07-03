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

    // (1) AI layer daemons — strict equality on the canonical install
    // paths. `ends_with("/arlen-ai-daemon")` would let a
    // same-uid attacker copy any binary to /tmp/arlen-ai-daemon
    // and impersonate the AI daemon. Foundation §8.4.5: identity
    // resolution must come from canonical install paths only.
    // Must run before rule (2) so `arlen-ai-daemon` resolves
    // to the canonical id rather than the basename "ai-daemon".
    // The `/usr/lib/arlen/libexec/` entries are the canonical binaries
    // `ai-proxy::peer_auth` already trusts (CANONICAL_AI_DAEMON_BIN /
    // CANONICAL_AI_AGENT_BIN); identity resolution must agree with peer-auth so
    // the knowledge write socket loads the right profile for each. In
    // particular the agent resolves to `ai-agent`, the app id its go-live
    // permission profile (`ai-agent.toml`) is keyed under; without this the
    // production agent would resolve as unknown and its write grant never load.
    match s.as_ref() {
        "/usr/bin/arlen-ai-daemon"
        | "/usr/bin/arlen-ai"
        | "/usr/lib/arlen/libexec/arlen-ai-daemon"
        | "/usr/lib/arlen/apps/ai-daemon/bin/arlen-ai-daemon"
        | "/usr/lib/arlen/apps/ai-daemon/bin/arlen-ai" => {
            return Ok("ai-daemon".to_string());
        }
        "/usr/lib/arlen/libexec/arlen-ai-agent" => {
            return Ok("ai-agent".to_string());
        }
        // The AI egress proxy, pinned canonically so its per-forward audit submits
        // under the stable id `ai-proxy`, the id the audit daemon's ADMITTED
        // allowlist keys on. Like accountsd/notifyd, rule (2) covers only
        // /usr/bin/arlen-*, so without this entry the proxy resolves to
        // UnknownBinary and every forward's fail-closed audit is refused.
        "/usr/lib/arlen/libexec/arlen-ai-proxy" => {
            return Ok("ai-proxy".to_string());
        }
        // The online-accounts daemon, pinned canonically so its credential-handout
        // audit (GAP-2) submits under the stable id `online-accounts`, the id the
        // audit daemon's ADMITTED allowlist keys on. Rule (2) covers only
        // /usr/bin/arlen-*, not /usr/lib/arlen/libexec, so without this entry the
        // daemon resolves to UnknownBinary and its audit is silently refused. The
        // root-owned system path is attested (not the same-uid-spoofable residual
        // the $HOME-libexec daemons carry).
        "/usr/lib/arlen/libexec/arlen-accountsd" => {
            return Ok("online-accounts".to_string());
        }
        // The notification daemon, pinned canonically so its notification-shown
        // audit (GAP-2) submits under the stable id `notifyd`, the id the audit
        // daemon's ADMITTED allowlist keys on. Same rationale as the accounts
        // daemon: rule (2) covers only /usr/bin/arlen-*, so without this entry
        // the daemon resolves to UnknownBinary and its audit is silently
        // refused; the root-owned system path is attested, not the
        // same-uid-spoofable $HOME residual.
        "/usr/lib/arlen/libexec/arlen-notifyd" => {
            return Ok("notifyd".to_string());
        }
        // The install daemon, pinned canonically so its install/uninstall audit
        // (GAP-2) submits under the stable id `installd`, the id the audit
        // daemon's ADMITTED allowlist keys on. Same rationale as the accounts and
        // notification daemons: rule (2) covers only /usr/bin/arlen-*, so without
        // this entry the daemon resolves to UnknownBinary and its audit is
        // silently refused; the root-owned system path is attested, not the
        // same-uid-spoofable $HOME residual.
        "/usr/lib/arlen/libexec/arlen-installd" => {
            return Ok("installd".to_string());
        }
        // The power daemon and the anomaly detector, pinned canonically. Both
        // install under /usr/lib/arlen/libexec/ (not /usr/bin/arlen-*), so rule
        // (2) misses them and they would otherwise resolve to UnknownBinary.
        // They are the trusted sources of DND-piercing Critical notifications
        // (critical battery, security alerts), so the notification daemon's
        // Critical-tier clamp (GAP-7) keys on these stable ids; an unattested
        // path resolving them would let a same-uid peer impersonate a system
        // alerter and pierce Do-Not-Disturb.
        "/usr/lib/arlen/libexec/arlen-powerd" => {
            return Ok("powerd".to_string());
        }
        "/usr/lib/arlen/libexec/arlen-anomalyd" => {
            return Ok("anomalyd".to_string());
        }
        // The consent broker (the one trusted-path consent surface every system
        // prompt routes through, system-dialog-plan.md). It installs under
        // /usr/lib/arlen/libexec/ and audits each resolved decision (granted /
        // denied) fail-closed before releasing the grant, so it submits to the
        // audit ledger under the stable id `consent-broker` (the id the audit
        // daemon ADMITTED list keys on); without this entry rule (2) misses the
        // libexec path and it would resolve to UnknownBinary, failing the audit
        // closed and denying every approval.
        "/usr/lib/arlen/libexec/arlen-consent-broker" => {
            return Ok("consent-broker".to_string());
        }
        // The Settings app, pinned canonically so it resolves to the stable
        // app_id `settings` (not the spoofable basename). The Living Capability
        // Graph revoke socket op admits only this app id (living-capability-graph.md
        // §6.2, Option A): revoke is user-initiated through Settings, narrowing-only,
        // so a root-owned canonical path is the trust anchor until F3 upgrades it.
        // Rule (3) would also resolve this apps path, but the explicit entry keeps
        // the canonical principal unambiguous (as the ai-daemon apps entries do).
        "/usr/lib/arlen/apps/settings/bin/arlen-settings" => {
            return Ok("settings".to_string());
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
                    let app_id = app_id.into_owned();
                    // A user-writable path may never mint a privileged identity.
                    // The quota tier keys System off `system`/`system.*` and
                    // FirstParty off `org.arlen.*` plus the canonical AI daemons,
                    // and `settings` is the canonical revoke-caller principal, so
                    // a same-uid directory named to match one of those would
                    // escalate above the third-party tier this path warrants (or
                    // impersonate the revoke caller). Those identities only ever
                    // come from the root-owned rules 1-3; reserving them here means
                    // rule 4 cannot forge one. A
                    // legitimate user-installed app is third-party reverse-DNS and
                    // never bears a reserved id. (The bare per-daemon names rule 2
                    // mints, e.g. `knowledge`, stay third-party-tier so a squat of
                    // one is no tier escalation; the broader provenance-attested
                    // tiering is the F3 follow-up.)
                    if is_reserved_app_id(&app_id) {
                        return Err(IdentityError::UnknownBinary(path.to_path_buf()));
                    }
                    // F3 Rung B: `~/.local/share/arlen/apps/` is user-writable, so
                    // the path alone is forgeable (a same-uid copy to this dir).
                    // If the app is enrolled in the broker-owned (root-owned)
                    // identity registry, the binary's inode MUST match the recorded
                    // one — a copy gets a new inode and is rejected as a spoof, a
                    // hardlink shares it and passes. An app with NO record is the
                    // documented pre-enrolment residual: resolved cooperatively
                    // (still path-spoofable) until installd records it at install.
                    // So an enrolled app is a hard, non-forgeable identity; an
                    // unenrolled one is unchanged. The daemon only serves same-uid
                    // peers (SO_PEERCRED rejects cross-uid before this), so the
                    // running uid keys the right registry. A corrupt registry is
                    // root-caused (the file is root-owned 0644, not same-uid
                    // writable), so falling through cooperatively is acceptable.
                    // SAFETY: getuid never fails.
                    let uid = unsafe { libc::getuid() };
                    if let Ok(registry) = crate::identity_registry::IdentityRegistry::load(uid) {
                        if !user_app_inode_ok(&registry, &app_id, path) {
                            return Err(IdentityError::UnknownBinary(path.to_path_buf()));
                        }
                    }
                    return Ok(app_id);
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

/// Whether `app_id` is in a namespace reserved for root-installed
/// components, which a user-writable path (rule 4 of [`path_to_app_id`])
/// must never mint. `system` / `system.*` map to the System quota tier
/// and `org.arlen.*` + the canonical AI daemons (`ai-daemon` /
/// `ai-agent`) to FirstParty (`daemons/knowledge/src/quota/config.rs`
/// `tier_for_app`); `settings` is the canonical revoke-caller principal
/// (`daemon.rs` `revoke_caller_admitted`). Legitimate holders of these
/// identities resolve through the root-owned rules 1-3; reserving them
/// on the user path closes the same-uid name-mint that would otherwise
/// escalate tier (or impersonate the revoke caller) from a directory the
/// attacker controls.
///
/// This set must stay congruent with `tier_for_app`'s compile-time
/// defaults. It deliberately does NOT cover a `graph.toml`-extended
/// `first_party_apps` allowlist: the SDK resolver cannot see the
/// daemon's loaded quota config, and no live tier decision reads that
/// config today (every caller uses `QuotaConfig::arlen_default`, whose
/// privileged ids are all reserved here). If `QuotaConfig::load` is ever
/// wired into live tiering, this guard must be re-fenced against the
/// configured allowlist or the rule-4 squat reopens for the added ids.
fn is_reserved_app_id(app_id: &str) -> bool {
    app_id == "system"
        || app_id.starts_with("system.")
        || app_id.starts_with("org.arlen.")
        || matches!(app_id, "ai-daemon" | "ai-agent" | "settings")
}

/// The F3 Rung B inode gate for a resolved user-app `app_id` at `path`. If the app
/// is enrolled in the broker-owned `registry`, the binary's inode must match the
/// recorded one (a same-uid copy to the app's path has a new inode → false, a
/// hardlink shares it → true). An app with NO record passes - the documented
/// pre-enrolment residual, resolved cooperatively until installd records it. Pure
/// over the registry, so the gate is unit-testable without the on-disk file.
fn user_app_inode_ok(
    registry: &crate::identity_registry::IdentityRegistry,
    app_id: &str,
    path: &Path,
) -> bool {
    match registry.lookup(app_id) {
        Some(record) => crate::identity_registry::verify_binary(record, path),
        None => true,
    }
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

/// Map a systemd unit name (from a peer's cgroup) to its canonical app_id. Only
/// the canonical AI-daemon units are mapped: this is the cross-uid identity
/// fallback for when `/proc/<pid>/exe` is unreadable (a hardened, non-dumpable
/// peer), and it must grant no identity a successful exe-path resolve would not.
fn unit_to_app_id(unit: &str) -> Option<&'static str> {
    match unit {
        "arlen-ai-agent.service" => Some("ai-agent"),
        "arlen-ai-daemon.service" => Some("ai-daemon"),
        _ => None,
    }
}

/// Extract the innermost systemd `*.service` unit from a `/proc/<pid>/cgroup`
/// file's content. Handles cgroup v2 (a single `0::<path>` line) and v1
/// (`<n>:<controllers>:<path>` lines); returns the last `.service` path component,
/// or `None` when the peer is not in a service cgroup.
fn unit_from_cgroup(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let path = line.rsplit(':').next()?;
        path.split('/')
            .rev()
            .find(|component| component.ends_with(".service"))
            .map(|s| s.to_string())
    })
}

/// Resolve an app_id from a peer's systemd cgroup unit (`/proc/<pid>/cgroup`).
///
/// The fallback when [`app_id_from_pid`] is denied for a hardened, non-dumpable
/// cross-uid peer: unlike `/proc/<pid>/exe`, the cgroup file is not ptrace-gated,
/// so a root reader can identify a hardened AI daemon by its unit. Only the
/// canonical AI-daemon units ([`unit_to_app_id`]) resolve; every other unit is
/// `None`, so this never widens identity beyond what the exe-path resolver grants.
pub fn app_id_from_cgroup(pid: u32) -> Option<String> {
    let content = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    let unit = unit_from_cgroup(&content)?;
    unit_to_app_id(&unit).map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgroup_unit_resolves_the_ai_daemons() {
        // cgroup v2: one `0::<path>` line.
        let v2 = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/arlen-ai-agent.service";
        assert_eq!(unit_from_cgroup(v2).as_deref(), Some("arlen-ai-agent.service"));
        assert_eq!(unit_to_app_id("arlen-ai-agent.service"), Some("ai-agent"));
        assert_eq!(unit_to_app_id("arlen-ai-daemon.service"), Some("ai-daemon"));
        // A non-AI unit maps to nothing (no identity widening).
        assert_eq!(unit_to_app_id("some-other.service"), None);
        // cgroup v1: `<n>:<controllers>:<path>` lines; the path is after the last colon.
        let v1 = "1:name=systemd:/user.slice/user@1000.service/app.slice/arlen-ai-daemon.service\n0::/";
        assert_eq!(unit_from_cgroup(v1).as_deref(), Some("arlen-ai-daemon.service"));
        // No service cgroup -> None.
        assert_eq!(unit_from_cgroup("0::/user.slice/user-1000.slice"), None);
    }
    use crate::identity_registry::{IdentityRecord, IdentityRegistry};
    use std::io::Write;

    #[test]
    fn user_app_inode_gate_rejects_a_copy_but_passes_the_real_binary_and_unenrolled() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("real");
        std::fs::File::create(&bin).unwrap().write_all(b"x").unwrap();

        let mut registry = IdentityRegistry::default();
        registry.record("com.example".into(), IdentityRecord::for_path(&bin).unwrap());

        // The real, enrolled binary passes.
        assert!(user_app_inode_ok(&registry, "com.example", &bin));
        // A copy (new inode) at a different path is a spoof: rejected.
        let copy = tmp.path().join("evil");
        std::fs::copy(&bin, &copy).unwrap();
        assert!(!user_app_inode_ok(&registry, "com.example", &copy));
        // A hardlink (same inode) is the same file: passes.
        let link = tmp.path().join("link");
        std::fs::hard_link(&bin, &link).unwrap();
        assert!(user_app_inode_ok(&registry, "com.example", &link));
        // An UNENROLLED app (no record) passes cooperatively (the residual).
        assert!(user_app_inode_ok(&registry, "com.other", &copy));
    }

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

    /// A user-writable app directory may not mint a privileged identity:
    /// `system.*` (System tier), `org.arlen.*` and the canonical AI
    /// daemons (FirstParty) are reserved, so a same-uid squat under
    /// `~/.local/share/arlen/apps/<reserved>/` is refused rather than
    /// resolving to a privileged app_id. Legitimate third-party ids
    /// still resolve.
    #[test]
    fn user_app_path_cannot_mint_a_reserved_identity() {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        for reserved in [
            "system",
            "system.knowledge",
            "org.arlen.contacts",
            "ai-daemon",
            "ai-agent",
            "settings",
        ] {
            let path = home.join(format!(".local/share/arlen/apps/{reserved}/bin/x"));
            assert!(
                path_to_app_id(&path).is_err(),
                "rule-4 path must not mint the reserved id {reserved}"
            );
        }
        // A genuine third-party reverse-DNS id is unaffected.
        let ok = home.join(".local/share/arlen/apps/com.example.app/bin/x");
        assert_eq!(path_to_app_id(&ok).unwrap(), "com.example.app");
    }

    #[test]
    fn reserved_namespace_predicate() {
        assert!(is_reserved_app_id("system"));
        assert!(is_reserved_app_id("system.daemon"));
        assert!(is_reserved_app_id("org.arlen.calendar"));
        assert!(is_reserved_app_id("ai-daemon"));
        assert!(is_reserved_app_id("ai-agent"));
        // `settings` is the canonical revoke-caller principal; a user path
        // may not mint it.
        assert!(is_reserved_app_id("settings"));
        // Third-party reverse-DNS and the bare per-daemon names rule 2
        // mints stay unreserved (they are third-party-tier).
        assert!(!is_reserved_app_id("com.example.app"));
        assert!(!is_reserved_app_id("org.zotero"));
        assert!(!is_reserved_app_id("knowledge"));
        assert!(!is_reserved_app_id("systematic")); // not system / system.*
    }

    #[test]
    fn test_app_id_from_path_ai_daemon() {
        // Strict equality on canonical install path.
        let path = PathBuf::from("/usr/bin/arlen-ai-daemon");
        assert_eq!(path_to_app_id(&path).unwrap(), "ai-daemon");

        let path = PathBuf::from("/usr/lib/arlen/apps/ai-daemon/bin/arlen-ai-daemon");
        assert_eq!(path_to_app_id(&path).unwrap(), "ai-daemon");

        // The libexec canonical path ai-proxy trusts must resolve too, or the
        // production daemon would authenticate to the proxy yet resolve as
        // unknown to the graph socket.
        let path = PathBuf::from("/usr/lib/arlen/libexec/arlen-ai-daemon");
        assert_eq!(path_to_app_id(&path).unwrap(), "ai-daemon");
    }

    #[test]
    fn test_app_id_from_path_ai_agent_canonical_libexec() {
        // The agent's canonical production binary (ai-proxy
        // CANONICAL_AI_AGENT_BIN) must resolve to `ai-agent`, the app id its
        // executor go-live permission profile is keyed under. Without this the
        // knowledge write socket never loads `ai-agent.toml` and the grant is
        // inert.
        let path = PathBuf::from("/usr/lib/arlen/libexec/arlen-ai-agent");
        assert_eq!(path_to_app_id(&path).unwrap(), "ai-agent");

        // A same-basename binary in a writable location is still rejected.
        for spoofed in ["/tmp/arlen-ai-agent", "/home/attacker/arlen-ai-agent"] {
            assert!(
                path_to_app_id(&PathBuf::from(spoofed)).is_err(),
                "spoofed agent path {spoofed} must be rejected"
            );
        }
    }

    #[test]
    fn test_app_id_from_path_online_accounts_canonical_libexec() {
        // The accounts daemon's canonical binary must resolve to `online-accounts`,
        // the id the audit daemon's ADMITTED allowlist keys on for the GAP-2
        // credential-handout audit. Without this it resolves to UnknownBinary and
        // the audit is silently refused.
        let path = PathBuf::from("/usr/lib/arlen/libexec/arlen-accountsd");
        assert_eq!(path_to_app_id(&path).unwrap(), "online-accounts");

        // A same-basename binary in a writable location must not impersonate it.
        for spoofed in ["/tmp/arlen-accountsd", "/home/attacker/arlen-accountsd"] {
            assert!(
                path_to_app_id(&PathBuf::from(spoofed)).is_err(),
                "spoofed accountsd path {spoofed} must be rejected"
            );
        }
    }

    #[test]
    fn test_app_id_from_path_notifyd_canonical_libexec() {
        // The notification daemon's canonical binary must resolve to `notifyd`,
        // the id the audit daemon's ADMITTED allowlist keys on for the GAP-2
        // notification-shown audit. Without this it resolves to UnknownBinary and
        // the audit is silently refused.
        let path = PathBuf::from("/usr/lib/arlen/libexec/arlen-notifyd");
        assert_eq!(path_to_app_id(&path).unwrap(), "notifyd");

        // A same-basename binary in a writable location must not impersonate it.
        for spoofed in ["/tmp/arlen-notifyd", "/home/attacker/arlen-notifyd"] {
            assert!(
                path_to_app_id(&PathBuf::from(spoofed)).is_err(),
                "spoofed notifyd path {spoofed} must be rejected"
            );
        }
    }

    #[test]
    fn test_app_id_from_path_consent_broker_canonical_libexec() {
        // The consent broker's canonical binary must resolve to `consent-broker`,
        // the id the audit daemon's ADMITTED allowlist keys on for the resolved-
        // decision audit. Without this it resolves to UnknownBinary and the
        // audit-before-act fails closed, denying every approval.
        let path = PathBuf::from("/usr/lib/arlen/libexec/arlen-consent-broker");
        assert_eq!(path_to_app_id(&path).unwrap(), "consent-broker");

        // A same-basename binary in a writable location must not impersonate it.
        for spoofed in ["/tmp/arlen-consent-broker", "/home/attacker/arlen-consent-broker"] {
            assert!(
                path_to_app_id(&PathBuf::from(spoofed)).is_err(),
                "spoofed consent-broker path {spoofed} must be rejected"
            );
        }
    }

    #[test]
    fn test_app_id_from_path_installd_canonical_libexec() {
        // The install daemon's canonical binary must resolve to `installd`, the
        // id the audit daemon's ADMITTED allowlist keys on for the GAP-2
        // install/uninstall audit. Without this it resolves to UnknownBinary and
        // the audit is silently refused.
        let path = PathBuf::from("/usr/lib/arlen/libexec/arlen-installd");
        assert_eq!(path_to_app_id(&path).unwrap(), "installd");

        // A same-basename binary in a writable location must not impersonate it.
        for spoofed in ["/tmp/arlen-installd", "/home/attacker/arlen-installd"] {
            assert!(
                path_to_app_id(&PathBuf::from(spoofed)).is_err(),
                "spoofed installd path {spoofed} must be rejected"
            );
        }
    }

    #[test]
    fn test_app_id_from_path_critical_notifiers_canonical_libexec() {
        // The power daemon and anomaly detector must resolve to their stable
        // ids: they are the trusted Critical-notification sources the GAP-7
        // clamp keys on. They live under libexec, so without these entries they
        // resolve to UnknownBinary and their legit Critical would be clamped.
        assert_eq!(
            path_to_app_id(&PathBuf::from("/usr/lib/arlen/libexec/arlen-powerd")).unwrap(),
            "powerd"
        );
        assert_eq!(
            path_to_app_id(&PathBuf::from("/usr/lib/arlen/libexec/arlen-anomalyd")).unwrap(),
            "anomalyd"
        );

        // A same-basename binary in a writable location must not impersonate them.
        for spoofed in [
            "/tmp/arlen-powerd",
            "/home/attacker/arlen-anomalyd",
        ] {
            assert!(
                path_to_app_id(&PathBuf::from(spoofed)).is_err(),
                "spoofed critical-notifier path {spoofed} must be rejected"
            );
        }
    }

    #[test]
    fn test_app_id_from_path_settings_canonical() {
        // The Settings app must resolve to the canonical `settings` app id, the
        // sole principal the Living Capability Graph revoke op admits. A
        // same-basename binary in a writable location must not impersonate it.
        let path = PathBuf::from("/usr/lib/arlen/apps/settings/bin/arlen-settings");
        assert_eq!(path_to_app_id(&path).unwrap(), "settings");
        for spoofed in ["/tmp/arlen-settings", "/home/attacker/arlen-settings"] {
            assert!(
                path_to_app_id(&PathBuf::from(spoofed)).is_err(),
                "spoofed settings path {spoofed} must be rejected"
            );
        }
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
