//! The seccomp syscall filter installed on a confined `run_command`.
//!
//! `run_command` runs a single user-confirmed, arbitrary command in the confiner's
//! bwrap sandbox. bwrap already unshares the namespaces (user/pid/mount/net) and sets
//! `no_new_privs`, but a syscall filter is the second wall: it denies the catastrophic
//! escape and host-compromise primitives (debugging another process, loading kernel
//! code, reconfiguring mounts/namespaces, the kernel keyring, io_uring, grabbing fds
//! from other processes) even if a bug in the namespace setup or a kernel weakness
//! would otherwise let them through.
//!
//! The filter is a deny-by-default ALLOWLIST, not a blocklist. A blocklist that names
//! the dangerous syscalls is bypassed by the x32 ABI aliases (the native number with
//! the x32 bit set is a different syscall number the blocklist never mentions); an
//! allowlist denies those aliases, and any future syscall, simply by their absence. So
//! the catastrophic operations are absent from the allowlist and answered `EPERM`,
//! and the x32 aliases of even allowed syscalls are denied too (they match no allow
//! rule). This is the identical design and x32 mitigation the arlen-run app filter
//! uses; the runtime test below proves it at exec time.
//!
//! The allowlist is deliberately BROADER than the arlen-run app filter: an app is one
//! known Tauri binary, whereas `run_command` runs an arbitrary CLI (coreutils, grep,
//! find, tar, git, compilers, language runtimes), so it adds the syscalls those tools
//! need (the xattr family, copy_file_range/sendfile/splice, job-control setpgid/setsid,
//! NUMA/mempolicy, memory-protection keys, membarrier) while still withholding every
//! escape primitive. A missing entry fails a legitimate command closed (`EPERM`/`SIGSYS`
//! at the syscall) rather than opening a hole, and the fix is to add the named syscall,
//! never to widen the default.

use std::collections::BTreeMap;

use seccompiler::{BpfProgram, SeccompAction, SeccompFilter};

/// A seccomp compilation error.
#[derive(Debug, thiserror::Error)]
pub enum SeccompError {
    /// The host architecture is not one seccompiler recognises.
    #[error("unsupported seccomp architecture: {0}")]
    Arch(String),
    /// The filter could not be compiled to a cBPF program.
    #[error("seccomp filter compile failed: {0}")]
    Compile(String),
}

