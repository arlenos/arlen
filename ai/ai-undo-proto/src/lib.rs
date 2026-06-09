//! The agent-to-signer undo-log wire protocol
//! (reversible-receipts-and-the-effect-model.md §6).
//!
//! The agent submits inverse-receipt entries and lifecycle transitions over a
//! socket to the separate-uid signer, and looks up sealed state back. This crate
//! is the one definition both sides share: the [`Request`]/[`Response`] types,
//! their validation, and the length-prefixed async framing. It is thin and
//! transport-shaped (the same role as `audit-proto`); the signer's serve loop and
//! its peer-auth gate, and the agent-side client, live in their own crates and
//! depend on this.
//!
//! Confidentiality and integrity against a same-uid compromised agent come from
//! the signer being a *different uid* (it owns the key and the log), not from
//! this protocol; the protocol only carries already-authorised submissions and
//! authorised lookups across the uid boundary.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use arlen_ai_undo_core::undo_log::{UndoEntry, UndoState};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Max framed message length: a created entry carries two canonical paths or a
/// setting value, all small, so 64 KiB is generous and bounds a hostile peer's
/// allocation before the body is read.
pub const MAX_FRAME: usize = 64 * 1024;

/// Max length of an `op_id` (a content hash, in practice ~64-71 chars).
pub const MAX_OP_ID_LEN: usize = 128;

/// Max length of a `correlation_id` (a gate-decision id).
pub const MAX_CORRELATION_ID_LEN: usize = 128;

/// A protocol error: a framing violation, a serialisation failure, or an I/O
/// error on the stream.
#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    /// A frame whose declared length is zero or exceeds [`MAX_FRAME`].
    #[error("frame error: {0}")]
    Frame(String),
    /// A request that failed validation (an over-long id).
    #[error("invalid request: {0}")]
    Invalid(String),
    /// A serde (de)serialisation failure.
    #[error("codec error: {0}")]
    Codec(String),
    /// An underlying I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// The protocol result alias.
pub type Result<T> = std::result::Result<T, ProtoError>;

/// A request from the agent to the signer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Seal a newly-created entry (its lifecycle begins `InFlight`).
    SubmitCreated(UndoEntry),
    /// Record a lifecycle transition for an existing entry.
    Transition {
        /// The entry whose state changes.
        op_id: String,
        /// The new state.
        state: UndoState,
    },
    /// Look up the current folded state of an entry.
    LookupState {
        /// The entry to look up.
        op_id: String,
    },
    /// Look up the immutable created data of an entry (the captured inverse).
    LookupEntry {
        /// The entry to look up.
        op_id: String,
    },
}

impl Request {
    /// Validate the request's bounded fields. The frame length already bounds the
    /// whole message; this rejects an over-long id before it is used as a log
    /// key, defense-in-depth against a malformed submission.
    pub fn validate(&self) -> Result<()> {
        let check = |label: &str, s: &str, max: usize| -> Result<()> {
            if s.is_empty() {
                return Err(ProtoError::Invalid(format!("{label} is empty")));
            }
            if s.len() > max {
                return Err(ProtoError::Invalid(format!(
                    "{label} length {} exceeds {max}",
                    s.len()
                )));
            }
            Ok(())
        };
        match self {
            Request::SubmitCreated(entry) => {
                check("op_id", &entry.op_id, MAX_OP_ID_LEN)?;
                check("correlation_id", &entry.correlation_id, MAX_CORRELATION_ID_LEN)?;
            }
            Request::Transition { op_id, .. }
            | Request::LookupState { op_id }
            | Request::LookupEntry { op_id } => {
                check("op_id", op_id, MAX_OP_ID_LEN)?;
            }
        }
        Ok(())
    }
}

/// The folded state of a looked-up entry, with no leaked detail beyond
/// present/absent/corrupt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateReply {
    /// No entry with that id was ever sealed.
    Absent,
    /// The entry's current folded lifecycle state.
    Present(UndoState),
    /// The entry exists but its record chain folds to an illegal sequence
    /// (fail-closed: a corrupt or forged chain yields this, never a guessed state).
    Corrupt,
}

/// A response from the signer to the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    /// A submit or transition was sealed.
    Sealed,
    /// The result of a [`Request::LookupState`].
    State(StateReply),
    /// The result of a [`Request::LookupEntry`]: the sealed entry, or `None`.
    Entry(Option<UndoEntry>),
    /// The request was rejected or failed; the message is a coarse reason.
    Error(String),
}

/// Read one length-prefixed frame: a 4-byte big-endian length, then that many
/// body bytes. Rejects a zero or over-[`MAX_FRAME`] length before allocating.
pub async fn read_frame<S>(stream: &mut S) -> Result<Vec<u8>>
where
    S: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(ProtoError::Frame(format!(
            "frame length {len} out of range (1..={MAX_FRAME})"
        )));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok(body)
}

