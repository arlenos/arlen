//! Linux sandbox: no_new_privs + Landlock (no filesystem) + seccomp
//! (no network). Unprivileged; requires Linux ≥5.13 for Landlock.

use std::collections::BTreeMap;

use landlock::{Access, AccessFs, Ruleset, RulesetAttr, RulesetStatus, ABI};
use seccompiler::{BpfProgram, SeccompAction, SeccompFilter};

use crate::SandboxError;

/// Lock the current process down: it may no longer gain privileges,
/// open any filesystem path, or create any socket. Already-open file
/// descriptors (stdin/stdout) keep working.
///
/// Call this once, early, before touching untrusted input. On any
/// failure it returns [`SandboxError::Setup`] and the caller must exit
/// rather than continue unsandboxed.
pub fn apply_sandbox() -> Result<(), SandboxError> {
    apply_sandbox_profile(SandboxProfile::Tight)
}

/// The seccomp profile a worker installs. Both are deny-by-default allowlists;
/// they differ ONLY in whether thread creation is permitted. The wider profile
/// is confined to the ONE worker that hosts a C-linked threaded decoder
/// (AVIF/dav1d, HEIC/libheif) - per the one-sandboxed-process-per-decoder model,
/// the pure-Rust decoders keep [`SandboxProfile::Tight`] and never get `clone`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxProfile {
    /// No thread creation: the default for every pure-Rust worker.
    Tight,
    /// The tight allowlist PLUS thread-creation syscalls (`clone`/`clone3` and
    /// the glibc thread-init calls), for a worker hosting a threaded C decoder.
    /// Still deny-by-default + no filesystem + no network; only `clone` & co.
    /// are added, NOT a blanket widen.
    Threaded,
}

impl SandboxProfile {
    /// The extra syscalls this profile allows beyond the tight base.
    fn extra_syscalls(self) -> &'static [i64] {
        match self {
            SandboxProfile::Tight => &[],
            // dav1d / libheif spawn a decode thread pool: thread creation
            // (`clone`/`clone3`) plus glibc's per-thread init (`set_robust_list`,
            // `rseq`). futex/mmap/madvise/sched_* are already in the tight base.
            SandboxProfile::Threaded => &[
                libc::SYS_clone,
                libc::SYS_clone3,
                libc::SYS_set_robust_list,
                libc::SYS_rseq,
            ],
        }
    }
}

/// Lock the current process down with a chosen seccomp [`SandboxProfile`]. Same
/// no_new_privs + Landlock (no filesystem, no network) as [`apply_sandbox`]; the
/// profile only widens the syscall allowlist for thread creation.
pub fn apply_sandbox_profile(profile: SandboxProfile) -> Result<(), SandboxError> {
    set_no_new_privs()?;
    restrict_filesystem()?;
    restrict_syscalls(profile.extra_syscalls())?;
    Ok(())
}

/// Lock down with the [`SandboxProfile::Threaded`] profile, for the AVIF/HEIC
/// codec worker only. Identical containment to [`apply_sandbox`] except thread
/// creation is permitted (the C decoders' thread pool needs it).
pub fn apply_sandbox_threaded() -> Result<(), SandboxError> {
    apply_sandbox_profile(SandboxProfile::Threaded)
}

