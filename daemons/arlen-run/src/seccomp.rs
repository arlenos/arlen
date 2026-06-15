//! The third confinement layer: a per-app seccomp allowlist (GAP-6).
//!
//! Namespaces (bwrap) and Landlock (the writable-set ruleset) are the first two
//! layers; this is the syscall filter. It is **deny-by-default**: only the
//! syscalls a normal confined GUI app needs are allowed, everything else returns
//! `EPERM`. An allowlist is used rather than a blocklist on purpose (the
//! `ai-sandbox` reasoning): a blocklist must name every dangerous syscall and is
//! bypassed by the x32 ABI aliases (the native number with the x32 bit set, a
//! different syscall number), whereas an allowlist denies those aliases and any
//! future syscall simply by their absence. So the catastrophic operations
//! (`ptrace`, `process_vm_*`, `bpf`, `perf_event_open`, the module-loading and
//! `kexec` calls, `mount`/`pivot_root`/`setns`/`unshare`, the key-management
//! calls) are blocked by *not being in the set*, not by a fragile deny entry.
//!
//! The filter is compiled to cBPF here and handed to `bwrap --seccomp <fd>` (the
//! wiring lives in `spawn`), so `bwrap` installs it on the app **after** its own
//! namespace and mount setup, just before exec - the launcher never filters
//! `bwrap` itself.
//!
//! Denial is `EPERM` (not kill), so an app probing a forbidden call sees it fail
//! and can degrade rather than being killed outright. The allowed set below is a
//! documented baseline broad enough for a Wayland GUI app (file I/O, memory,
//! threads/futex, poll/epoll, AF_UNIX IPC, timers, signals, process info); it is
//! tuned on a real kernel against real apps - a missing entry shows up as an app
//! that breaks on a specific call, which is then added.

#![cfg(target_os = "linux")]

use seccompiler::{BpfProgram, SeccompAction, SeccompFilter};
use std::collections::BTreeMap;

/// Errors building or compiling the app seccomp filter.
#[derive(Debug)]
pub enum SeccompError {
    /// The host architecture is not one seccompiler targets.
    Arch(String),
    /// The filter could not be built or compiled to cBPF.
    Compile(String),
}

impl std::fmt::Display for SeccompError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SeccompError::Arch(e) => write!(f, "seccomp arch: {e}"),
            SeccompError::Compile(e) => write!(f, "seccomp filter: {e}"),
        }
    }
}

impl std::error::Error for SeccompError {}

