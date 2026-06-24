//! The minimal viewer's format-detection + decoder-dispatch core
//! (`quickview-plan.md`).
//!
//! Routes a file to the sandboxed decoder that handles it. Detection is by
//! MAGIC BYTES first (the robust signal - an extension can lie), with the file
//! extension as a fallback when the head is unavailable or inconclusive. The
//! decoder set mirrors the plan's one-sandboxed-process-per-decoder model:
//! `image-rs` for the common image base, `jxl-oxide` for JPEG XL, `libheif` for
//! HEIC, `Symphonia` for audio, and a long-tail `Fallback` (ffmpeg/libvips) for
//! RAW + exotic codecs the pure-Rust path does not reach. This crate is PURE
//! (no decode deps): it only decides which decoder + media kind + MIME a file
//! is, so the dispatch is unit-tested without any heavy decoder linked.

/// The image MIME types the viewer registers as the default handler for (the
/// formats [`detect`] recognises). The default-handler registration + the
/// `.desktop` `MimeType=` are generated from this, so the registered set and the
/// detected set stay in step.
pub const IMAGE_MIMES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/tiff",
    "image/bmp",
    "image/avif",
    "image/heic",
    "image/jxl",
];

/// The audio MIME types the viewer handles (the simple player). Registered
/// alongside the images once the audio decode worker lands.
pub const AUDIO_MIMES: &[&str] = &["audio/flac", "audio/mpeg", "audio/wav", "audio/ogg"];

/// Whether the file is an image or audio (the two viewer surfaces).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// A still image (the zoom/pan surface).
    Image,
    /// An audio file (the simple player surface).
    Audio,
}

/// The sandboxed decoder that handles a format. Each runs as its own bwrap
/// process (no write/network, seccomp); a flawed decoder cannot take the viewer
/// down. The AVIF/HEIC path gets the wider seccomp profile (its C-linked
/// threaded codec needs `clone`); the pure-Rust decoders keep the tight one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoder {
    /// `image-rs`: PNG, JPEG, WebP, GIF, TIFF, BMP, AVIF.
    ImageRs,
    /// `jxl-oxide`: JPEG XL.
    JxlOxide,
    /// `libheif`: HEIC/HEIF.
    LibHeif,
    /// `Symphonia`: FLAC, MP3, WAV/PCM, Ogg/Vorbis.
    Symphonia,
    /// The long-tail fallback (ffmpeg/libvips): RAW camera formats + exotic
    /// codecs the pure-Rust decoders do not cover.
    Fallback,
}

/// A detected format: its media kind, the decoder that handles it, and the MIME
/// type (for the default-handler registration).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detected {
    /// Image or audio.
    pub kind: MediaKind,
    /// The sandboxed decoder to run.
    pub decoder: Decoder,
    /// The canonical MIME type.
    pub mime: &'static str,
}

impl Detected {
    const fn new(kind: MediaKind, decoder: Decoder, mime: &'static str) -> Self {
        Self { kind, decoder, mime }
    }
}

