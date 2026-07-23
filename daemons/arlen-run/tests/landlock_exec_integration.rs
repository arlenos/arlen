//! On-kernel proof of the `--landlock-exec` in-sandbox fence mode: the real
//! `arlen-run` binary, run as `--landlock-exec <dir> -- <program>`, must install a
//! Landlock fence that PERMITS a write under the granted dir and DENIES one
//! outside it, then exec the program under that fence. Metal-only (needs Landlock
//! >= 5.13); the wiring that has bwrap invoke this mode is a separate slice, so
//! this exercises the mechanism directly against the host filesystem.

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
