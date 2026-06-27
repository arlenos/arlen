// SPDX-FileCopyrightText: 2026 Tim Kicker
// SPDX-License-Identifier: Apache-2.0

//! The APC sidecar wire format (terminal.md §3, lines 319-320).
//!
//! An artifact travels to the terminal as TWO legs (the degradation guarantee):
//! first the plain `text` leg (printed visibly, so a pager, a pipe or a non-Arlen
//! terminal shows readable output), then this sidecar leg - the artifact envelope
//! base64'd and wrapped in APC (Application Program Command) frames. A terminal
//! that does not understand APC discards the whole `ESC _ ... ESC \` span per
//! ECMA-48, so the rich payload is invisible there and only the text leg shows; an
//! Arlen terminal recognises the frames, reassembles the envelope and renders the
//! rich view in place of the text leg.
//!
//! We frame as **APC with the literal sentinel `ARLEN-ART1`**, deliberately
//! avoiding numeric OSC space (1337 iTerm2, 633/133 VS Code, OSC 8 hyperlinks) and
//! Kitty's `APC G` graphics introducer, so the sidecar collides with no existing
//! terminal protocol. The frame payload is the base64 (standard alphabet) of the
//! canonical JSON envelope ([`Artifact::to_json`]), chunked at
//! [`CHUNK_BYTES`] base64 bytes per frame with a Kitty-style `m=<0|1>;`
//! continuation flag (`m=1` = more follows, `m=0` or absent = final). The decoder
//! reassembles, base64-decodes and routes through [`Artifact::receive_json`], so
//! the reconstructed artifact is re-stamped [`crate::ArtifactOrigin::ExternalContent`]
//! - a sidecar can never assert a trusted origin.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use crate::{Artifact, ArtifactError};

/// APC introducer: `ESC _`.
pub const APC: &[u8] = b"\x1b_";
/// String Terminator: `ESC \`.
pub const ST: &[u8] = b"\x1b\\";
/// The Arlen artifact sidecar sentinel + version. Deliberately a literal private
/// string, never a numeric OSC, to avoid every existing registry (1337, 633, 133,
/// 8) and Kitty's `APC G`.
pub const SENTINEL: &str = "ARLEN-ART1";
/// Base64 bytes per APC frame. Kitty's precedent (~4096) keeps each frame under a
/// terminal's input-buffer limit; base64 chars are single-byte ASCII so any byte
/// boundary is a safe split.
pub const CHUNK_BYTES: usize = 4096;

/// Encode a canonical JSON envelope into the full APC sidecar byte stream (all
/// frames concatenated in order). The envelope is base64'd (standard alphabet) and
/// split into [`CHUNK_BYTES`] chunks; each chunk becomes one
/// `ESC _ ARLEN-ART1;m=<0|1>;<chunk> ESC \` frame, with `m=1` on every frame but
/// the last and `m=0` on the last. A single-frame artifact gets one `m=0` frame.
pub fn encode_frames(envelope_json: &[u8]) -> Vec<u8> {
    let b64 = STANDARD.encode(envelope_json);
    let bytes = b64.as_bytes();
    let mut out = Vec::new();
    // chunks() never yields an empty slice for non-empty input; an empty envelope
    // (never produced by to_json) would yield zero frames, so emit one empty final
    // frame to keep the stream decodable in that degenerate case.
    if bytes.is_empty() {
        push_frame(&mut out, 0, b"");
        return out;
    }
    let chunks: Vec<&[u8]> = bytes.chunks(CHUNK_BYTES).collect();
    let last = chunks.len() - 1;
    for (i, chunk) in chunks.iter().enumerate() {
        let m = if i == last { 0 } else { 1 };
        push_frame(&mut out, m, chunk);
    }
    out
}

/// Append one APC frame to `out`.
fn push_frame(out: &mut Vec<u8>, m: u8, chunk: &[u8]) {
    out.extend_from_slice(APC);
    out.extend_from_slice(SENTINEL.as_bytes());
    out.extend_from_slice(b";m=");
    out.push(b'0' + m);
    out.push(b';');
    out.extend_from_slice(chunk);
    out.extend_from_slice(ST);
}

/// Decode the first artifact from an APC sidecar byte stream: scan for
/// `ARLEN-ART1` frames (foreign APC sequences, e.g. Kitty graphics, are skipped),
/// accumulate their base64 chunks in order until the final (`m=0`) frame,
/// base64-decode and route through [`Artifact::receive_json`] - so the result is
/// re-stamped [`crate::ArtifactOrigin::ExternalContent`], never a trusted origin
/// the stream claimed. Used by tests now and the terminal engine later.
pub fn decode_frames(stream: &[u8]) -> Result<Artifact, ArtifactError> {
    let mut acc: Vec<u8> = Vec::new();
    let mut saw_any = false;
    let mut saw_final = false;
    let mut pos = 0;

    while let Some(rel) = find(&stream[pos..], APC) {
        let frame_start = pos + rel + APC.len();
        let Some(end_rel) = find(&stream[frame_start..], ST) else {
            return Err(ArtifactError::Malformed(
                "unterminated APC sidecar frame".into(),
            ));
        };
        let inner = &stream[frame_start..frame_start + end_rel];
        pos = frame_start + end_rel + ST.len();

        let sentinel = SENTINEL.as_bytes();
        if !inner.starts_with(sentinel) || inner.get(sentinel.len()) != Some(&b';') {
            // A foreign APC sequence (not ours) - skip it.
            continue;
        }
        saw_any = true;
        let rest = &inner[sentinel.len() + 1..];
        let (m, chunk) = parse_continuation(rest)?;
        acc.extend_from_slice(chunk);
        if m == 0 {
            saw_final = true;
            break;
        }
    }

    if !saw_any {
        return Err(ArtifactError::Malformed("no artifact sidecar found".into()));
    }
    if !saw_final {
        return Err(ArtifactError::Malformed(
            "truncated sidecar: no final frame".into(),
        ));
    }
    let json = STANDARD
        .decode(&acc)
        .map_err(|e| ArtifactError::Malformed(format!("base64: {e}")))?;
    Artifact::receive_json(&json)
}

