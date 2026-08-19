//! The LSP wire format: `Content-Length` framing over a byte stream.
//!
//! A language server speaks JSON-RPC in HTTP-ish frames - a `Content-Length`
//! header, a blank line, then exactly that many BYTES of JSON. The count is
//! bytes and not characters, which is the first thing a hand-rolled reader gets
//! wrong: a German diagnostic ("Nicht gefunden") or a Japanese identifier makes
//! the two differ, and a reader that counts characters truncates the message and
//! then desynchronises the stream for every message after it.
//!
//! Pure over `&[u8]`, so the whole protocol is testable without spawning a
//! server: the process seam is the editor host's, and it hands bytes to this.

use serde::{Deserialize, Serialize};

/// What a frame reader can refuse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// The header block is present but malformed - no `Content-Length`, or a
    /// length that is not a number.
    BadHeader(String),
    /// A frame claims more bytes than any real message: a corrupt or hostile
    /// length must not become an allocation.
    TooLarge(usize),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadHeader(why) => write!(f, "malformed frame header: {why}"),
            Self::TooLarge(n) => write!(f, "frame claims {n} bytes, over the cap"),
        }
    }
}

impl std::error::Error for FrameError {}

/// The largest frame this client will accept, in bytes.
///
/// A language server's biggest messages are whole-file diagnostics and
/// completion lists; 16 MiB is far above any of those and far below a length
/// that would matter if a stream went wrong. The cap is checked BEFORE
/// reserving, so a bad length costs nothing.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

/// Frame one JSON payload for the wire.
pub fn encode(payload: &[u8]) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", payload.len()).into_bytes();
    out.extend_from_slice(payload);
    out
}

/// Try to take one frame off the front of `buf`.
///
/// Returns the payload and how many bytes of `buf` it consumed, or `Ok(None)`
/// when the buffer does not hold a whole frame yet - which is the normal state
/// of a stream reader and not a failure. The caller keeps the remainder.
pub fn decode(buf: &[u8]) -> Result<Option<(Vec<u8>, usize)>, FrameError> {
    // Headers end at the first blank line. `\r\n\r\n` is what the specification
    // says; a bare `\n\n` is accepted too, because some servers emit it and
    // refusing would break a working editor over a pedantic point.
    let (head_end, sep) = match find(buf, b"\r\n\r\n") {
        Some(i) => (i, 4),
        None => match find(buf, b"\n\n") {
            Some(i) => (i, 2),
            None => return Ok(None),
        },
    };
    let head = std::str::from_utf8(&buf[..head_end])
        .map_err(|_| FrameError::BadHeader("headers are not text".into()))?;
    let len = head
        .lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim())
        })
        .ok_or_else(|| FrameError::BadHeader("no Content-Length".into()))?;
    let len: usize = len
        .parse()
        .map_err(|_| FrameError::BadHeader(format!("Content-Length {len:?} is not a number")))?;
    if len > MAX_FRAME {
        return Err(FrameError::TooLarge(len));
    }
    let start = head_end + sep;
    if buf.len() < start + len {
        return Ok(None); // the body has not all arrived
    }
    Ok(Some((buf[start..start + len].to_vec(), start + len)))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// A JSON-RPC message as it goes out.
///
/// Requests carry an id and expect an answer; notifications carry none and never
/// do. Conflating them is how a client ends up waiting forever for a reply to
/// `textDocument/didOpen`, which by definition never comes.
#[derive(Debug, Clone, Serialize)]
pub struct Outgoing {
    pub jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl Outgoing {
    /// A call that expects a reply carrying this id.
    pub fn request(id: i64, method: &str, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id: Some(id),
            method: method.to_string(),
            params: Some(params),
        }
    }

    /// A one-way message. No id, so no reply is coming and none is waited for.
    pub fn notification(method: &str, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id: None,
            method: method.to_string(),
            params: Some(params),
        }
    }

    /// Serialise and frame in one step.
    pub fn to_frame(&self) -> Vec<u8> {
        encode(&serde_json::to_vec(self).expect("an Outgoing is always serialisable"))
    }
}

