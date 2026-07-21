//! Notification IPC wire format: the pure, CI-tested pieces the shell's socket
//! client uses to talk to the notification daemon.
//!
//! `types` is the serde-shaped notification / DND / sync payloads (mirroring the
//! `notification-proto` messages). `protocol` is the length-prefixed protobuf
//! framing (4-byte BE length + body) over any async reader/writer. The socket
//! client itself (connect, reconnect, Tauri events) stays in the shell host.
pub mod protocol;
pub mod types;
