// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Run an untrusted parser in a box, and read back one bounded frame.
//!
//! The caller holds the file read capability; the worker does NOT. The caller
//! reads the file and pipes the bytes into a worker running under bwrap - **no
//! network, no writable filesystem, no read access to the user's files** - which
//! writes back only its frame. A worker that crashes or is compromised cannot
//! reach the network, write anything, read `$HOME` or other apps' data, OOM the
//! caller (the read is bounded) or hang it (the watchdog kills past the timeout).
//! It CAN read the world-readable `/usr` it is given for its own dynamic
//! libraries, which is a bounded info surface rather than the user's data.
//!
//! WHY IT LIVES IN THE SDK. This was `apps/viewers/host` while the image and
//! audio decoders were its only callers, and every detail in it was paid for
//! there: the fail-closed `close_range`, the seccomp memfd that must survive
//! exec, the separate writer thread so a large input cannot deadlock on a pipe
//! buffer, and the timed-out flag recorded BEFORE the signal because "it took
//! too long" and "it crashed" arrive as the same exit status. The PDF page
//! renderer needs all of that and is not a viewer, so the choice was to copy it
//! or to move it. A second copy of a watchdog is a second thing to get right.
//!
//! The confinement and argv are pure and unit-tested; the real bwrap spawn is
//! the on-kernel test.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use arlen_confiner::{app_runtime_profile, Bind, Confinement, ConfinerError, NetworkPolicy};

pub mod seccomp;
pub use seccomp::WorkerProfile;

/// The largest frame the caller will read back from a worker, BEFORE parsing.
///
/// A compromised worker could write frame-looking bytes for ever and OOM the
/// caller on the read itself; this caps the read. Sized for the largest
/// legitimate output any current worker produces: a 16-megapixel RGBA raster
/// plus its header.
pub const MAX_OUTPUT_BYTES: u64 = 12 + 16 * 1024 * 1024 * 4;

/// The wall-clock budget for one worker run.
pub const DECODE_TIMEOUT: Duration = Duration::from_secs(20);

/// The decoder sandbox: read-only `/usr` (the worker's dynamic libs) + the
/// merged-usr loader symlinks `/lib64`/`/lib` so the ELF interpreter resolves +
/// read-only the worker's own directory (its binary). NO network, NO input file
/// bind (the worker reads its input from stdin, never the filesystem, so it
/// cannot open any other file). A tmpfs `/tmp` is provided by the app-runtime
/// base. The per-decoder seccomp filter is layered on top in
/// [`run_confined_worker`].
pub fn decoder_confinement(worker_dir: &str) -> Result<Confinement, ConfinerError> {
    let dir = require_abs(worker_dir)?;
    let skeleton =
        app_runtime_profile(Path::new("/usr"), &[], &[], BTreeMap::new(), NetworkPolicy::None)?;
    let mut binds = vec![Bind::ReadOnly(dir.clone(), dir)];
    // The worker is dynamically linked, so its ELF interpreter lives at
    // /lib64/ld-linux-*.so. On a merged-usr system /lib64 and /lib are symlinks
    // to usr/lib; bwrap resolves the source symlink and binds usr/lib there, so
    // the loader resolves inside the otherwise-/usr-only view. Bound only when
    // present so a pure-/usr host does not fail the spawn.
    for loader in ["/lib64", "/lib"] {
        if Path::new(loader).exists() {
            binds.push(Bind::ReadOnly(loader.into(), loader.into()));
        }
    }
    Ok(skeleton.complete(binds, vec![]))
}

/// The full confined spawn argv: the bwrap flags then `-- <worker_path>`. Pure.
pub fn decode_worker_argv(confinement: &Confinement, worker_path: &str) -> Vec<String> {
    let mut argv = confinement.bwrap_args();
    argv.push("--".to_string());
    argv.push(worker_path.to_string());
    argv
}

fn require_abs(path: &str) -> Result<String, ConfinerError> {
    if Path::new(path).is_absolute() {
        Ok(path.to_string())
    } else {
        Err(ConfinerError::RelativePath(path.to_string()))
    }
}

