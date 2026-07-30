//! Framing for the terminal read socket: a 4-byte big-endian length followed by
//! JSON, the same shape the audit and undo protocols use.
//!
//! Deliberately transport-agnostic. Encoding is `T -> Vec<u8>` and decoding takes
//! the length header and the body as separate steps, so the wire format is
//! decided and tested here, in a crate CI builds, while the async read/write loop
//! stays at the edge with the socket. A codec that needed a runtime to test would
//! end up in the app crate, which nothing builds until someone runs the app.
//!
//! The length is read before the body is allocated, and a frame larger than
//! [`MAX_FRAME`] is refused at that point rather than after reading it. That
//! ordering is the whole reason the header exists: a peer must not be able to
//! make the reader allocate an arbitrary buffer by claiming a large size.

use serde::de::DeserializeOwned;
use serde::Serialize;

/// The largest frame either direction may carry.
///
/// A request is a terminal id, three small fields and a token, so it is well
/// under a kilobyte. A response carries at most [`MAX_BLOCKS`] blocks, each with
/// a command line and a body, so 4 MiB is generous for the largest legitimate
/// answer and still small enough that a hostile length is refused before it costs
/// anything.
///
/// [`MAX_BLOCKS`]: crate::read_scope::MAX_BLOCKS
pub const MAX_FRAME: usize = 4 * 1024 * 1024;

/// What went wrong reading or writing a frame.
#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    /// The declared length exceeds [`MAX_FRAME`]. Carries the claim, not the
    /// data, since by design nothing was read.
    TooLarge(usize),
    /// The body was not the JSON this side expected.
    Malformed,
    /// The value could not be encoded, which means a bug on this side rather
    /// than bad input.
    Unencodable,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge(n) => write!(f, "frame claims {n} bytes, over the {MAX_FRAME} cap"),
            Self::Malformed => write!(f, "frame body is not the expected JSON"),
            Self::Unencodable => write!(f, "value could not be encoded"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Encode `value` as a length-prefixed frame.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, FrameError> {
    let body = serde_json::to_vec(value).map_err(|_| FrameError::Unencodable)?;
    if body.len() > MAX_FRAME {
        return Err(FrameError::TooLarge(body.len()));
    }
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// The body length a 4-byte header declares, refused if over the cap.
///
/// Separate from [`decode_body`] on purpose: the caller checks the length, then
/// decides whether to read that many bytes. Fusing the two would mean reading
/// first and judging after, which is the allocation the cap exists to prevent.
pub fn decode_len(header: [u8; 4]) -> Result<usize, FrameError> {
    let n = u32::from_be_bytes(header) as usize;
    if n > MAX_FRAME {
        return Err(FrameError::TooLarge(n));
    }
    Ok(n)
}

/// Decode a frame body of the length [`decode_len`] returned.
pub fn decode_body<T: DeserializeOwned>(body: &[u8]) -> Result<T, FrameError> {
    serde_json::from_slice(body).map_err(|_| FrameError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read_serve::ReadRequest;

    fn req() -> ReadRequest {
        ReadRequest {
            terminal_id: "t1".to_string(),
            limit: 3,
            include_user_blocks: false,
            include_running: false,
            consent: "tok".to_string(),
        }
    }

    #[test]
    fn a_frame_round_trips() {
        let bytes = encode(&req()).expect("encodes");
        let len = decode_len([bytes[0], bytes[1], bytes[2], bytes[3]]).expect("length");
        assert_eq!(len, bytes.len() - 4);
        let back: ReadRequest = decode_body(&bytes[4..]).expect("decodes");
        assert_eq!(back, req());
    }

    #[test]
    fn an_oversized_claim_is_refused_from_the_header_alone() {
        // The point of the cap: this is judged from four bytes, before anything
        // is allocated or read.
        let header = (MAX_FRAME as u32 + 1).to_be_bytes();
        assert_eq!(decode_len(header), Err(FrameError::TooLarge(MAX_FRAME + 1)));
    }

    #[test]
    fn the_largest_allowed_claim_is_accepted() {
        // Off-by-one guard: the cap is inclusive, so a frame of exactly MAX_FRAME
        // is legal and only one more is not.
        assert_eq!(decode_len((MAX_FRAME as u32).to_be_bytes()), Ok(MAX_FRAME));
    }

    #[test]
    fn a_body_that_is_not_the_expected_shape_is_malformed_not_a_panic() {
        let got: Result<ReadRequest, _> = decode_body(b"{\"terminal_id\":true}");
        assert_eq!(got, Err(FrameError::Malformed));
    }

    #[test]
    fn a_truncated_body_is_malformed() {
        let bytes = encode(&req()).expect("encodes");
        let got: Result<ReadRequest, _> = decode_body(&bytes[4..bytes.len() - 3]);
        assert_eq!(got, Err(FrameError::Malformed));
    }
}
