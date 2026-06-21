//! The intake socket transport: length-prefixed [`RequestBody`] frames in, the
//! [`IntakeReply`] framed back. Mirrors the bridge-ingest / undo-signer serve
//! loops. This is the transport boundary that feeds the pure dispatch in
//! [`crate::service`]; the daemon's accept loop resolves the peer's attested app
//! id (SO_PEERCRED -> `arlen_permissions` `path_to_app_id`) and passes it in -
//! the only source of requester identity. The trusted surface that resolves a
//! queued request into a decision (and the channel that returns that decision to
//! the waiting requester) is a later piece; this only does request -> enqueue ->
//! reply.

use std::io::{self, Read, Write};

use arlen_ai_core::capability::Capability;

use crate::queue::ConsentQueue;
use crate::service::{handle_intake, IntakeReply, RequestBody};

/// The largest inbound frame accepted. A consent request is small; an untrusted
/// client cannot make the broker allocate more than this per message.
pub const MAX_FRAME: usize = 64 * 1024;

/// Read one length-prefixed frame (4-byte little-endian length, then the body).
/// `Ok(None)` at a clean end of stream; `Err` on an oversized/truncated length.
pub fn read_frame<R: Read>(reader: &mut R, max: usize) -> io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    if reader.read(&mut len_buf[..1])? == 0 {
        return Ok(None); // clean EOF before any frame
    }
    reader.read_exact(&mut len_buf[1..])?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len == 0 || len > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame length out of bounds"));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

/// Write one length-prefixed frame (the [`read_frame`] inverse).
pub fn write_frame<W: Write>(writer: &mut W, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "reply too large"));
    }
    writer.write_all(&(bytes.len() as u32).to_le_bytes())?;
    writer.write_all(bytes)?;
    writer.flush()
}

/// Serve one connection's intake messages. `attested_app_id` MUST be the value
/// the accept loop resolved from SO_PEERCRED (`path_to_app_id`); every request
/// on this connection is attributed to it (the requester is never read from the
/// payload). Each framed [`RequestBody`] is dispatched through [`handle_intake`]
/// (enqueuing under the shared `queue`) and the [`IntakeReply`] framed back. A
/// malformed body is reported as an error reply without ending the connection; a
/// clean EOF ends it.
pub fn serve_intake<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    attested_app_id: &str,
    capability: &Capability,
    queue: &mut ConsentQueue,
) -> io::Result<()> {
    loop {
        let frame = match read_frame(reader, MAX_FRAME)? {
            Some(f) => f,
            None => return Ok(()),
        };
        let reply_bytes = match serde_json::from_slice::<RequestBody>(&frame) {
            Ok(body) => {
                let reply: IntakeReply = handle_intake(body, attested_app_id, capability, queue);
                serde_json::to_vec(&reply)
            }
            Err(_) => serde_json::to_vec(&ErrorReply {
                reply: "error",
                message: "unparseable request",
            }),
        };
        let reply_bytes =
            reply_bytes.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        write_frame(writer, &reply_bytes)?;
    }
}

/// The error reply shape (a malformed request that does not parse to a
/// [`RequestBody`]). Distinct from [`IntakeReply`] so a client can tell a
/// protocol error from a real outcome.
#[derive(serde::Serialize)]
struct ErrorReply {
    reply: &'static str,
    message: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConsentClass;
    use arlen_ai_core::capability::{AccessTier, ActionKind, ActionPermissions, BaselineMode};

    fn cap_suggest() -> Capability {
        Capability::new(
            AccessTier::Minimal,
            ActionPermissions::new(BaselineMode::Suggest, Vec::<String>::new()),
        )
    }

    fn cap_autonomous(app: &str) -> Capability {
        Capability::new(
            AccessTier::Minimal,
            ActionPermissions::new(BaselineMode::Suggest, [app.to_string()]),
        )
    }

    fn framed(bodies: &[RequestBody]) -> Vec<u8> {
        let mut buf = Vec::new();
        for b in bodies {
            let bytes = serde_json::to_vec(b).unwrap();
            buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(&bytes);
        }
        buf
    }

    fn replies(mut out: &[u8]) -> Vec<IntakeReply> {
        let mut v = Vec::new();
        while let Some(frame) = read_frame(&mut out, MAX_FRAME).unwrap() {
            // Only IntakeReply frames in these tests (no malformed bodies).
            v.push(serde_json::from_slice(&frame).unwrap());
        }
        v
    }

    fn body(kind: ActionKind) -> RequestBody {
        RequestBody {
            class: ConsentClass::CapabilityGrant,
            kind,
            triggered_by_external_content: false,
            summary: "s".to_string(),
            scope: None,
        }
    }

    #[test]
    fn frame_round_trips() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"abc").unwrap();
        let mut slice = buf.as_slice();
        assert_eq!(read_frame(&mut slice, MAX_FRAME).unwrap().as_deref(), Some(&b"abc"[..]));
        assert!(read_frame(&mut slice, MAX_FRAME).unwrap().is_none());
    }

    #[test]
    fn oversized_frame_is_rejected() {
        let mut input: &[u8] = &((MAX_FRAME as u32) + 1).to_le_bytes();
        assert!(read_frame(&mut input, MAX_FRAME).is_err());
    }

    #[test]
    fn a_request_is_enqueued_under_the_attested_peer_and_replies_queued() {
        let input = framed(&[body(ActionKind::PermanentDelete)]);
        let mut out = Vec::new();
        let mut q = ConsentQueue::new();
        serve_intake(&mut input.as_slice(), &mut out, "org.arlen.files", &cap_suggest(), &mut q).unwrap();
        let r = replies(&out);
        assert!(matches!(r[0], IntakeReply::Queued { .. }));
        assert_eq!(q.front().unwrap().request.requester.grant_recipient(), "org.arlen.files");
    }

    #[test]
    fn a_silent_request_replies_silent_granted_and_is_not_queued() {
        let input = framed(&[body(ActionKind::Ordinary)]);
        let mut out = Vec::new();
        let mut q = ConsentQueue::new();
        serve_intake(&mut input.as_slice(), &mut out, "org.arlen.files", &cap_autonomous("org.arlen.files"), &mut q).unwrap();
        assert_eq!(replies(&out)[0], IntakeReply::SilentGranted);
        assert!(q.is_empty());
    }

    #[test]
    fn a_malformed_frame_errors_without_ending_the_connection() {
        // A garbage frame, then a valid request: the valid one must still land.
        let mut input = Vec::new();
        let garbage = b"not json";
        input.extend_from_slice(&(garbage.len() as u32).to_le_bytes());
        input.extend_from_slice(garbage);
        input.extend_from_slice(&framed(&[body(ActionKind::PermanentDelete)]));
        let mut out = Vec::new();
        let mut q = ConsentQueue::new();
        serve_intake(&mut input.as_slice(), &mut out, "x", &cap_suggest(), &mut q).unwrap();
        // Two frames written: an error then the queued reply; the request landed.
        let mut slice = out.as_slice();
        let first = read_frame(&mut slice, MAX_FRAME).unwrap().unwrap();
        assert!(serde_json::from_slice::<IntakeReply>(&first).is_err(), "first reply is the error shape");
        let second = read_frame(&mut slice, MAX_FRAME).unwrap().unwrap();
        assert!(matches!(serde_json::from_slice::<IntakeReply>(&second).unwrap(), IntakeReply::Queued { .. }));
        assert_eq!(q.len(), 1);
    }
}
