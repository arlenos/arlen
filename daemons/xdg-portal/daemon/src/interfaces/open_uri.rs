//! `org.freedesktop.impl.portal.OpenURI` implementation.
//!
//! Two methods: `OpenURI` (a string URI) and `OpenFile` (a file
//! descriptor). Caller-controlled URIs go through a scheme allow-list
//! per Sprint-E A1 pre-read: `http(s)://` passes through to
//! `xdg-open`, `file://` is sandbox-validated for confined callers
//! before forwarding, `mailto:` / `tel:` / `sms:` pass through, and
//! everything else is rejected.
//!
//! Spec:
//! https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.OpenURI.html
//!
//! `OpenFile` (fd variant) is not yet wired — it returns OTHER with
//! a clear "not implemented" error so callers fall through to
//! whatever fallback they have (typically a file:// URI). Real apps
//! use `OpenURI` overwhelmingly; the fd path is rare and can land
//! as a follow-up.

use std::collections::HashMap;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use zbus::interface;
use zbus::zvariant::{Fd, ObjectPath, OwnedValue, Value};

use crate::request::{response, RequestHandle};
use crate::sandbox::CallerIdentity;
use crate::state::DaemonState;

/// Schemes we forward to `xdg-open` without question. `mailto:` and
/// friends are authorityless but the kernel's xdg-open dispatcher
/// understands them. `https?` is the bulk of real-world traffic.
const PASSTHROUGH_SCHEMES: &[&str] = &[
    "http://",
    "https://",
    "mailto:",
    "tel:",
    "sms:",
    "xmpp:",
    "ftps://",
];

/// Schemes we explicitly reject. Listed for readability and for the
/// rejection log lines; `classify_scheme` returns `Rejected` for
/// anything not in `PASSTHROUGH_SCHEMES` or starting with `file://`.
const REJECTED_SCHEMES: &[&str] = &["javascript:", "data:", "vbscript:", "arlen:"];

/// Document Portal mount root as a filesystem path.
/// `/run/user/<uid>/doc/`. Sandboxed callers can only OpenURI
/// `file://` URIs that resolve inside this mount.
fn document_portal_mount_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let mut p = PathBuf::from(runtime);
    p.push("doc");
    Some(p)
}

/// Same path-percent-encoding set as the FileChooser URI helper,
/// duplicated here for self-containment when the OpenFile (fd)
/// path constructs a `file://` URI from a /proc/self/fd readlink.
const URI_PATH_SET: &AsciiSet = &CONTROLS
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

/// Parse a `file://` URI into a filesystem path. Percent-decodes,
/// rejects path-traversal segments, and refuses anything that
/// does not resolve to an absolute path — Codex flagged that the
/// previous string-prefix authorization let
/// `file:///run/user/1000/doc/../../etc/passwd` pass because
/// `starts_with` is not a containment check.
fn parse_file_uri(uri: &str) -> Result<PathBuf, &'static str> {
    let suffix = uri.strip_prefix("file://").ok_or("not a file:// URI")?;
    // Strip optional host (always empty for local file://).
    let path_part = suffix.split('/').enumerate().fold(
        String::new(),
        |mut acc, (idx, seg)| {
            if idx == 0 && seg.is_empty() {
                acc.push('/');
            } else if idx == 0 && !seg.is_empty() {
                // file://host/path — the host segment we drop.
                // Path starts at the next slash.
            } else {
                if !acc.ends_with('/') {
                    acc.push('/');
                }
                acc.push_str(seg);
            }
            acc
        },
    );
    if path_part.is_empty() {
        return Err("empty path");
    }
    let decoded = percent_decode_str(&path_part)
        .decode_utf8()
        .map_err(|_| "invalid UTF-8 percent-encoding")?;
    let path = PathBuf::from(decoded.as_ref());
    if !path.is_absolute() {
        return Err("not absolute");
    }
    if path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err("NUL byte");
    }
    for comp in path.components() {
        if matches!(comp, std::path::Component::ParentDir) {
            return Err("contains ..");
        }
    }
    Ok(path)
}