/// Run the worker `worker_bin` (under `worker_dir`) in the sandbox, pipe `input`
/// to its stdin, and return its raw stdout frame bytes. The caller decodes the
/// frame (a raster for an image worker, an AudioInfo for audio). Hardened
/// against a COMPROMISED worker (the design's threat model):
/// - the stdout read is bounded at [`MAX_OUTPUT_BYTES`], so a worker that writes
///   forever cannot OOM the host;
/// - a watchdog SIGKILLs the worker past [`DECODE_TIMEOUT`], so a hung worker
///   cannot wedge the caller. Because the confinement sets `--die-with-parent`,
///   killing bwrap also tears down the inner decoder, which closes the pipes -
///   so the kill unblocks both the stdout read AND the stdin writer thread;
/// - input is written on a separate thread while stdout is drained, so a large
///   input + output cannot deadlock on the pipe buffers; a non-zero exit errs.
pub fn run_confined_worker(
    worker_dir: &str,
    worker_bin: &str,
    profile: WorkerProfile,
    args: &[String],
    input: &[u8],
) -> Result<Vec<u8>, String> {
    let worker_path = format!("{}/{worker_bin}", worker_dir.trim_end_matches('/'));
    let confinement = decoder_confinement(worker_dir).map_err(|e| e.to_string())?;
    let mut argv = decode_worker_argv(&confinement, &worker_path);
    // Whatever the worker itself needs told, after its own path.
    argv.extend(args.iter().cloned());

    // Install the per-decoder seccomp allowlist: compile it, hand the cBPF to
    // bwrap over a memfd via `--seccomp <fd>` (inserted before the `--` program
    // separator), and bwrap installs it on the worker just before exec. The
    // wider profile reaches only the HEIC/AVIF decoder; the pure-Rust workers
    // get the tight one.
    let bpf = seccomp::decoder_filter_bytes(profile).map_err(|e| e.to_string())?;
    let seccomp_fd = make_seccomp_memfd(&bpf).map_err(|e| format!("seccomp memfd: {e}"))?;
    let sep = argv.iter().position(|a| a == "--").unwrap_or(argv.len());
    argv.splice(sep..sep, ["--seccomp".to_string(), seccomp_fd.to_string()]);

    let mut command = Command::new("bwrap");
    command
        .args(&argv)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    // The seccomp memfd must survive exec into bwrap (it reads `--seccomp <fd>`).
    // `close_range` marks every other inherited fd CLOEXEC so no host fd leaks
    // into the worker, then the seccomp fd's CLOEXEC bit is re-cleared so it
    // alone stays open. stdin/stdout are dup'd to 0/1 (below 3), so close_range
    // spares them. async-signal-safe: raw libc calls and an allocation-free
    // errno read.
    unsafe {
        command.pre_exec(move || {
            // Fail closed: if the inherited fds cannot be marked CLOEXEC, refuse to
            // exec into the worker rather than risk leaking a host fd into it. On a
            // supported kernel this errors only when close_range is unavailable,
            // exactly the case where the isolation cannot be guaranteed.
            if libc::close_range(3, libc::c_uint::MAX, libc::CLOSE_RANGE_CLOEXEC as libc::c_int) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let flags = libc::fcntl(seccomp_fd, libc::F_GETFD);
            if flags >= 0 {
                libc::fcntl(seccomp_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
            }
            Ok(())
        });
    }
    let spawned = command.spawn();
    // The child inherited the memfd at fork; the parent's copy is done with.
    // SAFETY: closing the parent's own fd; the child keeps its inherited copy.
    unsafe { libc::close(seccomp_fd) };
    let mut child = spawned.map_err(|e| format!("spawn bwrap: {e}"))?;
    let pid = child.id() as libc::pid_t;

    let mut stdin = child.stdin.take().ok_or("no child stdin")?;
    let owned = input.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&owned);
        // Dropping stdin closes it, signalling EOF to the worker.
    });

    // A watchdog kills the worker on the timeout. The main thread signals `done`
    // once the read completes; a `recv_timeout` that expires first means the
    // worker is hung/slow, so SIGKILL it (pid not yet reaped - `wait()` is below
    // - so there is no pid-reuse window).
    let (done_tx, done_rx) = mpsc::channel::<()>();
    // Whether the watchdog is what ended this worker, recorded BEFORE the signal.
    //
    // A timeout and a signal are two facts, and only one of them survived here: a
    // killed worker cannot exit 0, so the status check below caught it, but it
    // reported `worker exited with signal: 9` - the same sentence a worker that
    // segfaulted on a malformed file produces. "It took too long" and "it crashed"
    // are different findings about a decoder, and the one this drops is the one
    // that says the timeout is too tight or the file is pathological.
    let timed_out = Arc::new(AtomicBool::new(false));
    let watchdog_flag = Arc::clone(&timed_out);
    let watchdog = std::thread::spawn(move || {
        if done_rx.recv_timeout(DECODE_TIMEOUT).is_err() {
            watchdog_flag.store(true, Ordering::SeqCst);
            // SAFETY: SIGKILL by pid; benign (ESRCH) if the worker already exited.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    });

    // Read at most MAX_OUTPUT_BYTES + 1, so an over-cap worker is detected rather
    // than silently truncated into a plausible-but-wrong frame.
    let mut out = Vec::new();
    let read_result = child
        .stdout
        .take()
        .ok_or("no child stdout")
        .and_then(|so| {
            so.take(MAX_OUTPUT_BYTES + 1).read_to_end(&mut out).map_err(|_| "read stdout").map(|_| ())
        });
    let _ = done_tx.send(()); // cancel the watchdog if the read finished in time
    let _ = writer.join();
    let status = child.wait().map_err(|e| format!("wait: {e}"))?;
    let _ = watchdog.join();
    read_result?;
    if out.len() as u64 > MAX_OUTPUT_BYTES {
        return Err("worker output exceeded the frame bound".to_string());
    }
    // Asked before the status, because the status cannot tell these apart: the
    // watchdog's SIGKILL and a worker that died on its own both arrive as a
    // signal, and only this flag knows which one this was.
    if timed_out.load(Ordering::SeqCst) {
        return Err(format!(
            "worker timed out after {}s and was killed",
            DECODE_TIMEOUT.as_secs()
        ));
    }
    if !status.success() {
        return Err(format!("worker exited with {status}"));
    }
    Ok(out)
}


/// Create an anonymous in-memory file holding the compiled seccomp cBPF for
/// `bwrap --seccomp <fd>`. The memfd is created without `MFD_CLOEXEC` and the
/// child's `pre_exec` re-clears the CLOEXEC bit so the fd survives the exec into
/// bwrap; the parent closes its own copy after spawn.
fn make_seccomp_memfd(bpf: &[u8]) -> std::io::Result<libc::c_int> {
    use std::ffi::CString;
    let name = CString::new("arlen-decoder-seccomp").expect("static name has no nul");
    // SAFETY: a plain memfd_create with a valid C string and no flags.
    let fd = unsafe { libc::memfd_create(name.as_ptr(), 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut written = 0usize;
    while written < bpf.len() {
        // SAFETY: writing `bpf[written..]` to the owned memfd.
        let n = unsafe {
            libc::write(fd, bpf[written..].as_ptr() as *const libc::c_void, bpf.len() - written)
        };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e);
        }
        written += n as usize;
    }
    // Rewind so bwrap reads the filter from the start.
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
    use arlen_confiner::ConfinerError;

    #[test]
    fn the_decoder_sandbox_has_no_network_no_file_binds_and_a_readonly_worker() {
        let conf = decoder_confinement("/opt/arlen/viewers").unwrap();
        let argv = decode_worker_argv(&conf, "/opt/arlen/viewers/arlen-decode-image");
        assert!(argv.contains(&"--unshare-net".to_string()), "no network");
        // The only bind beyond the base /usr is the read-only worker dir; there
        // is no --bind (read-write) of anything (the worker gets no FS write).
        assert!(!argv.iter().any(|a| a == "--bind"), "no read-write bind");
        let ro: Vec<_> = argv.windows(2).filter(|w| w[0] == "--ro-bind").map(|w| w[1].clone()).collect();
        assert!(ro.contains(&"/usr".to_string()), "/usr is read-only");
        assert!(ro.contains(&"/opt/arlen/viewers".to_string()), "the worker dir is read-only");
        // The program tail runs the worker.
        let sep = argv.iter().position(|s| s == "--").unwrap();
        assert_eq!(&argv[sep + 1..], &["/opt/arlen/viewers/arlen-decode-image".to_string()]);
    }

    #[test]
    fn a_relative_worker_dir_is_rejected() {
        assert!(matches!(decoder_confinement("opt/viewers"), Err(ConfinerError::RelativePath(_))));
    }
}
