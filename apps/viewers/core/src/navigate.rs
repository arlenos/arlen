// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Next and previous within the folder, which is what makes this a viewer
//! rather than a file-opener.
//!
//! The reference tool is imv, and its defining behaviour is that opening one
//! picture puts you in the folder: arrow keys walk the pictures beside it. That
//! is the affordance missing here - `quickview-plan.md` names it first, and
//! nothing in the app implements it.
//!
//! WHAT "BESIDE IT" MEANS, decided here rather than left to whatever `read_dir`
//! returns:
//!
//!   - the same directory only, never recursive. A viewer that wandered into
//!     subfolders would make "next" unpredictable, and the user asked to see one
//!     picture;
//!   - the same MEDIA KIND as the file that was opened. Landing on a song while
//!     arrowing through holiday photos is not next, it is a surprise - and the
//!     three kinds have different viewers, so it would swap the whole surface;
//!   - sorted by name, case-insensitively, with the raw name as the tie-break so
//!     the order is total. `read_dir` order is the filesystem's, which is stable
//!     for nobody and differs between the same folder on two machines;
//!   - wrapping at both ends. imv wraps, and a viewer that stops dead on the last
//!     picture makes the user guess whether it is the end or a bug.
//!
//! Pure, and takes the listing rather than reading the disk, so the ordering and
//! wrap rules are testable without a temp directory per case.

use crate::{detect_by_extension, MediaKind};

/// The viewable files beside `current`, in the order the viewer walks them.
///
/// `entries` is the directory's file names, in any order. The result contains
/// only names of the same media kind as `current`, and `current` itself when it
/// is viewable - a caller that opened a file the viewer can show always finds it
/// in its own neighbour list.
pub fn siblings(current: &str, entries: &[String]) -> Vec<String> {
    let Some(kind) = kind_of(current) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .iter()
        .filter(|name| kind_of(name) == Some(kind))
        .cloned()
        .collect();
    // Case-insensitive, then the raw name. Without the second key the order is
    // only a partial one - `A.png` and `a.png` would compare equal and their
    // relative order would come back to `read_dir` again, which is the thing this
    // sort exists to remove.
    out.sort_by(|a, b| {
        a.to_lowercase()
            .cmp(&b.to_lowercase())
            .then_with(|| a.cmp(b))
    });
    out
}

/// The file after `current` in its folder, wrapping to the first.
///
/// `None` when there is nothing to move to: `current` is not viewable, or it is
/// the only viewable file there. Returning `current` itself in that case would
/// make the key appear to work while changing nothing.
pub fn next(current: &str, entries: &[String]) -> Option<String> {
    step(current, entries, 1)
}

/// The file before `current` in its folder, wrapping to the last.
pub fn previous(current: &str, entries: &[String]) -> Option<String> {
    step(current, entries, -1)
}

fn step(current: &str, entries: &[String], delta: isize) -> Option<String> {
    let list = siblings(current, entries);
    if list.len() < 2 {
        return None;
    }
    let here = list.iter().position(|n| n == current)?;
    let len = list.len() as isize;
    let idx = ((here as isize + delta) % len + len) % len;
    Some(list[idx as usize].clone())
}

fn kind_of(name: &str) -> Option<MediaKind> {
    detect_by_extension(name).map(|d| d.kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_neighbours_are_the_same_kind_only() {
        let dir = names(&["b.png", "song.flac", "a.jpg", "notes.txt", "c.webp"]);
        assert_eq!(siblings("a.jpg", &dir), names(&["a.jpg", "b.png", "c.webp"]));
        // And from the song's side, the pictures are not its neighbours.
        assert_eq!(siblings("song.flac", &dir), names(&["song.flac"]));
    }

    #[test]
    fn the_order_does_not_depend_on_the_filesystem() {
        // The same folder handed over in two different orders walks the same way.
        let one = names(&["b.png", "A.png", "c.png", "a.png"]);
        let two = names(&["c.png", "a.png", "b.png", "A.png"]);
        assert_eq!(siblings("a.png", &one), siblings("a.png", &two));
        // Case-insensitive first, raw name as the tie-break, so `A.png` and
        // `a.png` have a defined order rather than read_dir's.
        assert_eq!(
            siblings("a.png", &one),
            names(&["A.png", "a.png", "b.png", "c.png"])
        );
    }

    #[test]
    fn next_and_previous_wrap_at_both_ends() {
        let dir = names(&["a.png", "b.png", "c.png"]);
        assert_eq!(next("a.png", &dir).as_deref(), Some("b.png"));
        assert_eq!(next("c.png", &dir).as_deref(), Some("a.png"), "wraps forward");
        assert_eq!(previous("a.png", &dir).as_deref(), Some("c.png"), "wraps back");
        assert_eq!(previous("b.png", &dir).as_deref(), Some("a.png"));
    }

    #[test]
    fn a_lone_picture_has_nowhere_to_go() {
        // `Some(current)` here would make the arrow key look like it worked.
        let dir = names(&["only.png", "song.flac", "notes.txt"]);
        assert_eq!(next("only.png", &dir), None);
        assert_eq!(previous("only.png", &dir), None);
    }

    #[test]
    fn a_file_the_viewer_cannot_show_has_no_neighbours() {
        let dir = names(&["notes.txt", "a.png"]);
        assert!(siblings("notes.txt", &dir).is_empty());
        assert_eq!(next("notes.txt", &dir), None);
    }

    #[test]
    fn a_file_missing_from_its_own_listing_does_not_move() {
        // Deleted between listing and keypress: stepping from a name that is not
        // there has no defined answer, so it refuses rather than guessing.
        let dir = names(&["a.png", "b.png"]);
        assert_eq!(next("gone.png", &dir), None);
    }
}
