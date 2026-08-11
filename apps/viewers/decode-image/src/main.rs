//! The sandboxed image decoder worker (`quickview-plan.md`).
//!
//! Runs inside the bwrap sandbox (no write/network, seccomp). Reads the encoded
//! image from stdin (bounded), decodes it with `image-rs`, and writes the
//! validated RGBA raster frame to stdout. A decode failure exits non-zero with
//! the reason on stderr and writes no frame, so the viewer treats it as an
//! unsupported/corrupt file rather than rendering garbage.

use std::io::{Read, Write};

/// The largest encoded image the worker will read from stdin: a coarse bound on
/// the input.
///
/// The decoded-RASTER bound is a separate limit and it is enforced HERE as well
/// as in the frame: `decode_image` reads the declared dimensions and refuses
/// anything over `MAX_PIXELS` BEFORE decoding, so a 69-byte PNG claiming
/// 20000x20000 exits non-zero without allocating (measured 12 Aug, under a
/// 700 MB address-space cap, to be sure the refusal came before the allocation
/// rather than after a lucky one). The frame check on the viewer side is the
/// second line, against a hostile worker rather than a hostile file.
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
