//! Document-parsing isolation for the Arlen AI layer (Foundation §8.4).
//!
//! Untrusted documents — a PDF, a web page, a file the user asked the AI
//! to summarise — are parsed in a **separate, sandboxed subprocess**
//! before any of their text reaches a prompt. The subprocess has no
//! network access and no filesystem access; it reads the document bytes
//! from stdin and writes only the extracted, stripped plain text to
//! stdout. A parser exploited by a crafted document therefore cannot
//! reach the network or the graph, and only inert text crosses the
//! sandbox boundary.
//!
//! This crate is both:
//! - the **library** the AI layer calls: [`parse_document`] spawns the
//!   sandbox worker, feeds it bytes, and returns the extracted text (or
//!   an error — callers fail closed and pass no text on);
//! - the **worker binary** (`arlen-doc-sandbox`): it calls
//!   [`apply_sandbox`] to lock itself down, then reads stdin, runs
//!   [`extract_text`], and writes stdout.
//!
//! The sandbox uses no_new_privs + a Landlock ruleset that grants no
//! filesystem access + a seccomp filter that blocks socket creation, so
//! it needs no privileges (unprivileged Landlock and seccomp, Linux
//! ≥5.13). Already-open fds (stdin/stdout) keep working; opening any new
//! path or socket is denied.

#![warn(missing_docs)]

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use thiserror::Error;

#[cfg(target_os = "linux")]
mod sandbox;
#[cfg(target_os = "linux")]
pub use sandbox::apply_sandbox;

/// The largest document the worker will accept, and the largest text it
/// will return. Bounds memory against a hostile or pathological input.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;

/// How long the worker is allowed to run before the parent kills it.
const PARSE_TIMEOUT: Duration = Duration::from_secs(20);

/// A document-isolation failure. Every variant means no trustworthy
/// text was produced, so callers must treat it as fail-closed and pass
/// nothing on to the model.
#[derive(Debug, Error)]
pub enum SandboxError {
    /// The sandbox could not be installed (Landlock/seccomp/prctl
    /// failed). The worker exits rather than parse unsandboxed.
    #[error("sandbox setup failed: {0}")]
    Setup(String),
    /// The worker could not be spawned, or I/O to it failed.
    #[error("worker process error: {0}")]
    Process(String),
    /// The worker exited non-zero (it failed to sandbox or to parse).
    #[error("worker failed: {0}")]
    WorkerFailed(String),
    /// The worker exceeded the time budget and was killed.
    #[error("worker timed out")]
    Timeout,
    /// The input or the produced text exceeded [`MAX_BYTES`].
    #[error("document too large")]
    TooLarge,
    /// An untrusted image could not be decoded, or the thumbnail could not be
    /// encoded. Fail-closed: no thumbnail is produced.
    #[error("image decode/encode failed: {0}")]
    Decode(String),
}

/// Extract inert plain text from raw document bytes.
///
/// This is the transformation that runs **inside** the sandbox. The
/// first version handles UTF-8 / plain text: it decodes lossily and
/// strips control characters that could carry hidden instructions
/// (ANSI escape sequences, C0/C1 controls), keeping only ordinary
/// printable text plus newlines and tabs. Richer extractors (PDF, HTML,
/// office formats) plug in here behind the same boundary, so the risky
/// parse always runs sandboxed.
pub fn extract_text(bytes: &[u8]) -> Result<String, SandboxError> {
    if bytes.len() > MAX_BYTES {
        return Err(SandboxError::TooLarge);
    }
    let decoded = String::from_utf8_lossy(bytes);
    // Normalise CR / CRLF to LF up front so the loop only has to keep
    // LF and tab; this avoids turning a CRLF into a double newline.
    let normalized = decoded.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(normalized.len());
    for ch in normalized.chars() {
        if ch == '\n' || ch == '\t' {
            out.push(ch);
        } else if ch.is_control() {
            // Drop every other control char (C0/C1, ANSI escape
            // introducer): no readable content, can hide instructions
            // or terminal escapes.
            continue;
        } else if is_invisible_or_format(ch) {
            // Invisible, format, and bidirectional-override characters:
            // text that is hidden, reordered, or smuggled past a reader.
            continue;
        } else {
            out.push(ch);
        }
    }
    Ok(out)
}