/// The syscalls a confined arbitrary command is allowed to make. Everything not
/// listed (including every x32 alias and every catastrophic operation) is answered
/// `EPERM`. Broader than the arlen-run app allowlist because the command is an
/// arbitrary CLI, not one known app binary; tuned so a broken command names the
/// syscall to add, never the other way around.
fn command_allowlist() -> Vec<libc::c_long> {
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
        libc::SYS_preadv2,
        libc::SYS_pwritev2,
        libc::SYS_open,
        libc::SYS_openat,
        libc::SYS_openat2,
        libc::SYS_close,
        libc::SYS_close_range,
        libc::SYS_lseek,
        libc::SYS_fsync,
        libc::SYS_fdatasync,
        libc::SYS_sync,
        libc::SYS_syncfs,
        libc::SYS_ftruncate,
        libc::SYS_truncate,
        libc::SYS_fallocate,
        libc::SYS_fadvise64,
        libc::SYS_readahead,
        libc::SYS_getdents64,
        libc::SYS_getcwd,
        libc::SYS_chdir,
        libc::SYS_fchdir,
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
        libc::SYS_lchown,
        libc::SYS_fchownat,
        libc::SYS_umask,
        libc::SYS_utimensat,
        libc::SYS_access,
        libc::SYS_faccessat,
        libc::SYS_faccessat2,
        libc::SYS_fcntl,
        libc::SYS_flock,
        libc::SYS_dup,
        libc::SYS_dup2,
        libc::SYS_dup3,
        libc::SYS_pipe,
        libc::SYS_pipe2,
        libc::SYS_ioctl,
        libc::SYS_memfd_create,
        // Zero-copy and bulk copy (cp, tar, coreutils, rsync).
        libc::SYS_copy_file_range,
        libc::SYS_sendfile,
        libc::SYS_splice,
        libc::SYS_tee,
        // Extended attributes (tar, cp -a, git, rsync preserve them).
        libc::SYS_getxattr,
        libc::SYS_lgetxattr,
        libc::SYS_fgetxattr,
        libc::SYS_setxattr,
        libc::SYS_lsetxattr,
        libc::SYS_fsetxattr,
        libc::SYS_listxattr,
        libc::SYS_llistxattr,
        libc::SYS_flistxattr,
        libc::SYS_removexattr,
        libc::SYS_lremovexattr,
        libc::SYS_fremovexattr,
        // File metadata.
        libc::SYS_stat,
        libc::SYS_lstat,
        libc::SYS_fstat,
        libc::SYS_newfstatat,
        libc::SYS_statx,
        libc::SYS_statfs,
        libc::SYS_fstatfs,
        // Inotify (file watchers, build tools).
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
        libc::SYS_mlock2,
        libc::SYS_msync,
        libc::SYS_mincore,
        // Memory-protection keys and NUMA mempolicy (language runtimes: Go, JVM).
        libc::SYS_pkey_alloc,
        libc::SYS_pkey_free,
        libc::SYS_pkey_mprotect,
        libc::SYS_mbind,
        libc::SYS_set_mempolicy,
        libc::SYS_get_mempolicy,
        libc::SYS_membarrier,
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
        libc::SYS_sched_setparam,
        libc::SYS_sched_getscheduler,
        libc::SYS_sched_setscheduler,
        libc::SYS_sched_get_priority_max,
        libc::SYS_sched_get_priority_min,
        libc::SYS_sched_rr_get_interval,
        libc::SYS_getpriority,
        libc::SYS_setpriority,
        libc::SYS_arch_prctl,
        libc::SYS_prctl,
        // Process creation, job control and reaping (shells, build drivers).
        libc::SYS_execve,
        libc::SYS_execveat,
        libc::SYS_wait4,
        libc::SYS_waitid,
        libc::SYS_setpgid,
        libc::SYS_setsid,
        libc::SYS_getrusage,
        libc::SYS_personality,
        libc::SYS_exit,
        libc::SYS_exit_group,
        // Event loops and readiness.
        libc::SYS_poll,
        libc::SYS_ppoll,
        libc::SYS_pselect6,
        libc::SYS_select,
        libc::SYS_epoll_create1,
        libc::SYS_epoll_ctl,
        libc::SYS_epoll_wait,
        libc::SYS_epoll_pwait,
        libc::SYS_epoll_pwait2,
        libc::SYS_eventfd2,
        libc::SYS_signalfd4,
        libc::SYS_timerfd_create,
        libc::SYS_timerfd_settime,
        libc::SYS_timerfd_gettime,
        // Sockets. IP exfiltration is prevented by the network namespace, NOT this
        // filter, so a command may speak the local sockets it legitimately needs
        // (e.g. a compiler talking to a local build server over AF_UNIX).
        // NB the netns does NOT bound AF_UNIX by pathname: it isolates only ABSTRACT
        // sockets, and a read-only bind still permits connect(). So these entries are
        // only safe because the command's read surface excludes every directory
        // holding a socket that matters - `/run` (the session bus, every arlen daemon)
        // and `$HOME`. See `server.rs::system_read_roots`: the two mechanisms compose
        // only while that curated set holds, so widening it re-opens this.
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
        libc::SYS_rt_sigqueueinfo,
        libc::SYS_rt_tgsigqueueinfo,
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
        // Process, credential (read-only) and system info.
        libc::SYS_getpid,
        libc::SYS_gettid,
        libc::SYS_getppid,
        libc::SYS_getuid,
        libc::SYS_geteuid,
        libc::SYS_getgid,
        libc::SYS_getegid,
        libc::SYS_getresuid,
        libc::SYS_getresgid,
        libc::SYS_getgroups,
        libc::SYS_getpgrp,
        libc::SYS_getpgid,
        libc::SYS_getsid,
        libc::SYS_getcpu,
        libc::SYS_getrandom,
        libc::SYS_uname,
        libc::SYS_sysinfo,
        libc::SYS_prlimit64,
        libc::SYS_getrlimit,
        libc::SYS_setrlimit,
        libc::SYS_capget,
    ]
}

