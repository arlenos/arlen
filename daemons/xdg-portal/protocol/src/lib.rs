//! Wire types for IPC between the daemon and the picker-ui Tauri app.
//!
//! Both processes serialize and deserialize the same message types. The
//! crate exists so a single source defines them — drift between the two
//! sides would silently corrupt picks.
//!
//! Frame format (used by both directions): 4-byte big-endian length, then
//! UTF-8 JSON body. Same as the notification daemon's broadcast socket.
//! Encode/decode helpers live in [`codec`].
//!
//! All types use `rename_all = "camelCase"` because the picker-ui side
//! crosses a Rust-TypeScript boundary inside Tauri.

pub mod codec;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Daemon -> picker-ui. The picker UI shows a dialog for the request and
/// eventually replies with a [`PickerResponse`] carrying the same `handle`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PickerRequest {
    /// Open one or more existing files for reading. Mirrors
    /// `org.freedesktop.impl.portal.FileChooser.OpenFile`.
    OpenFile {
        /// Unique correlation handle. The matching response carries the
        /// same value.
        handle: String,
        /// Caller-controlled string. Display only; do NOT trust for
        /// authorisation. Sandbox detection happens daemon-side via the
        /// caller's cgroup.
        app_id: String,
        title: String,
        /// File-extension or MIME filters. Empty array means "all files".
        filters: Vec<FileFilter>,
        /// Currently-active filter, if the caller pre-selected one.
        current_filter: Option<FileFilter>,
        /// Whether the user can pick more than one file.
        multiple: bool,
        /// Whether the picker should be modal relative to the parent
        /// window. Wayland has no cross-app modal concept; the flag is
        /// recorded but not enforced.
        modal: bool,
        /// Whether the picker is selecting directories, not files.
        /// When true the listing hides files and the confirm action
        /// returns the currently-displayed directory.
        directory: bool,
        /// Where the picker opens. Falls back to `$HOME` if absent or
        /// invalid (path traversal, non-existent directory, outside the
        /// caller's allowed roots).
        current_folder: Option<PathBuf>,
        /// Caller's parent window in `wayland:NNNN` or `x11:0xABCD` form.
        /// XWayland callers cannot be matched to a Wayland surface; the
        /// picker falls back to the focused output.
        parent_window: Option<String>,
    },
    /// Save a single file. Mirrors
    /// `org.freedesktop.impl.portal.FileChooser.SaveFile`.
    SaveFile {
        handle: String,
        app_id: String,
        title: String,
        filters: Vec<FileFilter>,
        current_filter: Option<FileFilter>,
        current_name: Option<String>,
        current_folder: Option<PathBuf>,
        current_file: Option<PathBuf>,
        parent_window: Option<String>,
    },
    /// Save multiple files into a single directory. Mirrors
    /// `org.freedesktop.impl.portal.FileChooser.SaveFiles`.
    SaveFiles {
        handle: String,
        app_id: String,
        title: String,
        files: Vec<PathBuf>,
        current_folder: Option<PathBuf>,
        parent_window: Option<String>,
    },
    /// Daemon-initiated cancellation, e.g. caller died (E2) or
    /// wall-clock timeout fired (E13). Picker UI hides immediately and
    /// does not respond.
    Cancel { handle: String },
}

/// Picker-ui -> daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PickerResponse {
    /// User confirmed; `paths` is non-empty.
    Picked {
        handle: String,
        paths: Vec<PathBuf>,
        /// Filter the user had selected at confirm time, if any. Echoed
        /// back to the caller via the `current_filter` result key.
        current_filter: Option<FileFilter>,
    },
    /// User dismissed the picker.
    Cancelled { handle: String },
    /// Picker UI hit a fatal error (filesystem access denied, regex DoS
    /// cap exceeded, etc.). Daemon converts this to an error response on
    /// the D-Bus side.
    Error { handle: String, message: String },
}

/// Filter declaration. Matches `org.freedesktop.impl.portal.FileChooser`
/// `filters` option type `a(sa(us))`: name plus a list of `(type, pattern)`
/// where `type` is 0 (glob) or 1 (mime).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileFilter {
    /// Display name shown in the filter dropdown, e.g. "Images".
    pub name: String,
    /// Patterns. Each is either a glob (`*.png`) or a MIME type
    /// (`image/png`).
    pub patterns: Vec<FilterPattern>,
}

