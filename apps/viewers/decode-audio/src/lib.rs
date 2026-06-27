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
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey};
use symphonia::core::probe::Hint;

/// Read a standard tag (title/artist) from a metadata revision, trimmed and
/// non-empty (an empty or whitespace tag is treated as absent).
fn read_tag(rev: &MetadataRevision, key: StandardTagKey) -> Option<String> {
    rev.tags()
        .iter()
        .find(|t| t.std_key == Some(key))
        .map(|t| t.value.to_string().trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Probe encoded audio bytes for its [`AudioInfo`]. Returns a human-readable
/// error on an unsupported/corrupt container or one with no decodable track.
pub fn probe_audio(bytes: &[u8]) -> Result<AudioInfo, String> {
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes.to_vec())), Default::default());
    let probed = symphonia::default::get_probe()
        .format(&Hint::new(), mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("probe: {e}"))?;
    let mut format = probed.format;
    let mut probed_meta = probed.metadata;
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

    // The `p`/`track` borrow of `format` ends here, so the metadata read can take
    // `&mut format`. Tags can sit in the format's own revision (Vorbis comments in
    // FLAC/OGG, RIFF INFO in WAV) or the probe log (ID3 ahead of an MP3 stream);
    // check the format first, fall back to the probe metadata.
    let mut title = None;
    let mut artist = None;
    if let Some(rev) = format.metadata().current() {
        title = read_tag(rev, StandardTagKey::TrackTitle);
        artist = read_tag(rev, StandardTagKey::Artist);
    }
    if title.is_none() || artist.is_none() {
        if let Some(rev) = probed_meta.get().as_ref().and_then(|m| m.current()) {
            title = title.or_else(|| read_tag(rev, StandardTagKey::TrackTitle));
            artist = artist.or_else(|| read_tag(rev, StandardTagKey::Artist));
        }
    }

    Ok(AudioInfo { codec, sample_rate, channels, duration_ms, title, artist })
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
    fn the_probe_round_trips_through_the_frame() {
        let info = probe_audio(&wav(48_000, 2, 24_000)).unwrap();
        let decoded = arlen_viewers_core::audio::decode_audio_frame(&info.encode()).unwrap();
        assert_eq!(decoded, info);
        assert_eq!(decoded.duration_ms, Some(500));
    }
}
