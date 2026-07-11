//! The confined command runner: build the sandbox argv from
//! [`arlen_confiner::command_profile`], spawn it, capture bounded stdout+stderr,
//! enforce a wall-clock timeout, and return a structured outcome.
//!
//! No shell is ever involved: the command and its arguments are passed as an argv
//! vector straight to `bwrap -- <command> <args...>`, so there is no shell-injection
//! surface (a value with spaces / `;` / `$()` is one argument, never re-parsed). The
//! command is ALWAYS Confirm-gated upstream; this is the confined + output-captured
//! mechanism, never a decision to run.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use arlen_confiner::{command_profile, ConfinerError, NetworkPolicy};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

/// Max bytes captured per stream (stdout, stderr). Past this the stream is marked
/// truncated and further output is drained-and-discarded, so a runaway command can
/// neither OOM the daemon nor block on a full pipe.
pub const MAX_STREAM_BYTES: usize = 1024 * 1024;

/// The default wall-clock budget for a confined command.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// The confinement launcher the runner execs.
const BWRAP: &str = "bwrap";

/// A confined command to run.
#[derive(Debug, Clone)]
pub struct RunRequest {
    /// The program to execute (resolved on the sandbox `PATH`).
    pub command: String,
    /// Its arguments, passed as a vector - never joined into a shell string.
    pub args: Vec<String>,
    /// Host directories exposed READ-ONLY inside the sandbox.
    pub read_only_roots: Vec<PathBuf>,
    /// A writable scratch dir (the sandbox cwd and `HOME`).
    pub workdir: PathBuf,
    /// Network policy ([`NetworkPolicy::None`] gives no exfiltration).
    pub network: NetworkPolicy,
    /// Wall-clock budget; on expiry the sandbox is killed.
    pub timeout: Duration,
}

/// The captured result of a confined run. A command that RAN and failed is an
/// `Ok(RunOutcome)` with a non-zero `exit_code`; only a runner failure (spawn / io
/// / confinement) is an `Err`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RunOutcome {
    /// The exit code, or `None` if the command was killed (a signal or the timeout).
    pub exit_code: Option<i32>,
    /// Captured stdout (UTF-8-lossy, bounded by [`MAX_STREAM_BYTES`]).
    pub stdout: String,
    /// Captured stderr (bounded).
    pub stderr: String,
    /// `stdout` was truncated at the cap.
    pub stdout_truncated: bool,
    /// `stderr` was truncated at the cap.
    pub stderr_truncated: bool,
    /// The command exceeded its wall-clock budget and was killed.
    pub timed_out: bool,
}

/// A runner failure (distinct from a command that ran and exited non-zero).
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The confinement could not be built (a bad path / reserved env).
    #[error("confinement error: {0}")]
    Confine(#[from] ConfinerError),
    /// The sandbox launcher could not be spawned.
    #[error("could not spawn the sandbox: {0}")]
    Spawn(String),
    /// An I/O error reading the command's output or reaping it.
    #[error("io error running the command: {0}")]
    Io(String),
    /// The command name was empty.
    #[error("the command name is empty")]
    EmptyCommand,
}

/// Build the full `bwrap` argv for a confined command: the `command_profile` flags,
/// then `-- <command> <args...>`. Pure and headless-testable (no spawn); the spawn
/// side needs an unprivileged user namespace, so its test is metal-gated.
pub fn confined_argv(req: &RunRequest) -> Result<Vec<String>, RunError> {
    if req.command.is_empty() {
        return Err(RunError::EmptyCommand);
    }
    let confinement = command_profile(
        &req.read_only_roots,
        &req.workdir,
        req.network.clone(),
        std::collections::BTreeMap::new(),
    )?;
    let mut argv = confinement.bwrap_args();
    argv.push("--".into());
    argv.push(req.command.clone());
    argv.extend(req.args.iter().cloned());
    Ok(argv)
}

/// Read from `r` up to `cap` bytes, returning the bytes and whether it was
/// truncated. Past `cap` the reader keeps draining to EOF but discards, so memory
/// is bounded AND a truncating-but-terminating command still finishes (it does not
/// block on a full pipe waiting for us to read).
async fn read_capped<R: AsyncRead + Unpin>(mut r: R, cap: usize) -> (Vec<u8>, bool) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        match r.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                if !truncated {
                    if buf.len() + n > cap {
                        let room = cap - buf.len();
                        buf.extend_from_slice(&chunk[..room]);
                        truncated = true;
                    } else {
                        buf.extend_from_slice(&chunk[..n]);
                    }
                }
                // Once truncated, keep reading (draining) but discard.
            }
            Err(_) => break,
        }
    }
    (buf, truncated)
}

