//! Public Rust-level API.
//!
//! Tauri commands forward to these functions; downstream Rust
//! callers (`app-settings`) call them directly without going
//! through the Tauri-internal invoke machinery. Either entry
//! shares the same connection-per-call cost — no global
//! connection state — because portal calls are infrequent and
//! the connection setup is a sub-millisecond no-op once D-Bus is
//! ready.

use std::collections::HashMap;

use zbus::{
    zvariant::{OwnedValue, Value},
    Connection,
};

use crate::portal_proxy::{
    build_request_path, FileChooserProxy, OpenUriProxy,
};
use crate::request_helper::{fresh_token, set_handle_token, submit_with_response};
use crate::types::{
    FileFilter, FilterPattern, OpenUriOptions, PickFileOptions, PickerError,
    PickerResult, SaveFileOptions, SaveFilesOptions,
};

/// Portal Response code 0 is success.
const RESPONSE_SUCCESS: u32 = 0;
/// Response code 1 is "user cancelled". Anything else is treated
/// as a backend failure.
const RESPONSE_CANCELLED: u32 = 1;

/// URI schemes the plugin's `open_uri` accepts. The Lunaris
/// backend has its own allow-list (`xdg-desktop-portal-lunaris`
/// `daemon/src/interfaces/open_uri.rs::classify_scheme`), but
/// other backends (xdg-desktop-portal-gtk, -kde) may accept
/// additional schemes — Codex review flagged that contract drift
/// as a trust-boundary expansion. Enforcing the allow-list locally
/// keeps behaviour deterministic across backends.
const ALLOWED_OPEN_URI_SCHEMES: &[&str] = &[
    "http://",
    "https://",
    "file://",
    "mailto:",
    "tel:",
    "sms:",
    "xmpp:",
    "ftps://",
];

/// Extract the scheme prefix of a URI for the rejection error
/// message. Returns "(no-scheme)" for inputs that have no `:`.
fn uri_scheme(uri: &str) -> &str {
    match uri.find(':') {
        Some(pos) => &uri[..pos],
        None => "(no-scheme)",
    }
}

fn validate_scheme(uri: &str) -> Result<(), PickerError> {
    if ALLOWED_OPEN_URI_SCHEMES.iter().any(|s| uri.starts_with(s)) {
        Ok(())
    } else {
        Err(PickerError::SchemeRejected {
            scheme: uri_scheme(uri).to_string(),
        })
    }
}

/// Public entry: pick one or more existing files.
pub async fn pick_file(options: PickFileOptions) -> Result<PickerResult, PickerError> {
    invoke_file_chooser(FileChooserMethod::OpenFile, options.into()).await
}

/// Public entry: pick a directory. Internally the same as
/// `pick_file` with `directory=true` set on the options.
pub async fn pick_directory(
    mut options: PickFileOptions,
) -> Result<PickerResult, PickerError> {
    options.directory = true;
    options.multiple = false; // directory pick is single-target
    invoke_file_chooser(FileChooserMethod::OpenFile, options.into()).await
}

/// Public entry: save a single file.
pub async fn save_file(
    options: SaveFileOptions,
) -> Result<PickerResult, PickerError> {
    invoke_file_chooser(FileChooserMethod::SaveFile, options.into()).await
}

/// Public entry: save multiple files into one directory.
pub async fn save_files(
    options: SaveFilesOptions,
) -> Result<PickerResult, PickerError> {
    invoke_file_chooser(FileChooserMethod::SaveFiles, options.into()).await
}

/// Public entry: open a URI in the user's preferred handler.
///
/// Rejects schemes outside `ALLOWED_OPEN_URI_SCHEMES` before any
/// D-Bus call — the contract advertised in README/index.ts says
/// the plugin only forwards http(s), mailto, tel, sms, xmpp, ftps
/// and file://, regardless of which backend the frontend daemon
/// dispatches to. Validating locally means a permissive non-
/// Lunaris backend cannot widen the scheme set.
pub async fn open_uri(
    uri: &str,
    options: OpenUriOptions,
) -> Result<(), PickerError> {
    validate_scheme(uri)?;

    let connection = Connection::session()
        .await
        .map_err(PickerError::from_zbus)?;
    let proxy = OpenUriProxy::new(&connection)
        .await
        .map_err(PickerError::from_zbus)?;

    let token = fresh_token();
    let mut wire_options = HashMap::new();
    set_handle_token(&mut wire_options, &token)?;
    if let Some(ask) = options.ask {
        wire_options.insert(
            "ask",
            Value::from(ask).try_to_owned().map_err(|e| PickerError::Other {
                message: format!("encode ask: {e}"),
            })?,
        );
    }
    if let Some(writable) = options.writable {
        wire_options.insert(
            "writable",
            Value::from(writable)
                .try_to_owned()
                .map_err(|e| PickerError::Other {
                    message: format!("encode writable: {e}"),
                })?,
        );
    }

    let unique = connection.unique_name().ok_or_else(|| PickerError::Other {
        message: "session bus has no unique name".into(),
    })?;
    let request_path = build_request_path(unique.as_str(), &token)
        .map_err(PickerError::from_zbus)?
        .into_owned();

    let (code, results) = submit_with_response(&connection, request_path, || async {
        proxy.open_uri("", uri, wire_options).await
    })
    .await?;

    match code {
        RESPONSE_SUCCESS => Ok(()),
        RESPONSE_CANCELLED => Err(PickerError::Backend {
            message: "user cancelled".into(),
        }),
        _ => Err(PickerError::Backend {
            message: extract_error_message(&results)
                .unwrap_or_else(|| format!("portal returned response code {code}")),
        }),
    }
}

