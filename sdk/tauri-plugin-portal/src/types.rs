//! Public types crossing the Rust ↔ TypeScript boundary.
//!
//! `serde(rename_all = "camelCase")` plus
//! `rename_all_fields = "camelCase"` on enums match the rest of
//! Lunaris's Tauri-DTO convention so the JS side never has to
//! bridge naming. See `xdg-portal-lunaris-protocol` for the
//! daemon-side mirror — these types are deliberately separate
//! because the public API smooths over D-Bus shapes the wire
//! types preserve verbatim.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One named filter in a `pickFile`/`saveFile` request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileFilter {
    pub name: String,
    pub patterns: Vec<FilterPattern>,
}

/// Filter pattern: glob (`*.png`) or MIME type (`image/png`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FilterPattern {
    Glob { pattern: String },
    Mime { mime_type: String },
}

/// Options for `pickFile` / `pickDirectory`.
///
/// Same struct serves both — `pickDirectory` flips `directory`
/// internally before invoking, so callers do not need to set it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PickFileOptions {
    /// Window title shown in the picker.
    pub title: Option<String>,
    /// Allow multi-select. Default false.
    pub multiple: bool,
    /// Modal hint. Wayland has no cross-app modal concept; the flag
    /// is recorded but not enforced.
    pub modal: Option<bool>,
    /// Pre-populate filters. First filter is selected by default
    /// unless `currentFilter` is also set.
    pub filters: Vec<FileFilter>,
    /// Pre-select a specific filter from `filters`.
    pub current_filter: Option<FileFilter>,
    /// Initial directory; falls back to `$HOME` if unset or invalid.
    pub current_folder: Option<PathBuf>,
    /// Filled internally by `pickDirectory` — public callers
    /// should leave this `false` and use the dedicated function.
    pub directory: bool,
}

/// Options for `saveFile`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SaveFileOptions {
    pub title: Option<String>,
    pub modal: Option<bool>,
    pub filters: Vec<FileFilter>,
    pub current_filter: Option<FileFilter>,
    /// Suggested filename to pre-fill in the picker's input field.
    pub current_name: Option<String>,
    pub current_folder: Option<PathBuf>,
    /// Pre-select a specific existing file (e.g. for "Save As").
    pub current_file: Option<PathBuf>,
}

/// Options for `saveFiles` (batch save into one directory).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SaveFilesOptions {
    pub title: Option<String>,
    pub modal: Option<bool>,
    /// The list of files to save. Picker presents the directory
    /// chooser; on confirm, each filename is appended to the
    /// chosen directory.
    pub files: Vec<PathBuf>,
    pub current_folder: Option<PathBuf>,
}

/// Options for `openUri`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OpenUriOptions {
    /// Whether the user should be prompted before opening. Defaults
    /// to whatever the portal frontend chooses (usually false for
    /// http(s), true for unfamiliar schemes).
    pub ask: Option<bool>,
    /// Whether the URI should be opened with write permission. Only
    /// meaningful for `file://` URIs that the caller intends to
    /// write to.
    pub writable: Option<bool>,
}

/// Outcome of a picker invocation.
///
/// `Cancelled` is the user-dismissed-the-dialog case — JavaScript
/// converts it to `null` so callers do not need a specific check.
/// `Err(PickerError)` is reserved for actual failures (portal
/// unavailable, scheme rejected, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PickerResult {
    Picked { uris: Vec<String> },
    Cancelled,
}

/// Error variants surfaced to the Tauri frontend.
///
/// JS sees these as `Error.message` strings via Tauri's serde
/// translation; the variant tag is also serialised for callers
/// that want to discriminate without parsing the message.
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PickerError {
    /// `xdg-desktop-portal` frontend daemon could not be reached.
    /// Typical cause: package not installed or service crashed.
    #[error("xdg-desktop-portal frontend unavailable: {message}")]
    PortalUnavailable { message: String },
    /// D-Bus method call timed out before the picker even started
    /// (distinct from the picker hanging mid-pick — that surfaces
    /// as `Backend`).
    #[error("portal request timed out: {message}")]
    Timeout { message: String },
    /// Connection to the portal lost mid-request.
    #[error("portal connection lost: {message}")]
    ConnectionLost { message: String },
    /// `open_uri` was called with a scheme outside the plugin's
    /// allow-list. Plugin enforces this locally so behaviour is
    /// consistent across backend implementations — a permissive
    /// frontend would otherwise let arbitrary handlers fire.
    #[error("scheme not allowed: {scheme}")]
    SchemeRejected { scheme: String },
    /// Backend returned a non-zero, non-cancel response code with
    /// an error message in the results dict.
    #[error("portal backend reported error: {message}")]
    Backend { message: String },
    /// Catch-all for anything else (zbus errors that do not fit
    /// the more specific buckets).
    #[error("portal call failed: {message}")]
    Other { message: String },
}

impl PickerError {
    pub(crate) fn from_zbus(err: zbus::Error) -> Self {
        match &err {
            zbus::Error::MethodError(name, _, _) if name.as_str().contains("ServiceUnknown") => {
                PickerError::PortalUnavailable {
                    message: err.to_string(),
                }
            }
            _ => PickerError::Other {
                message: err.to_string(),
            },
        }
    }
}
