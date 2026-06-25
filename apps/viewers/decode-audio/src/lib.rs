//! The audio probe worker's probe logic (`quickview-plan.md`).
//!
//! Pure probe: encoded audio bytes -> an [`AudioInfo`] (codec, sample rate,
//! channels, duration when the container declares it) via Symphonia. The binary
//! is the thin stdin/stdout shell run inside the bwrap sandbox; this function is
//! where Symphonia opens the container + reads the default track's parameters,
//! so it is unit-tested by probing a real in-memory file (a hand-built WAV)
//! without spawning a process. Only the pure-Rust formats are linked
//! (wav/ogg/flac/mp3/pcm/vorbis); the raw PCM for playback is a later stream.

use arlen_viewers_core::audio::AudioInfo;
use symphonia::core::codecs::CODEC_TYPE_NULL;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardVisualKey};
use symphonia::core::probe::Hint;

/// Probe encoded audio bytes for its [`AudioInfo`]. Returns a human-readable
/// error on an unsupported/corrupt container or one with no decodable track.
pub fn probe_audio(bytes: &[u8]) -> Result<AudioInfo, String> {
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes.to_vec())), Default::default());
    let probed = symphonia::default::get_probe()
        .format(&Hint::new(), mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("probe: {e}"))?;
    let format = probed.format;
    // The first track that carries a real codec (skip a null/metadata track).
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("no decodable audio track")?;
    let p = &track.codec_params;

    let sample_rate = p.sample_rate.ok_or("track has no sample rate")?;
    let channels = p.channels.map(|c| c.count() as u16).unwrap_or(0);
    let codec = symphonia::default::get_codecs()
        .get_codec(p.codec)
        .map(|cd| cd.short_name.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    // Duration only when the container declares the frame count.
    let duration_ms = match p.n_frames {
        Some(frames) if sample_rate > 0 => Some(frames.saturating_mul(1000) / u64::from(sample_rate)),
        _ => None,
    };

    Ok(AudioInfo { codec, sample_rate, channels, duration_ms })
}

/// Extract the embedded cover art from `bytes` as the raw encoded image (the
/// PNG/JPEG exactly as stored in the tag), for an audio thumbnail. Returns the
/// front-cover picture, else the first embedded image, else `None` when the file
/// carries no art. The bytes are themselves an encoded image, so the thumbnailer
/// hands them to the image decoder like any other picture. Runs in the same
/// sandboxed worker as [`probe_audio`] (untrusted media bytes are never decoded
/// in-process).
pub fn extract_cover_art(bytes: &[u8]) -> Option<Vec<u8>> {
    let mss =
        MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes.to_vec())), Default::default());
    let mut probed = symphonia::default::get_probe()
        .format(&Hint::new(), mss, &FormatOptions::default(), &MetadataOptions::default())
        .ok()?;
    // The picture may surface in the probe's own metadata (e.g. an ID3 tag ahead
    // of the stream) or in the format reader's metadata (e.g. a FLAC PICTURE block).
    if let Some(art) = probed.metadata.get().and_then(|m| m.current().and_then(cover_from)) {
        return Some(art);
    }
    probed.format.metadata().current().and_then(cover_from)
}

/// The front-cover picture (else the first visual) of a metadata revision, as
/// owned bytes; `None` when the revision carries no non-empty visual.
fn cover_from(rev: &MetadataRevision) -> Option<Vec<u8>> {
    let visuals = rev.visuals();
    visuals
        .iter()
        .find(|v| v.usage == Some(StandardVisualKey::FrontCover))
        .or_else(|| visuals.first())
        .filter(|v| !v.data.is_empty())
        .map(|v| v.data.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal canonical PCM WAV: 44-byte header + `frames` 16-bit samples per
    /// channel, so Symphonia probes a real container in-test (no fixture file).
    fn wav(sample_rate: u32, channels: u16, frames: u32) -> Vec<u8> {
        let bits = 16u16;
        let block_align = channels * bits / 8;
        let byte_rate = sample_rate * u32::from(block_align);
        let data_len = frames * u32::from(block_align);
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data_len).to_le_bytes());
        w.extend_from_slice(b"WAVE");
        w.extend_from_slice(b"fmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes()); // PCM
        w.extend_from_slice(&channels.to_le_bytes());
        w.extend_from_slice(&sample_rate.to_le_bytes());
        w.extend_from_slice(&byte_rate.to_le_bytes());
        w.extend_from_slice(&block_align.to_le_bytes());
        w.extend_from_slice(&bits.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&data_len.to_le_bytes());
        w.resize(w.len() + data_len as usize, 0);
        w
    }

    #[test]
    fn probes_a_real_wav() {
        // 8000 Hz mono, 8000 frames = exactly 1 second.
        let info = probe_audio(&wav(8000, 1, 8000)).expect("probe wav");
        assert_eq!(info.sample_rate, 8000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.duration_ms, Some(1000));
        assert!(info.codec.contains("pcm"), "codec is a PCM variant: {}", info.codec);
    }

    #[test]
    fn probes_stereo_44k() {
        let info = probe_audio(&wav(44_100, 2, 44_100)).expect("probe");
        assert_eq!((info.sample_rate, info.channels), (44_100, 2));
        assert_eq!(info.duration_ms, Some(1000));
    }

    #[test]
    fn rejects_garbage() {
        assert!(probe_audio(b"not audio at all, just text").is_err());
    }

    #[test]
    fn extracts_embedded_cover_art_from_a_flac() {
        // A tiny silent FLAC carrying a 16x16 front-cover PNG (committed fixture).
        let flac = include_bytes!("fixtures/cover_test.flac");
        let art = extract_cover_art(flac).expect("the fixture carries a front-cover picture");
        // The art is returned as the stored encoded image - here the PNG verbatim.
        assert_eq!(&art[..8], b"\x89PNG\r\n\x1a\n", "the cover is the embedded PNG");
    }

    #[test]
    fn no_cover_art_in_a_bare_wav() {
        // The synth WAV carries no picture block, so there is nothing to extract.
        assert!(extract_cover_art(&wav(8000, 1, 800)).is_none());
    }

    #[test]
    fn the_probe_round_trips_through_the_frame() {
        let info = probe_audio(&wav(48_000, 2, 24_000)).unwrap();
        let decoded = arlen_viewers_core::audio::decode_audio_frame(&info.encode()).unwrap();
        assert_eq!(decoded, info);
        assert_eq!(decoded.duration_ms, Some(500));
    }
}