/// One filter pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FilterPattern {
    /// Glob pattern like `*.png`. The picker UI matches against the file
    /// name only, not the full path.
    Glob { pattern: String },
    /// MIME type like `image/png`. The picker UI uses `xdg-mime` rules
    /// to derive the MIME from the file name; reading file content for
    /// magic-byte detection is intentionally avoided to keep listing
    /// fast on slow filesystems (E8).
    Mime { mime_type: String },
}

/// The print dialog's wire types (`printing-plan.md` PRN-R2/R3).
///
/// A SECOND SHAPE FOR A SECOND DIALOG, and the difference is why they do not
/// share one enum. The file picker is a subprocess the daemon spawns per request
/// and talks to over a socket it owns; the print dialog lives in the shell, which
/// is already running and asks. So the traffic runs the other way: the shell
/// polls for a pending request and posts back an answer, while the portal holds
/// the document and waits.
pub mod print {
    use serde::{Deserialize, Serialize};

    /// A document waiting for somebody to choose how to print it.
    ///
    /// Everything here is what the DIALOG needs to ask its question. The document
    /// itself never crosses: the portal holds the bytes, the shell chooses the
    /// printer, and the portal prints. A dialog that carried the document would
    /// be handing the shell a copy of something an app only meant to print.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct PrintRequest {
        /// Correlates the answer with the waiting document.
        pub id: String,
        /// The document name, for the dialog header.
        pub title: String,
        /// The requesting app as the portal attested it, not as it named itself.
        pub app_id: String,
        /// The name to show for that app.
        pub app_name: String,
        /// How many pages, which drives the range control and the pager.
        pub page_count: u32,
    }

    /// How the paper maps to the sheet. The shell's vocabulary, mapped to the
    /// portal's `sides` strings on the way to CUPS.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub enum Duplex {
        OneSided,
        /// Flipped along the long edge, the usual book-style two-sided.
        TwoSidedLong,
        TwoSidedShort,
    }

    /// Colour or not.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum Color {
        Color,
        Mono,
    }

    /// Which pages.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum RangeMode {
        All,
        Current,
        Range,
    }

    /// What the dialog came back with.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct PrintChoice {
        /// The printer's CUPS name.
        pub printer: String,
        pub copies: u32,
        pub range_mode: RangeMode,
        /// The typed range, meaningful only when `range_mode` is `Range`.
        pub range_text: String,
        pub duplex: Duplex,
        pub color: Color,
        /// The paper size as the dialog names it (`a4`, `letter`, `legal`).
        pub paper: String,
    }

    /// Shell -> portal.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "op", rename_all = "snake_case")]
    pub enum DialogRequest {
        /// Is anything waiting to be printed?
        Poll,
        /// Hold this connection open until something IS.
        ///
        /// THE WAKE SIGNAL, and without it the rest of this is unreachable. The
        /// dialog lives in the shell, which has no reason to ask unless somebody
        /// tells it to - so the first cut had a portal parked on a print nobody
        /// would ever come for, and an app whose print would hang until the
        /// dialog timed out. A held connection says it the moment it happens,
        /// costs one socket, and needs no event bus, no new dependency in the
        /// portal and no timer in the shell.
        Await,
        /// Print it with these choices.
        Submit { id: String, settings: PrintChoice },
        /// The person declined. Nothing is printed.
        Cancel { id: String },
    }

    /// Portal -> shell.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "result", rename_all = "snake_case")]
    pub enum DialogResponse {
        /// The next waiting document, or nothing.
        ///
        /// A named field rather than a newtype: an internally tagged enum cannot
        /// carry a bare `Option`, and `{"result":"pending","request":null}` is
        /// the clearer wire shape anyway - the absence is stated rather than
        /// inferred from a missing tag.
        Pending { request: Option<PrintRequest> },
        /// The answer was taken.
        Taken,
        /// No document is waiting under that id.
        ///
        /// Its own answer rather than an error: a dialog answering a request the
        /// portal already gave up on (the app went away, the wait timed out) has
        /// done nothing wrong, and the shell closes either way.
        Unknown,
    }

    /// The socket the shell polls: `$XDG_RUNTIME_DIR/arlen/portal-print.sock`.
    ///
    /// Under the same prefix as the picker socket, for the same reason: every
    /// Arlen runtime file in one place, and no collision with another portal
    /// backend a person may have installed beside this one.
    pub fn socket_path() -> std::path::PathBuf {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("/run"));
        base.join("arlen").join("portal-print.sock")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn choice() -> PrintChoice {
            PrintChoice {
                printer: "Office HP".into(),
                copies: 2,
                range_mode: RangeMode::Range,
                range_text: "1-5, 8".into(),
                duplex: Duplex::TwoSidedLong,
                color: Color::Mono,
                paper: "a4".into(),
            }
        }

        /// The shell reads these fields straight out of the IPC, so a snake_case
        /// name would arrive in the dialog as undefined and the control would
        /// render its fallback with nothing saying why.
        #[test]
        fn the_shell_sees_the_names_it_declares() {
            let json = serde_json::to_string(&choice()).unwrap();
            assert!(json.contains("\"rangeMode\""), "{json}");
            assert!(json.contains("\"rangeText\""), "{json}");
            assert!(!json.contains("range_mode"), "{json}");

            let req = PrintRequest {
                id: "p1".into(),
                title: "Quarterly report.pdf".into(),
                app_id: "org.arlen.files".into(),
                app_name: "Files".into(),
                page_count: 12,
            };
            let json = serde_json::to_string(&req).unwrap();
            assert!(json.contains("\"appId\"") && json.contains("\"pageCount\""), "{json}");
        }

        /// The dialog's own vocabulary, verbatim. `two-sided-long` is the shell's
        /// word; the portal's `sides` string is `two-sided-long-edge`, and the
        /// mapping between them is the portal's job rather than a rename here.
        #[test]
        fn the_choices_travel_as_the_dialog_spells_them() {
            let json = serde_json::to_string(&choice()).unwrap();
            assert!(json.contains("\"two-sided-long\""), "{json}");
            assert!(json.contains("\"mono\""), "{json}");
            assert!(json.contains("\"range\""), "{json}");
        }

        #[test]
        fn a_request_round_trips() {
            let r = DialogRequest::Submit {
                id: "p1".into(),
                settings: choice(),
            };
            let back: DialogRequest = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
            assert_eq!(r, back);
        }

        #[test]
        fn nothing_waiting_is_an_answer_not_an_error() {
            let json = serde_json::to_string(&DialogResponse::Pending { request: None }).unwrap();
            assert!(json.contains("\"request\":null"), "{json}");
            let back: DialogResponse = serde_json::from_str(&json).unwrap();
            assert_eq!(back, DialogResponse::Pending { request: None });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wire types must round-trip through JSON unchanged so the daemon
    /// and the picker-ui never disagree about field shapes.
    #[test]
    fn pick_request_round_trip() {
        let req = PickerRequest::OpenFile {
            handle: "h1".into(),
            app_id: "org.example.app".into(),
            title: "Open file".into(),
            filters: vec![FileFilter {
                name: "Images".into(),
                patterns: vec![
                    FilterPattern::Glob { pattern: "*.png".into() },
                    FilterPattern::Mime { mime_type: "image/png".into() },
                ],
            }],
            current_filter: None,
            multiple: false,
            modal: true,
            directory: false,
            current_folder: Some(PathBuf::from("/home/example/Pictures")),
            parent_window: Some("wayland:42".into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: PickerRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(format!("{req:?}"), format!("{back:?}"));
    }

    /// camelCase is required because the picker-ui frontend reads these
    /// directly from the IPC. Snake-case would silently produce undefined
    /// fields on the JS side.
    #[test]
    fn camel_case_field_names() {
        let req = PickerRequest::SaveFile {
            handle: "h2".into(),
            app_id: "".into(),
            title: "Save".into(),
            filters: vec![],
            current_filter: None,
            current_name: Some("draft.txt".into()),
            current_folder: None,
            current_file: None,
            parent_window: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"appId\""));
        assert!(json.contains("\"currentName\""));
        assert!(json.contains("\"parentWindow\""));
        assert!(!json.contains("app_id"));
        assert!(!json.contains("current_name"));
    }

    /// `Picked` carries paths and the filter the user had active at
    /// confirm time. Empty paths is invalid in practice but the type
    /// permits it for cleaner serde shape.
    #[test]
    fn picked_response_round_trip() {
        let resp = PickerResponse::Picked {
            handle: "h1".into(),
            paths: vec![PathBuf::from("/home/example/Pictures/logo.png")],
            current_filter: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: PickerResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(format!("{resp:?}"), format!("{back:?}"));
    }
}