/// A message arriving from the server, before it is interpreted.
#[derive(Debug, Clone, Deserialize)]
pub struct Incoming {
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_round_trips() {
        let framed = encode(br#"{"jsonrpc":"2.0"}"#);
        let (body, used) = decode(&framed).unwrap().expect("one whole frame");
        assert_eq!(body, br#"{"jsonrpc":"2.0"}"#);
        assert_eq!(used, framed.len());
    }

    /// The length is BYTES. A reader that counts characters cuts a multi-byte
    /// message short and then reads the next frame's header as a body, which
    /// desynchronises the stream permanently rather than failing once.
    #[test]
    fn the_length_counts_bytes_not_characters() {
        let payload = r#"{"message":"Nicht gefunden: ä"}"#;
        assert_ne!(payload.len(), payload.chars().count(), "the test needs a multi-byte payload");
        let framed = encode(payload.as_bytes());
        let head = String::from_utf8_lossy(&framed[..framed.iter().position(|&b| b == b'\r').unwrap()]);
        assert_eq!(head, format!("Content-Length: {}", payload.len()));
        let (body, _) = decode(&framed).unwrap().unwrap();
        assert_eq!(String::from_utf8(body).unwrap(), payload);
    }

    #[test]
    fn a_partial_frame_is_not_an_error() {
        let framed = encode(br#"{"a":1}"#);
        // Header complete, body still arriving: the reader waits rather than
        // failing, which is the normal state of a stream.
        assert_eq!(decode(&framed[..framed.len() - 3]).unwrap(), None);
        // Not even the header yet.
        assert_eq!(decode(b"Content-Len").unwrap(), None);
    }

    #[test]
    fn two_frames_in_one_buffer_are_taken_one_at_a_time() {
        let mut buf = encode(br#"{"n":1}"#);
        buf.extend(encode(br#"{"n":2}"#));
        let (first, used) = decode(&buf).unwrap().unwrap();
        assert_eq!(first, br#"{"n":1}"#);
        let (second, _) = decode(&buf[used..]).unwrap().unwrap();
        assert_eq!(second, br#"{"n":2}"#);
    }

    #[test]
    fn a_bare_newline_separator_is_accepted() {
        // Not what the specification says, but some servers emit it and an
        // editor that dies on it is broken for its user, not correct.
        let raw = b"Content-Length: 7\n\n{\"n\":1}";
        let (body, _) = decode(raw).unwrap().unwrap();
        assert_eq!(body, br#"{"n":1}"#);
    }

    #[test]
    fn a_header_without_a_length_is_refused() {
        assert!(matches!(
            decode(b"Content-Type: application/json\r\n\r\n{}"),
            Err(FrameError::BadHeader(_))
        ));
        assert!(matches!(
            decode(b"Content-Length: lots\r\n\r\n{}"),
            Err(FrameError::BadHeader(_))
        ));
    }

    /// A corrupt length must not become an allocation. Checked before any
    /// reserve, so the cost of a hostile header is the check itself.
    #[test]
    fn an_impossible_length_is_refused_before_allocating() {
        let raw = format!("Content-Length: {}\r\n\r\n", MAX_FRAME + 1).into_bytes();
        assert_eq!(decode(&raw), Err(FrameError::TooLarge(MAX_FRAME + 1)));
    }

    #[test]
    fn a_notification_carries_no_id_and_a_request_does() {
        let n = Outgoing::notification("textDocument/didOpen", serde_json::json!({}));
        let json: serde_json::Value =
            serde_json::from_slice(&serde_json::to_vec(&n).unwrap()).unwrap();
        assert!(json.get("id").is_none(), "a notification with an id waits for a reply that never comes");

        let r = Outgoing::request(1, "initialize", serde_json::json!({}));
        let json: serde_json::Value =
            serde_json::from_slice(&serde_json::to_vec(&r).unwrap()).unwrap();
        assert_eq!(json["id"], 1);
        assert_eq!(json["jsonrpc"], "2.0");
    }
}
