//! The native-messaging-style stdio host: the transport + mutual-identity-pin
//! boundary a foreign plugin connects to (foreign-app-bridges.md §1, piece 2).
//!
//! Messages are length-prefixed JSON over a byte stream (the native-messaging
//! framing: a 4-byte little-endian length, then that many bytes of JSON). The
//! host treats every inbound byte as untrusted: it bounds the frame size, pins
//! the plugin's declared id against the `bridge.toml` `allowed_plugin_id`
//! BEFORE accepting any ingest (a mismatch drops the connection), and routes
//! each ingest through the pure [`crate::interpret_message`] mapping. The
//! resolved [`UpsertPlan`] is handed to an injected [`PlanSink`] — the real
//! sink writes it through the bridge's macaroon-scoped, origin-tagged entity-
//! write (gated on the macaroon namespace-delegation, see the coder report);
//! tests inject a recording sink, so this transport + auth boundary is verified
//! independently of that write.

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::bridge::BridgeConfig;
use crate::interpret::{interpret_message, UpsertPlan};

/// The largest inbound frame the host will read. An untrusted plugin cannot
/// make the host allocate more than this per message.
pub const MAX_FRAME: usize = 1024 * 1024;

/// An inbound message from the foreign plugin. `hello` must come first to pin
/// the plugin's identity; `ingest` carries one mapped message.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InboundMessage {
    /// The identity handshake: the plugin declares its id, pinned against the
    /// bridge's `allowed_plugin_id`.
    Hello {
        /// The plugin's declared id (untrusted; pinned, never used as authority).
        plugin_id: String,
    },
    /// One ingestion message: a mapped type plus its flat value object.
    Ingest {
        /// The message type, looked up in the bridge map (never trusted for the
        /// node/edge names).
        msg_type: String,
        /// The message's flat field object (a pure value source).
        payload: Map<String, Value>,
    },
}

/// The host's reply to the plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutboundMessage {
    /// The handshake was accepted; ingests may follow.
    Ready,
    /// An ingest was interpreted and handed to the sink.
    Ingested {
        /// The stable external key the plan upserts.
        external_key: String,
        /// The qualified entity type the plan upserts.
        qualified_type: String,
        /// How many edges the plan creates.
        links: usize,
    },
    /// The message was refused; the connection may or may not continue (a
    /// handshake mismatch closes it, an ingest error does not).
    Error {
        /// A lay-readable reason (no internal detail).
        message: String,
    },
}

