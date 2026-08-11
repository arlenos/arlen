// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The worker's PROCESS contract, which is the part the host actually relies on.
//!
//! `decode_image` is well tested as a function - it rejects garbage, it round
//! trips a PNG. None of that says what the BINARY does, and the binary is what
//! `viewers/host` spawns: it reads the frame off stdout and treats an empty read
//! as "unsupported or corrupt". So the properties the host leans on are the exit
//! status, an empty stdout on failure, and a reason on stderr - and until now
//! nothing asserted any of them.
//!
//! Driven by hand on 12 Aug against all four workers, which is what prompted
//! writing it down: they agree on this contract today, and agreement nobody
//! checks is a coincidence waiting to be edited.

use std::io::Write;
use std::process::{Command, Stdio};

/// Run the worker with `input` on stdin and return (exit code, stdout, stderr).
fn run_worker(input: &[u8]) -> (Option<i32>, Vec<u8>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_arlen-decode-image"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the worker binary is built by the test harness");
    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(input)
        .ok(); // a worker that exits before reading closes the pipe: not an error here
    let out = child.wait_with_output().expect("the worker terminates");
    (
        out.status.code(),
        out.stdout,
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A minimal valid PNG of `w` by `h`, built here rather than checked in as a
/// fixture so the test carries no binary blob.
fn png(w: u32, h: u32) -> Vec<u8> {
    png_declaring(w, h, w, h)
}

/// A PNG whose HEADER declares `dw` by `dh` while its pixel data is `w` by `h`.
///
/// The bomb case needs exactly this: a header claiming an enormous raster with
/// almost no bytes behind it. Patching the dimensions into a finished file does
/// not work - IHDR carries a CRC, so a patched header fails to parse, the
/// dimension read never succeeds, and the size guard is never reached. The test
/// then passes because the DECODE failed instead, which is the wrong reason and
/// exactly what the first version of this file did.
fn png_declaring(w: u32, h: u32, dw: u32, dh: u32) -> Vec<u8> {
    fn crc(bytes: &[u8]) -> u32 {
        let mut table = [0u32; 256];
        for (i, e) in table.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *e = c;
        }
        let mut c = 0xFFFF_FFFFu32;
        for &b in bytes {
            c = table[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8);
        }
        c ^ 0xFFFF_FFFF
    }
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = (data.len() as u32).to_be_bytes().to_vec();
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        v.extend_from_slice(&body);
        v.extend_from_slice(&crc(&body).to_be_bytes());
        v
    }
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&dw.to_be_bytes());
    ihdr.extend_from_slice(&dh.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB
    let mut raw = Vec::new();
    for _ in 0..h {
        raw.push(0u8); // filter: none
        for _ in 0..w {
            raw.extend_from_slice(&[255, 0, 0]);
        }
    }
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend_from_slice(&chunk(b"IHDR", &ihdr));
    png.extend_from_slice(&chunk(b"IDAT", &deflate_stored(&raw)));
    png.extend_from_slice(&chunk(b"IEND", b""));
    png
}

/// A zlib stream of stored (uncompressed) deflate blocks: enough for a decoder
/// to read, and it keeps this test free of a compression dependency.
fn deflate_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    for (i, block) in data.chunks(65535).enumerate() {
        let last = u8::from((i + 1) * 65535 >= data.len());
        out.push(last);
        out.extend_from_slice(&(block.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        out.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

#[test]
fn a_valid_image_decodes_to_a_frame_on_stdout() {
    let (code, stdout, stderr) = run_worker(&png(2, 2));
    assert_eq!(code, Some(0), "stderr: {stderr}");
    // 12-byte header plus the RGBA raster.
    assert_eq!(stdout.len(), 12 + 2 * 2 * 4, "frame length");
}

/// The property the host reads as "unsupported or corrupt": nothing on stdout.
/// A worker that wrote a partial frame before failing would have the host render
/// garbage, which is the one outcome its own header promises never happens.
#[test]
fn garbage_exits_nonzero_and_writes_no_frame() {
    let (code, stdout, stderr) = run_worker(&[0x4a; 512]);
    assert_ne!(code, Some(0));
    assert!(stdout.is_empty(), "a failed decode must write no frame");
    assert!(!stderr.trim().is_empty(), "and must say why");
}

#[test]
fn empty_input_exits_nonzero_and_writes_no_frame() {
    let (code, stdout, _) = run_worker(&[]);
    assert_ne!(code, Some(0));
    assert!(stdout.is_empty());
}

/// The decompression-bomb guard, at the process boundary: a tiny file declaring
/// an enormous raster is refused on its DECLARED dimensions, before the decode.
/// Measured under a 700 MB address-space cap on 12 Aug to confirm the refusal
/// comes before the allocation rather than after a lucky one.
#[test]
fn a_declared_bomb_is_refused_without_a_frame() {
    // 20000x20000 RGBA would be 1.6 GB decoded; one pixel of data behind it.
    let (code, stdout, stderr) = run_worker(&png_declaring(1, 1, 20_000, 20_000));
    assert_ne!(code, Some(0));
    assert!(stdout.is_empty(), "no frame for a bomb");
    // Specifically the SIZE guard, not merely "some failure". This assertion
    // first read `contains("too large") || contains("decode")`, and with the
    // guard deleted it still passed - the decode failed anyway, because a header
    // claiming 20000x20000 has nothing like enough pixel data behind it. An
    // assertion with an OR in it will happily pass for the wrong reason, and the
    // reason is the whole point here: refused on the DECLARED dimensions, before
    // any allocation.
    assert!(
        stderr.contains("too large"),
        "must be refused by the size guard, not by a later decode error: {stderr}"
    );
}
