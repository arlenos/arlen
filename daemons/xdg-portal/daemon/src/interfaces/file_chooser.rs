//! `org.freedesktop.impl.portal.FileChooser` implementation.
//!
//! Three methods: `OpenFile`, `SaveFile`, `SaveFiles`. Each method
//! ensures the picker subprocess is running, dispatches a
//! `PickerRequest` over the IPC socket, awaits the response, and
//! translates it into the spec's `(response_code, results)` tuple.
//!
//! Document Portal integration for sandboxed callers (FA8) is not
//! yet wired here — that lands as a follow-up when the picker UI
//! returns real paths instead of placeholder paths. For unconfined
//! callers and Arlen-native apps the raw `file://` URIs in the
//! response are correct as-is.
//!
//! Spec:
//! https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.FileChooser.html

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use tracing::warn;
use zbus::interface;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use xdg_portal_arlen_protocol::{FileFilter, PickerRequest, PickerResponse};

use crate::document_portal;
use crate::interfaces::options;
use crate::request::{response, RequestHandle};
use crate::sandbox::CallerIdentity;
use crate::state::DaemonState;

/// Result-key the spec mandates for the URI list returned by
/// FileChooser methods. Picker UI returns absolute paths; we
/// `file://`-encode them here.
const RESULT_URIS: &str = "uris";

/// Result-key for the filter the user had selected at confirm
/// time. Echoed only when the picker actually carried one.
const RESULT_CURRENT_FILTER: &str = "current_filter";

/// Wall-clock timeout per FileChooser request (E13). Five minutes
/// is generous enough that real users browsing slow filesystems
/// have time to think while still bounding orphaned requests when
/// the Wayland compositor or the frontend portal daemon vanishes
/// mid-pick. Tests override this via `cfg(test)` if needed; runtime
/// changes would require config plumbing we have not added.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

#[derive(Clone)]
pub struct FileChooser {
    state: DaemonState,
}

impl FileChooser {
    pub fn new(state: DaemonState) -> Self {
        Self { state }
    }

    /// Common dispatch: ensure picker running, submit IPC request,
    /// translate `PickerResponse` to D-Bus tuple. Sandboxed callers
    /// have their picked paths re-exported through the Document
    /// Portal (FA8) so the URIs they receive are reachable inside
    /// their bubblewrap mount namespace.
    async fn dispatch(
        &self,
        method: &str,
        request_path: ObjectPath<'_>,
        request: PickerRequest,
        identity: CallerIdentity,
        connection: &zbus::Connection,
        writable: bool,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let _guard = self.state.track_request();
        let req = RequestHandle::from_object_path(request_path.into());
        let req_id = req.path.to_string();
        tracing::info!(
            method,
            request = %req.path,
            ?identity,
            writable,
            "FileChooser dispatch entered"
        );

        if let Err(e) = self.state.picker_lifecycle.ensure_running().await {
            tracing::warn!(request = %req.path, method, error = %e, "picker-ui spawn failed");
            return (response::OTHER, error_results(&format!("picker spawn: {e}")));
        }

        let rx = match self.state.picker_ipc.submit(request).await {
            Ok(rx) => rx,
            Err(e) => {
                tracing::warn!(request = %req.path, method, error = %e, "picker IPC submit failed");
                return (response::OTHER, error_results(&format!("picker submit: {e}")));
            }
        };

        // E13 wall-clock cap. If the picker UI hangs for any reason
        // (frontend portal daemon disappears, Wayland compositor
        // restarts, user walks away for an hour), we drop the
        // pending slot and tell the picker to dismiss the dialog
        // so the next request starts fresh.
        let response = match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => {
                tracing::warn!(request = %req.path, method, "picker IPC oneshot dropped");
                return (
                    response::OTHER,
                    error_results("picker IPC channel closed"),
                );
            }
            Err(_) => {
                tracing::warn!(
                    request = %req.path,
                    method,
                    timeout_secs = REQUEST_TIMEOUT.as_secs(),
                    "request timed out"
                );
                self.state.picker_ipc.cancel_pending(&req_id).await;
                self.state.picker_ipc.try_send_cancel(&req_id).await;
                return (
                    response::OTHER,
                    error_results(&format!(
                        "request timed out after {} seconds",
                        REQUEST_TIMEOUT.as_secs()
                    )),
                );
            }
        };