/// A host transport / authentication failure that ends the session.
#[derive(Debug, Error)]
pub enum HostError {
    /// A frame exceeded [`MAX_FRAME`] or had a malformed length.
    #[error("malformed frame")]
    BadFrame,
    /// Underlying stream I/O failed.
    #[error("io: {0}")]
    Io(#[from] io::Error),
}

/// The sink a resolved [`UpsertPlan`] is written through. The real sink writes
/// it via the bridge's macaroon-scoped, origin-tagged entity-write socket; a
/// test sink records the plans. Returning `Err(reason)` reports a write failure
/// to the plugin without ending the session.
pub trait PlanSink {
    /// Persist one resolved plan (origin-tagged as this bridge). The bridge
    /// name lets the sink stamp provenance.
    fn write_plan(&mut self, bridge: &str, plan: &UpsertPlan) -> Result<(), String>;
}

/// Read one length-prefixed frame: a 4-byte little-endian length, then that
/// many bytes. Returns `Ok(None)` at a clean end of stream (no bytes left),
/// `Err(BadFrame)` on an oversized or truncated length, propagating I/O errors.
pub fn read_frame<R: Read>(reader: &mut R, max: usize) -> Result<Option<Vec<u8>>, HostError> {
    let mut len_buf = [0u8; 4];
    match reader.read(&mut len_buf[..1]) {
        Ok(0) => return Ok(None), // clean EOF before any frame
        Ok(_) => {}
        Err(e) => return Err(HostError::Io(e)),
    }
    reader.read_exact(&mut len_buf[1..]).map_err(|_| HostError::BadFrame)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len == 0 || len > max {
        return Err(HostError::BadFrame);
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).map_err(|_| HostError::BadFrame)?;
    Ok(Some(body))
}

/// Write one length-prefixed frame (the [`read_frame`] inverse).
pub fn write_frame<W: Write>(writer: &mut W, bytes: &[u8]) -> Result<(), HostError> {
    if bytes.len() > MAX_FRAME {
        return Err(HostError::BadFrame);
    }
    let len = (bytes.len() as u32).to_le_bytes();
    writer.write_all(&len)?;
    writer.write_all(bytes)?;
    writer.flush()?;
    Ok(())
}

/// Serve one foreign-plugin connection to completion.
///
/// The state machine: the first accepted message must be `hello`, whose
/// `plugin_id` is pinned against `config.bridge.allowed_plugin_id`; a mismatch
/// writes an error and ENDS the session (fail-closed mutual id-pin). After the
/// handshake, each `ingest` is interpreted against the bridge map and handed to
/// `sink`; an ingest before the handshake, an unmapped type, a missing key, or
/// a malformed frame is reported as an error WITHOUT ending the session (a
/// single bad message must not drop a live sync). A clean EOF ends the session.
pub fn serve<R: Read, W: Write, S: PlanSink>(
    config: &BridgeConfig,
    reader: &mut R,
    writer: &mut W,
    sink: &mut S,
) -> Result<(), HostError> {
    let mut authenticated = false;
    loop {
        let frame = match read_frame(reader, MAX_FRAME)? {
            Some(f) => f,
            None => return Ok(()), // clean end of stream
        };
        let parsed: Result<InboundMessage, _> = serde_json::from_slice(&frame);
        let reply = match parsed {
            Err(_) => OutboundMessage::Error {
                message: "unparseable message".to_string(),
            },
            Ok(InboundMessage::Hello { plugin_id }) => {
                if authenticated {
                    OutboundMessage::Error {
                        message: "already authenticated".to_string(),
                    }
                } else if plugin_id == config.bridge.allowed_plugin_id {
                    authenticated = true;
                    OutboundMessage::Ready
                } else {
                    // Mutual id-pin mismatch: refuse and drop the connection.
                    let reply = OutboundMessage::Error {
                        message: "plugin id not allowed".to_string(),
                    };
                    write_reply(writer, &reply)?;
                    return Ok(());
                }
            }
            Ok(InboundMessage::Ingest { .. }) if !authenticated => OutboundMessage::Error {
                message: "handshake required".to_string(),
            },
            Ok(InboundMessage::Ingest { msg_type, payload }) => {
                match interpret_message(config, &msg_type, &payload) {
                    Ok(plan) => match sink.write_plan(&config.bridge.allowed_plugin_id, &plan) {
                        Ok(()) => OutboundMessage::Ingested {
                            external_key: plan.external_key,
                            qualified_type: plan.qualified_type,
                            links: plan.links.len(),
                        },
                        Err(_) => OutboundMessage::Error {
                            message: "write failed".to_string(),
                        },
                    },
                    Err(_) => OutboundMessage::Error {
                        message: "message not accepted by the bridge map".to_string(),
                    },
                }
            }
        };
        write_reply(writer, &reply)?;
    }
}

/// Frame and write one reply.
fn write_reply<W: Write>(writer: &mut W, reply: &OutboundMessage) -> Result<(), HostError> {
    let bytes = serde_json::to_vec(reply).map_err(|e| HostError::Io(io::Error::other(e)))?;
    write_frame(writer, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::BridgeConfig;

    const BRIDGE_TOML: &str = r#"
[bridge]
allowed_plugin_id = "maria-obsidian-bridge"
[map."note.upsert"]
upsert = "md.obsidian.Note"
key = "path"
set = { title = "$.title" }
for_each_link = { edge = "LINKS_TO", to_key = "path" }
"#;

    fn config() -> BridgeConfig {
        BridgeConfig::parse(BRIDGE_TOML).expect("fixture bridge.toml parses")
    }

    /// A recording sink: captures every plan the host hands it.
    #[derive(Default)]
    struct RecordingSink {
        plans: Vec<(String, UpsertPlan)>,
    }
    impl PlanSink for RecordingSink {
        fn write_plan(&mut self, bridge: &str, plan: &UpsertPlan) -> Result<(), String> {
            self.plans.push((bridge.to_string(), plan.clone()));
            Ok(())
        }
    }

    /// Concatenate framed messages into one input stream.
    fn framed(msgs: &[Value]) -> Vec<u8> {
        let mut buf = Vec::new();
        for m in msgs {
            let bytes = serde_json::to_vec(m).unwrap();
            buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(&bytes);
        }
        buf
    }

    /// Decode all framed replies the host wrote.
    fn replies(mut out: &[u8]) -> Vec<OutboundMessage> {
        let mut v = Vec::new();
        while let Some(frame) = read_frame(&mut out, MAX_FRAME).unwrap() {
            v.push(serde_json::from_slice(&frame).unwrap());
        }
        v
    }

    #[test]
    fn frame_round_trips() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"hello").unwrap();
        let mut slice = buf.as_slice();
        assert_eq!(read_frame(&mut slice, MAX_FRAME).unwrap().as_deref(), Some(&b"hello"[..]));
        assert!(read_frame(&mut slice, MAX_FRAME).unwrap().is_none());
    }