/// Detect from the file's leading bytes (`head`, the first few KiB is plenty).
/// Returns `None` when no signature matches; the caller then falls back to
/// [`detect_by_extension`]. Magic bytes are authoritative - an extension can lie.
pub fn detect_by_magic(head: &[u8]) -> Option<Detected> {
    let starts = |sig: &[u8]| head.len() >= sig.len() && &head[..sig.len()] == sig;
    // An ISO-BMFF `ftyp` box (AVIF/HEIC) carries its brand at bytes 8..12.
    let ftyp_brand = |brand: &[u8]| head.len() >= 12 && &head[4..8] == b"ftyp" && &head[8..12] == brand;

    // Images.
    if starts(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some(Detected::new(MediaKind::Image, Decoder::ImageRs, "image/png"));
    }
    if starts(&[0xFF, 0xD8, 0xFF]) {
        return Some(Detected::new(MediaKind::Image, Decoder::ImageRs, "image/jpeg"));
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        return Some(Detected::new(MediaKind::Image, Decoder::ImageRs, "image/gif"));
    }
    if starts(b"RIFF") && head.len() >= 12 && &head[8..12] == b"WEBP" {
        return Some(Detected::new(MediaKind::Image, Decoder::ImageRs, "image/webp"));
    }
    if starts(&[0x49, 0x49, 0x2A, 0x00]) || starts(&[0x4D, 0x4D, 0x00, 0x2A]) {
        return Some(Detected::new(MediaKind::Image, Decoder::ImageRs, "image/tiff"));
    }
    if starts(b"BM") {
        return Some(Detected::new(MediaKind::Image, Decoder::ImageRs, "image/bmp"));
    }
    if ftyp_brand(b"avif") || ftyp_brand(b"avis") {
        return Some(Detected::new(MediaKind::Image, Decoder::ImageRs, "image/avif"));
    }
    if ftyp_brand(b"heic") || ftyp_brand(b"heix") || ftyp_brand(b"mif1") || ftyp_brand(b"heif") {
        return Some(Detected::new(MediaKind::Image, Decoder::LibHeif, "image/heic"));
    }
    // JPEG XL: the raw codestream (FF 0A) or the ISO-BMFF container.
    if starts(&[0xFF, 0x0A]) || starts(&[0x00, 0x00, 0x00, 0x0C, b'J', b'X', b'L', 0x20]) {
        return Some(Detected::new(MediaKind::Image, Decoder::JxlOxide, "image/jxl"));
    }

    // Audio.
    if starts(b"fLaC") {
        return Some(Detected::new(MediaKind::Audio, Decoder::Symphonia, "audio/flac"));
    }
    if starts(b"OggS") {
        return Some(Detected::new(MediaKind::Audio, Decoder::Symphonia, "audio/ogg"));
    }
    if starts(b"ID3") || starts(&[0xFF, 0xFB]) || starts(&[0xFF, 0xF3]) || starts(&[0xFF, 0xF2]) {
        return Some(Detected::new(MediaKind::Audio, Decoder::Symphonia, "audio/mpeg"));
    }
    if starts(b"RIFF") && head.len() >= 12 && &head[8..12] == b"WAVE" {
        return Some(Detected::new(MediaKind::Audio, Decoder::Symphonia, "audio/wav"));
    }
    None
}

/// Detect from the file name's extension (case-insensitive), the fallback when
/// the head is unavailable or matched nothing. RAW + exotic extensions route to
/// the [`Decoder::Fallback`].
pub fn detect_by_extension(name: &str) -> Option<Detected> {
    let ext = name.rsplit('.').next().filter(|e| !e.is_empty() && *e != name)?.to_ascii_lowercase();
    let d = match ext.as_str() {
        "png" => Detected::new(MediaKind::Image, Decoder::ImageRs, "image/png"),
        "jpg" | "jpeg" | "jpe" => Detected::new(MediaKind::Image, Decoder::ImageRs, "image/jpeg"),
        "gif" => Detected::new(MediaKind::Image, Decoder::ImageRs, "image/gif"),
        "webp" => Detected::new(MediaKind::Image, Decoder::ImageRs, "image/webp"),
        "tif" | "tiff" => Detected::new(MediaKind::Image, Decoder::ImageRs, "image/tiff"),
        "bmp" => Detected::new(MediaKind::Image, Decoder::ImageRs, "image/bmp"),
        "avif" => Detected::new(MediaKind::Image, Decoder::ImageRs, "image/avif"),
        "heic" | "heif" => Detected::new(MediaKind::Image, Decoder::LibHeif, "image/heic"),
        "jxl" => Detected::new(MediaKind::Image, Decoder::JxlOxide, "image/jxl"),
        "flac" => Detected::new(MediaKind::Audio, Decoder::Symphonia, "audio/flac"),
        "mp3" => Detected::new(MediaKind::Audio, Decoder::Symphonia, "audio/mpeg"),
        "wav" => Detected::new(MediaKind::Audio, Decoder::Symphonia, "audio/wav"),
        "ogg" | "oga" => Detected::new(MediaKind::Audio, Decoder::Symphonia, "audio/ogg"),
        // RAW camera formats + exotic audio: the long-tail fallback decoder.
        "cr2" | "cr3" | "nef" | "arw" | "dng" | "raf" | "orf" | "rw2" => {
            Detected::new(MediaKind::Image, Decoder::Fallback, "image/x-raw")
        }
        "opus" | "aac" | "m4a" | "wma" | "aiff" => {
            Detected::new(MediaKind::Audio, Decoder::Fallback, "audio/x-unknown")
        }
        _ => return None,
    };
    Some(d)
}

