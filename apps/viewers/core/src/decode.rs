//! The decoder-worker image-transfer protocol (`quickview-plan.md`).
//!
//! Each image format is decoded in its own sandboxed worker process (bwrap, no
//! write/network, seccomp); the worker reads the encoded file from stdin and
//! writes the decoded raster back over stdout in this frame, which the viewer
//! reads. Keeping the wire shape here (pure + validated) lets the viewer trust a
//! decoder's output by construction: a hostile/garbled worker frame is rejected,
//! never turned into an out-of-bounds raster. The frame is deliberately trivial
//! (a tiny header + raw RGBA), so the worker stays minimal and the viewer does
//! no format parsing itself - that is the whole point of the isolation.

/// 8-bit RGBA, row-major, `width * height * 4` bytes. The one raster type the
/// viewer renders, whatever the source format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes of RGBA.
    pub rgba: Vec<u8>,
}

/// The largest raster the viewer accepts from a worker: a DoS bound on a
/// decoder's claimed dimensions (a hostile worker cannot make the viewer
/// allocate unbounded memory). 64M pixels = 256 MiB of RGBA, ample for any real
/// photo, far below a memory-exhaustion frame.
pub const MAX_PIXELS: u64 = 64 * 1024 * 1024;

/// The frame magic: identifies a well-formed worker raster frame.
const MAGIC: &[u8; 4] = b"ARV1";

/// A malformed or oversized worker frame (fail-closed: the viewer renders
/// nothing rather than a corrupt/over-large raster).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The frame did not start with the expected magic.
    BadMagic,
    /// The frame was shorter than its header / declared raster.
    Truncated,
    /// `width * height` exceeds [`MAX_PIXELS`].
    TooLarge,
    /// The RGBA length did not equal `width * height * 4`.
    SizeMismatch,
}

impl DecodedImage {
    /// Build a raster, checking the `rgba` length equals `width * height * 4`.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, DecodeError> {
        let pixels = u64::from(width) * u64::from(height);
        if pixels > MAX_PIXELS {
            return Err(DecodeError::TooLarge);
        }
        if rgba.len() as u64 != pixels * 4 {
            return Err(DecodeError::SizeMismatch);
        }
        Ok(Self { width, height, rgba })
    }

    /// Encode as a worker frame: `MAGIC` + width(LE u32) + height(LE u32) + RGBA.
    /// The worker writes this to stdout.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + self.rgba.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&self.rgba);
        out
    }
}

/// Parse a worker frame the viewer read from a decoder's stdout, validating the
/// magic, the dimension bound, and that the RGBA length matches the declared
/// dimensions exactly.
pub fn decode_frame(bytes: &[u8]) -> Result<DecodedImage, DecodeError> {
    if bytes.len() < 12 {
        return Err(DecodeError::Truncated);
    }
    if &bytes[..4] != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let width = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let height = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_PIXELS {
        return Err(DecodeError::TooLarge);
    }
    let body = &bytes[12..];
    if body.len() as u64 != pixels * 4 {
        return Err(DecodeError::SizeMismatch);
    }
    Ok(DecodedImage { width, height, rgba: body.to_vec() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32) -> DecodedImage {
        DecodedImage::new(w, h, vec![0xAB; (w * h * 4) as usize]).unwrap()
    }

    #[test]
    fn a_frame_round_trips() {
        let original = img(3, 2);
        assert_eq!(decode_frame(&original.encode()).unwrap(), original);
    }

    #[test]
    fn new_rejects_a_size_mismatch() {
        assert_eq!(DecodedImage::new(2, 2, vec![0; 4]), Err(DecodeError::SizeMismatch));
    }

    #[test]
    fn new_rejects_an_oversized_raster() {
        // 100M pixels > MAX_PIXELS; rejected before any allocation of the body.
        assert_eq!(DecodedImage::new(10_000, 10_000, vec![]), Err(DecodeError::TooLarge));
    }

    #[test]
    fn decode_frame_rejects_bad_magic() {
        let mut f = img(1, 1).encode();
        f[0] = b'X';
        assert_eq!(decode_frame(&f), Err(DecodeError::BadMagic));
    }

    #[test]
    fn decode_frame_rejects_truncation() {
        assert_eq!(decode_frame(b"ARV1\x01"), Err(DecodeError::Truncated));
        // A header claiming 2x2 but with a short body.
        let mut f = MAGIC.to_vec();
        f.extend_from_slice(&2u32.to_le_bytes());
        f.extend_from_slice(&2u32.to_le_bytes());
        f.extend_from_slice(&[0u8; 4]); // only one pixel, not four
        assert_eq!(decode_frame(&f), Err(DecodeError::SizeMismatch));
    }

    #[test]
    fn decode_frame_rejects_an_oversized_declared_dimension() {
        // A header claiming a huge raster is rejected on the bound, not by
        // trying to match a 256GB body.
        let mut f = MAGIC.to_vec();
        f.extend_from_slice(&65_536u32.to_le_bytes());
        f.extend_from_slice(&65_536u32.to_le_bytes()); // 4G pixels
        f.extend_from_slice(&[0u8; 16]);
        assert_eq!(decode_frame(&f), Err(DecodeError::TooLarge));
    }
}