    #[test]
    fn oversized_frame_is_rejected() {
        // A length header claiming more than MAX_FRAME, with no body.
        let mut input: &[u8] = &((MAX_FRAME as u32) + 1).to_le_bytes();
        assert!(matches!(read_frame(&mut input, MAX_FRAME), Err(HostError::BadFrame)));
    }

    #[test]
    fn handshake_then_ingest_reaches_the_sink() {
        let input = framed(&[
            serde_json::json!({ "kind": "hello", "plugin_id": "maria-obsidian-bridge" }),
            serde_json::json!({
                "kind": "ingest",
                "msg_type": "note.upsert",
                "payload": { "path": "notes/a.md", "title": "A", "links": ["notes/b.md"] }
            }),
        ]);
        let mut out = Vec::new();
        let mut sink = RecordingSink::default();
        serve(&config(), &mut input.as_slice(), &mut out, &mut sink).unwrap();

        let r = replies(&out);
        assert_eq!(r[0], OutboundMessage::Ready);
        assert_eq!(
            r[1],
            OutboundMessage::Ingested {
                external_key: "notes/a.md".to_string(),
                qualified_type: "md.obsidian.Note".to_string(),
                links: 1,
            }
        );
        assert_eq!(sink.plans.len(), 1);
        assert_eq!(sink.plans[0].0, "maria-obsidian-bridge", "plan is origin-stamped to the bridge");
        assert_eq!(sink.plans[0].1.external_key, "notes/a.md");
    }

    #[test]
    fn a_wrong_plugin_id_is_refused_and_closes_the_session() {
        let input = framed(&[
            serde_json::json!({ "kind": "hello", "plugin_id": "impostor" }),
            // This ingest must never be processed: the session ended at the bad hello.
            serde_json::json!({
                "kind": "ingest",
                "msg_type": "note.upsert",
                "payload": { "path": "x", "title": "X" }
            }),
        ]);
        let mut out = Vec::new();
        let mut sink = RecordingSink::default();
        serve(&config(), &mut input.as_slice(), &mut out, &mut sink).unwrap();

        let r = replies(&out);
        assert_eq!(r.len(), 1, "only the rejection is written, then the session ends");
        assert!(matches!(r[0], OutboundMessage::Error { .. }));
        assert!(sink.plans.is_empty(), "no plan is written for an unauthenticated impostor");
    }

    #[test]
    fn ingest_before_handshake_is_refused_without_writing() {
        let input = framed(&[serde_json::json!({
            "kind": "ingest",
            "msg_type": "note.upsert",
            "payload": { "path": "x", "title": "X" }
        })]);
        let mut out = Vec::new();
        let mut sink = RecordingSink::default();
        serve(&config(), &mut input.as_slice(), &mut out, &mut sink).unwrap();

        let r = replies(&out);
        assert!(matches!(r[0], OutboundMessage::Error { .. }));
        assert!(sink.plans.is_empty());
    }

    #[test]
    fn an_unmapped_type_errors_but_keeps_the_session() {
        let input = framed(&[
            serde_json::json!({ "kind": "hello", "plugin_id": "maria-obsidian-bridge" }),
            serde_json::json!({ "kind": "ingest", "msg_type": "note.delete", "payload": { "path": "x" } }),
            serde_json::json!({
                "kind": "ingest",
                "msg_type": "note.upsert",
                "payload": { "path": "notes/c.md", "title": "C" }
            }),
        ]);
        let mut out = Vec::new();
        let mut sink = RecordingSink::default();
        serve(&config(), &mut input.as_slice(), &mut out, &mut sink).unwrap();

        let r = replies(&out);
        assert_eq!(r[0], OutboundMessage::Ready);
        assert!(matches!(r[1], OutboundMessage::Error { .. }), "unmapped type errors");
        // The session survived: the following valid ingest still lands.
        assert!(matches!(r[2], OutboundMessage::Ingested { .. }));
        assert_eq!(sink.plans.len(), 1);
        assert_eq!(sink.plans[0].1.external_key, "notes/c.md");
    }
}