        match response {
            PickerResponse::Picked {
                paths,
                current_filter,
                ..
            } => {
                // Fail-closed on identity-resolution failure. Codex
                // flagged that returning raw file:// to a possibly-
                // sandboxed caller (because we coalesced the failure
                // into Unconfined) is a Document-Portal-bypass.
                // Real-world D-Bus glitches are rare; surfacing them
                // as a backend error here is preferable to silently
                // exposing host paths.
                if !identity.is_known() {
                    tracing::warn!(
                        request = %req.path,
                        method,
                        "rejecting picked response: caller identity could not be determined"
                    );
                    return (
                        response::OTHER,
                        error_results("could not determine caller identity"),
                    );
                }
                // Save paths come back from the picker UI as
                // `<currentDir>/<filename>` where filename is user-
                // typed. Defense in depth against the path-traversal
                // class Codex flagged: reject `..` / `.` components
                // before handing the path to the Document Portal or
                // the caller. The picker UI also rejects these in
                // the input field, but the daemon revalidates so a
                // compromised picker process cannot forge an escape.
                if writable {
                    for path in &paths {
                        if let Err(reason) = validate_save_path(path) {
                            tracing::warn!(
                                request = %req.path,
                                method,
                                path = %path.display(),
                                reason,
                                "rejected save path"
                            );
                            return (
                                response::OTHER,
                                error_results(&format!(
                                    "invalid save path {}: {reason}",
                                    path.display()
                                )),
                            );
                        }
                    }
                }
                let uris = match build_uris_for_caller(
                    &paths,
                    &identity,
                    connection,
                    writable,
                )
                .await
                {
                    Ok(uris) => uris,
                    Err(e) => {
                        tracing::warn!(
                            request = %req.path,
                            method,
                            "Document Portal export failed: {e}"
                        );
                        return (
                            response::OTHER,
                            error_results(&format!("portal export: {e}")),
                        );
                    }
                };
                (
                    response::SUCCESS,
                    success_results(&uris, current_filter.as_ref()),
                )
            }
            PickerResponse::Cancelled { .. } => (response::CANCELLED, HashMap::new()),
            PickerResponse::Error { message, .. } => {
                tracing::warn!(request = %req.path, method, "picker reported error: {message}");
                (response::OTHER, error_results(&message))
            }
        }
    }
}

/// Construct the URI list returned to the caller. Sandboxed callers
/// route through the Document Portal (FA8); unconfined callers get
/// raw `file://` URIs.
///
/// `writable` controls both the Document Portal permission list and
/// which portal call we use: read-only / OpenFile flows need
/// AddFull on existing files, write flows (Save*) need AddNamedFull
/// because the target file may not exist yet — Codex review found
/// that AddFull's underlying `open(path)` failed with ENOENT for
/// new save targets, breaking the entire SaveFile sandbox path.
async fn build_uris_for_caller(
    paths: &[PathBuf],
    identity: &CallerIdentity,
    connection: &zbus::Connection,
    writable: bool,
) -> anyhow::Result<Vec<String>> {
    match identity.app_id() {
        Some(app_id) => {
            // Sandboxed: hand paths to Document Portal so the caller
            // sees URIs that resolve inside its bubblewrap mount.
            if writable {
                document_portal::export_named_for_save(connection, app_id, paths, true).await
            } else {
                document_portal::export_for_caller(connection, app_id, paths, false).await
            }
        }
        None => {
            // Unconfined: raw `file://` URIs are reachable directly.
            Ok(paths.iter().map(|p| path_to_file_uri(p)).collect())
        }
    }
}