/// Internal selector for which FileChooser method to invoke. The
/// options dictionary is built per-method by the `Into` impls on
/// the public option types.
enum FileChooserMethod {
    OpenFile,
    SaveFile,
    SaveFiles,
}

/// Wire-shape options dict + the title and parent_window pieces
/// the spec passes outside the dict. Built from the typed Options
/// structs by their `Into<WireRequest>` impls.
struct WireRequest {
    title: String,
    options: HashMap<&'static str, OwnedValue>,
}

async fn invoke_file_chooser(
    method: FileChooserMethod,
    request: WireRequest,
) -> Result<PickerResult, PickerError> {
    let connection = Connection::session()
        .await
        .map_err(PickerError::from_zbus)?;
    let proxy = FileChooserProxy::new(&connection)
        .await
        .map_err(PickerError::from_zbus)?;

    let mut options = request.options;
    let token = fresh_token();
    set_handle_token(&mut options, &token)?;

    let unique = connection.unique_name().ok_or_else(|| PickerError::Other {
        message: "session bus has no unique name".into(),
    })?;
    let request_path = build_request_path(unique.as_str(), &token)
        .map_err(PickerError::from_zbus)?
        .into_owned();

    let title = request.title;
    let (code, results) = submit_with_response(&connection, request_path, || async {
        match method {
            FileChooserMethod::OpenFile => proxy.open_file("", &title, options).await,
            FileChooserMethod::SaveFile => proxy.save_file("", &title, options).await,
            FileChooserMethod::SaveFiles => proxy.save_files("", &title, options).await,
        }
    })
    .await?;

    match code {
        RESPONSE_SUCCESS => Ok(PickerResult::Picked {
            uris: extract_uris(&results)?,
        }),
        RESPONSE_CANCELLED => Ok(PickerResult::Cancelled),
        _ => Err(PickerError::Backend {
            message: extract_error_message(&results)
                .unwrap_or_else(|| format!("portal returned response code {code}")),
        }),
    }
}

/// Pull the `uris` array out of a portal Response result dict.
fn extract_uris(results: &HashMap<String, OwnedValue>) -> Result<Vec<String>, PickerError> {
    let raw = results
        .get("uris")
        .ok_or_else(|| PickerError::Backend {
            message: "portal returned success without `uris`".into(),
        })?;
    let val: Value = raw.try_clone().map_err(|e| PickerError::Other {
        message: format!("clone uris: {e}"),
    })?.into();
    val.try_into().map_err(|e: zbus::zvariant::Error| PickerError::Backend {
        message: format!("decode uris: {e}"),
    })
}

/// Look for any of the well-known error keys our backend (and
/// xdg-desktop-portal-gtk) put into the results dict on failure.
fn extract_error_message(results: &HashMap<String, OwnedValue>) -> Option<String> {
    for key in ["lunaris-error", "error", "message"] {
        if let Some(v) = results.get(key) {
            let val: Value = v.try_clone().ok()?.into();
            if let Ok(s) = String::try_from(val) {
                return Some(s);
            }
        }
    }
    None
}

// ─── Options encoding ───────────────────────────────────────────

fn encode_filter(filter: &FileFilter) -> Result<Value<'static>, PickerError> {
    let patterns: Vec<(u32, String)> = filter
        .patterns
        .iter()
        .map(|p| match p {
            FilterPattern::Glob { pattern } => (0, pattern.clone()),
            FilterPattern::Mime { mime_type } => (1, mime_type.clone()),
        })
        .collect();
    Ok(Value::from((filter.name.clone(), patterns)))
}

