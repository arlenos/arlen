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


/// One folder the client can show, and where its messages live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// The rail it belongs to.
    pub kind: FolderKind,
    /// Its path relative to the maildir root: `""` for the inbox, `.Sent` for a
    /// subfolder. Relative rather than absolute because it is half of a message
    /// id, and an id that carried the person's home directory would put their
    /// username in the surface's DOM.
    pub rel: String,
    /// How many of its messages have not been read.
    pub unread: usize,
}

/// One list row, before the boundary turns it into the surface's `Envelope`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The message's path under the maildir root, which is its identity here.
    pub id: String,
    /// The folder it was found in, as that folder's `rel`.
    pub folder: String,
    /// The `From` line as written. Unverified, like everywhere else.
    pub from: Option<String>,
    /// The `Subject` line as written.
    pub subject: Option<String>,
    /// Sent time in epoch milliseconds, when the message carried a date.
    pub date_ms: Option<i64>,
    /// Whether the filename says it has been seen.
    pub unread: bool,
}

/// Whether `dir` looks like a maildir: a `cur` and a `new` beneath it.
///
/// `tmp` is not required. It is part of the delivery protocol rather than of the
/// mailbox, and a mailbox copied without it is still a mailbox to read.
#[must_use]
pub fn is_maildir(dir: &std::path::Path) -> bool {
    dir.join("cur").is_dir() && dir.join("new").is_dir()
}

/// The folders under `root`, inbox first and the rest in a stable order.
///
/// A root that is not a maildir yields nothing, which is how "there is no
/// mailbox here" reaches the surface - as an empty list rather than an invented
/// inbox. Sorted so two runs on one machine agree; the surface's rail order is
/// its own business.
#[must_use]
pub fn folders(root: &std::path::Path) -> Vec<Folder> {
    if !is_maildir(root) {
        return Vec::new();
    }
    let mut out = vec![Folder {
        kind: FolderKind::Inbox,
        rel: String::new(),
        unread: unread_in(root),
    }];
    let mut subs: Vec<Folder> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let kind = kind_of(&name)?;
            let dir = e.path();
            is_maildir(&dir).then(|| Folder { kind, unread: unread_in(&dir), rel: name })
        })
        .collect();
    subs.sort_by(|a, b| a.rel.cmp(&b.rel));
    out.append(&mut subs);
    out
}

/// How many messages in this folder have not been read.
fn unread_in(dir: &std::path::Path) -> usize {
    let new = std::fs::read_dir(dir.join("new")).into_iter().flatten().flatten().count();
    let cur_unseen = std::fs::read_dir(dir.join("cur"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !seen(&e.file_name().to_string_lossy()))
        .count();
    new + cur_unseen
}

/// Turn an id from the surface into a path, or `None` if it must not become one.
///
/// Two gates, not one: [`safe_id`] rejects the shapes that traverse, and the
/// resolved path is then required to still be under `root` - so a symlink inside
/// the maildir cannot point out of it. The first gate is about what was typed,
/// the second about what the filesystem does with it, and neither implies the
/// other.
#[must_use]
pub fn message_path(root: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
    if !safe_id(id) {
        return None;
    }
    let joined = root.join(id);
    let real = std::fs::canonicalize(&joined).ok()?;
    let real_root = std::fs::canonicalize(root).ok()?;
    real.starts_with(&real_root).then_some(real)
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

    /// A maildir under a fresh temp directory. Built by hand rather than with a
    /// fixture crate: this crate has no dev-dependencies and one directory tree
    /// is not worth the first.
    fn a_maildir() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "arlen-maildir-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        for d in ["cur", "new", ".Sent/cur", ".Sent/new", ".Work.2019/cur", ".Work.2019/new"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("new/1.host"), b"Subject: unread\n\nbody\n").unwrap();
        std::fs::write(root.join("cur/2.host:2,S"), b"Subject: read\n\nbody\n").unwrap();
        std::fs::write(root.join("cur/3.host:2,R"), b"Subject: replied\n\nbody\n").unwrap();
        std::fs::write(root.join(".Sent/cur/4.host:2,S"), b"Subject: sent\n\nbody\n").unwrap();
        root
    }

    #[test]
    fn a_directory_that_is_not_a_maildir_yields_no_folders() {
        // The state on most machines, and it must reach the surface as "no
        // mailbox" rather than as an inbox with nothing in it.
        let empty = std::env::temp_dir().join(format!("arlen-not-a-maildir-{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        assert!(folders(&empty).is_empty());
        std::fs::remove_dir_all(&empty).ok();
    }

    #[test]
    fn the_inbox_leads_and_an_unrecognised_folder_is_left_out() {
        let root = a_maildir();
        let fs = folders(&root);
        assert_eq!(fs[0].kind, FolderKind::Inbox);
        assert_eq!(fs[0].rel, "");
        let rels: Vec<&str> = fs.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(rels, vec!["", ".Sent"], "`.Work.2019` has no rail: {rels:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn unread_counts_new_and_the_unflagged_in_cur() {
        let root = a_maildir();
        let fs = folders(&root);
        // `new/1` plus `cur/3` (flagged R, never S). `cur/2` is Seen.
        assert_eq!(fs[0].unread, 2);
        assert_eq!(fs[1].unread, 0, ".Sent's only message is Seen");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_id_resolves_to_a_path_inside_the_maildir() {
        let root = a_maildir();
        assert!(message_path(&root, "cur/2.host:2,S").is_some());
        assert!(message_path(&root, "../../etc/passwd").is_none());
        assert!(message_path(&root, "cur/nothing-here").is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_out_of_the_maildir_is_refused_by_the_second_gate() {
        // THE case for canonicalising. `safe_id` sees a name with no `..` in it
        // and passes; only resolving the link shows it leaves the mailbox.
        let root = a_maildir();
        let outside = std::env::temp_dir().join(format!("arlen-outside-{}", std::process::id()));
        std::fs::write(&outside, b"not yours\n").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("cur/escape")).unwrap();
        assert!(safe_id("cur/escape"), "the name itself looks ordinary");
        assert!(
            message_path(&root, "cur/escape").is_none(),
            "a link out of the maildir must not resolve"
        );
        std::fs::remove_file(&outside).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_nul_in_an_id_is_refused_before_it_reaches_the_syscall() {
        assert!(!safe_id("cur/mail\0.txt"));
    }
}
