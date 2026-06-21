//! Live verification of the per-decoder seccomp profiles (system-dialog... no:
//! quickview-plan.md, AVIF/HEIC isolation). Proves the isolation Tim required:
//! [`apply_sandbox_threaded`] permits thread creation (the C decoders' pool)
//! while the default [`apply_sandbox`] still DENIES it - so the wider profile is
//! confined to the codec worker and the pure-Rust workers keep `clone` blocked.
//!
//! Each case forks a child that installs a profile then tries to spawn a thread,
//! and exits 0/1 by the result; the parent (which never installs seccomp) checks
//! the child's exit code. `#[ignore]`d + run single-threaded, like the other
//! live-sandbox self-tests (needs a real kernel; fork in a test harness wants
//! `--test-threads=1`).

#![cfg(all(target_os = "linux", feature = "thumbnail"))]

/// Run `child` in a forked process and return its exit code (or None if it did
/// not exit cleanly). The child path does a short, bounded sequence ending in
/// `_exit`, so it is safe after fork.
fn fork_exit_code(child: impl FnOnce() -> i32) -> Option<i32> {
    // SAFETY: fork in a test; the child does a bounded sequence then _exit, and
    // the parent only waitpids. Run with --test-threads=1.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork failed");
    if pid == 0 {
        let code = child();
        // _exit, not exit: skip atexit/flush in the seccomped child.
        unsafe { libc::_exit(code) };
    }
    let mut status: libc::c_int = 0;
    let r = unsafe { libc::waitpid(pid, &mut status, 0) };
    assert_eq!(r, pid, "waitpid");
    if libc::WIFEXITED(status) {
        Some(libc::WEXITSTATUS(status))
    } else {
        None
    }
}

/// Try to create one thread; return whether it succeeded. Uses `Builder::spawn`
/// (returns `Err` instead of panicking) so a `clone`-denied profile is observed,
/// not aborted.
fn thread_spawn_works() -> bool {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| 7u8)
        .map(|h| h.join().ok() == Some(7))
        .unwrap_or(false)
}

#[test]
#[ignore = "fork-based live seccomp test; run with --ignored --test-threads=1"]
fn threaded_profile_permits_thread_creation() {
    let code = fork_exit_code(|| {
        if arlen_ai_sandbox::apply_sandbox_threaded().is_err() {
            return 2; // sandbox setup failed
        }
        if thread_spawn_works() {
            0
        } else {
            1
        }
    });
    assert_eq!(code, Some(0), "the threaded profile must permit thread creation");
}

#[test]
#[ignore = "fork-based live seccomp test; run with --ignored --test-threads=1"]
fn tight_profile_denies_thread_creation() {
    let code = fork_exit_code(|| {
        if arlen_ai_sandbox::apply_sandbox().is_err() {
            return 2;
        }
        // The tight profile blocks `clone`, so the spawn must FAIL.
        if thread_spawn_works() {
            1 // a thread spawned under the tight profile - the isolation leaked
        } else {
            0
        }
    });
    assert_eq!(code, Some(0), "the tight profile must keep thread creation denied");
}
