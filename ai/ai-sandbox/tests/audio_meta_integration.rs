//! Integration test for the sandboxed audio-metadata worker (`apps/viewers`
//! player surface): the worker parses an untrusted audio container and returns
//! its playback properties + tags end-to-end, through the real Landlock +
//! seccomp subprocess lockdown. This checks that container parsing works under
//! the sandbox (the seccomp allowlist permits what lofty needs), that the
//! structured metadata round-trips as JSON, and that garbage fails closed.

#![cfg(all(target_os = "linux", feature = "music"))]

use std::path::Path;

const BIN: &str = env!("CARGO_BIN_EXE_arlen-audio-meta-sandbox");

/// A 3-byte big-endian length, the FLAC metadata-block size field.
fn be24(v: u32) -> [u8; 3] {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

/// A minimal in-memory FLAC: the `fLaC` magic + a STREAMINFO block declaring
/// 44100 Hz / 2 channels. No audio frames - lofty reads the metadata block
/// alone. Mirrors the unit-test fixture so the worker path is exercised against
/// a real recognisable container.
fn minimal_flac() -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(b"fLaC");
    f.push(0x80); // last block, type 0 (STREAMINFO)
    f.extend_from_slice(&be24(34));
    let mut si = [0u8; 34];
    si[0..2].copy_from_slice(&4096u16.to_be_bytes());
    si[2..4].copy_from_slice(&4096u16.to_be_bytes());
    // sample_rate(20) | channels-1(3) | bps-1(5) | total_samples(36).
    let packed: u64 = (44_100u64 << 44) | (1u64 << 41) | (15u64 << 36);
    si[10..18].copy_from_slice(&packed.to_be_bytes());
    f.extend_from_slice(&si);
    f
}

#[test]
fn reads_audio_metadata_inside_the_sandbox() {
    let meta = arlen_ai_sandbox::audio_metadata(Path::new(BIN), &minimal_flac())
        .expect("the sandboxed worker should parse the container");
    assert_eq!(meta.sample_rate, Some(44_100));
    assert_eq!(meta.channels, Some(2));
    assert!(meta.duration_secs >= 0.0);
}

#[test]
fn metadata_worker_fails_closed_on_non_audio_input() {
    let err = arlen_ai_sandbox::audio_metadata(Path::new(BIN), b"this is plainly not audio")
        .expect_err("a non-audio input must not yield metadata");
    // The worker exits non-zero on the parse failure, surfaced as WorkerFailed.
    assert!(matches!(err, arlen_ai_sandbox::SandboxError::WorkerFailed(_)));
}