/// Determine caller identity from the frontend-supplied `app_id`
/// argument.
///
/// The impl-portal interface receives `app_id` set by the
/// `xdg-desktop-portal` frontend AFTER it has verified the caller
/// (by inspecting `.flatpak-info`, the snap cgroup, etc.). We trust
/// that verdict.
///
/// We deliberately do NOT use cgroup-based detection here even
/// though the helper exists in `crate::sandbox` — Flatpak callers
/// reach the bus through `xdg-dbus-proxy`, and `GetConnectionUnixProcessID`
/// reports the proxy's host-PID. Reading that PID's cgroup yields
/// the user-session scope (not the app's bubblewrap scope), so the
/// detection silently classifies real Flatpak callers as Unconfined.
/// The frontend's `app_id` argument sidesteps the proxy entirely.
///
/// Trusting that verdict is only sound while the sender IS the
/// frontend, which [`sender_is_frontend`] establishes before this runs.
fn caller_identity(method_app_id: &str) -> CallerIdentity {
    if !method_app_id.is_empty() {
        return CallerIdentity::Flatpak {
            app_id: method_app_id.to_string(),
        };
    }
    CallerIdentity::Unconfined
}

/// The public frontend whose `app_id` verdict this backend consumes.
const FRONTEND_NAME: &str = "org.freedesktop.portal.Desktop";

/// Whether this call came from the `xdg-desktop-portal` frontend.
///
/// `caller_identity` trusts the `app_id` ARGUMENT because the frontend
/// authenticated the app before re-dispatching to us. That reasoning
/// only holds for the frontend. A process that reaches our impl name
/// directly supplies the argument itself, and BOTH shapes of that claim
/// grant more than they should: a non-empty one names any grantee it
/// likes (the Document Portal permission lands on another app's id), and
/// an empty one is read as "unconfined" and answered with raw host
/// `file://` paths for whatever the user picks. Absence of evidence was
/// granting the widest access, so the check is positive: verified
/// frontend, or refuse.
///
/// The comparison is unique-name to unique-name — `GetNameOwner` returns
/// the owner's `:1.x`, which is exactly what the message header carries.
/// `arlen.portal` registers us for the frontend alone, so nothing else
/// is a legitimate caller and no supported flow is refused here.
async fn sender_is_frontend(connection: &zbus::Connection, sender: Option<&str>) -> bool {
    let owner = match zbus::fdo::DBusProxy::new(connection).await {
        Ok(proxy) => match proxy.get_name_owner(FRONTEND_NAME.try_into().unwrap()).await {
            Ok(owner) => Some(owner.as_str().to_string()),
            Err(e) => {
                // Unowned means the frontend is not running, so
                // whoever called us is not it.
                warn!("cannot resolve the {FRONTEND_NAME} owner: {e}");
                None
            }
        },
        Err(e) => {
            warn!("cannot reach the bus daemon to attest the sender: {e}");
            None
        }
    };
    let attested = sender_matches_owner(sender, owner.as_deref());
    // Both names are bus-assigned unique names, not caller text, so
    // logging them is safe and is the only way to tell a mismatch
    // apart from a sender the header never carried.
    tracing::debug!(?sender, ?owner, attested, "sender attestation");
    attested
}

/// The verdict itself, split out from the bus round-trip so every
/// branch is checkable without a broker. Both unknowns — a message
/// carrying no sender, an owner we could not resolve — are answers of
/// "not attested", never "close enough".
fn sender_matches_owner(sender: Option<&str>, owner: Option<&str>) -> bool {
    match (sender, owner) {
        (Some(sender), Some(owner)) => sender == owner,
        _ => false,
    }
}

/// The answer to a caller we could not attest as the frontend.
///
/// `OTHER` rather than `CANCELLED`: nothing was cancelled, the backend
/// declined to act. The message names the reason without echoing the
/// sender, which is attacker-controlled text.
fn refuse_unattested_sender() -> (u32, HashMap<String, OwnedValue>) {
    warn!("refusing a FileChooser call from a sender that is not the portal frontend");
    (
        response::OTHER,
        error_results("caller is not the xdg-desktop-portal frontend"),
    )
}

