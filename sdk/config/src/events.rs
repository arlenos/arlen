/// Event Bus integration for config change notifications.
///
/// Emits `config.changed` events when config files change on disk,
/// and `config.reload_requested` for manual reload triggers.
///
/// See `docs/architecture/config-system.md` (Live Reload section).

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

/// Payload for config.changed events (encoded as JSON in protobuf payload field).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfigChangedPayload {
    /// Component name (e.g. "shell", "theme", "keybindings").
    pub component: String,
    /// Absolute path of the changed file.
    pub path: String,
}

/// Lightweight Event Bus producer for config change events.
///
/// Connects to the producer socket and sends length-prefixed protobuf
/// events. Falls back silently if the Event Bus is not running.
pub struct ConfigEventEmitter {
    socket_path: PathBuf,
    session_id: String,
}

impl ConfigEventEmitter {
    /// Create a new emitter. Does not connect yet (lazy).
    pub fn new() -> Self {
        let socket_path = std::env::var("LUNARIS_PRODUCER_SOCKET")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/run/lunaris/event-bus-producer.sock"));
        let session_id = std::env::var("LUNARIS_SESSION_ID")
            .unwrap_or_else(|_| "unknown".into());
        Self {
            socket_path,
            session_id,
        }
    }

    /// Emit a `config.changed` event for a component.
    pub fn emit_changed(&self, component: &str, path: &str) {
        let payload = ConfigChangedPayload {
            component: component.into(),
            path: path.into(),
        };
        self.emit("config.changed", &payload);
    }

    /// Emit a `config.reload_requested` event for a component.
    pub fn emit_reload_requested(&self, component: &str) {
        let payload = ConfigChangedPayload {
            component: component.into(),
            path: String::new(),
        };
        self.emit("config.reload_requested", &payload);
    }

    /// Send an event to the Event Bus (best-effort, silent on failure).
    fn emit(&self, event_type: &str, payload: &ConfigChangedPayload) {
        let payload_json = match serde_json::to_vec(payload) {
            Ok(v) => v,
            Err(_) => return,
        };

        // Build a minimal protobuf Event by hand (avoids prost dependency).
        // Wire format: field 1 (id) = UUID, field 2 (type), field 3 (timestamp),
        // field 4 (source), field 5 (pid), field 6 (session_id), field 7 (payload).
        let event_bytes = encode_event(event_type, &self.session_id, &payload_json);

        // Connect, send length-prefixed message, close.
        let mut stream = match UnixStream::connect(&self.socket_path) {
            Ok(s) => s,
            Err(_) => return, // Event Bus not running, skip silently.
        };

        let len = (event_bytes.len() as u32).to_be_bytes();
        let _ = stream.write_all(&len);
        let _ = stream.write_all(&event_bytes);
    }
}

/// Encode a minimal protobuf Event message without the prost crate.
///
/// This avoids adding prost as a dependency to the config crate.
/// The wire format follows proto3 encoding for the Event message.
fn encode_event(event_type: &str, session_id: &str, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(128 + payload.len());

    // Field 1: id (string) - UUID v7 approximation using timestamp
    let id = format!(
        "{:016x}-config",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros()
    );
    encode_string(&mut buf, 1, &id);

    // Field 2: type (string)
    encode_string(&mut buf, 2, event_type);

    // Field 3: timestamp (int64, microseconds)
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64;
    encode_varint_field(&mut buf, 3, ts as u64);

    // Field 4: source (string)
    encode_string(&mut buf, 4, "config-watcher");

    // Field 5: pid (uint32)
    encode_varint_field(&mut buf, 5, std::process::id() as u64);

    // Field 6: session_id (string)
    encode_string(&mut buf, 6, session_id);

    // Field 7: payload (bytes)
    encode_bytes(&mut buf, 7, payload);

    // Field 8: uid (uint32) -- 0 = will be stamped by Event Bus from SO_PEERCRED
    // (omit, proto3 default is 0)

    buf
}

fn encode_string(buf: &mut Vec<u8>, field: u32, s: &str) {
    let tag = (field << 3) | 2; // wire type 2 = length-delimited
    encode_varint(buf, tag as u64);
    encode_varint(buf, s.len() as u64);
    buf.extend_from_slice(s.as_bytes());
}

fn encode_bytes(buf: &mut Vec<u8>, field: u32, b: &[u8]) {
    let tag = (field << 3) | 2;
    encode_varint(buf, tag as u64);
    encode_varint(buf, b.len() as u64);
    buf.extend_from_slice(b);
}

fn encode_varint_field(buf: &mut Vec<u8>, field: u32, value: u64) {
    let tag = field << 3; // wire type 0 = varint
    encode_varint(buf, tag as u64);
    encode_varint(buf, value);
}

fn encode_varint(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            buf.push(byte);
            break;
        } else {
            buf.push(byte | 0x80);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_changed_payload() {
        let p = ConfigChangedPayload {
            component: "shell".into(),
            path: "/home/user/.config/lunaris/shell.toml".into(),
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("shell"));
    }

    #[test]
    fn test_encode_event_produces_bytes() {
        let payload = serde_json::to_vec(&ConfigChangedPayload {
            component: "test".into(),
            path: "/tmp/test.toml".into(),
        })
        .unwrap();
        let bytes = encode_event("config.changed", "session-1", &payload);
        assert!(!bytes.is_empty());
        // Should contain the event type string.
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("config.changed"));
    }

    #[test]
    fn test_encode_varint() {
        let mut buf = Vec::new();
        encode_varint(&mut buf, 0);
        assert_eq!(buf, vec![0]);

        buf.clear();
        encode_varint(&mut buf, 127);
        assert_eq!(buf, vec![127]);

        buf.clear();
        encode_varint(&mut buf, 128);
        assert_eq!(buf, vec![0x80, 0x01]);

        buf.clear();
        encode_varint(&mut buf, 300);
        assert_eq!(buf, vec![0xAC, 0x02]);
    }

    #[test]
    fn test_emitter_new_does_not_panic() {
        let _emitter = ConfigEventEmitter::new();
    }

    #[test]
    fn test_emit_without_event_bus() {
        // Should not panic even if Event Bus is not running.
        let emitter = ConfigEventEmitter::new();
        emitter.emit_changed("test", "/tmp/test.toml");
        emitter.emit_reload_requested("test");
    }
}