#[derive(Debug, PartialEq, Eq)]
enum SchemeClass {
    /// http(s), mailto, tel, sms, xmpp — pass straight through.
    Passthrough,
    /// `file://` — needs caller-identity-aware validation.
    File,
    /// Explicitly rejected scheme (javascript:, data:, arlen:, ...).
    Rejected,
}

fn classify_scheme(uri: &str) -> SchemeClass {
    if uri.starts_with("file://") {
        return SchemeClass::File;
    }
    if PASSTHROUGH_SCHEMES.iter().any(|s| uri.starts_with(s)) {
        return SchemeClass::Passthrough;
    }
    SchemeClass::Rejected
}

/// Pure version of `file_uri_authorized` that takes the Document
/// Portal mount path as an argument. Cargo tests run in parallel
/// and share the process environment; tests that mutated
/// `XDG_RUNTIME_DIR` would race against each other, so the public
/// helper threads the resolved path through this function and
/// tests pass a literal.
///
/// Authorization rules:
/// - `Unknown` (identity-resolution failed) → deny. Codex flagged
///   that fail-open here let a transient D-Bus glitch waive the
///   sandbox check.
/// - `Unconfined` → allow. The caller could open the file from a
///   shell anyway.
/// - `Flatpak`/`Snap` → URI must parse cleanly (no traversal,
///   no NUL, percent-decode UTF-8) AND the resulting path must
///   start with the Document Portal mount path. The path-based
///   check replaces the previous string-prefix check that was
///   bypassable via `file:///mount/../escape` (Codex critical).
fn file_uri_authorized_with_prefix(
    uri: &str,
    identity: &CallerIdentity,
    document_mount: Option<&Path>,
) -> bool {
    if matches!(identity, CallerIdentity::Unknown) {
        return false;
    }
    if matches!(identity, CallerIdentity::Unconfined) {
        return true;
    }
    let Some(mount) = document_mount else {
        return false;
    };
    let Ok(path) = parse_file_uri(uri) else {
        return false;
    };
    // `starts_with` alone is a string comparison, and a symlink INSIDE the mount
    // pointing outside it satisfies it while resolving anywhere on the host. So
    // both sides are resolved before comparing: the mount too, because a
    // canonical path measured against a symlinked mount prefix would wrongly
    // fail. Either resolution failing - the file is gone, a component is not
    // traversable, the mount does not exist - refuses; a path we cannot resolve
    // is a path we cannot vouch for.
    let (Ok(real_path), Ok(real_mount)) = (path.canonicalize(), mount.canonicalize()) else {
        return false;
    };
    real_path.starts_with(real_mount)
}

/// Sandbox-authorisation gate for `file://` URIs. See the
/// `_with_prefix` variant for rules.
fn file_uri_authorized(uri: &str, identity: &CallerIdentity) -> bool {
    let mount = document_portal_mount_path();
    file_uri_authorized_with_prefix(uri, identity, mount.as_deref())
}

/// Same path-percent-encoding semantics as the FileChooser URI
/// helper, applied here only for the redacted log line. The
/// production response shape just forwards the URI as-is to
/// xdg-open — we do not rewrite caller URIs.
fn redact_uri(uri: &str) -> String {
    if let Some(scheme_end) = uri.find("://") {
        let after_scheme = &uri[scheme_end + 3..];
        let host_end = after_scheme
            .find(['/', '?', '#'])
            .unwrap_or(after_scheme.len());
        let host = &after_scheme[..host_end];
        return format!("{}://{}/...", &uri[..scheme_end], host);
    }
    if let Some(colon) = uri.find(':') {
        return format!("{}:...", &uri[..colon]);
    }
    "<unparseable>".to_string()
}

#[derive(Clone)]
pub struct OpenUri {
    state: DaemonState,
}

impl OpenUri {
    pub fn new(state: DaemonState) -> Self {
        Self { state }
    }

    /// Determine caller identity from the frontend-supplied
    /// `app_id` argument. See `file_chooser::caller_identity` for
    /// the rationale on not using cgroup detection.
    fn caller_identity(method_app_id: &str) -> CallerIdentity {
        if !method_app_id.is_empty() {
            return CallerIdentity::Flatpak {
                app_id: method_app_id.to_string(),
            };
        }
        CallerIdentity::Unconfined
    }
}