fn success_results(
    uris: &[String],
    current_filter: Option<&FileFilter>,
) -> HashMap<String, OwnedValue> {
    let mut map = HashMap::new();
    if let Ok(owned) = Value::new(uris.to_vec()).try_to_owned() {
        map.insert(RESULT_URIS.to_string(), owned);
    }
    // Codex P3: echo the user's selected filter back so callers with
    // multiple filters can disambiguate which one confirmed.
    if let Some(filter) = current_filter {
        match options::filter_to_value(filter).and_then(|v| v.try_to_owned()) {
            Ok(owned) => {
                map.insert(RESULT_CURRENT_FILTER.to_string(), owned);
            }
            Err(e) => {
                tracing::warn!("encode current_filter for results: {e}");
            }
        }
    }
    map
}

fn error_results(message: &str) -> HashMap<String, OwnedValue> {
    let mut map = HashMap::new();
    if let Ok(owned) = Value::new(message.to_string()).try_to_owned() {
        map.insert("arlen-error".to_string(), owned);
    }
    map
}

/// Bytes that need percent-encoding in the path component of a
/// `file://` URI per RFC 3986. The `pchar` production allows
/// `unreserved / pct-encoded / sub-delims / ":" / "@"`, plus `/`
/// for path separators. Anything outside that set must be encoded.
///
/// We start from `CONTROLS` (all 0x00-0x1F and 0x7F) and add the
/// ASCII characters that are reserved or otherwise forbidden in the
/// pchar set: space, the URI delimiters (`#`, `?`), the percent
/// itself (so `%20` round-trips through encoding), and the
/// gen-delims that are not pchars (`<`, `>`, `[`, `]`, etc.).
/// Non-ASCII bytes get encoded automatically because `utf8_percent_encode`
/// emits each multi-byte UTF-8 sequence as a series of `%xx`.
const FILE_URI_PATH_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// Encode an absolute path as a `file://` URI per RFC 8089 + RFC 3986.
/// Slashes are preserved as path separators. Reserved characters
/// (`#`, `?`, `%`) are percent-encoded so consumers cannot mis-parse
/// the URI as having a fragment, query, or partially-encoded byte.
fn path_to_file_uri(path: &Path) -> String {
    let s = path.to_string_lossy();
    let encoded = utf8_percent_encode(&s, FILE_URI_PATH_SET);
    format!("file://{encoded}")
}

/// Normalize the parent_window argument to `None` when empty so the
/// picker UI can distinguish "no parent" from "empty string". Empty
/// is what callers pass when they have no toplevel window available.
fn parse_parent_window(parent_window: &str) -> Option<String> {
    if parent_window.is_empty() {
        None
    } else {
        Some(parent_window.to_string())
    }
}

/// Validate that a Save target path does not escape its declared
/// directory. The picker UI builds save paths as
/// `<currentDir>/<typed-filename>` and a malicious or buggy filename
/// of `../../etc/passwd` would canonicalize to a path the user did
/// not see in the UI. Reject any path with `..` components, any
/// non-absolute path, and any path with NUL bytes.
///
/// `Path::components()` already normalises `.` segments away, so we
/// do not check for them explicitly — `/foo/./bar` and `/foo/bar`
/// resolve identically.
fn validate_save_path(path: &Path) -> Result<(), &'static str> {
    if !path.is_absolute() {
        return Err("not absolute");
    }
    let s = path.as_os_str().as_encoded_bytes();
    if s.contains(&0) {
        return Err("NUL byte in path");
    }
    for comp in path.components() {
        if matches!(comp, std::path::Component::ParentDir) {
            return Err("contains ..");
        }
    }
    Ok(())
}

