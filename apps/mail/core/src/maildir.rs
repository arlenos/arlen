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
    /// The opening of the body, whitespace collapsed.
    pub snippet: String,
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

/// The list rows for one folder, newest first.
///
/// WHY A LIGHT PARSE RATHER THAN [`crate::message::read`]. That one does the work
/// a READER needs - the alternative-part divergence, the exfiltration channels,
/// the ambiguity refusals - and a list row needs none of it. Running it per row
/// would do the expensive half of opening every message in the folder in order to
/// show a subject line. `mail_open` still uses it, so nothing the reader shows is
/// weakened; this is the same parser asked a smaller question.
///
/// THE COST IS STILL REAL and worth saying: building the list reads every message
/// in the folder. That is fine for a maildir somebody keeps and would not be for a
/// synced account, where the envelopes belong in an index. When the IMAP backend
/// lands it brings its own list; this is not the thing to grow into one.
///
/// An unreadable or unparseable file is SKIPPED rather than shown as a broken
/// row: it is not a message, and a row that cannot be opened is worse than a row
/// that is not there.
#[must_use]
pub fn envelopes(root: &std::path::Path, folder: &Folder) -> Vec<Row> {
    let dir = if folder.rel.is_empty() { root.to_path_buf() } else { root.join(&folder.rel) };
    let mut rows: Vec<Row> = ["new", "cur"]
        .iter()
        .flat_map(|sub| std::fs::read_dir(dir.join(sub)).into_iter().flatten().flatten())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let sub = e.path().parent()?.file_name()?.to_string_lossy().into_owned();
            let raw = std::fs::read(e.path()).ok()?;
            let parsed = mail_parser::MessageParser::default().parse(&raw)?;
            let rel = if folder.rel.is_empty() {
                format!("{sub}/{name}")
            } else {
                format!("{}/{sub}/{name}", folder.rel)
            };
            Some(Row {
                id: rel,
                folder: folder.rel.clone(),
                // The ADDRESS, not the display name, exactly as `message::read`
                // decided: a display name is whatever the sender typed.
                from: parsed.from().and_then(|a| a.first()).and_then(|a| a.address().map(str::to_string)),
                subject: parsed.subject().map(str::to_string),
                date_ms: parsed.date().map(|d| d.to_timestamp() * 1000),
                unread: sub == "new" || !seen(&name),
                snippet: snippet_of(parsed.body_text(0).as_deref()),
            })
        })
        .collect();
    // Newest first, and a message with no date sorts last rather than first: an
    // undated message is not news, and putting it at the top would let a sender
    // who omits a Date header lead somebody's inbox.
    rows.sort_by(|a, b| b.date_ms.unwrap_or(i64::MIN).cmp(&a.date_ms.unwrap_or(i64::MIN)));
    rows
}

/// The first line or so of a body, for the list row.
///
/// Whitespace is collapsed so a quoted-printable body full of newlines does not
/// become a row of nothing, and the cut is on a CHARACTER boundary rather than a
/// byte one - a snippet is somebody's mail and half a codepoint is a crash.
fn snippet_of(text: Option<&str>) -> String {
    let Some(text) = text else { return String::new() };
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(140) {
        Some((cut, _)) => flat[..cut].to_string(),
        None => flat,
    }
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

    /// A maildir whose messages carry dates, for the ordering tests.
    fn a_dated_maildir() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "arlen-maildir-dated-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(root.join("cur")).unwrap();
        std::fs::create_dir_all(root.join("new")).unwrap();
        std::fs::write(
            root.join("cur/old.host:2,S"),
            b"From: a@example.com\nSubject: older\nDate: Mon, 1 Jan 2024 10:00:00 +0000\n\nfirst\n",
        )
        .unwrap();
        std::fs::write(
            root.join("cur/new.host:2,S"),
            b"From: b@example.com\nSubject: newer\nDate: Tue, 2 Jan 2024 10:00:00 +0000\n\nsecond\n",
        )
        .unwrap();
        std::fs::write(
            root.join("new/undated.host"),
            b"From: c@example.com\nSubject: no date\n\nthird\n",
        )
        .unwrap();
        root
    }

    #[test]
    fn the_list_is_newest_first_and_an_undated_message_does_not_lead_it() {
        let root = a_dated_maildir();
        let inbox = folders(&root).into_iter().next().unwrap();
        let rows = envelopes(&root, &inbox);
        let subjects: Vec<&str> = rows.iter().filter_map(|r| r.subject.as_deref()).collect();
        // A sender who omits Date must not be able to lead somebody's inbox.
        assert_eq!(subjects, vec!["newer", "older", "no date"], "{subjects:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_row_carries_the_address_the_snippet_and_the_id_that_opens_it() {
        let root = a_dated_maildir();
        let inbox = folders(&root).into_iter().next().unwrap();
        let rows = envelopes(&root, &inbox);
        let newest = &rows[0];
        assert_eq!(newest.from.as_deref(), Some("b@example.com"));
        assert_eq!(newest.snippet, "second");
        assert_eq!(newest.id, "cur/new.host:2,S");
        // The id it just handed out must be one that opens.
        assert!(message_path(&root, &newest.id).is_some());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn everything_in_new_is_unread_however_its_name_reads() {
        let root = a_dated_maildir();
        let inbox = folders(&root).into_iter().next().unwrap();
        let rows = envelopes(&root, &inbox);
        let undated = rows.iter().find(|r| r.subject.as_deref() == Some("no date")).unwrap();
        assert!(undated.unread, "it is in new/");
        let seen_one = rows.iter().find(|r| r.subject.as_deref() == Some("newer")).unwrap();
        assert!(!seen_one.unread, "cur/ and flagged S");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_snippet_is_cut_on_a_character_and_never_mid_codepoint() {
        // A body of multi-byte characters longer than the cut. Slicing on byte
        // 140 here would panic; this is the test that says so.
        let long = "ü".repeat(300);
        let out = snippet_of(Some(&long));
        assert_eq!(out.chars().count(), 140);
    }

    #[test]
    fn a_body_of_newlines_becomes_a_readable_line() {
        assert_eq!(snippet_of(Some("one\n\n  two\r\nthree ")), "one two three");
        assert_eq!(snippet_of(None), "");
    }

    #[test]
    fn a_nul_in_an_id_is_refused_before_it_reaches_the_syscall() {
        assert!(!safe_id("cur/mail\0.txt"));
    }
}
