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
//! Unlike the HEIC and JPEG XL workers, this one gets its happy path too: a
//! canonical PCM WAV is 44 bytes of header and some zeros, so the success case is
//! constructible here rather than needing an encoded fixture.

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

/// A minimal canonical PCM WAV: 44-byte header plus `frames` 16-bit samples per
/// channel. Mirrors the builder in this crate's library tests, so the process
/// case and the function case are fed the same shape of input.
fn wav(sample_rate: u32, channels: u16, frames: u32) -> Vec<u8> {
    let bits = 16u16;
    let block_align = channels * bits / 8;
    let byte_rate = sample_rate * u32::from(block_align);
    let data_len = frames * u32::from(block_align);
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVE");
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&channels.to_le_bytes());
    w.extend_from_slice(&sample_rate.to_le_bytes());
    w.extend_from_slice(&byte_rate.to_le_bytes());
    w.extend_from_slice(&block_align.to_le_bytes());
    w.extend_from_slice(&bits.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    w.resize(w.len() + data_len as usize, 0);
    w
}

/// The success half of the contract: a real container in, something on stdout,
/// exit 0. Without this the failure tests alone would be satisfied by a worker
/// that failed on EVERYTHING, which is the loose-pass shape in yet another form.
#[test]
fn a_real_wav_probes_and_writes_to_stdout() {
    // 8000 Hz mono, 8000 frames: exactly one second.
    let (code, stdout, stderr) = run_worker(&wav(8000, 1, 8000));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(!stdout.is_empty(), "a successful probe must answer");
}