#[interface(name = "org.freedesktop.impl.portal.FileChooser")]
#[allow(clippy::too_many_arguments)] // spec-mandated method signatures
impl FileChooser {
    async fn open_file(
        &self,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
        opts: HashMap<&str, OwnedValue>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        if !sender_is_frontend(connection, hdr.sender().map(|s| s.as_str())).await {
            return refuse_unattested_sender();
        }
        let identity = caller_identity(app_id);
        let request = PickerRequest::OpenFile {
            handle: handle.to_string(),
            app_id: app_id.to_string(),
            title: title.to_string(),
            filters: options::read_filters(&opts),
            current_filter: options::read_current_filter(&opts),
            multiple: options::read_bool(&opts, "multiple", false),
            modal: options::read_bool(&opts, "modal", true),
            directory: options::read_bool(&opts, "directory", false),
            current_folder: options::read_path_bytes(&opts, "current_folder"),
            parent_window: parse_parent_window(parent_window),
        };
        self.dispatch("OpenFile", handle, request, identity, connection, false)
            .await
    }

    async fn save_file(
        &self,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
        opts: HashMap<&str, OwnedValue>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        if !sender_is_frontend(connection, hdr.sender().map(|s| s.as_str())).await {
            return refuse_unattested_sender();
        }
        let identity = caller_identity(app_id);
        let request = PickerRequest::SaveFile {
            handle: handle.to_string(),
            app_id: app_id.to_string(),
            title: title.to_string(),
            filters: options::read_filters(&opts),
            current_filter: options::read_current_filter(&opts),
            current_name: options::read_string(&opts, "current_name"),
            current_folder: options::read_path_bytes(&opts, "current_folder"),
            current_file: options::read_path_bytes(&opts, "current_file"),
            parent_window: parse_parent_window(parent_window),
        };
        self.dispatch("SaveFile", handle, request, identity, connection, true)
            .await
    }