/// Parse the optional `m=<0|1>;` continuation flag at the start of a frame's
/// post-sentinel bytes. Returns `(m, chunk)`; an absent flag means a final (`m=0`)
/// single-frame artifact, with the whole remainder as the chunk.
fn parse_continuation(rest: &[u8]) -> Result<(u8, &[u8]), ArtifactError> {
    if let Some(after) = rest.strip_prefix(b"m=") {
        // Expect a single digit then ';'.
        match (after.first(), after.get(1)) {
            (Some(d @ (b'0' | b'1')), Some(b';')) => Ok((d - b'0', &after[2..])),
            _ => Err(ArtifactError::Malformed(
                "malformed continuation flag".into(),
            )),
        }
    } else {
        Ok((0, rest))
    }
}

/// The index of the first occurrence of `needle` in `haystack`, or `None`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArtifactOrigin, ArtifactPayload};

    fn artifact(text: &str) -> Artifact {
        Artifact::new(
            ArtifactPayload::Markdown {
                source: "body".into(),
            },
            text.into(),
            ArtifactOrigin::AgentGenerated,
            None,
        )
        .unwrap()
    }

    #[test]
    fn encode_decode_is_identity_modulo_origin() {
        let art = artifact("hello");
        let stream = encode_frames(&art.to_json());
        let back = decode_frames(&stream).unwrap();
        // Payload + text survive; the origin is re-stamped ExternalContent by the
        // decoder (the sidecar cannot assert a trusted origin).
        assert_eq!(back.payload, art.payload);
        assert_eq!(back.text, art.text);
        assert_eq!(back.meta.origin, ArtifactOrigin::ExternalContent);
    }

    #[test]
    fn decode_re_stamps_external_content_even_from_a_trusted_envelope() {
        // Encode a SystemTrusted envelope; the decoder must still yield
        // ExternalContent - a sidecar can never elevate trust.
        let art = Artifact::new(
            ArtifactPayload::Markdown { source: "x".into() },
            "x".into(),
            ArtifactOrigin::SystemTrusted,
            None,
        )
        .unwrap();
        let back = decode_frames(&encode_frames(&art.to_json())).unwrap();
        assert_eq!(back.meta.origin, ArtifactOrigin::ExternalContent);
    }

    #[test]
    fn multi_frame_path_round_trips() {
        // A large text forces the base64 stream past CHUNK_BYTES, exercising the
        // multi-frame accumulation + the chunk-boundary split (the most likely bug).
        let big = "x".repeat(CHUNK_BYTES * 3);
        let art = artifact(&big);
        let stream = encode_frames(&art.to_json());
        // More than one frame was emitted.
        let frame_count = stream.windows(APC.len()).filter(|w| *w == APC).count();
        assert!(frame_count > 1, "expected multiple frames, got {frame_count}");
        let back = decode_frames(&stream).unwrap();
        assert_eq!(back.text, big);
    }

    #[test]
    fn foreign_apc_is_skipped() {
        // A Kitty-style APC G frame interleaved before ours must be ignored.
        let art = artifact("real");
        let mut stream = Vec::new();
        stream.extend_from_slice(APC);
        stream.extend_from_slice(b"Gf=100,a=T;junkpayload");
        stream.extend_from_slice(ST);
        stream.extend_from_slice(&encode_frames(&art.to_json()));
        let back = decode_frames(&stream).unwrap();
        assert_eq!(back.text, "real");
    }

    #[test]
    fn no_sidecar_is_malformed() {
        assert!(matches!(
            decode_frames(b"just plain text, no apc"),
            Err(ArtifactError::Malformed(_))
        ));
    }

    #[test]
    fn truncated_stream_without_final_frame_is_malformed() {
        let art = artifact(&"y".repeat(CHUNK_BYTES * 2));
        let stream = encode_frames(&art.to_json());
        // Keep only the first frame (m=1, never a final m=0).
        let first_st = find(&stream, ST).unwrap() + ST.len();
        let truncated = &stream[..first_st];
        assert!(matches!(
            decode_frames(truncated),
            Err(ArtifactError::Malformed(_))
        ));
    }

    #[test]
    fn unterminated_frame_is_malformed() {
        let mut stream = Vec::new();
        stream.extend_from_slice(APC);
        stream.extend_from_slice(SENTINEL.as_bytes());
        stream.extend_from_slice(b";m=0;abc"); // no ST
        assert!(matches!(
            decode_frames(&stream),
            Err(ArtifactError::Malformed(_))
        ));
    }
}
