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

/// The longest-side dimension (px) of a decoded image for the single-file
/// viewer (`apps/viewers`, quickview-plan.md). Far larger than a thumbnail so
/// the viewer shows the picture, not a preview, while still bounding the
/// re-encoded PNG: a source larger than this on either side is downscaled to
/// fit, smaller images are returned at their full resolution. 4096 covers a 4K
/// display 1:1 and keeps a typical photo's lossless PNG comfortably under
/// [`MAX_BYTES`].
///
/// CAVEAT: a noisy image near this cap can re-encode to a PNG above
/// [`MAX_BYTES`], which fails closed (the viewer falls back to its icon). A
/// lossy/WebP re-encode for large photos, and a larger cap with zoom-time
/// re-fetch for true detail, are the documented robustness follow-ups; PNG at
/// this cap is the safe-by-construction first cut.
#[cfg(feature = "thumbnail")]
pub const VIEWER_MAX_DIM: u32 = 4096;

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
///
/// MEMORY: [`MAX_DECODE_ALLOC`] bounds the DECODE allocation only. The
/// subsequent downscale and PNG re-encode allocate further (bounded by the
/// already-capped decoded dimensions, not by `image::Limits`), so the worker's
/// peak is somewhat above `MAX_DECODE_ALLOC`. It is still fail-closed (an OOM
/// kills only the worker), but a hard memory ceiling on the worker (a cgroup or
/// rlimit) is the robust follow-up if a constrained host needs a real bound.
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

/// Decode an untrusted image to a sanitised, full-resolution PNG for the viewer.
///
/// This is the transformation that runs **inside** the sandbox for
/// `apps/viewers` (quickview-plan.md). It shares [`generate_thumbnail`]'s
/// memory-unsafe-decode containment and decompression-bomb caps; the only
/// difference is the size bound: it downscales only when a side exceeds
/// [`VIEWER_MAX_DIM`] (a viewer shows the picture, not a 256px preview), and a
/// smaller image is returned at its native resolution. The PNG re-encode also
/// strips the source metadata. A decode failure is fail-closed: no image is
/// produced and the caller shows the file's icon.
#[cfg(feature = "thumbnail")]
pub fn decode_view_image(image_bytes: &[u8]) -> Result<Vec<u8>, SandboxError> {
    // The decode, bomb-cap, downscale-if-over-the-bound and PNG re-encode are
    // exactly the thumbnail path; only the bound differs, so reuse it rather
    // than duplicate the security-relevant decode + cap logic.
    generate_thumbnail(image_bytes, VIEWER_MAX_DIM)
}

/// Extract the first embedded cover-art picture from raw audio-file bytes.
///
/// Runs INSIDE the sandbox worker: parsing an untrusted media container header
/// (lofty) is the same memory-unsafe attack surface as an image decode, so it is
/// locked down the same way. It works purely in memory (a cursor, no filesystem),
/// so it runs under the worker's deny-all Landlock. Returns the embedded picture
/// bytes (the source JPEG/PNG, as stored) for the caller to decode and downscale
/// through [`generate_thumbnail`]; returns `Ok(None)` when the file carries no
/// embedded picture (the common case for music without art, a fall-to-icon
/// outcome, not a failure). A container that cannot be parsed is a
/// [`SandboxError::Decode`], fail-closed.
#[cfg(feature = "music")]
pub fn extract_album_art(audio_bytes: &[u8]) -> Result<Option<Vec<u8>>, SandboxError> {
    use lofty::config::ParseOptions;
    use lofty::file::TaggedFileExt;
    use lofty::probe::Probe;
    if audio_bytes.len() > MAX_BYTES {
        return Err(SandboxError::TooLarge);
    }
    // Skip audio-property parsing: cover art lives in the tags, so reading
    // bitrate/duration would only add work and fail on a file with an unusual or
    // missing audio stream that still carries readable art.
    let tagged = Probe::new(std::io::Cursor::new(audio_bytes))
        .guess_file_type()
        .map_err(|e| SandboxError::Decode(format!("probe: {e}")))?
        .options(ParseOptions::new().read_properties(false))
        .read()
        .map_err(|e| SandboxError::Decode(format!("read tags: {e}")))?;
    // The first picture of the primary tag (cover art is conventionally there),
    // falling back to any tag the container carries.
    let picture = tagged
        .primary_tag()
        .or_else(|| tagged.first_tag())
        .and_then(|tag| tag.pictures().first())
        .map(|pic| pic.data().to_vec());
    Ok(picture)
}

