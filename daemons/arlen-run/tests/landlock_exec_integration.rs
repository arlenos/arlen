//! On-kernel proof of the `--landlock-exec` in-sandbox fence mode: the real
//! `arlen-run` binary, run as `--landlock-exec <dir> -- <program>`, must install a
//! Landlock fence that PERMITS a write under the granted dir and DENIES one
//! outside it, then exec the program under that fence. Metal-only (needs Landlock
//! on 5.13 or newer); the wiring that has bwrap invoke this mode is a separate
//! slice, so this exercises the mechanism directly against the host filesystem.

#[cfg(target_os = "linux")]
#[test]
#[ignore = "needs Linux >=5.13 with Landlock enabled"]
fn the_fence_permits_a_granted_write_and_denies_one_outside() {
    use std::process::Command;

    let bin = env!("CARGO_BIN_EXE_arlen-run");
    let dir = tempfile::tempdir().expect("temp writable dir");
    let inside = dir.path().join("inside");

    // The app: write INSIDE the granted dir (must succeed), write to /dev/null (a
    // standard writable pseudo-file the fence must still permit, else real apps
    // break), then attempt a write to a NON-tmpfs host path outside the grant (must
    // be denied by the fence). The exit code carries the verdict. (/tmp is a
    // standard writable grant, so the out-of-grant probe targets a real host path.)
    let script = format!(
        "echo ok > '{inside}' || exit 10; \
         echo n > /dev/null || exit 11; \
         if echo x > /etc/arlen-landlock-exec-should-not-exist 2>/dev/null; then exit 20; fi; \
         exit 0",
        inside = inside.display(),
    );
    let status = Command::new(bin)
        .arg("--landlock-exec")
        .arg(dir.path())
        .arg("--")
        .arg("/bin/sh")
        .arg("-c")
        .arg(&script)
        .status()
        .expect("spawn arlen-run --landlock-exec");

    assert_eq!(
        status.code(),
        Some(0),
        "the granted + standard writes must succeed and the out-of-grant write must \
         be denied (10 = granted app-dir write failed, 11 = /dev/null write denied \
         = standard-writable regression, 20 = out-of-grant host write ALLOWED = \
         fence leak)"
    );
    assert!(inside.exists(), "the granted write landed on disk");
}

/// The socket half of the fence, on the kernel: with `--dial <one socket>` the
/// program connects to that socket and is refused on the one bound beside it,
/// and with no `--dial` it reaches both. Needs Landlock ABI 9 (`RESOLVE_UNIX`).
#[cfg(target_os = "linux")]
#[test]
#[ignore = "needs Linux with Landlock ABI 9"]
fn the_fence_opens_a_listed_socket_and_refuses_the_one_beside_it() {
    use std::os::unix::net::UnixListener;
    use std::process::Command;

    let bin = env!("CARGO_BIN_EXE_arlen-run");
    // Not under `/tmp`: the fence makes that writable for every app, and a
    // writable dir carries the right to dial what is bound in it.
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("a runtime dir");
    let granted = dir.path().join("granted.sock");
    let other = dir.path().join("other.sock");
    let _a = UnixListener::bind(&granted).expect("the granted socket is bound");
    let _b = UnixListener::bind(&other).expect("the other socket is bound");

    // Exit 0 when the connect succeeds, 7 when the kernel refuses it.
    let connect = "import socket,sys\n\
                   s=socket.socket(socket.AF_UNIX)\n\
                   try:\n s.connect(sys.argv[1])\n\
                   except PermissionError:\n sys.exit(7)\n";
    let run = |dial: Option<&std::path::Path>, target: &std::path::Path| {
        let mut cmd = Command::new(bin);
        cmd.arg("--landlock-exec");
        if let Some(socket) = dial {
            cmd.arg("--dial").arg(socket);
        }
        cmd.args(["--", "/usr/bin/python3", "-c", connect]).arg(target);
        cmd.status().expect("spawn arlen-run --landlock-exec").code()
    };

    assert_eq!(run(Some(&granted), &granted), Some(0), "the listed socket is reachable");
    assert_eq!(run(Some(&granted), &other), Some(7), "the socket beside it is refused");
    assert_eq!(run(None, &other), Some(0), "without --dial connecting is not fenced");
}
