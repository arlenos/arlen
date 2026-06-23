//! The sandboxed JPEG XL decoder worker (`quickview-plan.md`).
//!
//! Runs inside the bwrap sandbox (no write/network, seccomp). Reads the JXL from
//! stdin (bounded), decodes it with jxl-oxide, and writes the validated RGBA
//! raster frame to stdout. A decode failure exits non-zero with the reason on
//! stderr and writes no frame.

use std::io::{Read, Write};

/// The largest encoded JXL the worker reads from stdin.
const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;

fn main() {
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().lock().take(MAX_INPUT_BYTES).read_to_end(&mut input) {
        eprintln!("arlen-decode-jxl: read stdin: {e}");
        std::process::exit(1);
    }
    match arlen_decode_jxl::decode_jxl(&input) {
        Ok(decoded) => {
            if let Err(e) = std::io::stdout().lock().write_all(&decoded.encode()) {
                eprintln!("arlen-decode-jxl: write stdout: {e}");
                std::process::exit(1);
            }
        }
        Err(reason) => {
            eprintln!("arlen-decode-jxl: {reason}");
            std::process::exit(2);
        }
    }
}
