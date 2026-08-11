//! Stamping the confined child's identity into the launcher-stamped identity
//! broker (the Tier-1 half of `stamped-identity-plan.md`).
//!
//! `arlen-run` holds the authenticated `--app-id` (resolved from the root
//! `IdentityRegistry` before launch), so it is the ONE process the broker trusts
//! to `Register` a stamp. The problem is WHICH pid to register: bwrap runs the app
//! under `--unshare-pid`, so the app has its OWN pid namespace and its host-visible
//! pid (the one a daemon reads via `SO_PEERPIDFD` at `accept`) differs from bwrap's
//! pid. bwrap reports that host pid via `--info-fd`: it writes one JSON document
//! `{ "child-pid": <host pid>, "mnt-namespace": ..., "pid-namespace": ... }`.
//!
//! The stamp handshake ([`StampHandshake`], wired into `spawn::spawn_and_wait`) is:
//!   1. Make two pipes: an info pipe and a block pipe.
//!   2. Add [`stamp_bwrap_args`] to the bwrap argv: `--info-fd <w_info>` (bwrap
//!      writes `child-pid` early, after the clone, before the app execs) and
//!      `--block-fd <r_block>` (bwrap waits for a byte on it before exec'ing the
//!      app).
//!   3. Spawn bwrap; in the parent, read the info document from the pipe and
//!      [`parse_child_pid`] it.
//!   4. `pidfd_open(child_pid)` -> register it at the broker with the app id
//!      (BEST-EFFORT: a broker outage or a register failure must NOT abort the
//!      launch; the app then simply resolves via /proc as `LegacyProc`, never a
//!      fabricated identity).
//!   5. Write one byte to the block pipe so bwrap unblocks and execs the app - so
//!      the stamp is recorded BEFORE the app can make its first daemon connection.
//!
//! `--info-fd` (not `--json-status-fd`) because json-status writes a second
//! document after the app exits, which would `SIGPIPE` bwrap once the launcher
//! closes the read end; info-fd writes the child-pid document once and never again.

use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;

/// How long the launcher waits for bwrap to write the sandboxed child's pid before
/// giving up and launching unstamped. bwrap writes `child-pid` right after the
/// clone, then blocks on `--block-fd`, so this only fires if bwrap wedged; on a
/// timeout the app simply resolves via /proc (best-effort, never a hang).
const STAMP_READ_TIMEOUT_MS: libc::c_int = 5000;

/// The launcher's side of the bwrap identity-stamp handshake: two pipes. The
/// child-inherited ends ([`Self::child_keep_fds`]) go to bwrap via
/// [`Self::bwrap_args`]; the launcher-side ends read the child pid and release
/// bwrap in [`Self::complete`]. Both ends of both pipes are `O_CLOEXEC`, so nothing
/// leaks except the two the child pre-exec explicitly keeps.
pub struct StampHandshake {
    status_r: OwnedFd,
    status_w: OwnedFd,
    block_r: OwnedFd,
    block_w: OwnedFd,
}

impl StampHandshake {
    /// Make the two pipes (info + block).
    pub fn new() -> io::Result<Self> {
        let (status_r, status_w) = make_pipe()?;
        let (block_r, block_w) = make_pipe()?;
        Ok(Self {
            status_r,
            status_w,
            block_r,
            block_w,
        })
    }

    /// The fds bwrap must inherit past the child's `close_range`: the
    /// `--info-fd` write end and the `--block-fd` read end. Add these to the
    /// `child_pre_exec` keep-set so their `CLOEXEC` is cleared for the exec.
    pub fn child_keep_fds(&self) -> [RawFd; 2] {
        [self.status_w.as_raw_fd(), self.block_r.as_raw_fd()]
    }

    /// The bwrap flags turning on the handshake, referencing the inherited fds.
    pub fn bwrap_args(&self) -> Vec<String> {
        stamp_bwrap_args(self.status_w.as_raw_fd(), self.block_r.as_raw_fd())
    }