/// The syscalls a confined GUI app legitimately needs. Deliberately generous (a
/// Wayland/Tauri/GTK app touches a wide surface) but bounded: every entry is a
/// routine application syscall, and the catastrophic operations are absent so
/// they fail with `EPERM`. Tuned on metal - a broken app names the syscall to
/// add, never the other way around.
fn app_allowlist() -> Vec<libc::c_long> {
    vec![
        // File and directory I/O.
        libc::SYS_read,
        libc::SYS_write,
        libc::SYS_readv,
        libc::SYS_writev,
        libc::SYS_pread64,
        libc::SYS_pwrite64,
        libc::SYS_preadv,
        libc::SYS_pwritev,
        libc::SYS_open,
        libc::SYS_openat,
        libc::SYS_openat2,
        libc::SYS_close,
        libc::SYS_close_range,
        libc::SYS_lseek,
        libc::SYS_fsync,
        libc::SYS_fdatasync,
        libc::SYS_ftruncate,
        libc::SYS_fallocate,
        libc::SYS_getdents64,
        libc::SYS_getcwd,
        libc::SYS_readlink,
        libc::SYS_readlinkat,
        libc::SYS_unlink,
        libc::SYS_unlinkat,
        libc::SYS_rename,
        libc::SYS_renameat,
        libc::SYS_renameat2,
        libc::SYS_mkdir,
        libc::SYS_mkdirat,
        libc::SYS_rmdir,
        libc::SYS_symlink,
        libc::SYS_symlinkat,
        libc::SYS_link,
        libc::SYS_linkat,
        libc::SYS_chmod,
        libc::SYS_fchmod,
        libc::SYS_fchmodat,
        libc::SYS_chown,
        libc::SYS_fchown,
        libc::SYS_fchownat,
        libc::SYS_utimensat,
        libc::SYS_access,
        libc::SYS_faccessat,
        libc::SYS_faccessat2,
        libc::SYS_truncate,
        libc::SYS_fcntl,
        libc::SYS_flock,
        libc::SYS_dup,
        libc::SYS_dup2,
        libc::SYS_dup3,
        libc::SYS_pipe,
        libc::SYS_pipe2,
        libc::SYS_ioctl,
        libc::SYS_memfd_create,
        // File metadata.
        libc::SYS_stat,
        libc::SYS_lstat,
        libc::SYS_fstat,
        libc::SYS_newfstatat,
        libc::SYS_statx,
        libc::SYS_statfs,
        libc::SYS_fstatfs,
        // Inotify (file watchers).
        libc::SYS_inotify_init1,
        libc::SYS_inotify_add_watch,
        libc::SYS_inotify_rm_watch,
        // Memory.
        libc::SYS_mmap,
        libc::SYS_munmap,
        libc::SYS_mremap,
        libc::SYS_mprotect,
        libc::SYS_brk,
        libc::SYS_madvise,
        libc::SYS_mlock,
        libc::SYS_munlock,
        libc::SYS_msync,
        // Threads, futexes, scheduling.
        libc::SYS_clone,
        libc::SYS_clone3,
        libc::SYS_futex,
        libc::SYS_set_robust_list,
        libc::SYS_get_robust_list,
        libc::SYS_rseq,
        libc::SYS_set_tid_address,
        libc::SYS_sched_yield,
        libc::SYS_sched_getaffinity,
        libc::SYS_sched_setaffinity,
        libc::SYS_sched_getparam,
        libc::SYS_sched_getscheduler,
        libc::SYS_arch_prctl,
        libc::SYS_prctl,
        // Subprocess (still confined by the inherited namespaces, Landlock, and
        // this same filter): exec a helper, reap it, exit.
        libc::SYS_execve,
        libc::SYS_execveat,
        libc::SYS_wait4,
        libc::SYS_exit,
        libc::SYS_exit_group,
        // Polling and event fds.
        libc::SYS_poll,
        libc::SYS_ppoll,
        libc::SYS_pselect6,
        libc::SYS_select,
        libc::SYS_epoll_create1,
        libc::SYS_epoll_ctl,
        libc::SYS_epoll_wait,
        libc::SYS_epoll_pwait,
        libc::SYS_eventfd2,
        libc::SYS_signalfd4,
        libc::SYS_timerfd_create,
        libc::SYS_timerfd_settime,
        libc::SYS_timerfd_gettime,
        // AF_UNIX IPC: the Wayland display, the session bus, PipeWire. Network
        // containment is the namespace/egress layer's job (an unshared net ns or
        // the egress proxy), not this filter, so the socket calls are allowed for
        // the local sockets the app legitimately speaks.
        libc::SYS_socket,
        libc::SYS_socketpair,
        libc::SYS_connect,
        libc::SYS_bind,
        libc::SYS_listen,
        libc::SYS_accept,
        libc::SYS_accept4,
        libc::SYS_getsockname,
        libc::SYS_getpeername,
        libc::SYS_getsockopt,
        libc::SYS_setsockopt,
        libc::SYS_sendmsg,
        libc::SYS_recvmsg,
        libc::SYS_sendmmsg,
        libc::SYS_recvmmsg,
        libc::SYS_sendto,
        libc::SYS_recvfrom,
        libc::SYS_shutdown,
        // Signals.
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_rt_sigtimedwait,
        libc::SYS_rt_sigpending,
        libc::SYS_rt_sigsuspend,
        libc::SYS_sigaltstack,
        libc::SYS_tgkill,
        libc::SYS_tkill,
        libc::SYS_kill,
        // Time.
        libc::SYS_clock_gettime,
        libc::SYS_clock_getres,
        libc::SYS_clock_nanosleep,
        libc::SYS_nanosleep,
        libc::SYS_gettimeofday,
        libc::SYS_times,
        // Process and system info.
        libc::SYS_getpid,
        libc::SYS_gettid,
        libc::SYS_getppid,
        libc::SYS_getuid,
        libc::SYS_geteuid,
        libc::SYS_getgid,
        libc::SYS_getegid,
        libc::SYS_getresuid,
        libc::SYS_getresgid,
        libc::SYS_getpgrp,
        libc::SYS_getpgid,
        libc::SYS_getsid,
        libc::SYS_getcpu,
        libc::SYS_getrandom,
        libc::SYS_uname,
        libc::SYS_sysinfo,
        libc::SYS_prlimit64,
        libc::SYS_getrlimit,
        libc::SYS_capget,
    ]
}