/// Whether `ch` is an invisible, format, or bidirectional-control
/// character that should never survive into the extracted text. Covers
/// the practically-dangerous slice of Unicode's Default_Ignorable set:
/// soft hyphen, combining grapheme joiner, zero-width and bidi marks,
/// the word joiner and invisible math operators, deprecated format
/// controls, variation selectors, and the tag characters.
fn is_invisible_or_format(ch: char) -> bool {
    matches!(ch,
        '\u{00AD}'                  // soft hyphen
        | '\u{034F}'                // combining grapheme joiner
        | '\u{061C}'                // arabic letter mark
        | '\u{115F}'..='\u{1160}'   // hangul choseong/jungseong fillers
        | '\u{17B4}'..='\u{17B5}'   // khmer inherent vowels
        | '\u{180B}'..='\u{180F}'   // mongolian variation/separator
        | '\u{200B}'..='\u{200F}'   // zero-width + directional marks
        | '\u{202A}'..='\u{202E}'   // bidi embedding/override
        | '\u{2060}'..='\u{2064}'   // word joiner + invisible operators
        | '\u{2066}'..='\u{206F}'   // bidi isolates + deprecated format
        | '\u{3164}'                // hangul filler
        | '\u{FE00}'..='\u{FE0F}'   // variation selectors
        | '\u{FEFF}'                // zero-width no-break space / BOM
        | '\u{FFA0}'                // halfwidth hangul filler
        | '\u{1BCA0}'..='\u{1BCA3}' // shorthand format controls
        | '\u{1D173}'..='\u{1D17A}' // musical beam/slur format controls
        | '\u{E0000}'..='\u{E0FFF}' // tags + supplementary variation selectors
    )
}

/// The longest-side dimension (px) of a generated thumbnail. Thumbnails are
/// cached at this size; the UI scales down for smaller displays.
#[cfg(feature = "thumbnail")]
pub const THUMBNAIL_MAX_DIM: u32 = 256;

/// Largest decoded image dimension (px per side) accepted before thumbnailing.
/// Caps a decompression bomb's pixel buffer: 16384 covers any real photo while
/// refusing an absurd canvas. A source larger than this is a decode error.
#[cfg(feature = "thumbnail")]
const MAX_DECODE_DIM: u32 = 16_384;

/// The decoder's allocation budget (bytes). Bounds the working memory a hostile
/// image can force the decoder to allocate. 256 MiB comfortably decodes a
/// `MAX_DECODE_DIM`-class image while refusing a bomb that would OOM the worker.
#[cfg(feature = "thumbnail")]
const MAX_DECODE_ALLOC: u64 = 256 * 1024 * 1024;

/// Decode an untrusted image and produce a downscaled PNG thumbnail.
///
/// This is the transformation that runs **inside** the sandbox. Image decoders
/// are a memory-unsafe attack surface (the same class as the document parser),
/// so the decode happens in the locked-down worker and only the re-encoded
/// thumbnail bytes leave. The image is decoded, scaled to fit within `max_dim`
/// on its longest side (aspect preserved, never upscaled), and re-encoded as
/// PNG (which also strips the source file's metadata). A decode failure is
/// fail-closed: no thumbnail is produced.
#[cfg(feature = "thumbnail")]
pub fn generate_thumbnail(image_bytes: &[u8], max_dim: u32) -> Result<Vec<u8>, SandboxError> {
    if image_bytes.len() > MAX_BYTES {
        return Err(SandboxError::TooLarge);
    }
    // Bound the DECODE, not just the input. A small highly-compressed file can
    // decode to a gigantic pixel buffer (a decompression bomb); the worker has
    // no memory cgroup, so cap the decoded dimensions and the decoder's
    // allocation budget. A bomb is refused as a decode error, fail-closed.
    let mut reader = image::ImageReader::new(std::io::Cursor::new(image_bytes))
        .with_guessed_format()
        .map_err(|e| SandboxError::Decode(format!("format: {e}")))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODE_DIM);
    limits.max_image_height = Some(MAX_DECODE_DIM);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|e| SandboxError::Decode(e.to_string()))?;
    let dim = max_dim.max(1);
    // Only downscale. Re-encoding an already-small image still sanitises it (it
    // drops the source metadata and format quirks), but it is never enlarged.
    let thumb = if decoded.width() <= dim && decoded.height() <= dim {
        decoded
    } else {
        decoded.thumbnail(dim, dim)
    };
    let mut out = Vec::new();
    thumb
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| SandboxError::Decode(e.to_string()))?;
    if out.len() > MAX_BYTES {
        return Err(SandboxError::TooLarge);
    }
    Ok(out)
}

