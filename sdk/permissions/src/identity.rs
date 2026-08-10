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
    #[error("cannot read exe path for pid {pid}: {source}{why}")]
    CannotReadExe {
        /// The process whose identity was being resolved.
        pid: u32,
        /// What the kernel said.
        source: std::io::Error,
        /// What is readable about that process, when the exe link is not. See
        /// [`why_exe_unreadable`].
        why: String,
    },
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
/// What is knowable about a process whose `exe` link would not open, appended to
/// the error so the next occurrence explains itself.
///
/// `cannot read exe path: Permission denied` names a symptom that three unrelated
/// causes produce, and telling them apart has cost real time twice: once for the
/// daemons under the Landlock fence (see `sdk/landlock-fence`, where the ptrace
/// LSM hook denies the read and no filesystem grant can help), and again on 10
/// August, when the undo signer was found turning away every caller on the image
/// with nothing in the message to say which cause it was.
///
/// The three separate cleanly on facts that stay readable when `exe` does not:
/// a peer of a DIFFERENT uid, a target that is non-dumpable (its `/proc` entries
/// become root-owned), or neither - which points at the reader being confined.
/// So report the peer's uid, the owner of its `/proc` directory, and our own uid,
/// and let whoever reads the log stop guessing.
///
/// Best-effort by construction: this runs on an error path and must never mask
/// the original failure, so anything unreadable is simply omitted.
/// A one-line `/proc` value, or `"unreadable"`. Never propagates a failure: this
/// only ever decorates an error that has already happened.
/// How `/proc` is mounted for US, from `/proc/self/mountinfo`.
///
/// Reached for only after the whole `ptrace_may_access` condition was measured
/// SATISFIED on the image - matching uid and gid triples, no capability
/// difference, Yama off, both sides unconfined - and the read was refused anyway.
/// When the credential check provably passes, what is left is the filesystem the
/// reader is looking at, and `hidepid=` is the option that turns another
/// process's `/proc` entry unreadable without touching credentials at all.
fn proc_mount_options() -> String {
    std::fs::read_to_string("/proc/self/mountinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.split(' ').nth(4) == Some("/proc"))
                .map(|l| l.to_owned())
        })
        .unwrap_or_else(|| "unreadable".into())
}

/// One named field out of `/proc/<pid>/status`, or `"unreadable"`.
fn proc_field(pid: u32, field: &str) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix(&format!("{field}:")))
                .map(|v| v.trim().to_owned())
        })
        .unwrap_or_else(|| "unreadable".into())
}

fn read_trimmed(path: &str) -> String {
    std::fs::read_to_string(path)
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|_| "unreadable".into())
}

fn why_exe_unreadable(pid: u32) -> String {
    use std::os::unix::fs::MetadataExt;

    if let Ok(meta) = std::fs::metadata(format!("/proc/{pid}")) {
        // SAFETY: getuid is always successful and takes no arguments.
        let me = unsafe { libc::getuid() };
        let dir_owner = meta.uid();
        // The WHOLE `Uid:` line: real, effective, saved, fs. `ptrace_may_access`
        // requires the reader's uid to match every one of them, so printing only
        // the first can show a match while the kernel sees a mismatch - a probe
        // that reports agreement it has not checked. Found by re-reading the
        // kernel's condition after three mechanisms had been eliminated and the
        // peer still looked identical to us on everything I was printing.
        let uids = proc_field(pid, "Uid");
        // The kernel checks the GID triple next to the UID triple in the same
        // condition, so a group mismatch denies just as a uid mismatch does.
        // Printing one and not the other would leave half the condition unseen.
        let gids = proc_field(pid, "Gid");
        let peer = uids
            .split_whitespace()
            .next()
            .unwrap_or("unknown")
            .to_owned();
        let reading = if dir_owner != me {
            if peer == me.to_string() {
                " - the peer shares our uid but its /proc is root-owned, so it is \
                  non-dumpable"
            } else {
                " - the peer runs as another user"
            }
        } else {
            // Reader-side, but do NOT name a mechanism we have not established.
            // The first cut asserted a Landlock fence here and the very next boot
            // disproved it: the undo signer hit this branch while taking no fence
            // at all. Both plausible causes are readable, so read them instead of
            // picking one - a Landlock domain (`sdk/landlock-fence`, the ptrace
            // LSM hook) and Yama's ptrace scope, which above 0 stops a same-uid
            // process reading a non-descendant's `exe`.
            // The deciding field, found by measurement rather than by reasoning
            // from the symptom: `__ptrace_may_access` refuses a READ unless the
            // reader's permitted capabilities are a SUPERSET of the target's. A
            // peer holding one capability we lack is unreadable no matter that we
            // share a uid - measured on 10 August against `systemd --user`
            // (CapPrm 0x800000000, CAP_WAKE_ALARM), unreadable, while two
            // zero-capability processes of the same uid read fine.
            //
            // Report every candidate rather than the current favourite. Four
            // mechanisms have been proposed for this one refusal and three are
            // now dead - a Landlock fence (the signer takes none), Yama (the
            // image reports scope=0, and its hook governs ATTACH anyway), and a
            // capability difference (both sets read zero on the image). Each was
            // plausible, each fitted the evidence available when it was proposed.
            //
            // The pattern is the lesson: when the capability line was added it
            // REPLACED the LSM-label probe, and the label is exactly what the
            // next boot needed. Narrowing a diagnostic to the leading hypothesis
            // discards the evidence that would refute it. So this prints all of
            // them, cheaply, and lets the reader do the eliminating.
            return format!(
                " (peer Uid[real effective saved fs]=[{uids}], /proc/{pid} owned by \
                 {dir_owner}, we are {me}; peer Gid=[{gids}], ours={} - a READ \
                 needs OUR uid and gid to match EVERY one of those; \
                 capabilities: peer CapPrm={}, ours={} \
                 (a READ is refused unless ours is a superset); \
                 yama ptrace_scope={}; our LSM label={}, peer's={}; \
                 our /proc mount={})",
                proc_field(std::process::id(), "Gid"),
                proc_field(pid, "CapPrm"),
                proc_field(std::process::id(), "CapPrm"),
                read_trimmed("/proc/sys/kernel/yama/ptrace_scope"),
                read_trimmed("/proc/self/attr/current"),
                read_trimmed(&format!("/proc/{pid}/attr/current")),
                proc_mount_options(),
            );
        };
        return format!(" (peer uid {peer}, /proc/{pid} owned by {dir_owner}, we are {me}{reading})");
    }
    String::new()
}