fn error_results(message: &str) -> HashMap<String, OwnedValue> {
    let mut map = HashMap::new();
    if let Ok(owned) = Value::new(message.to_string()).try_to_owned() {
        map.insert("arlen-error".to_string(), owned);
    }
    map
}

#[interface(name = "org.freedesktop.impl.portal.OpenURI")]
#[allow(clippy::too_many_arguments)] // spec-mandated method signatures
impl OpenUri {
    /// Open a URI in the user's preferred handler.
    #[zbus(name = "OpenURI")]
    async fn open_uri(
        &self,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        uri: &str,
        _options: HashMap<&str, OwnedValue>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        // §2: only the authenticated frontend may reach this impl name; a
        // direct caller would open an arbitrary URI under a forged app_id.
        if !crate::interfaces::sender_is_frontend(connection, hdr.sender().map(|s| s.as_str())).await
        {
            tracing::warn!("refusing an OpenURI call from a sender that is not the portal frontend");
            return (
                response::OTHER,
                error_results("caller is not the xdg-desktop-portal frontend"),
            );
        }
        let _guard = self.state.track_request();
        let req = RequestHandle::from_object_path(handle.into());
        let identity = Self::caller_identity(app_id);
        let redacted = redact_uri(uri);

        match classify_scheme(uri) {
            SchemeClass::Passthrough => {
                tracing::info!(
                    request = %req.path,
                    app_id,
                    parent_window,
                    uri = %redacted,
                    identity = ?identity,
                    "OpenURI passthrough"
                );
                spawn_xdg_open(uri).await
            }
            SchemeClass::File => {
                if !file_uri_authorized(uri, &identity) {
                    tracing::warn!(
                        request = %req.path,
                        app_id,
                        uri = %redacted,
                        identity = ?identity,
                        "OpenURI file:// rejected — not in Document Portal mount"
                    );
                    return (
                        response::OTHER,
                        error_results(
                            "file:// URIs from sandboxed callers must point inside the Document Portal mount",
                        ),
                    );
                }
                tracing::info!(
                    request = %req.path,
                    app_id,
                    uri = %redacted,
                    identity = ?identity,
                    "OpenURI file:// authorised"
                );
                spawn_xdg_open(uri).await
            }
            SchemeClass::Rejected => {
                tracing::warn!(
                    request = %req.path,
                    app_id,
                    uri = %redacted,
                    "OpenURI rejected scheme"
                );
                let listed = REJECTED_SCHEMES
                    .iter()
                    .find(|s| uri.starts_with(*s))
                    .map(|s| s.trim_end_matches(':'))
                    .unwrap_or("unsupported");
                (
                    response::OTHER,
                    error_results(&format!("scheme not allowed: {listed}")),
                )
            }
        }
    }