    async fn save_files(
        &self,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        title: &str,
        opts: HashMap<&str, OwnedValue>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        if !sender_is_frontend(connection, hdr.sender().map(|s| s.as_str())).await {
            return refuse_unattested_sender();
        }
        let identity = caller_identity(app_id);
        let request = PickerRequest::SaveFiles {
            handle: handle.to_string(),
            app_id: app_id.to_string(),
            title: title.to_string(),
            files: options::read_path_bytes_array(&opts, "files"),
            current_folder: options::read_path_bytes(&opts, "current_folder"),
            parent_window: parse_parent_window(parent_window),
        };
        self.dispatch("SaveFiles", handle, request, identity, connection, true)
            .await
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_resolved_frontend_owner_is_attested() {
        use super::sender_matches_owner;
        // The one admitted shape: the message's sender IS the unique
        // name currently owning org.freedesktop.portal.Desktop.
        assert!(sender_matches_owner(Some(":1.42"), Some(":1.42")));
        // Any other peer on the bus is not the frontend, however
        // plausible the app_id it supplies.
        assert!(!sender_matches_owner(Some(":1.99"), Some(":1.42")));
    }

    #[test]
    fn an_unresolvable_sender_or_owner_is_refused_not_assumed() {
        use super::sender_matches_owner;
        // Neither unknown may resolve toward "let it through": a
        // senderless message and an unowned frontend name are exactly
        // the states a direct caller produces, and the old code path
        // answered an unattested caller with raw host file:// URIs.
        assert!(!sender_matches_owner(None, Some(":1.42")));
        assert!(!sender_matches_owner(Some(":1.42"), None));
        assert!(!sender_matches_owner(None, None));
    }

    #[test]
    fn refusing_an_unattested_sender_is_a_backend_error_not_a_cancel() {
        use super::{refuse_unattested_sender, response};
        let (code, results) = refuse_unattested_sender();
        // CANCELLED would tell the frontend the user declined, which
        // is a lie and hides the refusal from anyone reading logs.
        assert_eq!(code, response::OTHER);
        assert_ne!(code, response::CANCELLED);
        // No URIs may ride along with a refusal.
        assert!(!results.contains_key(super::RESULT_URIS));
    }

    use super::*;
    use std::ffi::OsString;
    use std::path::PathBuf;

    /// Ordinary ASCII paths pass through unchanged after the file://
    /// prefix.
    #[test]
    fn ascii_path_to_uri() {
        let p = PathBuf::from("/home/user/Documents/report.pdf");
        assert_eq!(path_to_file_uri(&p), "file:///home/user/Documents/report.pdf");
    }

    /// Spaces are percent-encoded so the URI parses cleanly across
    /// the D-Bus boundary.
    #[test]
    fn space_in_path() {
        let p = PathBuf::from("/home/user/My Documents/x.txt");
        assert_eq!(
            path_to_file_uri(&p),
            "file:///home/user/My%20Documents/x.txt"
        );
    }

    /// Control characters become %xx. NUL is the most important one
    /// because it cannot appear literally in a D-Bus string.
    #[test]
    fn control_chars_in_path() {
        let p = PathBuf::from("/tmp/a\tb");
        assert_eq!(path_to_file_uri(&p), "file:///tmp/a%09b");
    }

    /// Codex P2: `#`, `?`, `%` must be percent-encoded so they do not
    /// turn into URI fragment / query / partial-encoding markers in
    /// consumers.
    #[test]
    fn reserved_uri_chars_in_path() {
        let p = PathBuf::from("/tmp/a#b.txt");
        assert_eq!(path_to_file_uri(&p), "file:///tmp/a%23b.txt");
        let p = PathBuf::from("/tmp/q?x.txt");
        assert_eq!(path_to_file_uri(&p), "file:///tmp/q%3Fx.txt");
        let p = PathBuf::from("/tmp/p%c.txt");
        assert_eq!(path_to_file_uri(&p), "file:///tmp/p%25c.txt");
    }

    /// Codex P2 follow-on: non-ASCII bytes percent-encode each UTF-8
    /// byte. `ä` is U+00E4 → 0xC3 0xA4 → %C3%A4.
    #[test]
    fn non_ascii_in_path() {
        let p = PathBuf::from("/home/user/Über.txt");
        assert_eq!(path_to_file_uri(&p), "file:///home/user/%C3%9Cber.txt");
    }

    /// Slashes are preserved as path separators. Forward slash is in
    /// the pchar set for paths and must not be encoded.
    #[test]
    fn slashes_preserved() {
        let p = PathBuf::from("/a/b/c");
        assert_eq!(path_to_file_uri(&p), "file:///a/b/c");
    }

    /// Codex H2: save-path validator rejects any `..` component, no
    /// matter where it appears in the path. This is the trust
    /// boundary between picker-UI typed input and the Document
    /// Portal export.
    #[test]
    fn validate_rejects_parent_dir() {
        assert!(validate_save_path(&PathBuf::from("/home/user/Documents/../passwd")).is_err());
        assert!(validate_save_path(&PathBuf::from("/../etc/passwd")).is_err());
    }

    /// `.` segments are normalised away by `Path::components` so a
    /// path with `./` is still considered clean — same target file.
    #[test]
    fn validate_accepts_current_dir_segment() {
        assert!(validate_save_path(&PathBuf::from("/home/user/./report.pdf")).is_ok());
    }

    #[test]
    fn validate_rejects_relative() {
        assert!(validate_save_path(&PathBuf::from("relative.pdf")).is_err());
        assert!(validate_save_path(&PathBuf::from("./local.pdf")).is_err());
    }

    /// A clean absolute path with no `..` passes.
    #[test]
    fn validate_accepts_clean_path() {
        assert!(validate_save_path(&PathBuf::from("/home/user/Documents/report.pdf")).is_ok());
        assert!(validate_save_path(&PathBuf::from("/tmp/a.txt")).is_ok());
    }

    /// NUL byte anywhere in the path is rejected so the save target
    /// cannot smuggle a string-truncation past D-Bus or filesystem
    /// layers.
    #[test]
    fn validate_rejects_nul_byte() {
        let mut bad = std::ffi::OsString::from("/home/user/");
        // Construct a NUL by adding a byte that the platform-aware
        // OsString accepts; use raw bytes via OsStr::from_bytes.
        use std::os::unix::ffi::OsStringExt;
        let mut bytes = bad.into_vec();
        bytes.extend_from_slice(b"foo\0bar.txt");
        bad = OsString::from_vec(bytes);
        assert!(validate_save_path(&PathBuf::from(bad)).is_err());
    }
}
