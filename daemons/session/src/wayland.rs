//! Picking the compositor's Wayland socket out of the runtime directory.
//!
//! cosmic-comp auto-allocates its socket and publishes it under
//! `$XDG_RUNTIME_DIR`; the session captures the name into `WAYLAND_DISPLAY` so the
//! shell can connect. The subtlety worth extracting is the `.lock` sibling: every
//! `wayland-N` socket has a `wayland-N.lock` beside it, and taking that as the
//! display gives every client a name nothing is listening on - a session that
//! comes up with no shell and no error.

/// The display name for a socket path, or `None` when the entry is not one.
///
/// A lock file is not a socket, and a caller that only checks the prefix will
/// find one first about half the time.
pub fn display_name(file_name: &str) -> Option<&str> {
    if !file_name.starts_with("wayland-") || file_name.ends_with(".lock") {
        return None;
    }
    Some(file_name)
}

/// The first Wayland display among `entries`, in the order given.
///
/// Deterministic on purpose: a directory read has no guaranteed order, so the
/// caller sorts and this takes the first. A session that picks a different
/// compositor's socket on alternate boots is worse than one that always picks the
/// same wrong one, because only the second is diagnosable.
pub fn first_display<'a>(entries: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    entries.into_iter().find_map(display_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lock_file_is_not_a_display() {
        // The one that would hand every client a name nothing listens on.
        assert_eq!(display_name("wayland-1.lock"), None);
        assert_eq!(display_name("wayland-1"), Some("wayland-1"));
    }

    #[test]
    fn unrelated_entries_are_ignored() {
        for other in ["bus", "arlen", "pulse", "wayland"] {
            assert_eq!(display_name(other), None, "{other}");
        }
        // `wayland` alone is not a display either: the name carries the number.
        assert_eq!(display_name("wayland-0"), Some("wayland-0"));
    }

    #[test]
    fn the_lock_is_skipped_even_when_it_is_read_first() {
        assert_eq!(
            first_display(["wayland-1.lock", "wayland-1"]),
            Some("wayland-1")
        );
    }
}
