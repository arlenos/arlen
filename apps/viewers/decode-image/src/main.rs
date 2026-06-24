//! The sandboxed image decoder worker (`quickview-plan.md`).
//!
//! Runs inside the bwrap sandbox (no write/network, seccomp). Reads the encoded
//! image from stdin (bounded), decodes it with `image-rs`, and writes the
//! validated RGBA raster frame to stdout. A decode failure exits non-zero with
//! the reason on stderr and writes no frame, so the viewer treats it as an
//! unsupported/corrupt file rather than rendering garbage.

use std::io::{Read, Write};

/// The largest encoded image the worker will read from stdin: a coarse bound on
/// the input (the decoded-raster bound is enforced separately in the frame).
const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;

fn main() {
    // Self-confine before reading any untrusted bytes (read-only /usr, no write).
    if let Err(e) = arlen_viewers_core::sandbox::apply_decoder_landlock() {
        eprintln!("arlen-decode-image: landlock: {e}");
        std::process::exit(1);
    }
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().lock().take(MAX_INPUT_BYTES).read_to_end(&mut input) {
        eprintln!("arlen-decode-image: read stdin: {e}");
        std::process::exit(1);
    }
    match arlen_decode_image::decode_image(&input) {
        Ok(decoded) => {
            let frame = decoded.encode();
            if let Err(e) = std::io::stdout().lock().write_all(&frame) {
                eprintln!("arlen-decode-image: write stdout: {e}");
                std::process::exit(1);
            }
        }
        Err(reason) => {
            eprintln!("arlen-decode-image: {reason}");
            std::process::exit(2);
        }
    }
}