/// Detect a file's format: magic bytes first (authoritative), then the
/// extension. `None` means neither matched (the viewer reports an unsupported
/// file rather than guessing).
pub fn detect(name: &str, head: &[u8]) -> Option<Detected> {
    detect_by_magic(head).or_else(|| detect_by_extension(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_bytes_win_over_a_lying_extension() {
        // A PNG head with a .jpg name: the magic bytes decide.
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
        let d = detect("photo.jpg", &png).unwrap();
        assert_eq!(d.mime, "image/png");
        assert_eq!(d.decoder, Decoder::ImageRs);
        assert_eq!(d.kind, MediaKind::Image);
    }

    #[test]
    fn heic_and_avif_share_the_ftyp_box_but_route_to_different_decoders() {
        let mut heic = vec![0u8; 16];
        heic[4..8].copy_from_slice(b"ftyp");
        heic[8..12].copy_from_slice(b"heic");
        assert_eq!(detect_by_magic(&heic).unwrap().decoder, Decoder::LibHeif);

        let mut avif = vec![0u8; 16];
        avif[4..8].copy_from_slice(b"ftyp");
        avif[8..12].copy_from_slice(b"avif");
        let a = detect_by_magic(&avif).unwrap();
        assert_eq!(a.decoder, Decoder::ImageRs);
        assert_eq!(a.mime, "image/avif");
    }

    #[test]
    fn jpeg_xl_both_forms_detect() {
        assert_eq!(detect_by_magic(&[0xFF, 0x0A]).unwrap().decoder, Decoder::JxlOxide);
        let container = [0x00, 0x00, 0x00, 0x0C, b'J', b'X', b'L', 0x20, 0, 0];
        assert_eq!(detect_by_magic(&container).unwrap().mime, "image/jxl");
    }

    #[test]
    fn audio_signatures_route_to_symphonia() {
        assert_eq!(detect_by_magic(b"fLaC....").unwrap(), Detected::new(MediaKind::Audio, Decoder::Symphonia, "audio/flac"));
        assert_eq!(detect_by_magic(b"OggS....").unwrap().mime, "audio/ogg");
        assert_eq!(detect_by_magic(b"ID3\x03\x00").unwrap().mime, "audio/mpeg");
        let mut wav = b"RIFF\0\0\0\0WAVE".to_vec();
        wav.push(0);
        assert_eq!(detect_by_magic(&wav).unwrap().kind, MediaKind::Audio);
    }

    #[test]
    fn riff_disambiguates_webp_from_wav() {
        let mut webp = b"RIFF\0\0\0\0WEBP".to_vec();
        webp.push(0);
        assert_eq!(detect_by_magic(&webp).unwrap().mime, "image/webp");
    }

    #[test]
    fn extension_fallback_when_head_is_blank() {
        assert_eq!(detect("song.flac", &[]).unwrap().decoder, Decoder::Symphonia);
        assert_eq!(detect("IMG.CR2", &[]).unwrap().decoder, Decoder::Fallback);
        assert_eq!(detect("clip.opus", &[]).unwrap().kind, MediaKind::Audio);
    }

    #[test]
    fn an_unknown_format_is_none() {
        assert_eq!(detect("notes.txt", b"hello world"), None);
        assert_eq!(detect("noext", &[1, 2, 3]), None);
        assert_eq!(detect_by_extension("trailingdot."), None);
    }

    #[test]
    fn raw_extensions_route_to_the_fallback_decoder() {
        for raw in ["a.cr2", "a.nef", "a.arw", "a.dng"] {
            assert_eq!(detect(raw, &[]).unwrap().decoder, Decoder::Fallback);
        }
    }
}

/// The decoder worker's self-applied Landlock confinement (read-only `/usr`).
#[cfg(target_os = "linux")]
pub mod sandbox;

/// The decoder-worker image-transfer protocol (raster frame the viewer reads).
pub mod decode;

/// The audio-probe transfer frame (codec/sample-rate/channels/duration).
pub mod audio;