/// An audio file's playback properties and basic tags, for the viewer's
/// minimal player (`apps/viewers`, quickview-plan.md: elapsed/total + tags on
/// demand). All fields are best-effort: a container with no tags returns `None`
/// tag fields; missing properties stay `None`.
#[cfg(feature = "music")]
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AudioMeta {
    /// Total playback duration in seconds (0.0 when the container does not
    /// state it).
    pub duration_secs: f64,
    /// Sample rate in Hz, when the container states it.
    pub sample_rate: Option<u32>,
    /// Channel count, when the container states it.
    pub channels: Option<u8>,
    /// Track title from the primary tag.
    pub title: Option<String>,
    /// Track artist from the primary tag.
    pub artist: Option<String>,
    /// Album name from the primary tag.
    pub album: Option<String>,
}

/// Read an audio file's playback properties and basic tags from raw bytes.
///
/// Runs INSIDE the sandbox worker: parsing an untrusted media container (lofty)
/// is the same memory-unsafe attack surface as [`extract_album_art`], so it is
/// locked down the same way and works purely in memory (a cursor, no
/// filesystem). Unlike the cover-art path it reads audio properties (duration,
/// sample rate, channels) for the player's scrubber, plus the primary tag's
/// title/artist/album. A container that cannot be parsed is a
/// [`SandboxError::Decode`], fail-closed.
#[cfg(feature = "music")]
pub fn read_audio_metadata(audio_bytes: &[u8]) -> Result<AudioMeta, SandboxError> {
    use lofty::config::ParseOptions;
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::probe::Probe;
    use lofty::tag::Accessor;
    if audio_bytes.len() > MAX_BYTES {
        return Err(SandboxError::TooLarge);
    }
    // read_properties stays on (the default) so the player gets duration etc.;
    // the cover-art path disables it because it only needs the picture.
    let tagged = Probe::new(std::io::Cursor::new(audio_bytes))
        .guess_file_type()
        .map_err(|e| SandboxError::Decode(format!("probe: {e}")))?
        .options(ParseOptions::new())
        .read()
        .map_err(|e| SandboxError::Decode(format!("read: {e}")))?;
    let props = tagged.properties();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    Ok(AudioMeta {
        duration_secs: props.duration().as_secs_f64(),
        sample_rate: props.sample_rate(),
        channels: props.channels(),
        title: tag.and_then(|t| t.title().map(|c| c.to_string())),
        artist: tag.and_then(|t| t.artist().map(|c| c.to_string())),
        album: tag.and_then(|t| t.album().map(|c| c.to_string())),
    })
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
        // Exit code 3 is both workers' convention for "could not install the
        // sandbox" (`apply_sandbox` failed). Surface that distinctly so a host
        // where the sandbox never engages is observable, instead of being masked
        // as a routine parse/decode failure (which is exit 5). Both still fail
        // closed; this only separates the two so a degraded sandbox is visible.
        if status.code() == Some(3) {
            return Err(SandboxError::Setup(
                "worker could not install the sandbox".to_string(),
            ));
        }
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
///
/// TRUST CONTRACT: `sandbox_bin` MUST be a trusted, fixed path to the genuine
/// `arlen-thumbnail-sandbox` binary (a system install path, not a relative or
/// attacker-influenceable one). The containment guarantee rests on the spawned
/// process actually being the locked-down worker; the decode runs with whatever
/// sandbox *that* binary installs, so a substituted binary defeats it. Same
/// contract as [`parse_document`].
#[cfg(feature = "thumbnail")]
pub fn thumbnail(sandbox_bin: &Path, image: &[u8]) -> Result<Vec<u8>, SandboxError> {
    run_worker(sandbox_bin, image)
}

/// Decode a full-resolution viewer image by running the sandboxed image-view
/// worker as a subprocess (`apps/viewers`, quickview-plan.md).
///
/// `sandbox_bin` is the path to the `arlen-image-view-sandbox` binary. The
/// untrusted `image` bytes are written to the worker's stdin; the worker decodes
/// them inside its Landlock + seccomp lockdown, downscales only past
/// [`VIEWER_MAX_DIM`], and writes back the sanitised PNG, which is returned. The
/// worker is killed past the time budget and both input and output are bounded
/// by [`MAX_BYTES`]. Any failure is a [`SandboxError`]; the caller shows the
/// file's icon on error.
///
/// TRUST CONTRACT: same as [`thumbnail`] - `sandbox_bin` MUST be the genuine,
/// fixed worker path; the containment rests on the spawned process being the
/// real locked-down worker.
#[cfg(feature = "thumbnail")]
pub fn view_image(sandbox_bin: &Path, image: &[u8]) -> Result<Vec<u8>, SandboxError> {
    run_worker(sandbox_bin, image)
}

/// Read an audio file's playback metadata by running the sandboxed metadata
/// worker as a subprocess (`apps/viewers` player surface).
///
/// `sandbox_bin` is the path to the `arlen-audio-meta-sandbox` binary. The
/// untrusted `audio` bytes are written to the worker's stdin; the worker parses
/// the container inside its Landlock + seccomp lockdown and writes back the
/// [`AudioMeta`] as JSON, which is parsed and returned. The worker is killed
/// past the time budget and both input and output are bounded by [`MAX_BYTES`].
/// Any failure is a [`SandboxError`]; the caller shows no metadata on error.
///
/// TRUST CONTRACT: same as [`thumbnail`] - `sandbox_bin` MUST be the genuine,
/// fixed worker path; the containment rests on the spawned process being the
/// real locked-down worker.
#[cfg(feature = "music")]
pub fn audio_metadata(sandbox_bin: &Path, audio: &[u8]) -> Result<AudioMeta, SandboxError> {
    let out = run_worker(sandbox_bin, audio)?;
    serde_json::from_slice(&out)
        .map_err(|e| SandboxError::Process(format!("decode worker metadata: {e}")))
}

/// Generate a thumbnail of an audio file's embedded cover art by running the
/// sandboxed music worker as a subprocess.
///
/// `sandbox_bin` is the path to the `arlen-music-thumbnail-sandbox` binary. The
/// untrusted `audio` bytes are written to the worker's stdin; the worker extracts
/// the embedded picture and downscales it inside its Landlock + seccomp lockdown,
/// then writes back the PNG thumbnail. Returns `Ok(None)` when the file has no
/// usable embedded art (the worker produces no output), so the caller falls back
/// to a music-type icon. Any failure is a [`SandboxError`]; the caller produces
/// no thumbnail on error.
///
/// TRUST CONTRACT: same as [`thumbnail`] - `sandbox_bin` MUST be the genuine,
/// fixed worker path; the containment rests on the spawned process being the
/// real locked-down worker.
#[cfg(feature = "music")]
pub fn album_art_thumbnail(
    sandbox_bin: &Path,
    audio: &[u8],
) -> Result<Option<Vec<u8>>, SandboxError> {
    let out = run_worker(sandbox_bin, audio)?;
    // Empty output is the worker's "no usable art" signal (a real PNG thumbnail
    // is never empty); map it to a fall-back-to-icon outcome.
    Ok((!out.is_empty()).then_some(out))
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

    #[test]
    fn view_keeps_a_normal_image_at_full_resolution() {
        // Below VIEWER_MAX_DIM on both sides: returned at native resolution,
        // unlike the 256px thumbnail path. This is the viewer's core contract.
        let png = decode_view_image(&png_bytes(2000, 1000)).unwrap();
        let (w, h) = image::load_from_memory(&png).unwrap().dimensions();
        assert_eq!((w, h), (2000, 1000), "viewer must not downscale a sub-cap image");
    }

    #[test]
    fn view_caps_an_oversize_image_to_the_viewer_dim() {
        // Over VIEWER_MAX_DIM on the long side: downscaled to fit, aspect kept.
        let png = decode_view_image(&png_bytes(6000, 3000)).unwrap();
        let (w, h) = image::load_from_memory(&png).unwrap().dimensions();
        assert_eq!((w, h), (VIEWER_MAX_DIM, VIEWER_MAX_DIM / 2));
    }

    #[test]
    fn view_refuses_non_image_bytes_fail_closed() {
        let err = decode_view_image(b"plainly not an image at all").unwrap_err();
        assert!(matches!(err, SandboxError::Decode(_)));
    }

    #[test]
    fn view_refuses_oversize_input() {
        let big = vec![0u8; MAX_BYTES + 1];
        assert!(matches!(decode_view_image(&big), Err(SandboxError::TooLarge)));
    }
}

#[cfg(all(test, feature = "music"))]
mod music_tests {
    use super::*;

    /// A tiny PNG to embed as synthetic cover art.
    fn tiny_png() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([200, 30, 30]));
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        buf
    }

    /// A 3-byte big-endian length, the FLAC metadata-block size field.
    fn be24(v: u32) -> [u8; 3] {
        [(v >> 16) as u8, (v >> 8) as u8, v as u8]
    }

    /// A minimal in-memory FLAC: the `fLaC` magic, a STREAMINFO block, and
    /// optionally a PICTURE block carrying `png` as front cover. Audio frames are
    /// omitted - the reader is driven with `read_properties(false)`, so the
    /// metadata blocks alone parse. This is a real recognisable container (a bare
    /// tag dump is not), so the cover-art path is exercised end to end.
    fn minimal_flac(png: Option<&[u8]>) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(b"fLaC");
        // STREAMINFO (type 0): last only when there is no PICTURE block after it.
        let last_streaminfo = if png.is_some() { 0x00 } else { 0x80 };
        f.push(last_streaminfo);
        f.extend_from_slice(&be24(34));
        let mut si = [0u8; 34];
        si[0..2].copy_from_slice(&4096u16.to_be_bytes()); // min block size
        si[2..4].copy_from_slice(&4096u16.to_be_bytes()); // max block size
        // sample_rate(20) | channels-1(3) | bps-1(5) | total_samples(36).
        let packed: u64 = (44_100u64 << 44) | (1u64 << 41) | (15u64 << 36);
        si[10..18].copy_from_slice(&packed.to_be_bytes());
        f.extend_from_slice(&si);
        if let Some(png) = png {
            // PICTURE (type 6), marked last block.
            let mut pic = Vec::new();
            pic.extend_from_slice(&3u32.to_be_bytes()); // front cover
            let mime = b"image/png";
            pic.extend_from_slice(&(mime.len() as u32).to_be_bytes());
            pic.extend_from_slice(mime);
            pic.extend_from_slice(&0u32.to_be_bytes()); // description length
            pic.extend_from_slice(&0u32.to_be_bytes()); // width
            pic.extend_from_slice(&0u32.to_be_bytes()); // height
            pic.extend_from_slice(&0u32.to_be_bytes()); // colour depth
            pic.extend_from_slice(&0u32.to_be_bytes()); // colours used
            pic.extend_from_slice(&(png.len() as u32).to_be_bytes());
            pic.extend_from_slice(png);
            f.push(0x86); // last=1, type=6
            f.extend_from_slice(&be24(pic.len() as u32));
            f.extend_from_slice(&pic);
        }
        f
    }

    #[test]
    fn extracts_the_embedded_cover_art() {
        let png = tiny_png();
        let file = minimal_flac(Some(&png));
        let art = extract_album_art(&file).unwrap();
        assert_eq!(art.as_deref(), Some(png.as_slice()), "the exact embedded bytes");
    }

    #[test]
    fn a_file_without_a_picture_yields_none() {
        let file = minimal_flac(None);
        assert_eq!(extract_album_art(&file).unwrap(), None);
    }

    #[test]
    fn unparseable_input_is_fail_closed_not_a_panic() {
        // Random bytes are not a recognisable container: an error, never a panic
        // or a forged picture.
        assert!(extract_album_art(b"this is not an audio container at all").is_err());
    }

    #[test]
    fn refuses_oversize_input() {
        let big = vec![0u8; MAX_BYTES + 1];
        assert!(matches!(extract_album_art(&big), Err(SandboxError::TooLarge)));
    }

    #[test]
    fn reads_playback_properties_from_the_streaminfo() {
        // The minimal FLAC's STREAMINFO declares 44100 Hz / 2 channels
        // (channels field = stored value 1, i.e. channels-1); duration is 0
        // since no total-sample count / audio frames are present. The viewer's
        // scrubber reads exactly these.
        let meta = read_audio_metadata(&minimal_flac(None)).unwrap();
        assert_eq!(meta.sample_rate, Some(44_100));
        assert_eq!(meta.channels, Some(2));
        assert!(meta.duration_secs >= 0.0);
        // No tag block in the minimal container: tag fields are absent.
        assert_eq!(meta.title, None);
        assert_eq!(meta.artist, None);
    }

    #[test]
    fn metadata_is_fail_closed_on_unparseable_input() {
        assert!(read_audio_metadata(b"this is not an audio container at all").is_err());
    }

    #[test]
    fn metadata_refuses_oversize_input() {
        let big = vec![0u8; MAX_BYTES + 1];
        assert!(matches!(read_audio_metadata(&big), Err(SandboxError::TooLarge)));
    }
}
