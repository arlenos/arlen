//! Event Bus consumer.
//!
//! Connects to the Arlen Event Bus consumer socket, subscribes to
//! `focus.*` (desktop-shell emits these when the user enters/leaves
//! Focus Mode for a project) and `window.fullscreen_*` (compositor
//! will emit these once the wiring in `compositor/src/event_bus.rs`
//! is plumbed through the shell::Workspace fullscreen transitions),
//! decodes the protobuf payloads, and drives the corresponding state
//! changes on the `NotificationManager`.
//!
//! Failures never abort the daemon: connection errors log and retry
//! every 2s, malformed messages log and are skipped. The notification
//! daemon must keep functioning even when the event bus is down.

use std::sync::Arc;
use std::time::Duration;

use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::manager::NotificationManager;

pub mod proto {
    #![allow(clippy::doc_markdown)]
    include!(concat!(env!("OUT_DIR"), "/arlen.eventbus.rs"));
}

const CONSUMER_ID: &str = "notification-daemon";

/// Resolve the Event Bus consumer socket per the standard Arlen 3-tier
/// convention: `ARLEN_CONSUMER_SOCKET` (non-empty) wins, else the
/// per-user path `$XDG_RUNTIME_DIR/arlen/event-bus-consumer.sock` (i.e.
/// `/run/user/{uid}/arlen/...`), else `/run/arlen/event-bus-consumer.sock`.
///
/// notification-daemon does not depend on `os-sdk`, so the shared
/// `os_sdk::runtime::socket_path` resolver is reproduced here. The env
/// override stays tier 1, the contract the dev stack and the
/// integration harness pin the socket through; it must match the path
/// event-bus binds.
fn resolve_consumer_socket() -> String {
    if let Some(p) = std::env::var("ARLEN_CONSUMER_SOCKET")
        .ok()
        .filter(|s| !s.is_empty())
    {
        return p;
    }
    if let Some(dir) = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.is_empty())
    {
        return format!("{dir}/arlen/event-bus-consumer.sock");
    }
    "/run/arlen/event-bus-consumer.sock".to_string()
}
/// Prefix subscriptions. The registry supports `*` for all, exact type,
/// or `<prefix>.` for prefix match. Using two prefixes keeps the
/// registration readable.
const SUBSCRIPTIONS: &str = "focus.,window.fullscreen_";

/// The registration the bus reads: consumer id, subscription patterns, uid
/// filter, one per line.
///
/// A named function rather than a `format!` at the call site so the shape can be
/// tested. The bus reads exactly three lines and BLOCKS on the third
/// (`event-bus/src/socket.rs:694-696`), so a copy that sends two never finishes
/// registering and receives nothing - which is what happened to the knowledge
/// writer when the uid line was added in Phase 3.1 and one consumer was missed.
/// That went unnoticed until an integration test drove the assembled pipeline.
fn registration_line(uid: u32) -> String {
    format!("{CONSUMER_ID}\n{SUBSCRIPTIONS}\n{uid}\n")
}

const MAX_MESSAGE_BYTES: u32 = 1024 * 1024;

/// Starts the Event Bus consumer on the current tokio runtime.
///
/// Spawns a dedicated task that reconnects indefinitely; errors log at
/// `warn`. Callers typically wire this up from `main.rs` alongside the
/// other daemon tasks.
pub fn start(manager: Arc<NotificationManager>) {
    tokio::spawn(async move {
        let socket_path = resolve_consumer_socket();
        loop {
            if let Err(e) = run_once(&socket_path, &manager).await {
                tracing::warn!(
                    "event bus consumer: disconnected ({e}), retrying in 2s"
                );
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

async fn run_once(
    socket_path: &str,
    manager: &Arc<NotificationManager>,
) -> Result<(), String> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .map_err(|e| format!("connect {socket_path}: {e}"))?;

    // 3-line registration (Phase 3.1: added UID line).
    let uid = unsafe { libc::getuid() };
    let registration = registration_line(uid);
    stream
        .write_all(registration.as_bytes())
        .await
        .map_err(|e| format!("register: {e}"))?;
    stream
        .flush()
        .await
        .map_err(|e| format!("flush registration: {e}"))?;

    tracing::info!("event bus consumer: registered (subscribe to {SUBSCRIPTIONS})");

    // Read loop: 4-byte BE length + protobuf body.
    loop {
        let mut len_buf = [0u8; 4];
        stream
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| format!("read length: {e}"))?;
        let len = u32::from_be_bytes(len_buf);
        if len == 0 || len > MAX_MESSAGE_BYTES {
            return Err(format!("invalid message length: {len}"));
        }

        let mut buf = vec![0u8; len as usize];
        stream
            .read_exact(&mut buf)
            .await
            .map_err(|e| format!("read body: {e}"))?;

        match proto::Event::decode(&buf[..]) {
            Ok(event) => dispatch(event, manager).await,
            Err(e) => tracing::warn!("event bus: decode failed: {e}"),
        }
    }
}

async fn dispatch(event: proto::Event, manager: &Arc<NotificationManager>) {
    match event.r#type.as_str() {
        "focus.activated" => {
            match proto::FocusActivatedPayload::decode(&event.payload[..]) {
                Ok(payload) => {
                    tracing::info!(
                        project = %payload.project_name,
                        apps = payload.suppress_notifications_from.len(),
                        "focus mode activated via event bus"
                    );
                    manager
                        .activate_focus(
                            payload.project_id,
                            payload.suppress_notifications_from,
                        )
                        .await;
                }
                Err(e) => tracing::warn!("focus.activated decode failed: {e}"),
            }
        }
        "focus.deactivated" => {
            tracing::info!("focus mode deactivated via event bus");
            manager.deactivate_focus().await;
        }
        "window.fullscreen_entered" => {
            tracing::debug!("fullscreen entered via event bus");
            manager.set_fullscreen(true).await;
        }
        "window.fullscreen_exited" => {
            tracing::debug!("fullscreen exited via event bus");
            manager.set_fullscreen(false).await;
        }
        other => {
            // Shouldn't happen given our prefix subscriptions, but logs
            // catch compositor-side misconfig quickly.
            tracing::debug!("event bus: ignoring unknown type '{other}'");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registration_is_the_three_lines_the_bus_reads() {
        let r = registration_line(1000);
        assert_eq!(
            r.lines().collect::<Vec<_>>(),
            ["notification-daemon", "focus.,window.fullscreen_", "1000"],
            "the bus blocks on a missing third line and the consumer then gets nothing"
        );
        assert!(r.ends_with('\n'), "the last line needs its terminator too");
    }
}
