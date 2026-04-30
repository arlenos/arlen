//! zbus proxies for the `org.freedesktop.portal.Desktop` frontend.
//!
//! These are the *frontend* interfaces that any portal client
//! talks to — the daemon dispatches the actual work to whichever
//! backend is registered for the current desktop. We do not call
//! the Lunaris backend (`org.freedesktop.impl.portal.desktop.lunaris`)
//! directly; that would skip the routing layer and break under
//! non-Lunaris sessions.

use std::collections::HashMap;

use zbus::zvariant::{Fd, ObjectPath, OwnedObjectPath, OwnedValue};

#[zbus::proxy(
    interface = "org.freedesktop.portal.FileChooser",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
pub trait FileChooser {
    fn open_file(
        &self,
        parent_window: &str,
        title: &str,
        options: HashMap<&str, OwnedValue>,
    ) -> zbus::Result<OwnedObjectPath>;

    fn save_file(
        &self,
        parent_window: &str,
        title: &str,
        options: HashMap<&str, OwnedValue>,
    ) -> zbus::Result<OwnedObjectPath>;

    fn save_files(
        &self,
        parent_window: &str,
        title: &str,
        options: HashMap<&str, OwnedValue>,
    ) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.portal.OpenURI",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
pub trait OpenUri {
    #[zbus(name = "OpenURI")]
    fn open_uri(
        &self,
        parent_window: &str,
        uri: &str,
        options: HashMap<&str, OwnedValue>,
    ) -> zbus::Result<OwnedObjectPath>;

    fn open_file(
        &self,
        parent_window: &str,
        fd: Fd<'_>,
        options: HashMap<&str, OwnedValue>,
    ) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.portal.Request",
    default_service = "org.freedesktop.portal.Desktop"
)]
pub trait Request {
    #[zbus(signal)]
    fn response(
        &self,
        response: u32,
        results: HashMap<String, OwnedValue>,
    ) -> zbus::Result<()>;

    fn close(&self) -> zbus::Result<()>;
}

/// Build a deterministic Request object path for our caller-token.
///
/// The portal spec at
/// https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Request.html
/// defines the path as
/// `/org/freedesktop/portal/desktop/request/<sender>/<token>`,
/// where `<sender>` is the caller's unique D-Bus name with `.`
/// replaced by `_` and the leading `:` stripped. Knowing the path
/// up-front lets us subscribe to the Response signal BEFORE
/// invoking the method, which closes the race where a fast
/// backend could deliver Response before our subscription arrives.
pub fn build_request_path<'a>(
    unique_name: &str,
    token: &str,
) -> zbus::Result<ObjectPath<'a>> {
    let sanitized = unique_name
        .strip_prefix(':')
        .unwrap_or(unique_name)
        .replace('.', "_");
    let raw = format!("/org/freedesktop/portal/desktop/request/{sanitized}/{token}");
    ObjectPath::try_from(raw).map_err(zbus::Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Documented path-construction rule: leading colon stripped,
    /// dots replaced with underscores, token concatenated.
    #[test]
    fn request_path_construction() {
        let path = build_request_path(":1.42", "abc123").unwrap();
        assert_eq!(
            path.as_str(),
            "/org/freedesktop/portal/desktop/request/1_42/abc123"
        );
    }

    #[test]
    fn request_path_handles_already_sanitized() {
        // Defensive — a caller passing an already-stripped name
        // should still produce a valid path.
        let path = build_request_path("1.42", "tok").unwrap();
        assert_eq!(
            path.as_str(),
            "/org/freedesktop/portal/desktop/request/1_42/tok"
        );
    }
}
