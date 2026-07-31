//! The real end-to-end `run_command` exec: spawn actual commands through
//! `run_confined` and check the confinement from inside it.
//!
//! `#[ignore]`d because it needs a working `bwrap` and unprivileged user
//! namespaces, which the CI container does not have. Run it with
//! `cargo test --test confined_exec -- --ignored`.
//!
//! These exist because the seccomp filter was signed off as "needs a real
//! end-to-end exec run to verify" and never got one. Reading the allowlist is
//! how it came to be missing the POSIX timers while its comment said "timers":
//! the list looks thorough either way, and only a real command notices.

use std::path::PathBuf;
use std::time::Duration;

use arlen_confiner::NetworkPolicy;
use arlen_terminal_run_mcp::run::{run_confined, RunRequest};

/// A request running `command args...` with `/usr` readable and a fresh workdir.
fn req(dir: &std::path::Path, command: &str, args: &[&str]) -> RunRequest {
    RunRequest {
        command: command.to_string(),
        args: args.iter().map(|a| a.to_string()).collect(),
        read_only_roots: vec![PathBuf::from("/usr")],
        workdir: dir.to_path_buf(),
        network: NetworkPolicy::None,
        timeout: Duration::from_secs(20),
    }
}

fn run(command: &str, args: &[&str]) -> (String, String, Option<i32>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let out = rt
        .block_on(run_confined(&req(dir.path(), command, args)))
        .expect("the sandbox runs");
    (out.stdout, out.stderr, out.exit_code)
}

#[test]
#[ignore = "needs bwrap and unprivileged user namespaces"]
fn a_command_runs_and_its_output_comes_back() {
    let (stdout, _stderr, code) = run("/usr/bin/echo", &["confined"]);
    assert_eq!(stdout.trim(), "confined", "stderr was: {_stderr}");
    assert_eq!(code, Some(0));
}

/// The syscall the filter was missing. `timeout(1)` warns
/// `timer_create: Operation not permitted` and degrades when it is denied, so a
/// clean stderr here is the fix demonstrated rather than asserted.
#[test]
#[ignore = "needs bwrap and unprivileged user namespaces"]
fn a_command_using_posix_timers_is_not_refused() {
    let (stdout, stderr, code) = run("/usr/bin/timeout", &["5", "/usr/bin/echo", "timed"]);
    assert_eq!(stdout.trim(), "timed");
    assert!(
        !stderr.contains("timer_create"),
        "the POSIX timers are still refused: {stderr}"
    );
    assert_eq!(code, Some(0));
}

/// The confinement's actual job: a command may not write the host.
#[test]
#[ignore = "needs bwrap and unprivileged user namespaces"]
fn a_write_outside_the_workdir_does_not_reach_the_host() {
    let target = "/tmp/arlen-confined-exec-should-not-exist";
    let _ = std::fs::remove_file(target);
    let (_, _, _) = run("/usr/bin/sh", &["-c", &format!("echo x > {target}")]);
    assert!(
        !std::path::Path::new(target).exists(),
        "the sandbox wrote through to the host"
    );
}

/// No exfiltration under `NetworkPolicy::None`, checked with a raw IP so the
/// result cannot be an artefact of DNS being unavailable.
#[test]
#[ignore = "needs bwrap and unprivileged user namespaces"]
fn a_confined_command_cannot_reach_the_network() {
    let (stdout, _, _) = run(
        "/usr/bin/bash",
        &["-c", "</dev/tcp/1.1.1.1/443 && echo REACHED || echo REFUSED"],
    );
    assert_eq!(stdout.trim(), "REFUSED");
}
