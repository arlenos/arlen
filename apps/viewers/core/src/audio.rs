//! The audio-probe transfer frame (`quickview-plan.md`).
//!
//! The audio surface is a simple player, not a raster. Its sandboxed decoder
//! worker (Symphonia) probes the file and writes back this small metadata frame
//! - codec, sample rate, channels, and the duration when the container declares
//! it - which the player uses to size its transport + show the format. The raw
//! PCM for playback streams separately; this frame is just the up-front probe.
//! Like the image raster frame it is validated fail-closed, so a garbled worker
//! frame is rejected rather than trusted.

/// What a worker's probe reports about an audio file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInfo {
    /// The codec short name (e.g. "flac", "mp3", "pcm", "vorbis").
    pub codec: String,
    /// Samples per second per channel.
    pub sample_rate: u32,
    /// Channel count (1 = mono, 2 = stereo, ...).
    pub channels: u16,
    /// Duration in milliseconds, when the container declares it (`None` for a
    /// stream whose length is not known up front).
    pub duration_ms: Option<u64>,
    /// The track title tag, when the container carries one (the player falls
    /// back to the file name otherwise).
    pub title: Option<String>,
    /// The artist tag, when present.
    pub artist: Option<String>,
}

/// The largest codec name accepted in a frame (a bound on the variable field).
const MAX_CODEC_LEN: usize = 64;
/// The largest tag string accepted in a frame (a bound on title/artist); a longer
/// tag is truncated on encode and rejected on decode (fail-closed).
const MAX_TAG_LEN: usize = 512;
const MAGIC: &[u8; 4] = b"ARA1";

/// A malformed audio probe frame (fail-closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioFrameError {
    /// The frame did not start with the expected magic.
    BadMagic,
    /// The frame was shorter than its header / declared codec name.
    Truncated,
    /// The codec name length exceeded [`MAX_CODEC_LEN`].
    CodecTooLong,
    /// The codec name was not valid UTF-8.
    BadCodec,
    /// A tag (title/artist) length exceeded [`MAX_TAG_LEN`].
    TagTooLong,
    /// A tag had a bad presence flag or was not valid UTF-8.
    BadTag,
}

impl AudioInfo {
    /// Encode as a worker probe frame: `MAGIC` + sample_rate(LE u32) +
    /// channels(LE u16) + a duration flag byte (1 = present) + duration(LE u64,
    /// 0 when absent) + codec_len(u8) + codec bytes.
    pub fn encode(&self) -> Vec<u8> {
        let codec = self.codec.as_bytes();
        let codec_len = codec.len().min(MAX_CODEC_LEN);
        let mut out = Vec::with_capacity(4 + 4 + 2 + 1 + 8 + 1 + codec_len);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.sample_rate.to_le_bytes());
        out.extend_from_slice(&self.channels.to_le_bytes());
        let (flag, dur) = match self.duration_ms {
            Some(ms) => (1u8, ms),
            None => (0u8, 0u64),
        };
        out.push(flag);
        out.extend_from_slice(&dur.to_le_bytes());
        out.push(codec_len as u8);
        out.extend_from_slice(&codec[..codec_len]);
        // Tags follow the codec as an append-only tail: title then artist, each a
        // presence flag + (when present) a u16-LE length + bytes. A reader that
        // predates the tail (a short frame) decodes them as absent, so the format
        // is forward-extensible (peaks land the same way later).
        push_opt_str(&mut out, &self.title);
        push_opt_str(&mut out, &self.artist);
        out
    }
}

/// Append an optional tag string to a frame: a flag byte (1 = present, 0 =
/// absent) and, when present, a u16-LE length (capped at [`MAX_TAG_LEN`]) + the
/// UTF-8 bytes.
fn push_opt_str(out: &mut Vec<u8>, s: &Option<String>) {
    match s {
        Some(v) => {
            let b = v.as_bytes();
            let n = b.len().min(MAX_TAG_LEN);
            out.push(1);
            out.extend_from_slice(&(n as u16).to_le_bytes());
            out.extend_from_slice(&b[..n]);
        }
        None => out.push(0),
    }
}

/// Read an optional tag string from the front of `bytes`, returning it and the
/// rest. Fail-closed: a bad flag, an over-long length, a truncated body, or
/// non-UTF-8 is an error, never a guess.
fn read_opt_str(bytes: &[u8]) -> Result<(Option<String>, &[u8]), AudioFrameError> {
    let (&flag, rest) = bytes.split_first().ok_or(AudioFrameError::Truncated)?;
    match flag {
        0 => Ok((None, rest)),
        1 => {
            let len_bytes = rest.get(..2).ok_or(AudioFrameError::Truncated)?;
            let len = u16::from_le_bytes([len_bytes[0], len_bytes[1]]) as usize;
            if len > MAX_TAG_LEN {
                return Err(AudioFrameError::TagTooLong);
            }
            let body = rest.get(2..2 + len).ok_or(AudioFrameError::Truncated)?;
            let s = std::str::from_utf8(body).map_err(|_| AudioFrameError::BadTag)?.to_string();
            Ok((Some(s), &rest[2 + len..]))
        }
        _ => Err(AudioFrameError::BadTag),
    }
}