/// The catastrophic syscalls that must NEVER be in the allowlist: an escape or
/// host-compromise primitive (debugging another process, reading/writing another
/// process's memory, loading kernel code, reconfiguring namespaces/mounts, the kernel
/// keyring, io_uring's shared attack surface, grabbing an fd out of another process,
/// resolving a file handle back to a path outside the sandbox). They are denied by
/// omission, not by a deny entry; this list exists so a test can assert the allowlist
/// never accidentally grows to include one.
#[cfg(test)]
fn forbidden_syscalls() -> Vec<libc::c_long> {
    vec![
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_bpf,
        libc::SYS_perf_event_open,
        libc::SYS_kexec_load,
        libc::SYS_kexec_file_load,
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
        libc::SYS_seccomp,
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
        libc::SYS_pidfd_getfd,
        libc::SYS_open_by_handle_at,
        libc::SYS_name_to_handle_at,
    ]
}

/// Build the compiled cBPF program for the command filter: every allowed syscall is
/// an unconditional allow, every other syscall (including the x32 aliases) gets
/// `EPERM`. The program is what `bwrap --seccomp <fd>` installs on the confined
/// command right before it execs.
pub fn compile_command_filter() -> Result<BpfProgram, SeccompError> {
    let rules: BTreeMap<libc::c_long, Vec<seccompiler::SeccompRule>> =
        command_allowlist().into_iter().map(|nr| (nr, Vec::new())).collect();

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

/// The compiled command filter serialized to the raw cBPF byte blob `bwrap --seccomp
/// <fd>` expects: the `struct sock_filter[]` array, one 8-byte instruction each
/// (`code: u16`, `jt: u8`, `jf: u8`, `k: u32`) in native byte order, with no
/// `sock_fprog` header (bwrap derives the length from the blob size). Serialized field
/// by field rather than by reinterpreting the struct's memory, so it does not depend
/// on the compiler not inserting padding.
pub fn command_filter_bytes() -> Result<Vec<u8>, SeccompError> {
    let program = compile_command_filter()?;
    let mut bytes = Vec::with_capacity(program.len() * 8);
    for insn in &program {
        bytes.extend_from_slice(&insn.code.to_ne_bytes());
        bytes.push(insn.jt);
        bytes.push(insn.jf);
        bytes.extend_from_slice(&insn.k.to_ne_bytes());
    }
    Ok(bytes)
}

/// Create an anonymous in-memory file holding the compiled seccomp cBPF and return
/// its fd, positioned at offset 0 so `bwrap` reads the whole program. The fd is
/// created without `MFD_CLOEXEC` so it survives the exec into `bwrap` (every other fd
/// the daemon holds is `O_CLOEXEC` by Rust default, so nothing else leaks); the parent
/// closes its copy once the child has forked.
#[cfg(target_os = "linux")]
pub(crate) fn make_seccomp_memfd(bpf: &[u8]) -> std::io::Result<libc::c_int> {
    use std::ffi::CString;
    let name = CString::new("arlen-run-command-seccomp").expect("static name has no nul");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filter_compiles_to_a_non_empty_program() {
        let program = compile_command_filter().expect("filter compiles on this arch");
        // A real cBPF allow/deny program is far more than a couple of instructions
        // (the arch check, the per-syscall comparisons, the default action).
        assert!(program.len() > 10, "compiled program is implausibly short");
    }

    #[test]
    fn the_allowlist_covers_the_obvious_command_needs() {
        let allow = command_allowlist();
        for needed in [
            libc::SYS_read,
            libc::SYS_write,
            libc::SYS_mmap,
            libc::SYS_futex,
            libc::SYS_openat,
            libc::SYS_execve,   // run child programs (a shell driving subcommands)
            libc::SYS_clone,    // fork
            libc::SYS_wait4,    // reap
            libc::SYS_getdents64, // list a directory (ls/find)
            libc::SYS_getrandom,
            libc::SYS_copy_file_range, // cp / coreutils
            libc::SYS_fgetxattr,       // tar / git preserve xattrs
            libc::SYS_epoll_wait,
            libc::SYS_exit_group,
        ] {
            assert!(allow.contains(&needed), "allowlist is missing syscall {needed}");
        }
    }

    #[test]
    fn the_allowlist_excludes_every_catastrophic_syscall() {
        let allow = command_allowlist();
        for forbidden in forbidden_syscalls() {
            assert!(
                !allow.contains(&forbidden),
                "catastrophic syscall {forbidden} must not be in the command allowlist"
            );
        }
    }

    /// The x32 ABI syscall bit: an x32 caller invokes a native syscall number OR'd
    /// with this. A deny-by-default allowlist denies every x32 alias by its absence,
    /// closing the blocklist-bypass the module doc describes.
    #[cfg(target_arch = "x86_64")]
    const X32_SYSCALL_BIT: libc::c_long = 0x4000_0000;

    /// Runtime proof that the COMPILED filter denies at exec time, not merely that the
    /// allowlist SET excludes catastrophic entries. Forks a child, installs the real
    /// filter, and verifies that an allowed syscall (`getpid`) still succeeds, a
    /// catastrophic one (`unshare`) returns `EPERM` instead of executing, and the x32
    /// ALIAS of an allowed syscall (`getuid | X32`) is denied (the blocklist-bypass is
    /// closed). Mirrors the arlen-run runtime test. Gracefully skips where seccomp is
    /// unavailable (the child `_exit(90)`s).
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn the_compiled_filter_denies_catastrophic_syscalls_at_runtime() {
        let program = compile_command_filter().expect("filter compiles");

        // SAFETY: fork in a test; the child only makes async-signal-safe raw syscalls
        // and _exits, never returning to the Rust runtime.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");

        if pid == 0 {
            // Child: install the filter, then probe.
            if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
                unsafe { libc::_exit(90) };
            }
            if seccompiler::apply_filter(&program).is_err() {
                unsafe { libc::_exit(90) };
            }
            // An allowed syscall still works.
            if unsafe { libc::syscall(libc::SYS_getpid) } < 0 {
                unsafe { libc::_exit(91) };
            }
            // A catastrophic syscall is answered EPERM by the filter, not run.
            let un = unsafe { libc::syscall(libc::SYS_unshare, libc::CLONE_NEWUSER) };
            let un_errno = unsafe { *libc::__errno_location() };
            if !(un == -1 && un_errno == libc::EPERM) {
                unsafe { libc::_exit(92) };
            }
            // The x32 alias of an allowed syscall matches no rule, so the default
            // EPERM applies (the blocklist bypass is closed).
            let x32_r = unsafe { libc::syscall(libc::SYS_getuid | X32_SYSCALL_BIT) };
            let x32_errno = unsafe { *libc::__errno_location() };
            if !(x32_r == -1 && x32_errno == libc::EPERM) {
                unsafe { libc::_exit(93) };
            }
            unsafe { libc::_exit(0) };
        }

        // Parent: reap and interpret.
        let mut status = 0i32;
        let waited = unsafe { libc::waitpid(pid, &mut status, 0) };
        assert_eq!(waited, pid, "waitpid failed");
        let code = if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { -1 };
        match code {
            0 => {}
            90 => eprintln!(
                "SKIP the_compiled_filter_denies_catastrophic_syscalls_at_runtime: \
                 seccomp unavailable in this environment"
            ),
            91 => panic!("an allowed syscall (getpid) was denied by the filter"),
            92 => panic!("a catastrophic syscall (unshare) was NOT denied at runtime"),
            93 => panic!("the x32 alias of an allowed syscall was NOT denied (blocklist bypass open)"),
            other => panic!("unexpected child exit code {other}"),
        }
    }
}
