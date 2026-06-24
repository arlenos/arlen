//! The sandboxed audio probe worker (`quickview-plan.md`).
//!
//! Runs inside the bwrap sandbox (no write/network, seccomp). Reads the encoded
//! audio file from stdin (bounded), probes it with Symphonia, and writes the
//! validated `AudioInfo` frame to stdout. A probe failure exits non-zero with the
//! reason on stderr and writes no frame, so the player treats it as an
//! unsupported/corrupt file.

use std::io::{Read, Write};

/// The largest encoded audio file the worker reads from stdin.
const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;

fn main() {
    // Self-confine before reading any untrusted bytes (read-only /usr, no write).
    if let Err(e) = arlen_viewers_core::sandbox::apply_decoder_landlock() {
        eprintln!("arlen-decode-audio: landlock: {e}");
        std::process::exit(1);
    }
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().lock().take(MAX_INPUT_BYTES).read_to_end(&mut input) {
        eprintln!("arlen-decode-audio: read stdin: {e}");
        std::process::exit(1);
    }
    match arlen_decode_audio::probe_audio(&input) {
        Ok(info) => {
            if let Err(e) = std::io::stdout().lock().write_all(&info.encode()) {
                eprintln!("arlen-decode-audio: write stdout: {e}");
                std::process::exit(1);
            }
        }
        Err(reason) => {
            eprintln!("arlen-decode-audio: {reason}");
            std::process::exit(2);
        }
    }
}
