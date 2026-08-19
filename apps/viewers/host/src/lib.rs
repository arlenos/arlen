//! The viewer host-side decode dispatch (`quickview-plan.md`).
//!
//! The host (the viewer) holds the file read capability; the per-format decoder
//! does NOT. So the host reads the file, [`detect`](arlen_viewers_core::detect)s
//! its format, and pipes the bytes into the matching decoder running in a bwrap
//! sandbox - **no network, no writable filesystem, no read access to the user's
//! files** - which writes back only the validated raster/probe frame. A decoder
//! that crashes or is compromised cannot reach the network, write anything, read
//! `$HOME`/`/etc`/other apps' data, OOM the host (the read is bounded), or hang
//! it (the watchdog kills past the timeout). It CAN read the world-readable
//! `/usr` it is given for its own dynamic libraries (a bounded info surface, not
//! the user's data); narrowing that to a minimal lib set is a follow-up. The
//! confinement + argv are pure + unit-tested here; the real bwrap spawn is the
//! on-kernel `#[ignore]d` test.
//!
//! Seccomp is staged like arlen-run: v1 is the namespace + no-network + read-
//! only confinement; the `--seccomp <fd>` BPF filter (and the wider profile for
//! the C-linked AVIF/HEIC decoders) is the hardening follow-up.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use arlen_worker_sandbox::WorkerProfile;
use arlen_viewers_core::audio::{decode_audio_frame, AudioInfo};
use arlen_viewers_core::decode::{decode_frame, DecodedImage, MAX_PIXELS};
use arlen_viewers_core::{detect, Decoder};


/// The largest file the host reads + pipes to a decoder (mirrors the worker's
/// own input bound).
pub const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;

/// The largest frame the host will read back from a worker, BEFORE parsing. The
/// frame DoS bound ([`MAX_PIXELS`]) is enforced when the frame is parsed, but a
/// COMPROMISED worker (the stated threat) could write RGBA-looking bytes forever
/// and OOM the host on the read itself; this caps the read. The image raster
/// frame is the largest legitimate output (12-byte header + RGBA); the audio
/// probe frame is tiny, so this one cap covers both.
pub const MAX_OUTPUT_BYTES: u64 = 12 + MAX_PIXELS * 4;

/// The wall-clock budget for a single decode. A hung or pathologically-slow
/// worker (a malformed file hitting a codec loop, or a malicious worker that
/// never exits) is SIGKILLed past this, so a decode cannot wedge the caller.
pub const DECODE_TIMEOUT: Duration = Duration::from_secs(20);

/// The sandboxed worker binary name for an image [`Decoder`], or `None` for a
/// decoder with no image worker (audio Symphonia + the long-tail Fallback take
/// other paths). The names match the worker crates' `[[bin]]`.
pub fn worker_bin(decoder: Decoder) -> Option<&'static str> {
    match decoder {
        Decoder::ImageRs => Some("arlen-decode-image"),
        Decoder::JxlOxide => Some("arlen-decode-jxl"),
        Decoder::LibHeif => Some("arlen-decode-heic"),
        Decoder::Symphonia | Decoder::Fallback => None,
    }
}

/// Run `worker_bin` (under `worker_dir`) confined, pipe `input` to it and read
/// back its frame.
///
/// The machinery moved to `arlen-worker-sandbox` when the PDF page renderer
/// needed the same box: the watchdog, the bounded read, the seccomp memfd and
/// the fail-closed fd sweep are one implementation now rather than two. What
/// stays here is the viewer's own question - which syscall profile a given
/// decoder needs - because that is a fact about these decoders and not about
/// sandboxing.
///
/// # Errors
/// The worker's own reason, or the sandbox's: a timeout, an over-cap frame, a
/// non-zero exit.
pub fn run_confined_worker(
    worker_dir: &str,
    worker_bin: &str,
    decoder: Decoder,
    input: &[u8],
) -> Result<Vec<u8>, String> {
    arlen_worker_sandbox::run_confined_worker(worker_dir, worker_bin, profile_for(decoder), &[], input)
}

/// The syscall profile a decoder needs.
///
/// Only libheif's backends (`dav1d`, `libde265`) make threads of their own; the
/// pure-Rust decoders run on one. This is the whole of what the filter varied
/// on when it took a `Decoder`, written out.
#[must_use]
pub fn profile_for(decoder: Decoder) -> WorkerProfile {
    match decoder {
        Decoder::LibHeif => WorkerProfile::THREADED,
        Decoder::ImageRs | Decoder::JxlOxide | Decoder::Symphonia | Decoder::Fallback => {
            WorkerProfile::SINGLE_THREADED
        }
    }
}