    /// Parent side, AFTER the spawn: drop the child-inherited ends, learn the
    /// child's host pid from bwrap (bounded wait), register it at the broker
    /// (best-effort), then release bwrap so it execs the app. A failed stamp NEVER
    /// wedges the launch - the release is unconditional; the app then resolves via
    /// /proc as `LegacyProc`.
    pub fn complete(self, app_id: &str, broker_socket: &Path) {
        let Self {
            status_r,
            status_w,
            block_r,
            block_w,
        } = self;
        // The child holds its own inherited copies; drop the parent's so the status
        // read is not blocked by our own writer and we do not pin bwrap's block end.
        drop(status_w);
        drop(block_r);
        // `File` gives the OwnedFds Read/Write; the pidfd write end stays raw.
        let mut block = std::fs::File::from(block_w);
        if wait_readable(status_r.as_raw_fd(), STAMP_READ_TIMEOUT_MS) {
            let mut status = std::fs::File::from(status_r);
            complete_over(&mut status, &mut block, app_id, broker_socket);
        } else {
            // bwrap did not report a child pid in time: release it unstamped.
            let _ = block.write_all(&[1u8]);
        }
    }
}

/// The post-spawn core, generic over reader/writer so it is unit-testable without
/// a real pipe or bwrap: read the child pid from `status`, register it at the
/// broker (best-effort), then write the unblock byte to `block` so bwrap execs the
/// app. The release is UNCONDITIONAL - a failed stamp must not withhold the launch.
fn complete_over(status: &mut impl Read, block: &mut impl Write, app_id: &str, broker_socket: &Path) {
    if let Some(pid) = read_child_pid(status) {
        register_child(pid, app_id, broker_socket);
    }
    let _ = block.write_all(&[1u8]);
}

/// Read bwrap's info document and extract the host child pid. bwrap
/// writes the small `child-pid` document in one write, so a single read captures
/// it; a read error or an unparseable document yields `None` (launch unstamped).
fn read_child_pid(status: &mut impl Read) -> Option<u32> {
    let mut buf = [0u8; 4096];
    let n = status.read(&mut buf).ok()?;
    parse_child_pid(&String::from_utf8_lossy(&buf[..n]))
}

/// Open a pidfd for the sandboxed child and register its identity stamp at the
/// broker. BEST-EFFORT: a vanished child, an unreachable/unauthenticated broker,
/// or a refusal all just leave the app to resolve via /proc - never fatal, never a
/// panic, so a broken broker cannot break app launching.
fn register_child(pid: u32, app_id: &str, broker_socket: &Path) {
    let Some(pidfd) = arlen_permissions::peer_pidfd::pidfd_open(pid) else {
        return;
    };
    if let Err(e) =
        arlen_permissions::identity_wire::register_identity(broker_socket, pidfd.as_fd(), app_id)
    {
        eprintln!(
            "arlen-run: warning: identity stamp not registered (app resolves via /proc): {e}"
        );
    }
}

