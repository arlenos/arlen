// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The worker's PROCESS contract: what `viewers/host` reads, rather than what the
//! decode function returns.
//!
//! The host spawns this binary and treats an empty stdout as "unsupported or
//! corrupt". So the exit status, the empty stdout on failure and the reason on
//! stderr are load-bearing, and the library tests next door say nothing about
//! them. All four workers were driven by hand on 12 Aug and agreed on this
//! contract; these tests are what keeps the agreement from being a coincidence.
//!
//! **Failure cases only, and deliberately.** A valid input for this format is a
//! real encoded file, which this test cannot synthesise the way the PNG one can -
//! so the happy path stays with the library tests and the round-trip fixtures,
//! and what is pinned here is the part the host depends on when things go wrong.

use std::io::Write;
use std::process::{Command, Stdio};

/// Run the worker with `input` on stdin and return (exit code, stdout, stderr).
fn run_worker(input: &[u8]) -> (Option<i32>, Vec<u8>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_arlen-decode-audio"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the worker binary is built by the test harness");
    child.stdin.take().expect("stdin was piped").write_all(input).ok();
    let out = child.wait_with_output().expect("the worker terminates");
    (out.status.code(), out.stdout, String::from_utf8_lossy(&out.stderr).into_owned())
}

/// The property the host reads as "unsupported or corrupt". A worker that wrote
/// a partial frame before failing would have the viewer render garbage, which is
/// the one outcome every one of these binaries promises never happens.
#[test]
fn garbage_exits_nonzero_and_writes_nothing() {
    let (code, stdout, stderr) = run_worker(&[0x4a; 512]);
    assert_ne!(code, Some(0));
    assert!(stdout.is_empty(), "a failed decode must write no output");
    assert!(!stderr.trim().is_empty(), "and must say why");
}

#[test]
fn empty_input_exits_nonzero_and_writes_nothing() {
    let (code, stdout, _) = run_worker(&[]);
    assert_ne!(code, Some(0));
    assert!(stdout.is_empty());
}