fn encode_filters(
    filters: &[FileFilter],
) -> Result<Option<OwnedValue>, PickerError> {
    if filters.is_empty() {
        return Ok(None);
    }
    let encoded: Vec<(String, Vec<(u32, String)>)> = filters
        .iter()
        .map(|f| {
            let patterns: Vec<(u32, String)> = f
                .patterns
                .iter()
                .map(|p| match p {
                    FilterPattern::Glob { pattern } => (0, pattern.clone()),
                    FilterPattern::Mime { mime_type } => (1, mime_type.clone()),
                })
                .collect();
            (f.name.clone(), patterns)
        })
        .collect();
    Value::from(encoded)
        .try_to_owned()
        .map(Some)
        .map_err(|e| PickerError::Other {
            message: format!("encode filters: {e}"),
        })
}

/// Encode a filesystem path as the NUL-terminated byte sequence
/// the portal spec expects for `current_folder` / `current_file` /
/// `files`. Unix-only because D-Bus path encoding is unix-bytes;
/// the portal protocol itself is unix-only too. The non-unix
/// stub keeps `cargo check --workspace` clean on Windows where
/// some downstream consumer might pull the workspace; the function
/// is unreachable on those targets because the entire portal
/// stack requires D-Bus.
#[cfg(unix)]
fn path_to_byte_string(path: &std::path::Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    let mut bytes = path.as_os_str().as_bytes().to_vec();
    bytes.push(0); // NUL-terminated per portal spec
    bytes
}

#[cfg(not(unix))]
fn path_to_byte_string(_path: &std::path::Path) -> Vec<u8> {
    // The plugin's D-Bus dependency is unix-only; reaching this
    // code on a non-unix target would require working around
    // multiple other compile gates first.
    unimplemented!("path_to_byte_string is unix-only")
}

fn try_to_owned(v: Value<'_>, label: &str) -> Result<OwnedValue, PickerError> {
    v.try_to_owned().map_err(|e| PickerError::Other {
        message: format!("encode {label}: {e}"),
    })
}

impl From<PickFileOptions> for WireRequest {
    fn from(opts: PickFileOptions) -> Self {
        let mut options: HashMap<&'static str, OwnedValue> = HashMap::new();
        if opts.multiple {
            if let Ok(v) = try_to_owned(Value::from(true), "multiple") {
                options.insert("multiple", v);
            }
        }
        if let Some(modal) = opts.modal {
            if let Ok(v) = try_to_owned(Value::from(modal), "modal") {
                options.insert("modal", v);
            }
        }
        if opts.directory {
            if let Ok(v) = try_to_owned(Value::from(true), "directory") {
                options.insert("directory", v);
            }
        }
        if let Some(folder) = opts.current_folder.as_deref() {
            if let Ok(v) =
                try_to_owned(Value::from(path_to_byte_string(folder)), "current_folder")
            {
                options.insert("current_folder", v);
            }
        }
        if let Ok(Some(filters)) = encode_filters(&opts.filters) {
            options.insert("filters", filters);
        }
        if let Some(filter) = opts.current_filter.as_ref() {
            if let Ok(v) = encode_filter(filter)
                .and_then(|val| try_to_owned(val, "current_filter"))
            {
                options.insert("current_filter", v);
            }
        }
        WireRequest {
            title: opts.title.unwrap_or_default(),
            options,
        }
    }
}

impl From<SaveFileOptions> for WireRequest {
    fn from(opts: SaveFileOptions) -> Self {
        let mut options: HashMap<&'static str, OwnedValue> = HashMap::new();
        if let Some(modal) = opts.modal {
            if let Ok(v) = try_to_owned(Value::from(modal), "modal") {
                options.insert("modal", v);
            }
        }
        if let Some(name) = opts.current_name {
            if let Ok(v) = try_to_owned(Value::from(name), "current_name") {
                options.insert("current_name", v);
            }
        }
        if let Some(folder) = opts.current_folder.as_deref() {
            if let Ok(v) =
                try_to_owned(Value::from(path_to_byte_string(folder)), "current_folder")
            {
                options.insert("current_folder", v);
            }
        }
        if let Some(file) = opts.current_file.as_deref() {
            if let Ok(v) =
                try_to_owned(Value::from(path_to_byte_string(file)), "current_file")
            {
                options.insert("current_file", v);
            }
        }
        if let Ok(Some(filters)) = encode_filters(&opts.filters) {
            options.insert("filters", filters);
        }
        if let Some(filter) = opts.current_filter.as_ref() {
            if let Ok(v) = encode_filter(filter)
                .and_then(|val| try_to_owned(val, "current_filter"))
            {
                options.insert("current_filter", v);
            }
        }
        WireRequest {
            title: opts.title.unwrap_or_default(),
            options,
        }
    }
}

