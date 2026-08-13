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

/// How long to wait for the compositor to publish its socket, and how often to
/// look. Ten seconds in hundred-millisecond steps, as the script polled.
pub const WAIT_STEPS: u32 = 100;
pub const STEP: std::time::Duration = std::time::Duration::from_millis(100);

/// Poll `list` until a Wayland display appears, or the budget runs out.
///
/// The budget is what makes this worth extracting. A session that gives up too
/// early comes up with no shell, and the cause - a compositor that was still
/// starting - is indistinguishable afterwards from one that failed to start at
/// all. So the caller gets `None` and says which it was; this only decides when to
/// stop asking.
///
/// `list` and `sleep` are injected so the timing is a test rather than a wait.
pub fn wait_for_display(
    mut list: impl FnMut() -> Vec<String>,
    mut sleep: impl FnMut(std::time::Duration),
    steps: u32,
) -> Option<String> {
    for step in 0..steps {
        let entries = list();
        let mut names: Vec<&str> = entries.iter().map(String::as_str).collect();
        // A directory read has no guaranteed order; sort so two boots of the same
        // machine pick the same socket.
        names.sort_unstable();
        if let Some(found) = first_display(names) {
            return Some(found.to_string());
        }
        // No sleep after the last look: the budget is the waiting, not the asking.
        if step + 1 < steps {
            sleep(STEP);
        }
    }
    None
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
    fn a_socket_that_appears_late_is_still_found() {
        // The compositor takes a moment; the session must not conclude failure
        // from the first look. Counted rather than waited: the sleep is injected.
        let mut looks = 0;
        let mut slept = 0;
        let found = wait_for_display(
            || {
                looks += 1;
                if looks < 5 { vec![] } else { vec!["wayland-1".into()] }
            },
            |_| slept += 1,
            100,
        );
        assert_eq!(found.as_deref(), Some("wayland-1"));
        assert_eq!(slept, 4, "one sleep per look that found nothing");
    }

    #[test]
    fn a_compositor_that_never_publishes_gives_up_and_says_nothing_was_found() {
        let mut slept = 0;
        let found = wait_for_display(Vec::new, |_| slept += 1, 3);
        assert_eq!(found, None);
        // Three looks, two waits: the budget is the waiting between them, so the
        // last look does not sleep afterwards for nothing.
        assert_eq!(slept, 2);
    }

    #[test]
    fn the_pick_does_not_depend_on_the_order_the_directory_is_read_in() {
        // Two boots of one machine must choose the same socket, or a session that
        // works intermittently looks like a compositor bug.
        for entries in [
            vec!["wayland-2".to_string(), "wayland-1".to_string()],
            vec!["wayland-1".to_string(), "wayland-2".to_string()],
        ] {
            let found = wait_for_display(|| entries.clone(), |_| {}, 1);
            assert_eq!(found.as_deref(), Some("wayland-1"));
        }
    }

    #[test]
    fn the_lock_is_skipped_even_when_it_is_read_first() {
        assert_eq!(
            first_display(["wayland-1.lock", "wayland-1"]),
            Some("wayland-1")
        );
    }
}