/// The catastrophic syscalls that must NEVER be in the allowlist: an escape or
/// host-compromise primitive (debugging another process, loading kernel code,
/// reconfiguring namespaces/mounts, the kernel keyring). They are denied by
/// omission, not by a deny entry; this list exists so a test can assert the
/// allowlist never accidentally grows to include one.
#[cfg(test)]
fn forbidden_syscalls() -> Vec<libc::c_long> {
    vec![
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_kexec_load,
        libc::SYS_init_module,
        libc::SYS_finit_module,
        libc::SYS_delete_module,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_chroot,
        libc::SYS_setns,
        libc::SYS_unshare,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
        libc::SYS_reboot,
        libc::SYS_swapon,
        libc::SYS_swapoff,
        libc::SYS_settimeofday,
        libc::SYS_clock_settime,
    ]
}

/// Build the compiled cBPF program for the app filter: every allowed syscall is
/// an unconditional allow, every other syscall (including the x32 aliases) gets
/// `EPERM`. The program is what `bwrap --seccomp <fd>` installs on the app.
pub fn compile_app_filter() -> Result<BpfProgram, SeccompError> {
    let rules: BTreeMap<libc::c_long, Vec<seccompiler::SeccompRule>> =
        app_allowlist().into_iter().map(|nr| (nr, Vec::new())).collect();

    let arch = std::env::consts::ARCH
        .try_into()
        .map_err(|e| SeccompError::Arch(format!("{e:?}")))?;

    let filter = SeccompFilter::new(
        rules,
        // Anything not in the allow set (incl. x32 aliases): EPERM.
        SeccompAction::Errno(libc::EPERM as u32),
        // Allowed syscalls: permit.
        SeccompAction::Allow,
        arch,
    )
    .map_err(|e| SeccompError::Compile(format!("{e}")))?;

    filter
        .try_into()
        .map_err(|e| SeccompError::Compile(format!("{e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filter_compiles_to_a_non_empty_program() {
        let program = compile_app_filter().expect("filter compiles on this arch");
        // A real cBPF allow/deny program is more than a couple of instructions
        // (the arch check, the per-syscall comparisons, the default action).
        assert!(program.len() > 10, "compiled program is implausibly short");
    }

    #[test]
    fn the_allowlist_covers_the_obvious_app_needs() {
        let allow = app_allowlist();
        for needed in [
            libc::SYS_read,
            libc::SYS_write,
            libc::SYS_mmap,
            libc::SYS_futex,
            libc::SYS_openat,
            libc::SYS_socket, // AF_UNIX IPC
            libc::SYS_epoll_wait,
            libc::SYS_exit_group,
        ] {
            assert!(allow.contains(&needed), "allowlist is missing syscall {needed}");
        }
    }

    #[test]
    fn the_allowlist_excludes_every_catastrophic_syscall() {
        // The security property: the deny-by-default set must never accidentally
        // grow to include an escape/compromise primitive.
        let allow = app_allowlist();
        for forbidden in forbidden_syscalls() {
            assert!(
                !allow.contains(&forbidden),
                "catastrophic syscall {forbidden} must not be in the allowlist"
            );
        }
    }

    #[test]
    fn the_allowlist_has_no_duplicates() {
        // A duplicate would mean the BTreeMap silently collapses two intents; keep
        // the list a clean set so review sees exactly what is allowed.
        let allow = app_allowlist();
        let mut sorted = allow.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), allow.len(), "the allowlist has duplicate entries");
    }
}
