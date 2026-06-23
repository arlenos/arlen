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
}

/// The largest codec name accepted in a frame (a bound on the variable field).
const MAX_CODEC_LEN: usize = 64;
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
        out
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
    Ok(AudioInfo { codec, sample_rate, channels, duration_ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_with_a_known_duration_round_trips() {
        let info = AudioInfo { codec: "flac".into(), sample_rate: 44_100, channels: 2, duration_ms: Some(180_000) };
        assert_eq!(decode_audio_frame(&info.encode()).unwrap(), info);
    }

    #[test]
    fn a_frame_with_an_unknown_duration_round_trips() {
        let info = AudioInfo { codec: "vorbis".into(), sample_rate: 48_000, channels: 1, duration_ms: None };
        let decoded = decode_audio_frame(&info.encode()).unwrap();
        assert_eq!(decoded.duration_ms, None);
        assert_eq!(decoded, info);
    }

    #[test]
    fn bad_magic_and_truncation_are_rejected() {
        let mut f = AudioInfo { codec: "pcm".into(), sample_rate: 8000, channels: 1, duration_ms: Some(1) }.encode();
        assert_eq!(decode_audio_frame(&f[..10]), Err(AudioFrameError::Truncated));
        f[0] = b'X';
        assert_eq!(decode_audio_frame(&f), Err(AudioFrameError::BadMagic));
    }

    #[test]
    fn an_overlong_codec_length_is_rejected() {
        let mut f = AudioInfo { codec: "mp3".into(), sample_rate: 44_100, channels: 2, duration_ms: None }.encode();
        f[19] = (MAX_CODEC_LEN + 1) as u8; // claim a codec name longer than allowed
        assert_eq!(decode_audio_frame(&f), Err(AudioFrameError::CodecTooLong));
    }
}
