//! Frontend-facing types for the clipboard plugin.
//!
//! These mirror `os_sdk::clipboard` but with serde camelCase
//! representation for direct TypeScript consumption.

use serde::{Deserialize, Serialize};

/// Sensitivity classification for a clipboard entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardLabel {
    /// Standard entry, eligible for history.
    Normal,
    /// Sensitive entry, broadcast once but never persisted.
    Sensitive,
}

impl Default for ClipboardLabel {
    fn default() -> Self {
        ClipboardLabel::Normal
    }
}

impl From<os_sdk::ClipboardLabel> for ClipboardLabel {
    fn from(value: os_sdk::ClipboardLabel) -> Self {
        match value {
            os_sdk::ClipboardLabel::Normal => ClipboardLabel::Normal,
            os_sdk::ClipboardLabel::Sensitive => ClipboardLabel::Sensitive,
        }
    }
}

impl From<ClipboardLabel> for os_sdk::ClipboardLabel {
    fn from(value: ClipboardLabel) -> Self {
        match value {
            ClipboardLabel::Normal => os_sdk::ClipboardLabel::Normal,
            ClipboardLabel::Sensitive => os_sdk::ClipboardLabel::Sensitive,
        }
    }
}

/// A clipboard entry as seen by the plugin caller.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardEntry {
    pub id: String,
    /// Bytes of the content. `None` if the caller lacks read scope
    /// or if the entry was sensitive and is no longer retained.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<u8>>,
    pub mime: String,
    pub label: ClipboardLabel,
    pub timestamp_ms: i64,
    pub source_app_id: String,
}

impl From<os_sdk::ClipboardEntry> for ClipboardEntry {
    fn from(entry: os_sdk::ClipboardEntry) -> Self {
        Self {
            id: entry.id,
            content: entry.content,
            mime: entry.mime,
            label: entry.label.into(),
            timestamp_ms: entry.timestamp_ms,
            source_app_id: entry.source_app_id,
        }
    }
}

/// Parameters for a write call.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteParams {
    pub content: Vec<u8>,
    pub mime: String,
    #[serde(default)]
    pub label: ClipboardLabel,
}

/// Errors surfaced to the frontend.
#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum ClipboardError {
    #[error("clipboard daemon is not reachable")]
    ConnectionFailed,
    #[error("permission denied")]
    PermissionDenied,
    #[error("content too large")]
    ContentTooLarge,
    #[error("unsupported MIME type: {0}")]
    UnsupportedMime(String),
    #[error("system error: {0}")]
    System(String),
    #[error("subscription already active")]
    AlreadySubscribed,
    #[error("no active subscription")]
    NotSubscribed,
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<os_sdk::ClipboardError> for ClipboardError {
    fn from(err: os_sdk::ClipboardError) -> Self {
        match err {
            os_sdk::ClipboardError::ConnectionFailed(_) => ClipboardError::ConnectionFailed,
            os_sdk::ClipboardError::Io(e) => ClipboardError::System(e.to_string()),
            os_sdk::ClipboardError::Protocol(msg) => ClipboardError::Internal(msg),
            os_sdk::ClipboardError::PermissionDenied(_) => ClipboardError::PermissionDenied,
            os_sdk::ClipboardError::ContentTooLarge(_) => ClipboardError::ContentTooLarge,
            os_sdk::ClipboardError::UnsupportedMime(mime) => {
                ClipboardError::UnsupportedMime(mime)
            }
            os_sdk::ClipboardError::System(msg) => ClipboardError::System(msg),
            os_sdk::ClipboardError::UnexpectedResponse => {
                ClipboardError::Internal("unexpected response".into())
            }
        }
    }
}