/// Parse a document by running the sandbox worker as a subprocess.
///
/// `sandbox_bin` is the path to the `arlen-doc-sandbox` binary. The
/// `document` bytes are written to the worker's stdin; its stdout (the
/// extracted text) is returned. The worker is killed if it runs past
/// the time budget, and both input and output are bounded by
/// [`MAX_BYTES`]. Any failure is a [`SandboxError`]; the caller passes
/// no text to the model on error.
pub fn parse_document(sandbox_bin: &Path, document: &[u8]) -> Result<String, SandboxError> {
    let output = run_worker(sandbox_bin, document)?;
    // The worker already emitted valid UTF-8 from extract_text.
    String::from_utf8(output).map_err(|e| SandboxError::Process(format!("non-utf8 output: {e}")))
}

/// Spawn the sandbox worker `sandbox_bin`, feed it `input` on stdin, and return
/// its stdout (capped at [`MAX_BYTES`]).
///
/// Shared by [`parse_document`] and the thumbnail worker. It owns the
/// deadlock-safe threaded piping (stdin and stdout each on their own thread so a
/// large input cannot deadlock against a full output pipe), the
/// [`PARSE_TIMEOUT`] budget with a kill, and the exit-status and output-size
/// checks. The caller interprets the returned bytes (text, an image, ...).
fn run_worker(sandbox_bin: &Path, input: &[u8]) -> Result<Vec<u8>, SandboxError> {
    if input.len() > MAX_BYTES {
        return Err(SandboxError::TooLarge);
    }

    let mut child = Command::new(sandbox_bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Discard the worker's stderr rather than pipe it: nothing reads it, so a
        // piped stderr would let a worker that writes a lot to it block on a full
        // pipe (reaped only on the timeout, not promptly on exit). Discarding also
        // avoids propagating a future content-bearing parser's stderr.
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| SandboxError::Process(format!("spawn: {e}")))?;

    // Feed stdin from a thread so a large input cannot deadlock
    // against a full stdout pipe.
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| SandboxError::Process("no stdin".to_string()))?;
    let owned = input.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&owned);
        // Drop closes stdin so the worker sees EOF.
    });

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| SandboxError::Process("no stdout".to_string()))?;

    // Read stdout (capped) on a thread, so the wait can time out even if
    // the worker wedges mid-write.
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout
            .by_ref()
            .take((MAX_BYTES as u64) + 1)
            .read_to_end(&mut buf);
        buf
    });

    // Poll for exit up to the timeout, then kill.
    let deadline = std::time::Instant::now() + PARSE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = writer.join();
                    let _ = reader.join();
                    return Err(SandboxError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(SandboxError::Process(format!("wait: {e}"))),
        }
    };

    let _ = writer.join();
    let output = reader
        .join()
        .map_err(|_| SandboxError::Process("stdout reader panicked".to_string()))?;

    if !status.success() {
        return Err(SandboxError::WorkerFailed(format!("exit status {status}")));
    }
    if output.len() > MAX_BYTES {
        return Err(SandboxError::TooLarge);
    }
    Ok(output)
}