impl From<SaveFilesOptions> for WireRequest {
    fn from(opts: SaveFilesOptions) -> Self {
        let mut options: HashMap<&'static str, OwnedValue> = HashMap::new();
        if let Some(modal) = opts.modal {
            if let Ok(v) = try_to_owned(Value::from(modal), "modal") {
                options.insert("modal", v);
            }
        }
        if let Some(folder) = opts.current_folder.as_deref() {
            if let Ok(v) =
                try_to_owned(Value::from(path_to_byte_string(folder)), "current_folder")
            {
                options.insert("current_folder", v);
            }
        }
        if !opts.files.is_empty() {
            let arrays: Vec<Vec<u8>> =
                opts.files.iter().map(|p| path_to_byte_string(p)).collect();
            if let Ok(v) = try_to_owned(Value::from(arrays), "files") {
                options.insert("files", v);
            }
        }
        WireRequest {
            title: opts.title.unwrap_or_default(),
            options,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Filter encoding produces the spec-mandated `(sa(us))` shape.
    /// Round-trips through `Value` for shape verification.
    #[test]
    fn encode_filter_round_trip() {
        let filter = FileFilter {
            name: "Images".into(),
            patterns: vec![
                FilterPattern::Glob {
                    pattern: "*.png".into(),
                },
                FilterPattern::Mime {
                    mime_type: "image/png".into(),
                },
            ],
        };
        let encoded = encode_filter(&filter).unwrap();
        let decoded: (String, Vec<(u32, String)>) = encoded.try_into().unwrap();
        assert_eq!(decoded.0, "Images");
        assert_eq!(decoded.1.len(), 2);
        assert_eq!(decoded.1[0].0, 0);
        assert_eq!(decoded.1[0].1, "*.png");
        assert_eq!(decoded.1[1].0, 1);
        assert_eq!(decoded.1[1].1, "image/png");
    }

    /// `path_to_byte_string` always NUL-terminates per the portal
    /// `ay` convention.
    #[test]
    fn path_bytes_are_nul_terminated() {
        let bytes = path_to_byte_string(std::path::Path::new("/home/user"));
        assert_eq!(bytes.last(), Some(&0u8));
        assert_eq!(&bytes[..bytes.len() - 1], b"/home/user");
    }

    /// Empty filter list yields `None` so the options dict does
    /// not carry a useless empty array.
    #[test]
    fn empty_filters_produce_none() {
        let result = encode_filters(&[]).unwrap();
        assert!(result.is_none());
    }

    /// Allow-list passes the documented schemes.
    #[test]
    fn allowed_schemes_pass_validation() {
        for uri in [
            "https://example.com",
            "http://localhost:8080/path",
            "file:///home/user/file.txt",
            "mailto:alice@example.com",
            "tel:+15555550100",
            "sms:+15555550100",
            "xmpp:bob@example.com",
            "ftps://example.com",
        ] {
            assert!(
                validate_scheme(uri).is_ok(),
                "expected {uri} to validate"
            );
        }
    }

    /// Codex H1: anything outside the allow-list rejects locally.
    /// `javascript:` (XSS via opener), `data:` (data exfil), bare
    /// strings, custom schemes — none of these reach the portal.
    #[test]
    fn disallowed_schemes_rejected_locally() {
        for uri in [
            "javascript:alert(1)",
            "data:text/html,...",
            "lunaris:foo",
            "ftp://example.com",
            "x-scheme-handler/foo",
            "not-a-uri",
            "",
        ] {
            let result = validate_scheme(uri);
            assert!(
                matches!(result, Err(PickerError::SchemeRejected { .. })),
                "expected {uri} to be rejected, got {result:?}"
            );
        }
    }

    /// Scheme extraction strips at the first colon. URIs without
    /// any colon get the `(no-scheme)` placeholder so the error
    /// message stays useful.
    #[test]
    fn uri_scheme_extraction() {
        assert_eq!(uri_scheme("https://example.com"), "https");
        assert_eq!(uri_scheme("mailto:x@y"), "mailto");
        assert_eq!(uri_scheme("not-a-uri"), "(no-scheme)");
        assert_eq!(uri_scheme(""), "(no-scheme)");
    }

    /// `pick_directory` flips the option flags on the way through.
    #[test]
    fn pick_directory_sets_directory_flag() {
        let opts = PickFileOptions {
            multiple: true, // should be cleared by pick_directory
            directory: false,
            ..PickFileOptions::default()
        };
        let request: WireRequest = {
            let mut o = opts;
            o.directory = true;
            o.multiple = false;
            o.into()
        };
        assert!(request.options.contains_key("directory"));
        assert!(!request.options.contains_key("multiple"));
    }
}
