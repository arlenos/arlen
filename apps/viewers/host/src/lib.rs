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
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use arlen_confiner::{app_runtime_profile, Bind, Confinement, ConfinerError, NetworkPolicy};
use arlen_viewers_core::audio::{decode_audio_frame, AudioInfo};
use arlen_viewers_core::decode::{decode_frame, DecodedImage, MAX_PIXELS};
use arlen_viewers_core::{detect, Decoder};

pub mod seccomp;

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

/// The decoder sandbox: read-only `/usr` (the worker's dynamic libs) + read-only
/// the worker's own directory (its binary), NO network, and NO file binds at all
/// (the worker reads its input from stdin, never the filesystem, so it cannot
/// open any other file). A tmpfs `/tmp` is provided by the app-runtime base.
pub fn decoder_confinement(worker_dir: &str) -> Result<Confinement, ConfinerError> {
    let dir = require_abs(worker_dir)?;
    let skeleton = app_runtime_profile(Path::new("/usr"), &[], BTreeMap::new(), NetworkPolicy::None)?;
    Ok(skeleton.complete(vec![Bind::ReadOnly(dir.clone(), dir)], vec![]))
}

/// The full confined spawn argv: the bwrap flags then `-- <worker_path>`. Pure.
pub fn decode_worker_argv(confinement: &Confinement, worker_path: &str) -> Vec<String> {
    let mut argv = confinement.bwrap_args();
    argv.push("--".to_string());
    argv.push(worker_path.to_string());
    argv
}

fn require_abs(path: &str) -> Result<String, ConfinerError> {
    if Path::new(path).is_absolute() {
        Ok(path.to_string())
    } else {
        Err(ConfinerError::RelativePath(path.to_string()))
    }
}

