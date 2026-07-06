//! Best-effort emit of `permission.changed` on the Event Bus at enroll time.
//!
//! When installd writes an app's permission profile, it emits this event so the
//! knowledge daemon projects the app's DECLARED grants into the LCG from the
//! profile alone (no running pid). Without it, an installed-but-never-run app -
//! or a system-tier enroll the desktop-shell profile watcher never sees - has a
//! profile on disk but zero Grant nodes until its first graph connect.
//!
//! A direct length-prefixed `UnixStream` write to the producer socket, mirroring
//! the desktop-shell `permission_watcher` emitter, so installd stays off the
//! `os-sdk` dep tree (its security-critical dep set is deliberately tight).

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

mod proto {
    #![allow(dead_code, clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/arlen.eventbus.rs"));
}

/// Producer socket path; the same env override and default as the other
/// Event Bus emitters.
fn producer_socket_path() -> String {
    std::env::var("ARLEN_PRODUCER_SOCKET")
        .unwrap_or_else(|_| "/run/arlen/event-bus-producer.sock".to_string())
}

fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

fn write_varint(buf: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        buf.push((value as u8) | 0x80);
        value >>= 7;
    }
    buf.push(value as u8);
}

/// Hand-encode `PermissionChangedPayload { app_id, exists }`: field 1 is the
/// length-delimited UTF-8 app_id, field 2 is the varint `exists` bool. Matches
/// the desktop-shell emitter and the knowledge daemon's
/// `extract_app_id_from_payload`, which reads field 1 the same way.
fn build_payload(app_id: &str, exists: bool) -> Vec<u8> {
    let mut buf = Vec::with_capacity(app_id.len() + 6);
    buf.push(0x0A);
    write_varint(&mut buf, app_id.len() as u64);
    buf.extend_from_slice(app_id.as_bytes());
    buf.push(0x10);
    buf.push(if exists { 1 } else { 0 });
    buf
}

/// Emit `permission.changed` for `app_id` so the knowledge daemon projects its
/// declared grants at enroll (no running pid). `exists` is true for a
/// write/enroll, false for a removal. Best-effort: a down bus never fails the
/// install - the LCG projection degrades, the install does not.
pub fn emit_permission_changed(app_id: &str, exists: bool) {
    use prost::Message;
    let event = proto::Event {
        id: uuid::Uuid::new_v4().to_string(),
        r#type: "permission.changed".to_string(),
        timestamp: now_micros(),
        source: "installd".to_string(),
        pid: std::process::id(),
        session_id: String::new(),
        payload: build_payload(app_id, exists),
        uid: unsafe { libc::getuid() },
        project_id: String::new(),
    };
    let encoded = event.encode_to_vec();
    let len = (encoded.len() as u32).to_be_bytes();
    let socket = producer_socket_path();
    let send = move || -> Result<(), std::io::Error> {
        let mut stream = std::os::unix::net::UnixStream::connect(&socket)?;
        stream.write_all(&len)?;
        stream.write_all(&encoded)?;
        stream.flush()?;
        Ok(())
    };
    if let Err(e) = send() {
        tracing::debug!("installd: permission.changed emit for {app_id} failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_encodes_app_id_at_field_one() {
        // Field 1 tag (0x0A), len, bytes; then field 2 tag (0x10), bool.
        let p = build_payload("com.example.app", true);
        assert_eq!(p[0], 0x0A);
        assert_eq!(p[1] as usize, "com.example.app".len());
        assert_eq!(&p[2..2 + "com.example.app".len()], b"com.example.app");
        assert_eq!(p[p.len() - 2], 0x10);
        assert_eq!(p[p.len() - 1], 1);
    }

    #[test]
    fn varint_encodes_multibyte() {
        let mut buf = Vec::new();
        write_varint(&mut buf, 300);
        assert_eq!(buf, vec![0xAC, 0x02]);
    }
}
