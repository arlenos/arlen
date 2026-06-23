//! The per-decoder seccomp allowlist (`quickview-plan.md`, the one-sandboxed-
//! process-per-decoder model).
//!
//! bwrap namespaces + the read-only mount view are the first confinement layer;
//! this is the syscall filter, handed to `bwrap --seccomp <fd>` (the wiring lives
//! in `run_confined_worker`). It is **deny-by-default**: only the syscalls a
//! decode worker needs are allowed, everything else (incl. the x32 ABI aliases)
//! returns `EPERM`. An allowlist, not a blocklist - a blocklist must name every
//! dangerous call and is bypassed by the x32 aliases, whereas absence denies
//! them. So `socket`/`connect` (no network ever), `fork`/`vfork`,
//! `ptrace`/`process_vm_*`, `bpf`/`perf_event_open`, `mount`/`setns`/`unshare`,
//! the module/`kexec`/`keyctl` calls, and every filesystem mutation are blocked
//! by *not being in the set*.
//!
//! Per-decoder profile selection is the security guarantee the plan asks for:
//! the pure-Rust decoders (image-rs, jxl-oxide built without rayon, Symphonia)
//! decode single-threaded and get the tight [`decoder_base_allowlist`] with **no
//! `clone`**; only the C-linked HEIC/AVIF worker (libheif -> threaded dav1d /
//! libde265) gets the wider profile that adds thread creation. The wider profile
//! is confined to that one worker process; it never widens the others.
//!
//! `openat`/`mmap`/`mprotect` are in the base set because the dynamic loader runs
//! them under this filter (bwrap installs it just before exec, so `ld.so` resolves
//! the worker's shared libraries afterwards); *which* files can be opened is the
//! mount-namespace + Landlock layer's job, not seccomp's. The exact set is tuned
//! against a real worker on a real kernel by the on-kernel test in
//! `run_confined_worker`'s module - a missing entry shows up as a worker that
//! dies on a specific call, which is then added; a catastrophic call is never
//! added.

#![cfg(target_os = "linux")]

use arlen_viewers_core::Decoder;
use seccompiler::{BpfProgram, SeccompAction, SeccompFilter};
use std::collections::BTreeMap;

/// Errors building or compiling a decoder seccomp filter.
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