/// Run the worker `worker_bin` (under `worker_dir`) in the sandbox, pipe `input`
/// to its stdin, and return its raw stdout frame bytes. The caller decodes the
/// frame (a raster for an image worker, an AudioInfo for audio). Hardened
/// against a COMPROMISED worker (the design's threat model):
/// - the stdout read is bounded at [`MAX_OUTPUT_BYTES`], so a worker that writes
///   forever cannot OOM the host;
/// - a watchdog SIGKILLs the worker past [`DECODE_TIMEOUT`], so a hung worker
///   cannot wedge the caller. Because the confinement sets `--die-with-parent`,
///   killing bwrap also tears down the inner decoder, which closes the pipes -
///   so the kill unblocks both the stdout read AND the stdin writer thread;
/// - input is written on a separate thread while stdout is drained, so a large
///   input + output cannot deadlock on the pipe buffers; a non-zero exit errs.
pub fn run_confined_worker(worker_dir: &str, worker_bin: &str, input: &[u8]) -> Result<Vec<u8>, String> {
    let worker_path = format!("{}/{worker_bin}", worker_dir.trim_end_matches('/'));
    let confinement = decoder_confinement(worker_dir).map_err(|e| e.to_string())?;
    let argv = decode_worker_argv(&confinement, &worker_path);

    let mut child = Command::new("bwrap")
        .args(&argv)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("spawn bwrap: {e}"))?;
    let pid = child.id() as libc::pid_t;

    let mut stdin = child.stdin.take().ok_or("no child stdin")?;
    let owned = input.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&owned);
        // Dropping stdin closes it, signalling EOF to the worker.
    });

    // A watchdog kills the worker on the timeout. The main thread signals `done`
    // once the read completes; a `recv_timeout` that expires first means the
    // worker is hung/slow, so SIGKILL it (pid not yet reaped - `wait()` is below
    // - so there is no pid-reuse window).
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let watchdog = std::thread::spawn(move || {
        if done_rx.recv_timeout(DECODE_TIMEOUT).is_err() {
            // SAFETY: SIGKILL by pid; benign (ESRCH) if the worker already exited.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    });

    // Read at most MAX_OUTPUT_BYTES + 1, so an over-cap worker is detected rather
    // than silently truncated into a plausible-but-wrong frame.
    let mut out = Vec::new();
    let read_result = child
        .stdout
        .take()
        .ok_or("no child stdout")
        .and_then(|so| {
            so.take(MAX_OUTPUT_BYTES + 1).read_to_end(&mut out).map_err(|_| "read stdout").map(|_| ())
        });
    let _ = done_tx.send(()); // cancel the watchdog if the read finished in time
    let _ = writer.join();
    let status = child.wait().map_err(|e| format!("wait: {e}"))?;
    let _ = watchdog.join();
    read_result?;
    if out.len() as u64 > MAX_OUTPUT_BYTES {
        return Err("worker output exceeded the frame bound".to_string());
    }
    if !status.success() {
        return Err(format!("worker exited with {status}"));
    }
    Ok(out)
}

/// Spawn the image decoder confined and read back the validated [`DecodedImage`].
pub fn spawn_decode(worker_dir: &str, worker_bin: &str, input: &[u8]) -> Result<DecodedImage, String> {
    let frame = run_confined_worker(worker_dir, worker_bin, input)?;
    decode_frame(&frame).map_err(|e| format!("invalid decoder frame: {e:?}"))
}

/// Spawn the audio probe worker confined and read back the validated [`AudioInfo`].
pub fn spawn_probe(worker_dir: &str, worker_bin: &str, input: &[u8]) -> Result<AudioInfo, String> {
    let frame = run_confined_worker(worker_dir, worker_bin, input)?;
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
    spawn_decode(worker_dir, bin, &input)
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
    spawn_probe(worker_dir, bin, &input)
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
    fn the_decoder_sandbox_has_no_network_no_file_binds_and_a_readonly_worker() {
        let conf = decoder_confinement("/opt/arlen/viewers").unwrap();
        let argv = decode_worker_argv(&conf, "/opt/arlen/viewers/arlen-decode-image");
        assert!(argv.contains(&"--unshare-net".to_string()), "no network");
        // The only bind beyond the base /usr is the read-only worker dir; there
        // is no --bind (read-write) of anything (the worker gets no FS write).
        assert!(!argv.iter().any(|a| a == "--bind"), "no read-write bind");
        let ro: Vec<_> = argv.windows(2).filter(|w| w[0] == "--ro-bind").map(|w| w[1].clone()).collect();
        assert!(ro.contains(&"/usr".to_string()), "/usr is read-only");
        assert!(ro.contains(&"/opt/arlen/viewers".to_string()), "the worker dir is read-only");
        // The program tail runs the worker.
        let sep = argv.iter().position(|s| s == "--").unwrap();
        assert_eq!(&argv[sep + 1..], &["/opt/arlen/viewers/arlen-decode-image".to_string()]);
    }

    #[test]
    fn a_relative_worker_dir_is_rejected() {
        assert!(matches!(decoder_confinement("opt/viewers"), Err(ConfinerError::RelativePath(_))));
    }

    #[test]
    fn the_output_cap_covers_the_largest_image_frame_and_the_audio_frame() {
        use arlen_viewers_core::audio::AudioInfo;
        use arlen_viewers_core::decode::MAX_PIXELS;
        // The cap equals the largest legitimate image frame (header + max RGBA).
        assert_eq!(MAX_OUTPUT_BYTES, 12 + MAX_PIXELS * 4);
        // A real audio probe frame is far under the cap (so one cap covers both).
        let audio = AudioInfo { codec: "vorbis".into(), sample_rate: 48_000, channels: 2, duration_ms: Some(1) };
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
        // A real PNG read by the host (no image dep here) and piped to the worker.
        let png = std::fs::read("/usr/share/pixmaps/endeavouros-logo.png").expect("a test PNG");
        let decoded = spawn_decode(&dir, "arlen-decode-image", &png).expect("decode");
        assert!(decoded.width > 0 && decoded.height > 0);
        assert_eq!(decoded.rgba.len(), (decoded.width * decoded.height * 4) as usize);
    }
}

/// Default-handler registration (xdg mimeapps) for the viewer.
pub mod mimeapps;