/// Generate a thumbnail by running the sandboxed image worker as a subprocess.
///
/// `sandbox_bin` is the path to the `arlen-thumbnail-sandbox` binary. The
/// untrusted `image` bytes are written to the worker's stdin; the worker decodes
/// and downscales them inside its Landlock + seccomp lockdown and writes back the
/// PNG thumbnail, which is returned. The worker is killed if it runs past the
/// time budget, and both input and output are bounded by [`MAX_BYTES`]. Any
/// failure is a [`SandboxError`]; the caller produces no thumbnail on error.
#[cfg(feature = "thumbnail")]
pub fn thumbnail(sandbox_bin: &Path, image: &[u8]) -> Result<Vec<u8>, SandboxError> {
    run_worker(sandbox_bin, image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_keeps_plain_text_and_newlines() {
        let t = extract_text(b"Hello world.\nSecond line.\tTabbed.").unwrap();
        assert_eq!(t, "Hello world.\nSecond line.\tTabbed.");
    }

    #[test]
    fn extract_strips_ansi_and_control_chars() {
        // ESC[31m ... ESC[0m and a bell should not survive.
        let raw = b"normal \x1b[31mred\x1b[0m text\x07end";
        let t = extract_text(raw).unwrap();
        assert!(!t.contains('\x1b'));
        assert!(!t.contains('\x07'));
        assert!(t.contains("normal"));
        assert!(t.contains("text"));
        assert!(t.contains("end"));
    }

    #[test]
    fn extract_strips_zero_width_and_bidi() {
        let raw = "vis\u{200B}ible\u{202E}reversed\u{FEFF}".as_bytes();
        let t = extract_text(raw).unwrap();
        assert!(!t.contains('\u{200B}'));
        assert!(!t.contains('\u{202E}'));
        assert!(!t.contains('\u{FEFF}'));
        assert!(t.contains("visible"));
    }

    #[test]
    fn extract_strips_the_wider_default_ignorable_set() {
        // soft hyphen, word joiner, variation selector, a tag char, CGJ.
        let raw =
            "a\u{00AD}b\u{2060}c\u{FE0F}d\u{E0041}e\u{034F}f".as_bytes();
        let t = extract_text(raw).unwrap();
        for c in ['\u{00AD}', '\u{2060}', '\u{FE0F}', '\u{E0041}', '\u{034F}'] {
            assert!(!t.contains(c), "must strip U+{:04X}", c as u32);
        }
        assert_eq!(t, "abcdef");
    }

    #[test]
    fn extract_normalises_crlf() {
        let t = extract_text(b"a\r\nb\rc").unwrap();
        assert_eq!(t, "a\nb\nc");
    }

    #[test]
    fn extract_rejects_oversize_input() {
        let big = vec![b'x'; MAX_BYTES + 1];
        assert!(matches!(extract_text(&big), Err(SandboxError::TooLarge)));
    }
}

#[cfg(all(test, feature = "thumbnail"))]
mod thumbnail_tests {
    use super::*;
    use image::GenericImageView;

    /// A synthetic PNG of `w`x`h` to feed the thumbnailer.
    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, _| image::Rgb([(x % 256) as u8, 128, 200]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn downscales_a_large_image_within_the_box_preserving_aspect() {
        let thumb = generate_thumbnail(&png_bytes(800, 400), 256).unwrap();
        let (w, h) = image::load_from_memory(&thumb).unwrap().dimensions();
        // 2:1 aspect preserved, long side hits the 256 box.
        assert_eq!((w, h), (256, 128));
    }

    #[test]
    fn does_not_upscale_a_small_image() {
        let thumb = generate_thumbnail(&png_bytes(64, 48), 256).unwrap();
        let (w, h) = image::load_from_memory(&thumb).unwrap().dimensions();
        assert_eq!((w, h), (64, 48));
    }

    #[test]
    fn refuses_non_image_bytes_fail_closed() {
        let err = generate_thumbnail(b"this is plainly not an image", 256).unwrap_err();
        assert!(matches!(err, SandboxError::Decode(_)));
    }

    #[test]
    fn refuses_oversize_input() {
        let big = vec![0u8; MAX_BYTES + 1];
        assert!(matches!(generate_thumbnail(&big, 256), Err(SandboxError::TooLarge)));
    }
}