/// Write one length-prefixed frame, enforcing the same bounds the reader does
/// before any byte is sent.
pub async fn write_frame<S>(stream: &mut S, body: &[u8]) -> Result<()>
where
    S: AsyncWriteExt + Unpin,
{
    if body.is_empty() || body.len() > MAX_FRAME {
        return Err(ProtoError::Frame(format!(
            "frame length {} out of range (1..={MAX_FRAME})",
            body.len()
        )));
    }
    let len = u32::try_from(body.len())
        .map_err(|_| ProtoError::Frame("frame exceeds u32 length".to_string()))?;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    Ok(())
}

/// Read and decode a [`Request`] frame (does not validate; the caller validates).
pub async fn read_request<S>(stream: &mut S) -> Result<Request>
where
    S: AsyncReadExt + Unpin,
{
    let body = read_frame(stream).await?;
    serde_json::from_slice(&body).map_err(|e| ProtoError::Codec(e.to_string()))
}

/// Encode and write a [`Request`] frame.
pub async fn write_request<S>(stream: &mut S, request: &Request) -> Result<()>
where
    S: AsyncWriteExt + Unpin,
{
    let body = serde_json::to_vec(request).map_err(|e| ProtoError::Codec(e.to_string()))?;
    write_frame(stream, &body).await
}

/// Read and decode a [`Response`] frame.
pub async fn read_response<S>(stream: &mut S) -> Result<Response>
where
    S: AsyncReadExt + Unpin,
{
    let body = read_frame(stream).await?;
    serde_json::from_slice(&body).map_err(|e| ProtoError::Codec(e.to_string()))
}

/// Encode and write a [`Response`] frame.
pub async fn write_response<S>(stream: &mut S, response: &Response) -> Result<()>
where
    S: AsyncWriteExt + Unpin,
{
    let body = serde_json::to_vec(response).map_err(|e| ProtoError::Codec(e.to_string()))?;
    write_frame(stream, &body).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_ai_undo_core::effect_model::{CanonicalPath, InverseReceipt};

    fn entry(op: &str) -> UndoEntry {
        UndoEntry {
            op_id: op.to_string(),
            correlation_id: "run".to_string(),
            inverse: InverseReceipt::RestorePath {
                now: CanonicalPath::new("/b/x").unwrap(),
                prior: CanonicalPath::new("/a/x").unwrap(),
            },
        }
    }

    #[tokio::test]
    async fn requests_round_trip_through_frame_and_json() {
        let cases = vec![
            Request::SubmitCreated(entry("op-1")),
            Request::Transition { op_id: "op-1".into(), state: UndoState::Committed },
            Request::LookupState { op_id: "op-1".into() },
            Request::LookupEntry { op_id: "op-1".into() },
        ];
        for req in cases {
            let (mut a, mut b) = tokio::io::duplex(4096);
            write_request(&mut a, &req).await.unwrap();
            let got = read_request(&mut b).await.unwrap();
            assert_eq!(got, req);
        }
    }

    #[tokio::test]
    async fn responses_round_trip_through_frame_and_json() {
        let cases = vec![
            Response::Sealed,
            Response::State(StateReply::Absent),
            Response::State(StateReply::Present(UndoState::Compensated)),
            Response::State(StateReply::Corrupt),
            Response::Entry(Some(entry("op-1"))),
            Response::Entry(None),
            Response::Error("denied".into()),
        ];
        for resp in cases {
            let (mut a, mut b) = tokio::io::duplex(4096);
            write_response(&mut a, &resp).await.unwrap();
            let got = read_response(&mut b).await.unwrap();
            assert_eq!(got, resp);
        }
    }

    #[test]
    fn validate_rejects_empty_and_over_long_ids() {
        assert!(Request::LookupState { op_id: String::new() }.validate().is_err());
        let long = "x".repeat(MAX_OP_ID_LEN + 1);
        assert!(Request::LookupState { op_id: long.clone() }.validate().is_err());
        let mut e = entry("op-1");
        e.correlation_id = "y".repeat(MAX_CORRELATION_ID_LEN + 1);
        assert!(Request::SubmitCreated(e).validate().is_err());
        // A normal request validates.
        assert!(Request::SubmitCreated(entry("op-1")).validate().is_ok());
    }

    #[tokio::test]
    async fn write_frame_enforces_the_same_bounds_as_read() {
        let (mut a, _b) = tokio::io::duplex(64);
        assert!(write_frame(&mut a, b"").await.is_err(), "empty rejected");
        let oversized = vec![0u8; MAX_FRAME + 1];
        assert!(write_frame(&mut a, &oversized).await.is_err(), "oversized rejected");
    }

    #[tokio::test]
    async fn read_frame_rejects_an_oversized_declared_length() {
        // A 4-byte length header declaring more than MAX_FRAME, with no body.
        let header = ((MAX_FRAME + 1) as u32).to_be_bytes();
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&header).await.unwrap();
        a.flush().await.unwrap();
        assert!(read_frame(&mut b).await.is_err(), "oversized declared length rejected");
    }
}
