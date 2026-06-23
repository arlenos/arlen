//! The sandboxed HEIC/AVIF decoder worker (`quickview-plan.md`).
//!
//! Runs inside the bwrap sandbox with the wider per-decoder seccomp profile (the
//! threaded C codecs need `clone`); no write, no network. Reads the HEIF from
//! stdin (bounded), decodes it with the system libheif, and writes the validated
//! RGBA raster frame to stdout. A decode failure exits non-zero with the reason
//! on stderr and writes no frame.

use std::io::{Read, Write};

/// The largest encoded HEIF the worker reads from stdin.
const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;

fn main() {
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().lock().take(MAX_INPUT_BYTES).read_to_end(&mut input) {
        eprintln!("arlen-decode-heic: read stdin: {e}");
        std::process::exit(1);
    }
    match arlen_decode_heic::decode_heic(&input) {
        Ok(decoded) => {
            if let Err(e) = std::io::stdout().lock().write_all(&decoded.encode()) {
                eprintln!("arlen-decode-heic: write stdout: {e}");
                std::process::exit(1);
            }
        }
        Err(reason) => {
            eprintln!("arlen-decode-heic: {reason}");
            std::process::exit(2);
        }
    }
}