/// Run the confined command: spawn `bwrap` with the argv, drain bounded stdout +
/// stderr concurrently, enforce the wall-clock timeout (killing the sandbox on
/// expiry - `bwrap --die-with-parent` tears the namespace down), and return the
/// structured outcome. Fail-closed on a spawn / confinement error. `stdin` is
/// `/dev/null` (a confined command reads no interactive input).
pub async fn run_confined(req: &RunRequest) -> Result<RunOutcome, RunError> {
    let argv = confined_argv(req)?;
    let mut child = Command::new(BWRAP)
        .args(&argv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| RunError::Spawn(e.to_string()))?;

    let stdout = child.stdout.take().ok_or_else(|| RunError::Io("no stdout pipe".into()))?;
    let stderr = child.stderr.take().ok_or_else(|| RunError::Io("no stderr pipe".into()))?;
    // Drain both pipes concurrently (owned handles moved into the futures).
    let drain = async move {
        tokio::join!(read_capped(stdout, MAX_STREAM_BYTES), read_capped(stderr, MAX_STREAM_BYTES))
    };

    // Race draining + reaping against the timeout; `child` is borrowed (not moved)
    // so the timeout arm can still kill it.
    let run = async {
        let (out, err) = drain.await;
        let status = child.wait().await;
        (out, err, status)
    };
    match tokio::time::timeout(req.timeout, run).await {
        Ok(((sout, sout_trunc), (serr, serr_trunc), status)) => {
            let status = status.map_err(|e| RunError::Io(e.to_string()))?;
            Ok(RunOutcome {
                exit_code: status.code(),
                stdout: String::from_utf8_lossy(&sout).into_owned(),
                stderr: String::from_utf8_lossy(&serr).into_owned(),
                stdout_truncated: sout_trunc,
                stderr_truncated: serr_trunc,
                timed_out: false,
            })
        }
        Err(_elapsed) => {
            // Kill the sandbox and reap it; the partial output is discarded (a
            // timed-out command's output is not trustworthy).
            let _ = child.start_kill();
            let _ = child.wait().await;
            Ok(RunOutcome {
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
                timed_out: true,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(command: &str, args: &[&str]) -> RunRequest {
        RunRequest {
            command: command.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            read_only_roots: vec![PathBuf::from("/usr"), PathBuf::from("/bin")],
            workdir: PathBuf::from("/tmp/run-core-test"),
            network: NetworkPolicy::None,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    #[test]
    fn confined_argv_wraps_the_command_after_the_sandbox_flags() {
        let argv = confined_argv(&req("ls", &["-la", "/work"])).unwrap();
        let joined = argv.join(" ");
        // The sandbox flags come first, no-network, read-only roots, then the split.
        assert!(joined.contains("--unshare-net"), "no network: {joined}");
        assert!(joined.contains("--ro-bind /usr /usr"), "usr read-only");
        assert!(joined.contains("--bind /tmp/run-core-test /tmp/work"), "scratch writable");
        // The program + args follow `--`, verbatim (no shell).
        let sep = argv.iter().position(|a| a == "--").expect("the -- separator");
        assert_eq!(&argv[sep + 1..], &["ls".to_string(), "-la".to_string(), "/work".to_string()]);
    }

    #[test]
    fn a_command_arg_with_shell_metacharacters_is_one_argument() {
        // No shell is involved, so `; rm -rf ~` is a single literal argument, never
        // a second command.
        let argv = confined_argv(&req("echo", &["hi; rm -rf ~"])).unwrap();
        assert_eq!(argv.last().unwrap(), "hi; rm -rf ~");
    }

    #[test]
    fn an_empty_command_is_rejected() {
        assert!(matches!(confined_argv(&req("", &[])), Err(RunError::EmptyCommand)));
    }

    #[tokio::test]
    async fn read_capped_truncates_past_the_cap_and_drains() {
        let data = [b'x'; 100];
        let (buf, trunc) = read_capped(&data[..], 40).await;
        assert_eq!(buf.len(), 40, "capped at 40");
        assert!(trunc, "truncation flagged");
        // Under the cap: the full content, no truncation.
        let (buf2, trunc2) = read_capped(&b"hello"[..], 40).await;
        assert_eq!(buf2, b"hello");
        assert!(!trunc2);
    }

    // The real confined spawn needs an unprivileged user namespace (bwrap), so it is
    // metal-gated. Run with `--ignored` on a host that permits userns.
    #[tokio::test]
    #[ignore = "needs bwrap + an unprivileged user namespace"]
    async fn a_confined_echo_captures_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let request = RunRequest {
            command: "echo".to_string(),
            args: vec!["hello-from-sandbox".to_string()],
            read_only_roots: vec![PathBuf::from("/")],
            workdir: dir.path().to_path_buf(),
            network: NetworkPolicy::None,
            timeout: DEFAULT_TIMEOUT,
        };
        let out = run_confined(&request).await.unwrap();
        assert_eq!(out.exit_code, Some(0));
        assert!(out.stdout.contains("hello-from-sandbox"), "stdout: {:?}", out.stdout);
        assert!(!out.timed_out);
    }

    #[tokio::test]
    #[ignore = "needs bwrap + an unprivileged user namespace"]
    async fn a_slow_command_times_out_and_is_killed() {
        let dir = tempfile::tempdir().unwrap();
        let request = RunRequest {
            command: "sleep".to_string(),
            args: vec!["30".to_string()],
            read_only_roots: vec![PathBuf::from("/")],
            workdir: dir.path().to_path_buf(),
            network: NetworkPolicy::None,
            timeout: Duration::from_millis(300),
        };
        let out = run_confined(&request).await.unwrap();
        assert!(out.timed_out, "the slow command timed out");
        assert_eq!(out.exit_code, None);
    }
}