pub(crate) fn exe_path_openat(pid: u32) -> Result<PathBuf, IdentityError> {
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
            IdentityError::CannotReadExe {
                pid,
                why: format!(" [failed at open(/proc/{pid})]{}", why_exe_unreadable(pid)),
                source: err,
            }
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
        let err = std::io::Error::last_os_error();
        return Err(IdentityError::CannotReadExe {
            pid,
            why: format!(" [failed at readlinkat(exe)]{}", why_exe_unreadable(pid)),
            source: err,
        });
    }
    let bytes = &buf[..n as usize];
    let s = std::str::from_utf8(bytes)
        .map_err(|_| IdentityError::CannotReadExe { pid, why: String::new(), source: std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "exe path not UTF-8",
        )})?;
    Ok(PathBuf::from(s))
}

/// The `(ino, dev)` of a process's exe binary, read via `fstatat` on the process's
/// own `/proc/{pid}` directory fd (following the `exe` magic symlink to the real
/// binary), NOT a re-stat of the exe PATH string. Because it never re-resolves the
/// user-controllable exe path, a same-uid attacker cannot swap a path component
/// between a readlink and this stat to forge an inode match (the TOCTOU an
/// inode-registry attestation must not admit). `None` on any failure — the caller
/// treats absence as not-inode-attested (fail-safe). MUST be called while the
/// peer's pidfd is held, so the pid names the pinned process and cannot be recycled.
pub(crate) fn exe_ino_dev(pid: u32) -> Option<(u64, u64)> {
    use std::ffi::CString;
    let dir_cstr = CString::new(format!("/proc/{pid}")).ok()?;
    // SAFETY: dir_cstr is a valid C string; we own the returned fd.
    let dir_fd = unsafe {
        libc::open(
            dir_cstr.as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    if dir_fd < 0 {
        return None;
    }
    let dir = unsafe { OwnedFd::from_raw_fd(dir_fd) };
    let exe_cstr = CString::new("exe").ok()?;
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // No AT_SYMLINK_NOFOLLOW: follow the `exe` magic link to the real binary and
    // stat IT (its ino/dev), relative to the pinned /proc/{pid} fd.
    // SAFETY: dir.as_raw_fd() is valid for the call; exe_cstr + st live for it.
    let r = unsafe {
        libc::fstatat(
            dir.as_raw_fd(),
            exe_cstr.as_ptr(),
            &mut st as *mut libc::stat,
            0,
        )
    };
    if r != 0 {
        return None;
    }
    Some((st.st_ino as u64, st.st_dev as u64))
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
        // The pi-based engine daemon is the drop-in replacement for the retired
        // ai-agent (pi-agent-adoption.md step 9): it fills the same autonomous-
        // curator ROLE, so it resolves to the same principal `ai-agent`, reusing
        // the go-live permission profile and the audit ADMITTED entry that role is
        // keyed under. This matches the planner's reuse-the-existing-name ruling
        // (it reverted a redundant `AIEngine1` D-Bus name so the engine owns the
        // existing `AI1`/`AIAgent1`); an app id is a role, not a binary name. In a
        // dev build the debug rule resolves the cargo-run binary to
        // `dev.arlen-ai-engine-daemon` before this canonical path is reached.
        "/usr/lib/arlen/libexec/arlen-ai-agent"
        | "/usr/lib/arlen/libexec/arlen-ai-engine-daemon" => {
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
        // The confined-launch launcher, pinned canonically so it resolves to
        // `arlen-run` - the sole id the identity broker admits as a REGISTRAR
        // (`stamped-identity-plan.md`). arlen-run holds the authenticated
        // --app-id it resolved from the root IdentityRegistry before the child
        // ran, and registers the child's pidfd against it; every other same-uid
        // caller may look up but never register. Without this canonical entry the
        // launcher would resolve to UnknownBinary and the broker would refuse its
        // registrations, leaving every confined app unstamped. In a dev build the
        // debug rule resolves the cargo-run binary to `dev.arlen-run` first, which
        // the registrar allowlist also admits.
        "/usr/lib/arlen/libexec/arlen-run" => {
            return Ok("arlen-run".to_string());
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
        // The clock daemon, which installs beside them and asks `Power1` to wake
        // the machine for an alarm. Without an entry it resolves to
        // UnknownBinary, gets no profile, and every wake request is refused - so
        // alarms would quietly never wake anyone.
        "/usr/lib/arlen/libexec/arlen-clockd" => {
            return Ok("clockd".to_string());
        }
        // The session undo service. It reads the signed undo log through the undo
        // signer and the audit ledger through the audit daemon, and BOTH admit by
        // resolved app id - so without this entry it resolves to UnknownBinary and
        // every read it makes is refused, which is a recent-actions panel that is
        // permanently empty on a shipped image while working perfectly in a
        // developer build (where the `dev.*` fallback resolves it). Caught by the
        // shipped-unit identity test, not by anything that ran the daemon.
        "/usr/lib/arlen/libexec/arlen-undod" => {
            return Ok("undod".to_string());
        }
        // The cross-profile transfer daemon (profile-system, PR-R4). It audits
        // every transfer to BOTH profiles' ledgers fail-closed BEFORE any byte
        // crosses a boundary, so without this entry it resolves to UnknownBinary,
        // the audit ingest refuses it, and every transfer is denied. Named for
        // its binary like its `notifyd` / `powerd` / `anomalyd` siblings.
        "/usr/lib/arlen/libexec/arlen-transferd" => {
            return Ok("transferd".to_string());
        }
        // The module runtime. Its shipped unit (`daemons/modulesd/dist`) execs
        // from libexec, which rule (2) does not reach, while the only resolver
        // test for it asserts the `/usr/bin` path - so the deployed daemon would
        // resolve to UnknownBinary and the consent broker could never recognise
        // it as a trusted intermediary, leaving every module's capability grant
        // unattributable. Both paths resolve here so either deployment works.
        "/usr/lib/arlen/libexec/arlen-modulesd" => {
            return Ok("modulesd".to_string());
        }
        // The remaining first-party daemons, each named for its binary. Derived
        // from the shipped units by the `every_shipped_unit_binary_has_an_identity`
        // guard in `dev/integration`: a daemon with no identity at its deployed
        // path is refused by every peer-authenticated socket, so the cheap thing
        // is for all of them to have one rather than to track which do not need it
        // yet. `code-indexer` is the live case - it writes the code graph through
        // the knowledge daemon's tier-gated write path.
        "/usr/lib/arlen/libexec/arlen-code-indexer" => {
            return Ok("code-indexer".to_string());
        }
        "/usr/lib/arlen/libexec/arlen-journald-parser" => {
            return Ok("journald-parser".to_string());
        }
        "/usr/lib/arlen/libexec/arlen-auditd" => {
            return Ok("auditd".to_string());
        }
        "/usr/lib/arlen/libexec/arlen-settings-broker" => {
            return Ok("settings-broker".to_string());
        }
        "/usr/lib/arlen/libexec/arlen-wallpaperd" => {
            return Ok("wallpaperd".to_string());
        }
        "/usr/lib/arlen/libexec/arlen-install-helper" => {
            return Ok("install-helper".to_string());
        }
        // NB this one is NOT under libexec, unlike every sibling - its unit execs
        // `/usr/lib/arlen/permission-helper` directly. Matched as shipped.
        "/usr/lib/arlen/permission-helper" => {
            return Ok("permission-helper".to_string());
        }
        // The foreign-app bridge ingestion daemon (foreign-app-bridges.md §4). It
        // installs under /usr/lib/arlen/libexec/ and writes the KG under its
        // delegated namespace (the Obsidian floor's `md.obsidian.*` entity
        // upserts), so it must resolve to the stable id `bridge-ingest` its
        // delegated write profile is keyed under + the `first_party_apps` tiering
        // grants. Rule (2) covers only /usr/bin/arlen-*, so without this entry the
        // deployed daemon resolves to UnknownBinary and its writes are refused at
        // the write-tier gate (a debug build resolves the cargo-run binary to
        // `dev.arlen-bridge-ingest` first). The scoped macaroon + one-time install
        // consent are the layered deployment grants above this attested identity.
        "/usr/lib/arlen/libexec/arlen-bridge-ingest" => {
            return Ok("bridge-ingest".to_string());
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
        // The xdg-desktop-portal impl backend. It submits the coarse print audit
        // and the no-silent-capture screenshot audit (SC-R6) to the ledger, so it
        // must resolve to the stable id `xdg-desktop-portal` the audit daemon's
        // ADMITTED allowlist keys on. Same GAP-2 rationale as the other libexec
        // daemons: rule (2) covers only /usr/bin/arlen-*, so without this entry the
        // portal resolves to UnknownBinary and every print/capture audit is silently
        // refused; the root-owned system path is attested.
        "/usr/lib/arlen/libexec/xdg-desktop-portal-arlen" => {
            return Ok("xdg-desktop-portal".to_string());
        }
        // The Context-Capsule daemon. Its capsule serve audits the read BEFORE
        // returning the slice and FAILS CLOSED on a down/refused ledger, so without
        // this entry it resolves to UnknownBinary, the audit is refused, and EVERY
        // capsule read returns "audit unavailable". Resolves to the stable id
        // `capsuled` the audit daemon's ADMITTED allowlist keys on; root-owned
        // canonical path attested.
        "/usr/lib/arlen/libexec/arlen-capsuled" => {
            return Ok("capsuled".to_string());
        }
        // The config-broker (the separate-uid owner of the AI master switches).
        // It records every privileged switch change to the OWNER's audit ledger
        // (an escalation is refused if it cannot be recorded), so it must resolve
        // to the stable id `config-broker` the audit daemon's ADMITTED allowlist
        // keys on; without this entry rule (2) misses the libexec path, it resolves
        // to UnknownBinary, and every master-switch-change audit is silently
        // refused. It runs as root (a strictly more-privileged uid, so the user's
        // own uid cannot write the switch store); the root-owned canonical path is
        // attested. The auditd accepts this root producer via
        // `ConnectionAuth::extract_from_trusting_root`.
        "/usr/lib/arlen/libexec/arlen-config-broker" => {
            return Ok("config-broker".to_string());
        }
        // The knowledge (graph) daemon. Its binary ships as `arlen-graph-daemon`
        // (the systemd unit + crate `[[bin]]` name), but its stable audit id - the
        // one the audit daemon's ADMITTED allowlist keys on, and the id its own
        // `audit.rs` names as the submitter - is `knowledge`. Rule (2) would strip
        // the /usr/bin path to `graph-daemon`, which is NOT admitted, so the
        // app-tier entity-write (foreign-app-bridges) audit - which is FAIL-CLOSED
        // before the write persists - would be refused and every bridge write would
        // fail in a release image (masked in dev, where the `dev.arlen-graph-daemon`
        // cargo id is admitted). This explicit override pins the deployed path to
        // `knowledge`; the /usr/bin path is root-owned and attested.
        "/usr/bin/arlen-graph-daemon" => {
            return Ok("knowledge".to_string());
        }
        // The Connections credential-governance daemon. A credential release audits
        // BEFORE handing the credential over and FAILS CLOSED on a down/refused
        // ledger, so without this entry it resolves to UnknownBinary, the audit is
        // refused, and EVERY credential handout is refused with AuditUnavailable.
        // Resolves to the stable id `connections` the audit daemon's ADMITTED
        // allowlist keys on; root-owned canonical path attested.
        "/usr/lib/arlen/libexec/arlen-connectionsd" => {
            return Ok("connections".to_string());
        }
        // The Settings app, pinned canonically so it resolves to the stable
        // app_id `settings` (not the spoofable basename). The Living Capability
        // Graph revoke socket op admits only this app id (living-capability-graph.md
        // §6.2, Option A): revoke is user-initiated through Settings, narrowing-only,
        // so a root-owned canonical path is the trust anchor until F3 upgrades it.
        // Rule (3) would also resolve this apps path, but the explicit entry keeps
        // the canonical principal unambiguous (as the ai-daemon apps entries do).
        "/usr/lib/arlen/apps/dev.arlen.settings/bin/arlen-settings" => {
            return Ok("dev.arlen.settings".to_string());
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
/// The user-facing surfaces allowed to ask a daemon to read or act on the user's
/// behalf: the harness and Settings.
///
/// **Here, not in each daemon, because this is the half that drifts.** Two
/// daemons now answer for the same actions - the AI engine explains them, the
/// undo service lists and reverses them - and a surface admitted by one and not
/// the other is a button that works on one page and fails on the next, with
/// nothing to say why. The resolve of a caller's id is mechanism and can be
/// spelled per daemon; WHICH ids count is policy and has to be one list.
///
/// **The rule, so the next addition is measured against something rather than
/// against these three names: this list gates surfaces that ARE the system
/// talking to the user - not ordinary apps.** The desktop shell is that by
/// definition; it owns the top bar, the notifications and the launcher, and
/// anything it could do with undo it can already effect by other means, so adding
/// it widens nothing. Settings and the harness qualify for the same reason, one
/// step further from the centre.
///
/// **An ordinary app never joins.** An app that wants something undone asks the
/// user through the shell; it does not hold the capability itself. If a candidate
/// is a place the user goes to work rather than a place the system speaks from,
/// the answer is no.
///
/// The dev ids are the same surfaces as they resolve from a cargo target dir,
/// exact and never a `dev.` prefix match, compiled out of a release build the way
/// the audit ingest gate does it.
const USER_SURFACES: &[&str] = &["dev.arlen.desktop-shell", "dev.arlen.harness", "dev.arlen.settings"];

/// The same surfaces as a cargo target dir resolves them.
#[cfg(debug_assertions)]
const USER_SURFACES_DEV: &[&str] = &[
    "dev.arlen-desktop-shell",
    "dev.arlen-settings",
    "dev.arlen-harness",
];

pub fn is_user_surface(app_id: &str) -> bool {
    if USER_SURFACES.contains(&app_id) {
        return true;
    }
    #[cfg(debug_assertions)]
    if USER_SURFACES_DEV.contains(&app_id) {
        return true;
    }
    false
}

#[cfg(test)]
mod user_surface_rule {
    use super::is_user_surface;

    /// The three system surfaces, and the rule that keeps the list from growing
    /// by sympathy: a place the system speaks from is in, a place the user goes to
    /// work is out. The file manager is the sharpest test of that line - it is
    /// first-party, it is trusted, it moves the user's files, and it is still an
    /// app, so it asks through the shell rather than holding the capability.
    #[test]
    fn the_system_surfaces_are_in_and_an_ordinary_app_is_not() {
        for id in ["dev.arlen.desktop-shell", "dev.arlen.harness", "dev.arlen.settings"] {
            assert!(is_user_surface(id), "{id} is the system talking to the user");
        }
        for id in [
            "dev.arlen.files",
            "dev.arlen.terminal",
            "dev.arlen.store",
            "dev.arlen.viewers",
            "com.example.app",
            "ai-agent",
        ] {
            assert!(
                !is_user_surface(id),
                "{id} is an app: it asks the user through the shell, it does not \
                 hold the capability"
            );
        }
    }
}

/// configured allowlist or the rule-4 squat reopens for the added ids.
pub fn is_reserved_app_id(app_id: &str) -> bool {
    app_id == "system"
        || app_id.starts_with("system.")
        || app_id.starts_with("org.arlen.")
        // Every first-party app now lives in this namespace, so the whole prefix
        // is unclaimable from a user-writable directory. Without this line the
        // rename would MOVE the squat rather than close it: a user could create
        // `~/.local/share/arlen/apps/dev.arlen.settings/` and mint the identity
        // that the revoke socket, the consent broker's grant management and the
        // config broker's writer allowlist all key on. The debug ids are
        // `dev.arlen-settings` with a hyphen, so they are a different namespace
        // and unaffected.
        || app_id.starts_with("dev.arlen.")
        || matches!(
            app_id,
            // `modulesd` and `xdg-desktop-portal` are the consent broker's
            // trusted intermediaries: each may attribute a grant to a principal
            // it resolved itself. A same-uid app able to MINT either id would
            // inherit that power and redirect grants to any app it named, so
            // both must be unclaimable from a user-app directory.
            "ai-daemon" | "ai-agent" | "settings" | "xdg-desktop-portal" | "modulesd"
        )
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



#[cfg(test)]
mod tests {
    use super::*;

    /// A release build admits exactly `USER_SURFACES`, so the property worth
    /// pinning is that the list carries no development id: one there would admit
    /// any cargo binary of that name on a real system. Asserted on the list
    /// rather than by calling under `cfg(not(debug))`, which never runs here.
    #[test]
    fn the_release_surface_list_carries_no_development_id() {
        // A debug id is `dev.<binary>` and every binary here is `arlen-…`, so the
        // shape to keep out is `dev.arlen-` with the hyphen. `dev.` alone is no
        // longer the test it once was: the app ids ARE `dev.arlen.…` now, and the
        // two namespaces are told apart by that one character.
        assert!(USER_SURFACES.iter().all(|id| !id.starts_with("dev.arlen-")));
        #[cfg(debug_assertions)]
        assert!(
            USER_SURFACES.iter().all(|id| !USER_SURFACES_DEV.contains(id)),
            "the release list and the debug list must not overlap"
        );
        assert!(is_user_surface("dev.arlen.settings") && is_user_surface("dev.arlen.harness"));
        assert!(!is_user_surface("ai-agent") && !is_user_surface("dev."));
        // The bare form the tree used to carry is not a surface any more, which is
        // the half of this rename that has teeth: an id left behind somewhere
        // stops being trusted rather than quietly keeping its old power.
        assert!(!is_user_surface("settings") && !is_user_surface("harness"));
    }

    /// Every canonical binary resolves to the id the rest of the system keys on.
    ///
    /// These are pinned one arm per binary because the generic rules do not reach
    /// them: rule (2) covers only `/usr/bin/arlen-*`, so a `libexec` daemon
    /// without its own arm falls through to `UnknownBinary`. The consequences are
    /// named in each arm's comment and they are not small - `ai-proxy` losing its
    /// id means every forward's fail-closed audit is refused, and `settings`
    /// losing its id means the revoke op refuses the one caller it admits.
    ///
    /// Mutation testing found each of these arms could be DELETED with no test
    /// failing. Asserted as a table so a new canonical binary is added here too,
    /// rather than shipping an arm nothing checks.
    #[test]
    fn every_canonical_binary_resolves_to_its_own_id() {
        let canonical: &[(&str, &str)] = &[
            ("/usr/bin/arlen-ai-daemon", "ai-daemon"),
            ("/usr/lib/arlen/libexec/arlen-ai-agent", "ai-agent"),
            ("/usr/lib/arlen/libexec/arlen-ai-proxy", "ai-proxy"),
            ("/usr/lib/arlen/libexec/arlen-run", "arlen-run"),
            ("/usr/lib/arlen/libexec/arlen-accountsd", "online-accounts"),
            ("/usr/lib/arlen/libexec/arlen-notifyd", "notifyd"),
            ("/usr/lib/arlen/libexec/arlen-transferd", "transferd"),
            ("/usr/lib/arlen/libexec/arlen-installd", "installd"),
            ("/usr/lib/arlen/libexec/arlen-powerd", "powerd"),
            ("/usr/lib/arlen/libexec/arlen-anomalyd", "anomalyd"),
            ("/usr/lib/arlen/libexec/arlen-bridge-ingest", "bridge-ingest"),
            ("/usr/lib/arlen/libexec/arlen-consent-broker", "consent-broker"),
            ("/usr/lib/arlen/libexec/xdg-desktop-portal-arlen", "xdg-desktop-portal"),
            ("/usr/lib/arlen/libexec/arlen-capsuled", "capsuled"),
            ("/usr/lib/arlen/libexec/arlen-config-broker", "config-broker"),
            ("/usr/bin/arlen-graph-daemon", "knowledge"),
            ("/usr/lib/arlen/libexec/arlen-connectionsd", "connections"),
            ("/usr/lib/arlen/apps/dev.arlen.settings/bin/arlen-settings", "dev.arlen.settings"),
        ];
        for (path, want) in canonical {
            assert_eq!(
                path_to_app_id(Path::new(path)).ok().as_deref(),
                Some(*want),
                "{path} must resolve to {want}"
            );
        }
    }

    /// A path that only LOOKS canonical resolves to no privileged id. The arms
    /// match exactly, so a copy placed beside one of them - the same basename in
    /// a directory anyone can write - must not inherit its identity.
    #[test]
    fn a_lookalike_path_does_not_inherit_a_canonical_id() {
        for imposter in [
            "/tmp/arlen-ai-proxy",
            "/usr/lib/arlen/libexec/../libexec/arlen-ai-proxy",
            "/home/u/.local/bin/arlen-settings",
            "/usr/lib/arlen/libexec/arlen-ai-proxy-evil",
        ] {
            let got = path_to_app_id(Path::new(imposter));
            assert_ne!(got.as_deref().ok(), Some("ai-proxy"), "{imposter}");
            assert_ne!(got.as_deref().ok(), Some("settings"), "{imposter}");
        }
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

        // The pi-based engine daemon is the drop-in replacement for the retired
        // ai-agent and fills the same curator role, so its canonical binary
        // resolves to the same `ai-agent` principal (reusing the go-live profile
        // and the audit ADMITTED entry).
        let engine = PathBuf::from("/usr/lib/arlen/libexec/arlen-ai-engine-daemon");
        assert_eq!(path_to_app_id(&engine).unwrap(), "ai-agent");

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
    fn test_app_id_from_path_arlen_run_canonical_libexec() {
        // The confined-launch launcher's canonical binary must resolve to
        // `arlen-run`, the sole registrar the identity broker admits. Without
        // this it resolves to UnknownBinary and the broker refuses its
        // registrations, leaving confined apps unstamped.
        let path = PathBuf::from("/usr/lib/arlen/libexec/arlen-run");
        assert_eq!(path_to_app_id(&path).unwrap(), "arlen-run");

        // A same-basename binary in a writable location must not impersonate the
        // launcher (which would let a same-uid process become a registrar).
        for spoofed in ["/tmp/arlen-run", "/home/attacker/arlen-run"] {
            assert!(
                path_to_app_id(&PathBuf::from(spoofed)).is_err(),
                "spoofed arlen-run path {spoofed} must be rejected"
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
    fn test_app_id_from_path_graph_daemon_resolves_to_knowledge() {
        // The knowledge daemon ships as `arlen-graph-daemon` but its stable audit id
        // is `knowledge` (what the audit ADMITTED allowlist + its own audit.rs use).
        // Rule (2) would strip /usr/bin/arlen-graph-daemon to `graph-daemon`, which
        // is NOT admitted, so the fail-closed app-tier-write audit would be refused
        // in a release image; the explicit override must pin it to `knowledge`.
        let path = PathBuf::from("/usr/bin/arlen-graph-daemon");
        assert_eq!(path_to_app_id(&path).unwrap(), "knowledge");
        // A same-basename binary in a writable location must not impersonate it.
        for spoofed in ["/tmp/arlen-graph-daemon", "/home/attacker/arlen-graph-daemon"] {
            assert!(
                path_to_app_id(&PathBuf::from(spoofed)).is_err(),
                "spoofed graph-daemon path {spoofed} must be rejected"
            );
        }
    }

    #[test]
    fn test_app_id_from_path_config_broker_canonical_libexec() {
        // The config-broker's canonical binary must resolve to `config-broker`,
        // the id the audit daemon's ADMITTED allowlist keys on for the AI
        // master-switch-change audit. Without this it resolves to UnknownBinary and
        // every switch change fails closed (an escalation can never be recorded).
        let path = PathBuf::from("/usr/lib/arlen/libexec/arlen-config-broker");
        assert_eq!(path_to_app_id(&path).unwrap(), "config-broker");

        // A same-basename binary in a writable location must not impersonate it.
        for spoofed in ["/tmp/arlen-config-broker", "/home/attacker/arlen-config-broker"] {
            assert!(
                path_to_app_id(&PathBuf::from(spoofed)).is_err(),
                "spoofed config-broker path {spoofed} must be rejected"
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
    fn test_app_id_from_path_bridge_ingest_canonical_libexec() {
        // The foreign-app bridge daemon's canonical binary must resolve to
        // `bridge-ingest`, the id its delegated md.obsidian.* write profile is keyed
        // under + the first_party_apps tiering grants. Without this a DEPLOYED bridge
        // resolves to UnknownBinary and its KG writes are refused at the write gate.
        let path = PathBuf::from("/usr/lib/arlen/libexec/arlen-bridge-ingest");
        assert_eq!(path_to_app_id(&path).unwrap(), "bridge-ingest");

        // A same-basename binary in a writable location must not impersonate it.
        for spoofed in ["/tmp/arlen-bridge-ingest", "/home/attacker/arlen-bridge-ingest"] {
            assert!(
                path_to_app_id(&PathBuf::from(spoofed)).is_err(),
                "spoofed bridge-ingest path {spoofed} must be rejected"
            );
        }
    }

    #[test]
    fn test_app_id_from_path_xdg_portal_canonical_libexec() {
        // The xdg-desktop-portal impl backend's canonical binary must resolve to
        // `xdg-desktop-portal`, the id the audit daemon's ADMITTED allowlist keys on
        // for the print + no-silent-capture screenshot audits. Without this it
        // resolves to UnknownBinary and every capture/print audit is silently
        // refused.
        let path = PathBuf::from("/usr/lib/arlen/libexec/xdg-desktop-portal-arlen");
        assert_eq!(path_to_app_id(&path).unwrap(), "xdg-desktop-portal");

        // A same-basename binary in a writable location must not impersonate it.
        for spoofed in [
            "/tmp/xdg-desktop-portal-arlen",
            "/home/attacker/xdg-desktop-portal-arlen",
        ] {
            assert!(
                path_to_app_id(&PathBuf::from(spoofed)).is_err(),
                "spoofed portal path {spoofed} must be rejected"
            );
        }
    }

    #[test]
    fn test_app_id_from_path_capsuled_and_connections_canonical_libexec() {
        // Both daemons audit fail-closed BEFORE their action (capsule serve /
        // credential release), so a wrong id means the audit is refused and the
        // action is refused. They must resolve to the stable ids the audit daemon's
        // ADMITTED allowlist keys on.
        assert_eq!(
            path_to_app_id(&PathBuf::from("/usr/lib/arlen/libexec/arlen-capsuled")).unwrap(),
            "capsuled"
        );
        assert_eq!(
            path_to_app_id(&PathBuf::from("/usr/lib/arlen/libexec/arlen-connectionsd")).unwrap(),
            "connections"
        );
        // Same-basename binaries in writable locations must not impersonate them.
        for spoofed in [
            "/tmp/arlen-capsuled",
            "/home/attacker/arlen-connectionsd",
        ] {
            assert!(
                path_to_app_id(&PathBuf::from(spoofed)).is_err(),
                "spoofed path {spoofed} must be rejected"
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
        // The Settings app must resolve to the canonical `dev.arlen.settings` app
        // id, the sole principal the Living Capability Graph revoke op admits. A
        // same-basename binary in a writable location must not impersonate it.
        let path = PathBuf::from("/usr/lib/arlen/apps/dev.arlen.settings/bin/arlen-settings");
        assert_eq!(path_to_app_id(&path).unwrap(), "dev.arlen.settings");
        // And the id cannot be minted from a user-writable app directory, which is
        // the squat the reserved namespace closes. Rule (3) reads the directory
        // name, so without that guard the rename would simply have moved it.
        assert!(is_reserved_app_id("dev.arlen.settings"));
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
            // The shipped unit execs from libexec; both must resolve alike.
            ("/usr/lib/arlen/libexec/arlen-modulesd", "modulesd"),
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

    /// A child process running a binary at a known path, reaped on drop.
    ///
    /// Everything below this point tests the syscall layer against a real
    /// process rather than against a fake `/proc`. That is deliberate: a
    /// redirectable proc root would be a new production-reachable override on
    /// the file the whole peer-authentication story rests on, and the thing
    /// worth proving is that the real `openat`/`readlinkat`/`fstatat` sequence
    /// returns the truth about a real pid.
    struct Child {
        inner: Option<std::process::Child>,
        exe: PathBuf,
    }

    impl Child {
        /// Spawn a long-lived child from a copy of `/bin/sh` at `name`, so the
        /// test controls both the pid and the exact path `/proc/{pid}/exe`
        /// must resolve to.
        fn spawn_named(dir: &Path, name: &str) -> Option<Self> {
            let src = ["/bin/sleep", "/usr/bin/sleep"]
                .into_iter()
                .map(Path::new)
                .find(|p| p.exists())?;
            let exe = dir.join(name);
            std::fs::copy(src, &exe).ok()?;
            let inner = std::process::Command::new(&exe)
                .arg("60")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok()?;
            let me = Self {
                inner: Some(inner),
                exe,
            };
            me.wait_for_exec();
            Some(me)
        }

        /// Block until the child has actually `execve`d its own binary.
        ///
        /// Between `fork` and `exec` the child still carries the *parent's*
        /// image, so `/proc/{pid}/exe` reports the test binary. Reading it
        /// straight after `spawn` therefore races, and the race is invisible
        /// most of the time: the first run of these tests passed on luck.
        ///
        /// Polled with `std::fs::read_link` rather than the resolver under
        /// test, so readiness never depends on the thing being asserted.
        fn wait_for_exec(&self) {
            let link = format!("/proc/{}/exe", self.pid());
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while std::time::Instant::now() < deadline {
                if std::fs::read_link(&link).is_ok_and(|p| p == self.exe) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            panic!("child never exec'd {}", self.exe.display());
        }

        fn pid(&self) -> u32 {
            self.inner.as_ref().expect("child is live").id()
        }

        /// Kill and reap, so the pid is genuinely gone rather than a zombie
        /// (a zombie keeps its `/proc/{pid}` directory).
        fn reap(&mut self) {
            if let Some(mut c) = self.inner.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }

    impl Drop for Child {
        fn drop(&mut self) {
            self.reap();
        }
    }

    /// The resolver must name the binary the process is *actually* running.
    /// Every mutant that shortcuts the `openat`/`readlinkat` pair - a hardcoded
    /// path, an empty string, dropping `O_NOFOLLOW`, treating a negative fd as
    /// success - answers something other than this exact path.
    #[test]
    fn the_exe_resolver_names_the_binary_a_real_process_is_running() {
        let dir = tempfile::tempdir().unwrap();
        let Some(child) = Child::spawn_named(dir.path(), "arlen-probe") else {
            return; // no sleep binary on this host
        };

        let got = exe_path_openat(child.pid()).expect("resolve a live child");
        assert_eq!(
            got, child.exe,
            "the resolver must report the binary actually running, not a guess"
        );
    }

    /// The inode gate is what makes a copied binary fail identity (F3), so it
    /// has to read the inode of the *running* image. Cross-checking it against
    /// a plain `stat` of the same path pins both halves: if either the fd
    /// pinning or the `fstatat` were mutated, the pair would disagree.
    #[test]
    fn the_inode_of_a_live_process_matches_a_stat_of_its_binary() {
        use std::os::unix::fs::MetadataExt;

        let dir = tempfile::tempdir().unwrap();
        let Some(child) = Child::spawn_named(dir.path(), "arlen-probe") else {
            return;
        };

        let (ino, dev) = exe_ino_dev(child.pid()).expect("stat a live child's image");
        let meta = std::fs::metadata(&child.exe).unwrap();
        assert_eq!((ino, dev), (meta.ino(), meta.dev()));
        assert_ne!(ino, 0, "a zeroed stat buffer is not an identity");
    }

    /// An arbitrary binary is not an app. This is the security property the
    /// resolver exists for: identity comes from a trusted install root, so a
    /// process running out of a temp directory gets no id at all rather than
    /// an id derived from its filename.
    #[test]
    fn a_process_running_from_a_temp_dir_earns_no_app_id() {
        let dir = tempfile::tempdir().unwrap();
        let Some(child) = Child::spawn_named(dir.path(), "arlen-ai-daemon") else {
            return;
        };

        // Named to look exactly like the canonical daemon, and still refused:
        // only the anchored path counts, never the basename.
        assert!(
            app_id_from_pid(child.pid()).is_err(),
            "a lookalike binary outside a trusted root must not resolve"
        );
    }

    /// `process_alive` backs the PID-reuse guard, so it has to distinguish a
    /// running pid from a reaped one. A mutant answering a constant passes
    /// either half alone; it cannot pass both.
    #[test]
    fn liveness_tracks_a_child_across_its_death() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mut child) = Child::spawn_named(dir.path(), "arlen-probe") else {
            return;
        };
        let pid = child.pid();

        assert!(process_alive(pid), "a running child must read as alive");
        child.reap();
        assert!(
            !process_alive(pid),
            "a reaped pid must read as gone, or the reuse guard never fires"
        );
    }

    /// The start-time parse skips `comm` by splitting on the LAST `") "`, and
    /// the doc comment says why: `comm` is the binary's basename and may itself
    /// contain parens. That reasoning was never exercised - the old test for it
    /// was an empty body with a comment explaining that it tested nothing.
    ///
    /// A child whose basename is `we (ird) name` puts a `") "` *inside* comm.
    /// A leftmost split lands there and reads one field early, which is
    /// `itrealvalue` (0 for any normal process) instead of the start time.
    #[test]
    fn a_binary_name_containing_parens_does_not_shift_the_start_time_field() {
        let dir = tempfile::tempdir().unwrap();
        let Some(child) = Child::spawn_named(dir.path(), "we (ird) name") else {
            return;
        };
        let pid = child.pid();

        let got = pid_start_time(pid).expect("parse a paren-laden comm");

        // Independent oracle: the spec is "column 22, where comm ends at the
        // last `)`". Parsed here by a different route than the code under test.
        let raw = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let tail = &raw[raw.rfind(')').unwrap() + 1..];
        let expected: u64 = tail.split_whitespace().nth(19).unwrap().parse().unwrap();

        assert_eq!(got, expected, "comm parens must not shift the field index");
        assert!(got > 0, "a leftmost split would read itrealvalue, which is 0");
        assert!(
            got >= pid_start_time(std::process::id()).unwrap(),
            "the child started after the test process, so its tick cannot be earlier"
        );
    }
}

#[cfg(test)]
mod shipped_desktop_entry_tests {
    use super::path_to_app_id;
    use std::path::{Path, PathBuf};

    /// The repo root, from this crate's manifest dir.
    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("sdk/permissions sits two levels below the repo root")
            .to_path_buf()
    }

    /// Every shipped desktop entry must name the identity the resolver actually
    /// derives from the binary it launches.
    ///
    /// The launcher takes an app's id from `X-Arlen-AppId` and the permission
    /// system takes it from the binary's path, so these are two statements of one
    /// fact and nothing else compares them. A drift here is invisible until a
    /// confined launch looks for a profile under the wrong name.
    ///
    /// The path this resolves against is the app's install directory, not
    /// `/usr/bin`. That IS the convention: an app installed to `/usr/bin` took the
    /// system-daemon rule and resolved to the bare basename, which is where the
    /// two dialects of app id came from. The image now installs each app to
    /// `/usr/lib/arlen/apps/<id>/bin/` and symlinks it into the path, so rule (3)
    /// reads the id off the directory and `/proc/<pid>/exe` resolves through the
    /// symlink to the same place.
    #[test]
    fn a_shipped_entry_names_the_id_the_resolver_derives() {
        let root = repo_root();
        let mut checked = 0;
        for app in std::fs::read_dir(root.join("apps")).expect("apps/ exists").flatten() {
            let dist = app.path().join("dist");
            if !dist.is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(&dist).expect("readable dist").flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) != Some("desktop") {
                    continue;
                }
                let text = std::fs::read_to_string(&p).expect("readable entry");
                let declared = text
                    .lines()
                    .find_map(|l| l.strip_prefix("X-Arlen-AppId="))
                    .unwrap_or_else(|| panic!("{} has no X-Arlen-AppId", p.display()))
                    .trim();
                let stem = p.file_stem().and_then(|s| s.to_str()).expect("utf-8 stem");
                // The id comes from the app's own `tauri.conf.json`, NOT from the
                // entry being checked. Building the install path out of `declared`
                // made this a tautology - the resolver reads the directory name,
                // so it would echo whatever the entry said and agree with itself.
                // Caught by changing an entry to a wrong id and watching it pass.
                let conf = std::fs::read_to_string(app.path().join("src-tauri/tauri.conf.json"))
                    .unwrap_or_else(|e| panic!("{}: no tauri.conf.json ({e})", app.path().display()));
                let identifier = conf
                    .lines()
                    .find_map(|l| l.trim().strip_prefix("\"identifier\":"))
                    .map(|v| v.trim().trim_matches(|c| c == '"' || c == ',' || c == ' '))
                    .unwrap_or_else(|| panic!("{}: no identifier", app.path().display()))
                    .to_string();
                let install = format!("/usr/lib/arlen/apps/{identifier}/bin/{stem}");
                let resolved = path_to_app_id(&PathBuf::from(&install))
                    .unwrap_or_else(|e| panic!("{} does not resolve: {e:?}", p.display()));
                assert_eq!(
                    declared,
                    resolved,
                    "{} declares {declared} but {install} resolves to {resolved}",
                    p.display()
                );
                checked += 1;
            }
        }
        // An empty sweep would pass silently, and this test exists precisely
        // because nothing else looks at these files.
        assert!(checked > 0, "no shipped desktop entries were found to check");
    }
}

#[cfg(test)]
mod exe_diagnosis_tests {
    use super::*;

    /// The message must name the cause, not just the symptom.
    ///
    /// pid 1 is root-owned on every Linux box, so an unprivileged test reproduces
    /// exactly the shape that had the undo signer refusing callers with nothing to
    /// go on: `Permission denied` and no indication of which of the three causes
    /// it was. Skipped when run as root, where the read succeeds and there is
    /// nothing to diagnose.
    /// The capability line must carry a real value, not the fallback.
    ///
    /// `CapPrm` is the field `__ptrace_may_access` actually decides on, so a
    /// diagnosis that silently printed "unreadable" there would look informative
    /// and say nothing - the failure mode this whole function exists to avoid.
    #[test]
    fn the_capability_field_is_actually_read() {
        let mine = proc_field(std::process::id(), "CapPrm");
        assert!(
            mine != "unreadable" && mine.chars().all(|c| c.is_ascii_hexdigit()),
            "our own CapPrm should read as hex, got {mine:?}"
        );
        assert_eq!(proc_field(u32::MAX, "CapPrm"), "unreadable", "absent pid");
    }

    #[test]
    fn a_refused_exe_read_says_which_cause_it_was() {
        // SAFETY: getuid takes no arguments and always succeeds.
        if unsafe { libc::getuid() } == 0 {
            return;
        }
        let err = app_id_from_pid(1).expect_err("pid 1 is not readable unprivileged");
        let msg = err.to_string();
        assert!(msg.contains("pid 1"), "names the process: {msg}");
        assert!(
            msg.contains("/proc/1 owned by 0"),
            "names the /proc owner, which is what separates the causes: {msg}"
        );
        assert!(
            msg.contains("another user"),
            "reaches a conclusion rather than leaving the reader to: {msg}"
        );
    }
}