/// Set `PR_SET_NO_NEW_PRIVS`, required before installing an
/// unprivileged seccomp filter and a good hardening baseline in its own
/// right (no setuid escalation).
fn set_no_new_privs() -> Result<(), SandboxError> {
    // SAFETY: `prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0)` only flips a
    // per-process flag. It takes no pointers and cannot corrupt memory.
    let rc = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if rc != 0 {
        return Err(SandboxError::Setup(format!(
            "PR_SET_NO_NEW_PRIVS failed: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

/// Apply a Landlock ruleset that handles every filesystem access right
/// up to ABI v5 (read, write, create, remove, **truncate**, **refer**,
/// **ioctl-dev**, ...) but grants none, so no path can be opened or
/// mutated. The worker reads and writes only its inherited stdin/stdout
/// fds, which are unaffected. v5 covers the rights (Truncate, Refer,
/// IoctlDev) that the original v1 left under plain DAC; on an older
/// kernel best-effort downgrades and the seccomp backstop below still
/// denies the path-mutating syscalls.
fn restrict_filesystem() -> Result<(), SandboxError> {
    // ABI v5 includes every filesystem right through IoctlDev. The
    // crate defaults to best-effort, so on an older kernel unsupported
    // rights are dropped rather than failing the call.
    let abi = ABI::V5;
    let status = Ruleset::default()
        .handle_access(AccessFs::from_all(abi))
        .map_err(|e| SandboxError::Setup(format!("landlock handle_access: {e}")))?
        .create()
        .map_err(|e| SandboxError::Setup(format!("landlock create: {e}")))?
        // No `add_rule`: nothing is allowed.
        .restrict_self()
        .map_err(|e| SandboxError::Setup(format!("landlock restrict_self: {e}")))?;

    // If the kernel did not enforce the ruleset at all, fail closed
    // rather than run believing we are sandboxed when we are not.
    if status.ruleset == RulesetStatus::NotEnforced {
        return Err(SandboxError::Setup(
            "landlock not enforced by kernel".to_string(),
        ));
    }
    Ok(())
}

/// Install a seccomp filter that **allows only** the syscalls the
/// worker needs after lockdown and denies everything else with `EPERM`.
///
/// An allowlist (deny-by-default) is used rather than a blocklist on
/// purpose: a blocklist must name every dangerous syscall and is bypassed
/// by aliases such as the x32 ABI numbers (native number with the x32
/// bit set), which are different syscall numbers not on any blocklist.
/// With an allowlist those aliases — and any future or unforeseen
/// syscall — are simply not in the allowed set and are denied. The
/// allowed set below covers reading stdin, allocating, writing stdout,
/// signal handling, and exiting; it deliberately excludes socket,
/// process-creation, ptrace/pidfd, and path-opening syscalls, so the
/// worker cannot reach the network, spawn a descendant, inspect the
/// parent, or open a file. Denial is `EPERM` (not kill), so a parser
/// that probes a forbidden operation just sees it fail.
fn restrict_syscalls(extra: &[i64]) -> Result<(), SandboxError> {
    // Syscalls the worker legitimately issues after the filter is in
    // place: stdio I/O, the allocator, futexes, signal return/handling,
    // randomness (HashMap/std), clocks, fd close, stat, and exit. The
    // happy path is exercised by the integration tests, so a missing
    // entry surfaces as a failing parse rather than a silent gap.
    let allowed: &[i64] = &[
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_readv,
        libc::SYS_writev,
        libc::SYS_close,
        libc::SYS_lseek,
        // NB: fcntl is deliberately NOT allowed. F_SETOWN/F_SETSIG plus
        // F_SETFL|O_ASYNC on the inherited stdio pipes can make the
        // kernel deliver a signal to another same-UID process, which is
        // a signal-delivery primitive even with kill/tgkill removed. The
        // worker's happy path does not need fcntl.
        libc::SYS_mmap,
        libc::SYS_munmap,
        libc::SYS_mremap,
        libc::SYS_mprotect,
        libc::SYS_brk,
        libc::SYS_madvise,
        libc::SYS_futex,
        libc::SYS_sched_yield,
        libc::SYS_sched_getaffinity,
        libc::SYS_getrandom,
        libc::SYS_clock_gettime,
        libc::SYS_clock_nanosleep,
        libc::SYS_nanosleep,
        libc::SYS_rt_sigreturn,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigaction,
        libc::SYS_sigaltstack,
        libc::SYS_getpid,
        libc::SYS_gettid,
        libc::SYS_exit,
        libc::SYS_exit_group,
        // fd-based stat only. Path-based stat (statx/newfstatat on a
        // path) is deliberately excluded: Landlock does not mediate the
        // stat family, so allowing it would let an exploited parser probe
        // arbitrary path existence/metadata and exfiltrate it via stdout.
        // `tgkill` is excluded too — it can signal arbitrary same-UID
        // processes (a host DoS), and the happy path never needs it.
        libc::SYS_fstat,
    ];
    let rules: BTreeMap<i64, Vec<seccompiler::SeccompRule>> = allowed
        .iter()
        .chain(extra.iter())
        .map(|&nr| (nr, Vec::new()))
        .collect();

    let arch = std::env::consts::ARCH
        .try_into()
        .map_err(|e| SandboxError::Setup(format!("seccomp arch: {e}")))?;

    let filter = SeccompFilter::new(
        rules,
        // Syscalls NOT in the allow set (incl. x32 aliases): EPERM.
        SeccompAction::Errno(libc::EPERM as u32),
        // Allowed syscalls: permit.
        SeccompAction::Allow,
        arch,
    )
    .map_err(|e| SandboxError::Setup(format!("seccomp filter: {e}")))?;

    let program: BpfProgram = filter
        .try_into()
        .map_err(|e| SandboxError::Setup(format!("seccomp compile: {e}")))?;
    seccompiler::apply_filter(&program)
        .map_err(|e| SandboxError::Setup(format!("seccomp apply: {e}")))?;
    Ok(())
}