/// Spawn the image decoder confined (under its per-decoder seccomp profile) and
/// read back the validated [`DecodedImage`].
pub fn spawn_decode(
    worker_dir: &str,
    worker_bin: &str,
    decoder: Decoder,
    input: &[u8],
) -> Result<DecodedImage, String> {
    let frame = run_confined_worker(worker_dir, worker_bin, decoder, input)?;
    decode_frame(&frame).map_err(|e| format!("invalid decoder frame: {e:?}"))
}

/// Spawn the audio probe worker confined and read back the validated [`AudioInfo`].
pub fn spawn_probe(
    worker_dir: &str,
    worker_bin: &str,
    decoder: Decoder,
    input: &[u8],
) -> Result<AudioInfo, String> {
    let frame = run_confined_worker(worker_dir, worker_bin, decoder, input)?;
    decode_audio_frame(&frame).map_err(|e| format!("invalid probe frame: {e:?}"))
}

/// Decode an on-disk image file: read it (bounded), detect the format, and run
/// the matching sandboxed decoder. Errors for an audio/fallback file (no image
/// worker), an unsupported format, or a decode failure.
pub fn decode_image_path(worker_dir: &str, path: &Path) -> Result<DecodedImage, String> {
    let mut input = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_INPUT_BYTES).read_to_end(&mut input).map(|_| ()))
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let detected = detect(name, &input).ok_or("unsupported file format")?;
    let bin = worker_bin(detected.decoder).ok_or("no image decoder for this format")?;
    spawn_decode(worker_dir, bin, detected.decoder, &input)
}

/// The sandboxed worker binary for an audio [`Decoder`], or `None` for a
/// non-audio decoder. Separate from [`worker_bin`] because the audio worker
/// returns an [`AudioInfo`] probe, not an image raster.
pub fn audio_worker_bin(decoder: Decoder) -> Option<&'static str> {
    match decoder {
        Decoder::Symphonia => Some("arlen-decode-audio"),
        // The Fallback also handles exotic audio, but its worker is a later slice.
        _ => None,
    }
}

