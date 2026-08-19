// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The sandboxed PDF page renderer.
//!
//! Reads a document on stdin (bounded), draws one page, writes the RGBA frame on
//! stdout. A failure exits non-zero with the reason on stderr and writes no
//! frame, so the reader treats it as a page it cannot show rather than drawing
//! whatever bytes arrived.
//!
//! The confinement is the viewer's, deliberately: `apply_decoder_landlock` is
//! one reviewed implementation of "read-only /usr, no write anywhere" and a
//! second copy of it here would be a second thing to keep correct. If it ever
//! moves to the sdk both callers move together.

use std::io::{Read, Write};

/// The largest document the worker will read from stdin.
///
/// A coarse bound on the input; the raster bound that actually matters is
/// enforced against the page's own declared size before anything is allocated.
const MAX_INPUT_BYTES: u64 = 512 * 1024 * 1024;

fn main() {
    // Self-confine before reading a single untrusted byte.
    if let Err(e) = arlen_viewers_core::sandbox::apply_decoder_landlock() {
        eprintln!("arlen-pdf-decode-page: landlock: {e}");
        std::process::exit(1);
    }
    let mut args = std::env::args().skip(1);
    let page: usize = match args.next().as_deref().map(str::parse) {
        Some(Ok(n)) => n,
        _ => {
            eprintln!("usage: arlen-pdf-decode-page <page> [scale] < document.pdf");
            std::process::exit(64);
        }
    };
    // A missing scale is 1.0 rather than an error: the common call is "draw this
    // page", and a reader that has not decided its zoom yet still wants a page.
    let scale: f32 = args.next().as_deref().map_or(1.0, |s| s.parse().unwrap_or(1.0));

    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().lock().take(MAX_INPUT_BYTES).read_to_end(&mut input) {
        eprintln!("arlen-pdf-decode-page: read stdin: {e}");
        std::process::exit(1);
    }
    match arlen_pdf_decode_page::render_page(&input, page, scale) {
        Ok(raster) => {
            if let Err(e) = std::io::stdout().lock().write_all(&raster.encode()) {
                eprintln!("arlen-pdf-decode-page: write stdout: {e}");
                std::process::exit(1);
            }
        }
        Err(reason) => {
            eprintln!("arlen-pdf-decode-page: {reason}");
            std::process::exit(2);
        }
    }
}