    /// Open a file descriptor in the user's preferred handler.
    /// The fd is dup'd into the daemon process so we can resolve
    /// its filesystem path via `/proc/self/fd/<n>`, then the path
    /// is authorized against the caller's sandbox identity exactly
    /// like a file:// URI before xdg-open sees it.
    async fn open_file(
        &self,
        handle: ObjectPath<'_>,
        app_id: &str,
        parent_window: &str,
        fd: Fd<'_>,
        _options: HashMap<&str, OwnedValue>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        // §2: frontend-only, as with OpenURI (a direct caller could open a
        // handed fd under a forged app_id, bypassing the sandbox check).
        if !crate::interfaces::sender_is_frontend(connection, hdr.sender().map(|s| s.as_str())).await
        {
            tracing::warn!("refusing an OpenFile call from a sender that is not the portal frontend");
            return (
                response::OTHER,
                error_results("caller is not the xdg-desktop-portal frontend"),
            );
        }
        let _guard = self.state.track_request();
        let req = RequestHandle::from_object_path(handle.into());
        let identity = Self::caller_identity(app_id);

        let path = match resolve_fd_to_path(&fd) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    request = %req.path,
                    app_id,
                    parent_window,
                    "OpenFile fd resolution failed: {e}"
                );
                return (
                    response::OTHER,
                    error_results(&format!("resolve fd: {e}")),
                );
            }
        };

        let uri = format!(
            "file://{}",
            utf8_percent_encode(&path.to_string_lossy(), URI_PATH_SET)
        );

        if !file_uri_authorized(&uri, &identity) {
            tracing::warn!(
                request = %req.path,
                app_id,
                identity = ?identity,
                path = %path.display(),
                "OpenFile fd rejected — not in Document Portal mount or identity unknown"
            );
            return (
                response::OTHER,
                error_results(
                    "fd target is not authorised for this caller — file must be inside the Document Portal mount",
                ),
            );
        }

        // The containment check above ran on the name this fd resolved to. Confirm
        // the name STILL leads to that same file before handing it to a process
        // that will resolve it again; fail closed if it does not, or if we cannot
        // tell.
        match fd_still_points_at(&fd, &path) {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!(
                    request = %req.path,
                    app_id,
                    path = %path.display(),
                    "OpenFile refused - the path no longer resolves to the descriptor that was authorised"
                );
                return (
                    response::OTHER,
                    error_results("fd target changed between the check and the open"),
                );
            }
            Err(e) => {
                tracing::warn!(
                    request = %req.path,
                    app_id,
                    "OpenFile refused - could not re-confirm the fd target: {e}"
                );
                return (
                    response::OTHER,
                    error_results("could not confirm the fd target"),
                );
            }
        }

        tracing::info!(
            request = %req.path,
            app_id,
            parent_window,
            path = %path.display(),
            "OpenFile fd authorised"
        );
        spawn_xdg_open(&uri).await
    }
}

/// Resolve a borrowed D-Bus file descriptor to the absolute path
/// it currently points at. Works by dup-ing the fd into our
/// process (so the kernel keeps the inode pinned for our lookup)
/// and reading the magic `/proc/self/fd/<n>` symlink that the
/// kernel maintains for every open fd.
fn resolve_fd_to_path(fd: &Fd<'_>) -> Result<PathBuf, std::io::Error> {
    let owned = fd.as_fd().try_clone_to_owned()?;
    let raw = std::os::fd::AsRawFd::as_raw_fd(&owned);
    let proc_path = format!("/proc/self/fd/{raw}");
    std::fs::read_link(proc_path)
}

/// Does `path` still name the very file the caller handed us?
///
/// `xdg-open` takes a URI, not a descriptor, so the fd the caller passed cannot
/// be handed onward - the name gets resolved a second time, by another process,
/// after we have decided. Between our decision and that resolution the name can
/// be pointed somewhere else, and the authorisation we granted for the fd's
/// inode would be spent on a different file.
///
/// So the name is checked against the fd it came from, immediately before the
/// spawn: same device, same inode, following symlinks exactly as `xdg-open`
/// will. A swapped target no longer matches and the call is refused.
///
/// This narrows the window rather than closing it - the name is still resolved
/// once more inside `xdg-open`, and nothing here can hold a lock across another
/// process's `open`. Closing it entirely needs a handler that accepts the
/// descriptor itself. What this removes is the wide window between resolving
/// the fd, running the containment check and spawning, and it makes a swap in
/// that window detectable rather than silent.
fn fd_still_points_at(fd: &Fd<'_>, path: &Path) -> Result<bool, std::io::Error> {
    use std::os::unix::fs::MetadataExt;
    // The dup is ours; dropping the `File` below closes the copy, not the
    // caller's descriptor. And `fstat` is permitted on an `O_PATH` handle - which
    // this may well be, since that is how the portal opens files itself - so the
    // metadata read does not refuse the very descriptors it is meant to check.
    // Verified rather than assumed: an O_PATH fd reports dev/ino here.
    let owned = fd.as_fd().try_clone_to_owned()?;
    let from_fd = std::fs::File::from(owned).metadata()?;
    // `metadata`, not `symlink_metadata`: xdg-open follows symlinks, so the
    // question is what the name resolves TO, not what the last component is.
    let from_name = std::fs::metadata(path)?;
    Ok(from_fd.dev() == from_name.dev() && from_fd.ino() == from_name.ino())
}