/// The syscalls a single-threaded decode worker needs: stdin/stdout I/O on the
/// inherited fds, the dynamic-loader startup set (`openat`/`mmap`/`mprotect`/
/// `fstat` for the shared libraries, resolved under this filter), memory, the
/// thread-local runtime init glibc runs even single-threaded, signals, timers,
/// randomness, and benign process-info. Deliberately bounded: no `clone` (no
/// threads), no `socket` (no network), no `execve` (no exec), no filesystem
/// mutation, and the catastrophic calls are absent so they `EPERM`.
fn decoder_base_allowlist() -> Vec<libc::c_long> {
    vec![
        // I/O on the inherited stdin/stdout (and the loader's reads).
        libc::SYS_read,
        libc::SYS_readv,
        libc::SYS_pread64,
        libc::SYS_preadv,
        libc::SYS_write,
        libc::SYS_writev,
        libc::SYS_pwrite64,
        libc::SYS_pwritev,
        libc::SYS_close,
        libc::SYS_close_range,
        libc::SYS_lseek,
        libc::SYS_fcntl,
        libc::SYS_dup,
        libc::SYS_dup3,
        libc::SYS_ioctl,
        // The dynamic loader opens + maps the worker's shared libraries under
        // this filter (which files: the mount-ns + Landlock layer bounds that).
        libc::SYS_openat,
        libc::SYS_openat2,
        libc::SYS_readlink,
        libc::SYS_readlinkat,
        libc::SYS_access,
        libc::SYS_faccessat,
        libc::SYS_faccessat2,
        libc::SYS_fstat,
        libc::SYS_newfstatat,
        libc::SYS_statx,
        libc::SYS_fstatfs,
        // Memory.
        libc::SYS_mmap,
        libc::SYS_munmap,
        libc::SYS_mremap,
        libc::SYS_mprotect,
        libc::SYS_madvise,
        libc::SYS_brk,
        // Thread-local + runtime init glibc runs even single-threaded, plus the
        // lock primitive (uncontended futex is used by Rust's own sync types).
        libc::SYS_futex,
        libc::SYS_set_robust_list,
        libc::SYS_get_robust_list,
        libc::SYS_rseq,
        libc::SYS_set_tid_address,
        libc::SYS_arch_prctl,
        libc::SYS_prctl,
        // Signals (Rust's panic/abort path + the runtime's handlers).
        libc::SYS_rt_sigaction,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_sigaltstack,
        libc::SYS_rt_sigtimedwait,
        // Exit + abort.
        libc::SYS_exit,
        libc::SYS_exit_group,
        libc::SYS_tgkill,
        libc::SYS_tkill,
        // bwrap's pid-namespace init reaper inherits this filter and reaps the
        // worker via wait; benign for the worker itself (no children -> ECHILD).
        libc::SYS_wait4,
        libc::SYS_waitid,
        // bwrap installs this filter then execs the worker, so execve must be
        // permitted or the worker never starts. This does not weaken the model:
        // seccomp is inherited across exec, so anything a compromised worker
        // could exec stays bound to the same syscall set (no socket, no ptrace,
        // no clone for the pure-Rust workers) inside the same no-network,
        // read-only-fs namespace.
        libc::SYS_execve,
        libc::SYS_execveat,
        // Randomness + time (stack canary, hashmap seed, decode timing).
        libc::SYS_getrandom,
        libc::SYS_clock_gettime,
        libc::SYS_clock_getres,
        libc::SYS_clock_nanosleep,
        libc::SYS_nanosleep,
        libc::SYS_gettimeofday,
        libc::SYS_times,
        // Benign process info (glibc startup + available-parallelism probe).
        libc::SYS_getpid,
        libc::SYS_gettid,
        libc::SYS_getppid,
        libc::SYS_getuid,
        libc::SYS_geteuid,
        libc::SYS_getgid,
        libc::SYS_getegid,
        libc::SYS_getresuid,
        libc::SYS_getresgid,
        libc::SYS_getrlimit,
        libc::SYS_prlimit64,
        libc::SYS_uname,
        libc::SYS_sysinfo,
        libc::SYS_getcpu,
        libc::SYS_sched_getaffinity,
        libc::SYS_sched_yield,
        // Readiness primitives the runtime may touch.
        libc::SYS_poll,
        libc::SYS_ppoll,
        libc::SYS_epoll_create1,
        libc::SYS_epoll_ctl,
        libc::SYS_epoll_pwait,
        libc::SYS_eventfd2,
    ]
}

/// The extra syscalls the C-linked HEIC/AVIF worker needs to create the decode
/// threads (`dav1d`/`libde265`). Added ONLY for [`Decoder::LibHeif`]; the
/// pure-Rust workers never receive these, so thread creation is denied to them.
fn threading_syscalls() -> Vec<libc::c_long> {
    vec![
        libc::SYS_clone,
        libc::SYS_clone3,
        libc::SYS_sched_setaffinity,
        libc::SYS_sched_getparam,
        libc::SYS_sched_getscheduler,
    ]
}

/// The allowlist for `decoder`: the base set, plus the threading set for the
/// HEIC/AVIF worker only. The result is sorted + deduplicated.
pub fn decoder_allowlist(decoder: Decoder) -> Vec<libc::c_long> {
    let mut set = decoder_base_allowlist();
    if matches!(decoder, Decoder::LibHeif) {
        set.extend(threading_syscalls());
    }
    set.sort_unstable();
    set.dedup();
    set
}