/// Probe an on-disk audio file: read it (bounded), detect the format, and run
/// the matching sandboxed probe worker, returning its [`AudioInfo`]. Errors for
/// an image/unsupported file or a probe failure.
pub fn probe_audio_path(worker_dir: &str, path: &Path) -> Result<AudioInfo, String> {
    let mut input = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_INPUT_BYTES).read_to_end(&mut input).map(|_| ()))
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let detected = detect(name, &input).ok_or("unsupported file format")?;
    let bin = audio_worker_bin(detected.decoder).ok_or("no audio probe worker for this format")?;
    spawn_probe(worker_dir, bin, detected.decoder, &input)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_decoders_map_to_their_workers() {
        assert_eq!(worker_bin(Decoder::ImageRs), Some("arlen-decode-image"));
        assert_eq!(worker_bin(Decoder::JxlOxide), Some("arlen-decode-jxl"));
        assert_eq!(worker_bin(Decoder::LibHeif), Some("arlen-decode-heic"));
        assert_eq!(worker_bin(Decoder::Symphonia), None);
        assert_eq!(worker_bin(Decoder::Fallback), None);
    }



    #[test]
    fn the_output_cap_covers_the_largest_image_frame_and_the_audio_frame() {
        use arlen_viewers_core::audio::AudioInfo;
        use arlen_viewers_core::decode::MAX_PIXELS;
        // The cap equals the largest legitimate image frame (header + max RGBA).
        assert_eq!(MAX_OUTPUT_BYTES, 12 + MAX_PIXELS * 4);
        // A real audio probe frame is far under the cap (so one cap covers both).
        let audio = AudioInfo {
            codec: "vorbis".into(),
            sample_rate: 48_000,
            channels: 2,
            duration_ms: Some(1),
            title: None,
            artist: None,
            peaks: Vec::new(),
        };
        assert!((audio.encode().len() as u64) < MAX_OUTPUT_BYTES);
    }

    #[test]
    fn audio_routes_to_the_probe_worker_and_images_do_not() {
        assert_eq!(audio_worker_bin(Decoder::Symphonia), Some("arlen-decode-audio"));
        assert_eq!(audio_worker_bin(Decoder::ImageRs), None);
        assert_eq!(audio_worker_bin(Decoder::Fallback), None);
        // The two dispatch tables are disjoint: an image decoder has an image
        // worker but no audio worker, and vice versa.
        assert!(worker_bin(Decoder::ImageRs).is_some() && audio_worker_bin(Decoder::ImageRs).is_none());
        assert!(audio_worker_bin(Decoder::Symphonia).is_some() && worker_bin(Decoder::Symphonia).is_none());
    }

    /// On-kernel (needs a userns-capable host + the built `arlen-decode-image`
    /// worker in `ARLEN_VIEWERS_WORKER_DIR`): a real PNG piped through the
    /// confined worker yields its raster. Validates the confinement + the spawn +
    /// the frame round-trip end to end. `#[ignore]d` like the other bwrap tests.
    #[test]
    #[ignore = "needs a userns-capable host + the built decoder worker"]
    fn a_confined_worker_decodes_a_real_png() {
        let dir = std::env::var("ARLEN_VIEWERS_WORKER_DIR").expect("set ARLEN_VIEWERS_WORKER_DIR");
        // A real PNG read by the host (no image dep here) and piped to the worker;
        // the path is env-driven so any distro/CI can point it at a present PNG.
        let png_path = std::env::var("ARLEN_VIEWERS_TEST_PNG")
            .unwrap_or_else(|_| "/usr/share/pixmaps/archlinux-logo.png".to_string());
        let png = std::fs::read(&png_path).expect("a test PNG at ARLEN_VIEWERS_TEST_PNG");
        // Decodes UNDER the installed per-decoder seccomp filter, so a success
        // also proves the tight base allowlist permits a real decode on metal.
        let decoded = spawn_decode(&dir, "arlen-decode-image", Decoder::ImageRs, &png).expect("decode");
        assert!(decoded.width > 0 && decoded.height > 0);
        assert_eq!(decoded.rgba.len(), (decoded.width * decoded.height * 4) as usize);
    }

    /// On-kernel: the single-threaded JXL worker decodes UNDER the tight base
    /// filter (no `clone`), proving jxl-oxide-without-rayon really fits the
    /// pure-Rust profile. Point `ARLEN_VIEWERS_JXL_DIR` at the built worker and
    /// `ARLEN_VIEWERS_TEST_JXL` at a `.jxl` (the decode-jxl crate ships one).
    #[test]
    #[ignore = "needs a userns-capable host + the built JXL worker"]
    fn a_confined_jxl_worker_decodes_under_the_tight_filter() {
        let dir = std::env::var("ARLEN_VIEWERS_JXL_DIR").expect("set ARLEN_VIEWERS_JXL_DIR");
        let path = std::env::var("ARLEN_VIEWERS_TEST_JXL").expect("set ARLEN_VIEWERS_TEST_JXL");
        let jxl = std::fs::read(&path).expect("a test JXL at ARLEN_VIEWERS_TEST_JXL");
        let decoded = spawn_decode(&dir, "arlen-decode-jxl", Decoder::JxlOxide, &jxl).expect("decode");
        assert!(decoded.width > 0 && decoded.height > 0);
    }

    /// On-kernel: the C-linked HEIC/AVIF worker decodes UNDER the wider filter
    /// (the one profile that adds thread creation), proving the threaded
    /// dav1d/libde265 codecs run with the extra `clone`/`sched_*` and nothing
    /// more. Point `ARLEN_VIEWERS_HEIC_DIR` + `ARLEN_VIEWERS_TEST_HEIC`.
    #[test]
    #[ignore = "needs a userns-capable host + the built HEIC worker"]
    fn a_confined_heic_worker_decodes_under_the_wider_filter() {
        let dir = std::env::var("ARLEN_VIEWERS_HEIC_DIR").expect("set ARLEN_VIEWERS_HEIC_DIR");
        let path = std::env::var("ARLEN_VIEWERS_TEST_HEIC").expect("set ARLEN_VIEWERS_TEST_HEIC");
        let heic = std::fs::read(&path).expect("a test HEIC/AVIF at ARLEN_VIEWERS_TEST_HEIC");
        let decoded = spawn_decode(&dir, "arlen-decode-heic", Decoder::LibHeif, &heic).expect("decode");
        assert!(decoded.width > 0 && decoded.height > 0);
    }

    /// On-kernel: the audio probe worker (Symphonia, single-threaded) returns an
    /// `AudioInfo` UNDER the tight base filter, confirming the `ENOSYS` mismatch
    /// action does not break the pure-Rust audio path. Point
    /// `ARLEN_VIEWERS_AUDIO_DIR` + `ARLEN_VIEWERS_TEST_AUDIO` (the decode-audio
    /// crate ships a `.wav`).
    #[test]
    #[ignore = "needs a userns-capable host + the built audio worker"]
    fn a_confined_audio_worker_probes_under_the_tight_filter() {
        let dir = std::env::var("ARLEN_VIEWERS_AUDIO_DIR").expect("set ARLEN_VIEWERS_AUDIO_DIR");
        let path = std::env::var("ARLEN_VIEWERS_TEST_AUDIO").expect("set ARLEN_VIEWERS_TEST_AUDIO");
        let audio = std::fs::read(&path).expect("a test audio file at ARLEN_VIEWERS_TEST_AUDIO");
        let info = spawn_probe(&dir, "arlen-decode-audio", Decoder::Symphonia, &audio).expect("probe");
        // sample_rate is the universally-surfaced field; channels can be 0 for a
        // metadata-only AAC probe (the channel config lives in the decoded
        // AudioSpecificConfig), so it is not asserted here.
        assert!(info.sample_rate > 0);
    }
}

/// Default-handler registration (xdg mimeapps) for the viewer.
pub mod mimeapps;