/// Spawn `xdg-open` for the given URI, fire-and-forget. xdg-open
/// is the standard freedesktop dispatcher; it forwards to the
/// user's configured browser, mail client, or default file
/// handler depending on the URI.
async fn spawn_xdg_open(uri: &str) -> (u32, HashMap<String, OwnedValue>) {
    let result = tokio::process::Command::new("xdg-open")
        .arg(uri)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn();
    match result {
        Ok(_child) => (response::SUCCESS, HashMap::new()),
        Err(e) => (
            response::OTHER,
            error_results(&format!("xdg-open spawn failed: {e}")),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The swap this check exists for: the caller hands a descriptor to a file
    /// inside the mount, we authorise it by name, and the name is then pointed at
    /// something else before the opener resolves it. The descriptor still holds
    /// the original inode, so the two stop agreeing and the call is refused.
    #[test]
    fn a_path_pointed_elsewhere_no_longer_matches_its_descriptor() {
        use std::os::fd::AsFd;

        let dir = tempfile::tempdir().unwrap();
        let authorised = dir.path().join("authorised.txt");
        let attacker = dir.path().join("attacker.txt");
        std::fs::write(&authorised, b"the file the caller handed us").unwrap();
        std::fs::write(&attacker, b"something else entirely").unwrap();

        let file = std::fs::File::open(&authorised).unwrap();
        let fd = Fd::from(file.as_fd());

        assert!(
            fd_still_points_at(&fd, &authorised).unwrap(),
            "the untouched path is the descriptor's own file"
        );

        // The swap. The descriptor keeps the original inode open; the NAME now
        // leads somewhere else, which is what the opener would follow.
        std::fs::rename(&attacker, &authorised).unwrap();
        assert!(
            !fd_still_points_at(&fd, &authorised).unwrap(),
            "a path pointed at another file must stop matching"
        );
    }

    /// A path that has gone entirely is an error, not a pass. Fail closed.
    #[test]
    fn a_vanished_path_cannot_be_confirmed() {
        use std::os::fd::AsFd;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.txt");
        std::fs::write(&target, b"x").unwrap();
        let file = std::fs::File::open(&target).unwrap();
        let fd = Fd::from(file.as_fd());
        std::fs::remove_file(&target).unwrap();

        assert!(fd_still_points_at(&fd, &target).is_err());
    }

    /// http(s), mailto, tel, sms classify as Passthrough.
    #[test]
    fn classify_passthrough_schemes() {
        assert_eq!(
            classify_scheme("https://example.com"),
            SchemeClass::Passthrough
        );
        assert_eq!(
            classify_scheme("http://example.com/path?x=1"),
            SchemeClass::Passthrough
        );
        assert_eq!(
            classify_scheme("mailto:alice@example.com"),
            SchemeClass::Passthrough
        );
        assert_eq!(classify_scheme("tel:+15555550100"), SchemeClass::Passthrough);
        assert_eq!(classify_scheme("sms:+15555550100"), SchemeClass::Passthrough);
    }

    /// `file://` is its own class (sandbox-validated).
    #[test]
    fn classify_file_scheme() {
        assert_eq!(
            classify_scheme("file:///home/user/x.txt"),
            SchemeClass::File
        );
        assert_eq!(
            classify_scheme("file:///run/user/1000/doc/abc/x"),
            SchemeClass::File
        );
    }

    /// Anything outside the allow-list classifies as Rejected.
    /// Notable: javascript: (XSS via opener), data: (data exfil),
    /// arlen: (no-op), and bare strings (no scheme).
    #[test]
    fn classify_rejected_schemes() {
        assert_eq!(
            classify_scheme("javascript:alert(1)"),
            SchemeClass::Rejected
        );
        assert_eq!(classify_scheme("data:text/html,..."), SchemeClass::Rejected);
        assert_eq!(classify_scheme("arlen:foo"), SchemeClass::Rejected);
        assert_eq!(classify_scheme("ftp://example.com"), SchemeClass::Rejected);
        assert_eq!(classify_scheme("not-a-uri"), SchemeClass::Rejected);
        assert_eq!(classify_scheme(""), SchemeClass::Rejected);
    }

    fn doc_mount() -> Option<&'static Path> {
        Some(Path::new("/run/user/1000/doc"))
    }

    /// Unconfined callers can open any `file://` URI — they could
    /// already do that from a shell anyway. Independent of
    /// `XDG_RUNTIME_DIR`, so we test through the pure helper to
    /// avoid env-var races with parallel tests.
    #[test]
    fn file_uri_unconfined_always_authorised() {
        let id = CallerIdentity::Unconfined;
        assert!(file_uri_authorized_with_prefix(
            "file:///etc/passwd",
            &id,
            doc_mount()
        ));
        assert!(file_uri_authorized_with_prefix(
            "file:///home/user/file.txt",
            &id,
            doc_mount()
        ));
        // Even with `None` mount — unconfined is unconditional.
        assert!(file_uri_authorized_with_prefix(
            "file:///etc/passwd",
            &id,
            None
        ));
    }

    /// Sandboxed callers (Flatpak, Snap) only get `file://` URIs that resolve
    /// inside the Document Portal mount.
    ///
    /// Against a REAL directory, because the check resolves both sides: a test
    /// over invented paths would pass against a version that never looks at the
    /// filesystem, which is precisely the version that missed the symlink below.
    #[test]
    fn file_uri_sandboxed_only_doc_portal() {
        let id = CallerIdentity::Flatpak {
            app_id: "org.gnome.Calculator".into(),
        };
        let dir = tempfile::tempdir().unwrap();
        let mount = dir.path().join("doc");
        std::fs::create_dir_all(mount.join("abc")).unwrap();
        let inside = mount.join("abc/report.pdf");
        std::fs::write(&inside, b"report").unwrap();
        let outside = dir.path().join("secret.txt");
        std::fs::write(&outside, b"not for the app").unwrap();

        assert!(file_uri_authorized_with_prefix(
            &format!("file://{}", inside.display()),
            &id,
            Some(&mount)
        ));
        assert!(!file_uri_authorized_with_prefix(
            &format!("file://{}", outside.display()),
            &id,
            Some(&mount)
        ));
        assert!(!file_uri_authorized_with_prefix(
            "file:///etc/passwd",
            &id,
            Some(&mount)
        ));
    }

    /// The reason the check resolves rather than compares strings: a symlink
    /// sitting inside the mount, pointing out of it. Its NAME is under the mount
    /// and satisfies any prefix test; what it opens is not. The Document Portal
    /// hands out app-scoped paths, so a link placed there is exactly the shape an
    /// escape would take.
    #[test]
    fn a_symlink_inside_the_mount_cannot_reach_outside_it() {
        let id = CallerIdentity::Flatpak {
            app_id: "org.gnome.Calculator".into(),
        };
        let dir = tempfile::tempdir().unwrap();
        let mount = dir.path().join("doc");
        std::fs::create_dir_all(&mount).unwrap();
        let secret = dir.path().join("secret.txt");
        std::fs::write(&secret, b"not for the app").unwrap();

        let escape = mount.join("looks-innocent.txt");
        std::os::unix::fs::symlink(&secret, &escape).unwrap();
        // The name really is under the mount, so the old string test said yes.
        assert!(escape.starts_with(&mount));

        assert!(
            !file_uri_authorized_with_prefix(
                &format!("file://{}", escape.display()),
                &id,
                Some(&mount)
            ),
            "a link out of the mount must not be authorised by its name"
        );
    }

    /// And a symlink that stays inside the mount is still fine - resolving must
    /// not turn into refusing every link.
    #[test]
    fn a_symlink_within_the_mount_is_still_authorised() {
        let id = CallerIdentity::Flatpak {
            app_id: "org.gnome.Calculator".into(),
        };
        let dir = tempfile::tempdir().unwrap();
        let mount = dir.path().join("doc");
        std::fs::create_dir_all(mount.join("abc")).unwrap();
        let real = mount.join("abc/report.pdf");
        std::fs::write(&real, b"report").unwrap();
        let link = mount.join("shortcut.pdf");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        assert!(file_uri_authorized_with_prefix(
            &format!("file://{}", link.display()),
            &id,
            Some(&mount)
        ));
    }

    /// Without a mount (no `XDG_RUNTIME_DIR`), sandboxed callers
    /// cannot reach any file:// URI — better to refuse than to
    /// guess at a mount path.
    #[test]
    fn file_uri_sandboxed_without_prefix() {
        let id = CallerIdentity::Flatpak {
            app_id: "x".into(),
        };
        assert!(!file_uri_authorized_with_prefix(
            "file:///run/user/1000/doc/abc/x",
            &id,
            None
        ));
    }

    /// Codex CRITICAL: path traversal in the URI must not bypass
    /// the mount-membership check. `file:///mount/../etc/passwd`
    /// previously satisfied `starts_with(mount-prefix)`; now it
    /// is rejected at parse time before the prefix check runs.
    #[test]
    fn file_uri_traversal_rejected_for_sandboxed() {
        let id = CallerIdentity::Flatpak {
            app_id: "x".into(),
        };
        assert!(!file_uri_authorized_with_prefix(
            "file:///run/user/1000/doc/../../../etc/passwd",
            &id,
            doc_mount()
        ));
        assert!(!file_uri_authorized_with_prefix(
            "file:///run/user/1000/doc/abc/../etc/passwd",
            &id,
            doc_mount()
        ));
    }

    /// Percent-encoded `..` is also caught. The classic
    /// `..` → `%2E%2E` smuggling is rejected because we
    /// percent-decode before walking the components.
    #[test]
    fn file_uri_percent_encoded_traversal_rejected() {
        let id = CallerIdentity::Flatpak {
            app_id: "x".into(),
        };
        // %2E%2E is `..`
        assert!(!file_uri_authorized_with_prefix(
            "file:///run/user/1000/doc/%2E%2E/%2E%2E/etc/passwd",
            &id,
            doc_mount()
        ));
    }

    /// Codex HIGH: identity-resolution failure must fail-closed
    /// for file:// URIs even if the URI looks safe.
    #[test]
    fn file_uri_unknown_identity_denies() {
        let id = CallerIdentity::Unknown;
        assert!(!file_uri_authorized_with_prefix(
            "file:///run/user/1000/doc/abc/x",
            &id,
            doc_mount()
        ));
        // Including paths an Unconfined caller would be allowed.
        assert!(!file_uri_authorized_with_prefix(
            "file:///home/user/notes.md",
            &id,
            doc_mount()
        ));
    }

    /// `parse_file_uri` rejects malformed and unsafe inputs.
    /// Path-traversal segments are caught before the prefix check
    /// runs.
    #[test]
    fn parse_rejects_traversal_and_non_file_schemes() {
        assert!(parse_file_uri("file:///foo/../bar").is_err());
        assert!(parse_file_uri("file:foo").is_err());
        assert!(parse_file_uri("https://example.com").is_err());
    }

    /// RFC 8089 `file://host/path` form drops the host and parses
    /// to `/path`. We accept this since the path is still absolute
    /// and clean, but the test pins the behaviour so future
    /// refactors don't accidentally start accepting traversal in
    /// the host segment.
    #[test]
    fn parse_drops_host_segment() {
        let p = parse_file_uri("file://localhost/etc/hostname").unwrap();
        assert_eq!(p, PathBuf::from("/etc/hostname"));
    }

    #[test]
    fn parse_decodes_percent_encoded_path() {
        let p = parse_file_uri("file:///home/user/My%20Documents/x.txt").unwrap();
        assert_eq!(p, PathBuf::from("/home/user/My Documents/x.txt"));
    }

    /// Codex review-style coverage for the redactor: secret-bearing
    /// query strings strip cleanly.
    #[test]
    fn redact_strips_query_and_fragment() {
        assert_eq!(
            redact_uri("https://example.com/secret?token=abc#frag"),
            "https://example.com/..."
        );
        assert_eq!(
            redact_uri("mailto:secret@example.com"),
            "mailto:..."
        );
        assert_eq!(redact_uri(""), "<unparseable>");
    }
}