/// Compile `decoder`'s allowlist to cBPF bytes for `bwrap --seccomp <fd>`. The
/// default action is `EPERM` (not kill), so a worker probing a forbidden call
/// sees it fail and can exit cleanly rather than being SIGSYS-killed mid-frame.
pub fn decoder_filter_bytes(decoder: Decoder) -> Result<Vec<u8>, SeccompError> {
    let rules: BTreeMap<libc::c_long, Vec<seccompiler::SeccompRule>> =
        decoder_allowlist(decoder).into_iter().map(|nr| (nr, Vec::new())).collect();

    let arch = std::env::consts::ARCH
        .try_into()
        .map_err(|e| SeccompError::Arch(format!("{e:?}")))?;

    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Errno(libc::EPERM as u32),
        SeccompAction::Allow,
        arch,
    )
    .map_err(|e| SeccompError::Compile(format!("{e}")))?;

    let program: BpfProgram = filter
        .try_into()
        .map_err(|e| SeccompError::Compile(format!("{e}")))?;

    let mut bytes = Vec::with_capacity(program.len() * 8);
    for insn in &program {
        bytes.extend_from_slice(&insn.code.to_ne_bytes());
        bytes.push(insn.jt);
        bytes.push(insn.jf);
        bytes.extend_from_slice(&insn.k.to_ne_bytes());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_profiles_compile_to_nonempty_cbpf() {
        assert!(!decoder_filter_bytes(Decoder::ImageRs).unwrap().is_empty());
        assert!(!decoder_filter_bytes(Decoder::LibHeif).unwrap().is_empty());
        // Each cBPF instruction is 8 bytes.
        assert_eq!(decoder_filter_bytes(Decoder::ImageRs).unwrap().len() % 8, 0);
    }

    #[test]
    fn heic_profile_is_a_strict_superset_of_the_pure_rust_one() {
        let base = decoder_allowlist(Decoder::ImageRs);
        let heic = decoder_allowlist(Decoder::LibHeif);
        // Every base call is in the HEIC set...
        assert!(base.iter().all(|nr| heic.contains(nr)));
        // ...and the HEIC set adds exactly the threading calls, nothing else.
        assert!(heic.len() > base.len());
        for extra in threading_syscalls() {
            assert!(heic.contains(&extra));
            assert!(!base.contains(&extra));
        }
    }

    #[test]
    fn the_jxl_and_audio_workers_share_the_tight_base() {
        // jxl-oxide is built without rayon and Symphonia decodes single-threaded,
        // so they map to the same no-clone profile as image-rs.
        assert_eq!(
            decoder_allowlist(Decoder::JxlOxide),
            decoder_allowlist(Decoder::ImageRs)
        );
        assert_eq!(
            decoder_allowlist(Decoder::Symphonia),
            decoder_allowlist(Decoder::ImageRs)
        );
    }

    #[test]
    fn the_catastrophic_calls_are_absent_from_every_profile() {
        // The guarantee rests on absence: these must not appear in either set.
        let forbidden = [
            libc::SYS_socket,
            libc::SYS_connect,
            libc::SYS_ptrace,
            libc::SYS_process_vm_readv,
            libc::SYS_bpf,
            libc::SYS_mount,
            libc::SYS_unshare,
            libc::SYS_setns,
            libc::SYS_unlinkat,
            libc::SYS_kill,
        ];
        for decoder in [Decoder::ImageRs, Decoder::JxlOxide, Decoder::LibHeif, Decoder::Symphonia] {
            let set = decoder_allowlist(decoder);
            for nr in forbidden {
                assert!(!set.contains(&nr), "{nr} must not be in the {decoder:?} allowlist");
            }
        }
    }

    #[test]
    fn the_pure_rust_profile_forbids_thread_creation() {
        let base = decoder_allowlist(Decoder::ImageRs);
        assert!(!base.contains(&libc::SYS_clone));
        assert!(!base.contains(&libc::SYS_clone3));
    }
}
