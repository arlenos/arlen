//! Notification client for the shell: the socket connection to the notification
//! daemon and the Tauri commands + events it drives.
//!
//! The wire format (the serde payload `types` and the length-prefixed protobuf
//! `protocol` framing) lives in the shell core crate
//! (`arlen_desktop_shell_core::notifications`), unit-tested in CI.
pub mod client;
