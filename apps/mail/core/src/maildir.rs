// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! A mailbox on this disk, in Maildir layout.
//!
//! WHY A MAILDIR AND NOT THE SYNC MODEL. `sync.rs` is §1's invariant and it is
//! right: a message is keyed on `(identity, UIDVALIDITY, UID)` and a key that can
//! be compared across validities must not exist. Both halves of that are things
//! only a SERVER issues, and §5 leaves the IMAP client layer an open choice - so
//! there is no account, no store and nobody to issue one. Minting a UIDVALIDITY
//! for a local folder to make the types fit would be a key claiming a server
//! stood behind it, which is the one thing this crate is careful never to do.
//!
//! A maildir needs no server, no account and no network, and a message's identity
//! there IS its filename - stable without anyone promising it. So this is a
//! SECOND source beside the eventual IMAP one, not a stand-in for it, and
//! `sync.rs` stays untouched and correct for when that lands.
//!
//! WHAT MAKES A MESSAGE UNREAD, from the Maildir convention rather than a guess:
//! anything under `new/` has not been looked at, and under `cur/` the filename
//! carries flags after `:2,` where `S` means Seen. A file in `cur/` with no flag
//! section is therefore unread, which is the reading a person expects and the one
//! a naive "it is in cur, so it is read" gets wrong.

/// Which of the five rails a folder belongs to.
///
/// The surface's `FolderKind` verbatim: it renders one icon and one empty
/// sentence per kind, so a sixth would arrive as an unhandled string rather
/// than as a new rail.
/// No `Serialize` here on purpose: this crate carries no serde and the Tauri
/// side owns the wire shape, the way `MessageDto` already maps `message::Message`
/// rather than the core deriving its own JSON. The boundary is where a rename or
/// a casing decision belongs - and the mail surface needs both casings, since its
/// `Envelope` is camelCase while its `Message` is the snake_case `mail_read`
/// already emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderKind {
    /// The maildir root itself.
    Inbox,
    /// `.Sent`.
    Sent,
    /// `.Drafts`.
    Drafts,
    /// `.Archive`.
    Archive,
    /// `.Trash`.
    Trash,
}

/// The kind a maildir subdirectory name stands for, or `None` for one this
/// client has no rail for.
///
/// Maildir++ names a subfolder with a leading dot; the comparison is
/// case-insensitive because the name is whatever created the folder wrote, and
/// `Sent` and `sent` are the same rail to a person. A folder this does not
/// recognise is skipped rather than guessed at: showing somebody's `.Work.2019`
/// as an Archive would file their mail somewhere they did not put it.
#[must_use]
pub fn kind_of(dir_name: &str) -> Option<FolderKind> {
    match dir_name.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "sent" | "sent items" | "sent messages" => Some(FolderKind::Sent),
        "drafts" => Some(FolderKind::Drafts),
        "archive" | "archives" => Some(FolderKind::Archive),
        "trash" | "deleted" | "deleted items" => Some(FolderKind::Trash),
        _ => None,
    }
}

/// Whether a maildir filename in `cur/` has been seen.
///
/// The flag section is everything after the LAST `:2,`, per the convention. A
/// name with no section is unread - a message that has never been flagged has
/// never been read.
#[must_use]
pub fn seen(file_name: &str) -> bool {
    match file_name.rsplit_once(":2,") {
        Some((_, flags)) => flags.contains('S'),
        None => false,
    }
}

/// Whether an id from the surface is one this may turn into a path.
///
/// AN ID COMES BACK FROM THE FRONTEND, so it is untrusted input that becomes a
/// filesystem path - the same shape `mail_save_attachment` already guards, and
/// the reason it takes an index rather than a name. Rejected: anything absolute,
/// any `..` component, any NUL, any empty segment. A rejection is silent to the
/// caller by design; there is no id a person can type, so a bad one is a bug or
/// an attack and neither deserves a sentence explaining the layout.
#[must_use]
pub fn safe_id(id: &str) -> bool {
    if id.is_empty() || id.starts_with('/') || id.contains('\0') {
        return false;
    }
    id.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rails_are_named_however_the_folder_was_created() {
        assert_eq!(kind_of(".Sent"), Some(FolderKind::Sent));
        assert_eq!(kind_of("sent"), Some(FolderKind::Sent));
        assert_eq!(kind_of(".Sent Items"), Some(FolderKind::Sent));
        assert_eq!(kind_of(".Trash"), Some(FolderKind::Trash));
        assert_eq!(kind_of(".Deleted Items"), Some(FolderKind::Trash));
    }

    #[test]
    fn a_folder_with_no_rail_is_skipped_rather_than_guessed_at() {
        // Somebody's own filing. Calling this an Archive would move their mail
        // to a rail they did not choose.
        assert_eq!(kind_of(".Work.2019"), None);
        assert_eq!(kind_of(".Lists.rust"), None);
    }

    #[test]
    fn a_message_is_read_only_when_its_name_says_so() {
        assert!(seen("1699999999.M1P2.host:2,S"));
        assert!(seen("1699999999.M1P2.host:2,RS"));
        assert!(!seen("1699999999.M1P2.host:2,R"));
        // No flag section at all: never flagged, so never read. The reading a
        // "it is in cur/, so it is read" shortcut gets wrong.
        assert!(!seen("1699999999.M1P2.host"));
    }

    #[test]
    fn an_id_that_would_leave_the_maildir_is_refused() {
        assert!(safe_id("cur/1699999999.M1P2.host:2,S"));
        assert!(safe_id(".Sent/cur/1699999999.M1P2.host"));
        for bad in [
            "",
            "/etc/passwd",
            "../../.ssh/id_rsa",
            "cur/../../../etc/passwd",
            "cur//passwd",
            "cur/./passwd",
        ] {
            assert!(!safe_id(bad), "{bad} must be refused");
        }
    }

    #[test]
    fn a_nul_in_an_id_is_refused_before_it_reaches_the_syscall() {
        assert!(!safe_id("cur/mail\0.txt"));
    }
}