/// An `O_CLOEXEC` pipe pair `(read, write)`.
fn make_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` is a valid 2-element array; pipe2 fills it or returns -1.
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the kernel handed us two fresh owned fds.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Poll `fd` for readability up to `timeout_ms`. `true` iff data is ready (so a
/// wedged bwrap cannot hang the launcher's wait for the child pid).
fn wait_readable(fd: RawFd, timeout_ms: libc::c_int) -> bool {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd for the duration of the call.
    let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    rc > 0 && (pfd.revents & libc::POLLIN) != 0
}

/// The bwrap flags that turn on the stamp handshake, for the given inherited fds:
/// `--info-fd <status_fd>` (bwrap writes the container info, incl. `child-pid`, to
/// it ONCE) and `--block-fd <block_fd>` (bwrap blocks reading it until the launcher
/// has registered the stamp, then execs the app). Added among the confinement's
/// own bwrap args.
///
/// `--info-fd` (not `--json-status-fd`): json-status writes a SECOND document
/// (`{"exit-code":N}`) after the app exits, so a launcher that closes the read end
/// once it has the child-pid would make bwrap take `SIGPIPE` on that late write.
/// `--info-fd` writes the single child-pid document and never writes again, so the
/// read end can be dropped right after parsing.
pub fn stamp_bwrap_args(status_fd: RawFd, block_fd: RawFd) -> Vec<String> {
    vec![
        "--info-fd".to_string(),
        status_fd.to_string(),
        "--block-fd".to_string(),
        block_fd.to_string(),
    ]
}

/// Parse the host `child-pid` from bwrap's `--info-fd` output.
///
/// bwrap writes one or more JSON documents to the status fd; the FIRST carries
/// `"child-pid": <host pid>` (a later one carries `"exit-code"`). This scans for
/// the first `"child-pid"` key and reads the integer after its colon - a minimal
/// hand parse (no serde dep for one field), tolerant of surrounding whitespace and
/// the other keys in the document. Returns `None` if the key is absent, has no
/// integer, or the pid is `0` (never a real child pid; a bug guard so the caller
/// cannot `pidfd_open(0)`).
pub fn parse_child_pid(json_status: &str) -> Option<u32> {
    const KEY: &str = "\"child-pid\"";
    let after = &json_status[json_status.find(KEY)? + KEY.len()..];
    let after = after.trim_start().strip_prefix(':')?.trim_start();
    let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    match digits.parse::<u32>() {
        Ok(0) | Err(_) => None,
        Ok(pid) => Some(pid),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real bwrap 0.11 status stream: two documents, `child-pid` in the first.
    #[test]
    fn parses_child_pid_from_the_real_status_stream() {
        let stream = "{ \"child-pid\": 479882, \"mnt-namespace\": 4026534051, \"pid-namespace\": 4026534052 }\n{ \"exit-code\": 0 }\n";
        assert_eq!(parse_child_pid(stream), Some(479882));
    }

    /// Only the exit-code document (no child-pid) -> None, never a guessed pid.
    #[test]
    fn returns_none_without_a_child_pid() {
        assert_eq!(parse_child_pid("{ \"exit-code\": 0 }"), None);
        assert_eq!(parse_child_pid(""), None);
    }

    /// A child-pid key with no integer (truncated write) -> None.
    #[test]
    fn returns_none_on_a_malformed_child_pid() {
        assert_eq!(parse_child_pid("{ \"child-pid\": "), None);
        assert_eq!(parse_child_pid("{ \"child-pid\": abc }"), None);
    }

    /// A child-pid of 0 is refused (never a real pid; guards against pidfd_open(0)).
    #[test]
    fn refuses_a_zero_child_pid() {
        assert_eq!(parse_child_pid("{ \"child-pid\": 0 }"), None);
    }

    /// The post-spawn core reads the child pid, registers it at the broker (a real
    /// Register with the child pidfd over SCM_RIGHTS), and unconditionally releases
    /// bwrap. Driven with an in-memory status doc (our own live pid, so pidfd_open
    /// succeeds) + an in-process test broker; no real bwrap needed.
    #[test]
    fn complete_over_registers_the_child_and_releases_bwrap() {
        use arlen_permissions::fd_passing::{recv_fd_msg, MAX_FD_MSG};
        use arlen_permissions::identity_wire::{write_response, IdentityResponse};
        use std::os::unix::net::UnixListener;

        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("id.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let srv = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let (bytes, fd) = recv_fd_msg(&conn, MAX_FD_MSG).unwrap();
            // A Register naming our app, with the child pidfd attached.
            assert!(fd.is_some(), "the child pidfd must arrive over SCM_RIGHTS");
            let body = String::from_utf8_lossy(&bytes);
            assert!(body.contains("Register"), "must be a Register: {body}");
            assert!(body.contains("com.example.app"), "must name the app: {body}");
            write_response(&mut conn, &IdentityResponse::Registered).unwrap();
        });

        // A status document carrying our own live pid so pidfd_open + register work.
        let status_json = format!(
            "{{ \"child-pid\": {}, \"pid-namespace\": 1 }}\n",
            std::process::id()
        );
        let mut status = std::io::Cursor::new(status_json.into_bytes());
        let mut block: Vec<u8> = Vec::new();
        complete_over(&mut status, &mut block, "com.example.app", &sock);

        assert_eq!(block, vec![1u8], "bwrap must be released with the unblock byte");
        srv.join().unwrap();
    }

    /// Even when the broker is UNREACHABLE (no listener), the launch is released:
    /// a failed stamp never withholds the unblock byte, so the app still runs.
    #[test]
    fn complete_over_releases_bwrap_even_when_the_stamp_fails() {
        let status_json = format!("{{ \"child-pid\": {} }}", std::process::id());
        let mut status = std::io::Cursor::new(status_json.into_bytes());
        let mut block: Vec<u8> = Vec::new();
        complete_over(
            &mut status,
            &mut block,
            "com.example.app",
            std::path::Path::new("/nonexistent/arlen/id.sock"),
        );
        assert_eq!(block, vec![1u8], "a failed stamp still releases the launch");
    }

    /// End-to-end against REAL bwrap: spawn a trivial confined process under
    /// --unshare-pid with the stamp handshake wired, and assert bwrap's reported
    /// sandboxed-child host pid round-trips through the pipe and lands as a real
    /// Register (with the child pidfd over SCM_RIGHTS) at an in-process broker. This
    /// is the one proof of the fd-inheritance + block handshake that unit tests of
    /// the pure pieces cannot give. `#[ignore]`d: needs bwrap + unprivileged userns.
    /// Run: `cargo test -p arlen-run --  --ignored real_bwrap`.
    #[test]
    #[ignore = "spawns real bwrap; needs bwrap + unprivileged user namespaces"]
    fn real_bwrap_child_pid_round_trips_and_registers() {
        use arlen_permissions::fd_passing::{recv_fd_msg, MAX_FD_MSG};
        use arlen_permissions::identity_wire::{write_response, IdentityResponse};
        use std::os::unix::net::UnixListener;
        use std::os::unix::process::CommandExt;
        use std::process::Command;

        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("id.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let srv = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let (bytes, fd) = recv_fd_msg(&conn, MAX_FD_MSG).unwrap();
            let ok = fd.is_some() && String::from_utf8_lossy(&bytes).contains("com.example.confined");
            write_response(&mut conn, &IdentityResponse::Registered).unwrap();
            ok
        });

        let handshake = StampHandshake::new().unwrap();
        let mut argv = handshake.bwrap_args();
        argv.extend(
            ["--ro-bind", "/", "/", "--unshare-pid", "--", "/bin/true"]
                .iter()
                .map(|s| s.to_string()),
        );
        let keep = handshake.child_keep_fds();
        let mut cmd = Command::new("bwrap");
        cmd.args(&argv);
        // SAFETY: post-fork child, single-threaded; only clears CLOEXEC on the two
        // stamp fds so bwrap inherits them (the whole point of the handshake).
        unsafe {
            cmd.pre_exec(move || {
                for &fd in &keep {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let mut child = cmd.spawn().expect("bwrap spawns");
        // Read the child pid, register it, unblock bwrap.
        handshake.complete("com.example.confined", &sock);
        let status = child.wait().expect("bwrap waits");
        assert!(status.success(), "the confined /bin/true exits 0");
        assert!(srv.join().unwrap(), "the broker received a Register with the child pidfd");
    }

    /// The stamp args carry the two flags with the fds rendered as decimals.
    #[test]
    fn stamp_args_carry_the_two_bwrap_flags() {
        assert_eq!(
            stamp_bwrap_args(7, 9),
            vec![
                "--info-fd".to_string(),
                "7".to_string(),
                "--block-fd".to_string(),
                "9".to_string(),
            ]
        );
    }
}