/// Parse a worker probe frame the player read from a decoder's stdout.
pub fn decode_audio_frame(bytes: &[u8]) -> Result<AudioInfo, AudioFrameError> {
    // Fixed header is MAGIC(4) + sample_rate(4) + channels(2) + flag(1) +
    // duration(8) + codec_len(1) = 20 bytes.
    if bytes.len() < 20 {
        return Err(AudioFrameError::Truncated);
    }
    if &bytes[..4] != MAGIC {
        return Err(AudioFrameError::BadMagic);
    }
    let sample_rate = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let channels = u16::from_le_bytes([bytes[8], bytes[9]]);
    let flag = bytes[10];
    let dur = u64::from_le_bytes([
        bytes[11], bytes[12], bytes[13], bytes[14], bytes[15], bytes[16], bytes[17], bytes[18],
    ]);
    let duration_ms = if flag == 1 { Some(dur) } else { None };
    let codec_len = bytes[19] as usize;
    if codec_len > MAX_CODEC_LEN {
        return Err(AudioFrameError::CodecTooLong);
    }
    let codec_bytes = bytes.get(20..20 + codec_len).ok_or(AudioFrameError::Truncated)?;
    let codec = std::str::from_utf8(codec_bytes).map_err(|_| AudioFrameError::BadCodec)?.to_string();
    // The tag tail (title then artist). A frame that ends right after the codec
    // predates the tail and decodes to no tags (backward/forward-compatible).
    let tail = &bytes[20 + codec_len..];
    let (title, artist) = if tail.is_empty() {
        (None, None)
    } else {
        let (title, rest) = read_opt_str(tail)?;
        let (artist, _) = read_opt_str(rest)?;
        (title, artist)
    };
    Ok(AudioInfo { codec, sample_rate, channels, duration_ms, title, artist })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_with_a_known_duration_round_trips() {
        let info = AudioInfo { codec: "flac".into(), sample_rate: 44_100, channels: 2, duration_ms: Some(180_000), title: None, artist: None };
        assert_eq!(decode_audio_frame(&info.encode()).unwrap(), info);
    }

    #[test]
    fn a_frame_with_an_unknown_duration_round_trips() {
        let info = AudioInfo { codec: "vorbis".into(), sample_rate: 48_000, channels: 1, duration_ms: None, title: None, artist: None };
        let decoded = decode_audio_frame(&info.encode()).unwrap();
        assert_eq!(decoded.duration_ms, None);
        assert_eq!(decoded, info);
    }

    #[test]
    fn bad_magic_and_truncation_are_rejected() {
        let mut f = AudioInfo { codec: "pcm".into(), sample_rate: 8000, channels: 1, duration_ms: Some(1), title: None, artist: None }.encode();
        assert_eq!(decode_audio_frame(&f[..10]), Err(AudioFrameError::Truncated));
        f[0] = b'X';
        assert_eq!(decode_audio_frame(&f), Err(AudioFrameError::BadMagic));
    }

    #[test]
    fn an_overlong_codec_length_is_rejected() {
        let mut f = AudioInfo { codec: "mp3".into(), sample_rate: 44_100, channels: 2, duration_ms: None, title: None, artist: None }.encode();
        f[19] = (MAX_CODEC_LEN + 1) as u8; // claim a codec name longer than allowed
        assert_eq!(decode_audio_frame(&f), Err(AudioFrameError::CodecTooLong));
    }

    #[test]
    fn tags_round_trip_through_the_frame() {
        let info = AudioInfo {
            codec: "flac".into(),
            sample_rate: 44_100,
            channels: 2,
            duration_ms: Some(220_000),
            title: Some("Nightswim".into()),
            artist: Some("Unknown Artist".into()),
        };
        assert_eq!(decode_audio_frame(&info.encode()).unwrap(), info);
        // A title with no artist round-trips the mixed-presence case.
        let one = AudioInfo {
            codec: "mp3".into(),
            sample_rate: 48_000,
            channels: 2,
            duration_ms: None,
            title: Some("Solo".into()),
            artist: None,
        };
        assert_eq!(decode_audio_frame(&one.encode()).unwrap(), one);
    }

    #[test]
    fn a_pre_tags_frame_decodes_to_no_tags() {
        // A frame produced before the tag tail existed ends right after the codec;
        // it must decode cleanly with no tags (backward/forward compatibility).
        let info = AudioInfo { codec: "pcm".into(), sample_rate: 8000, channels: 1, duration_ms: Some(1000), title: None, artist: None };
        let full = info.encode();
        let codec_len = full[19] as usize;
        let legacy = &full[..20 + codec_len];
        let decoded = decode_audio_frame(legacy).unwrap();
        assert_eq!((decoded.title, decoded.artist, decoded.codec.as_str()), (None, None, "pcm"));
    }

    #[test]
    fn an_overlong_tag_length_is_rejected() {
        // Craft a frame whose title length claims more than MAX_TAG_LEN.
        let mut f = AudioInfo { codec: "x".into(), sample_rate: 8000, channels: 1, duration_ms: None, title: None, artist: None }.encode();
        let tag_flag = 20 + 1; // 20-byte header + the 1-byte codec "x"
        f.truncate(tag_flag);
        f.push(1); // title present
        f.extend_from_slice(&((MAX_TAG_LEN + 1) as u16).to_le_bytes());
        assert_eq!(decode_audio_frame(&f), Err(AudioFrameError::TagTooLong));
    }
}
